"""Heurísticas sem IA para condensar logs de agentes de código em um resumo curto.

Não conhece o formato exato de nenhuma ferramenta: percorre qualquer JSON,
procura sinais genéricos (erros, comandos repetidos, correções do usuário,
--no-verify, segredos) e devolve só contagens e trechos curtos já mascarados.
"""

import json
import re
from collections import Counter
from dataclasses import dataclass, field

SNIPPET = 200

ERROR_RE = re.compile(
    r"(?i)(traceback \(most recent|(?<![A-Za-z_])(?<!except )(?:[A-Za-z]*Error|[A-Za-z]*Exception|error|fatal):|"
    r"command not found|permission denied|exit code [1-9]|(?<!\b0 )\bfailed\b|\bkilled\b|timed out)"
)
CORRECTION_RE = re.compile(
    r"(?i)(\b(errado|de novo|já falei|ja falei|não era isso|nao era isso|como eu disse|isso não|"
    r"wrong|not what i|stop (doing|that|it|this)|revert|undo|desfaz|i said|that'?s not)\b|"
    r"não funcion|nao funcion|continua (dando|com|errado|quebrad)|ainda (não|nao|dá|da|está|esta)\b|"
    r"still (fail|not|broken|wrong|happen)|(doesn'?t|didn'?t|not) work|"
    r"merda|porra|caralho|\bdroga\b|\bwtf\b|\bdamn\b|\bfuck|\bshit)"
)
SECRET_RES = [
    re.compile(r"(?i)bearer\s+[a-z0-9._\-]{16,}"),
    re.compile(r"\beyJ[a-zA-Z0-9_\-]{10,}\.[a-zA-Z0-9_\-]{10,}\.[a-zA-Z0-9_\-]*"),
    re.compile(r"\b(sk|ghp|gho|xox[bp])[-_][a-zA-Z0-9\-_]{16,}"),
    re.compile(r"(?i)(password|senha|passwd|secret|api[_-]?key|token)\s*[=:]\s*\S{6,}"),
    re.compile(r"\b(AKIA|ASIA)[A-Z0-9]{16}\b"),
    re.compile(r"-----BEGIN [A-Z ]*PRIVATE KEY-----"),
    re.compile(r"(?i)\b[a-z][a-z0-9+.\-]*://[^\s:/@]+:[^\s@/]{4,}@\S*"),
]
IP_RE = re.compile(r"\b\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}\b")
USER_ROLES = {"user", "human", "user_message"}
ASSISTANT_ROLES = {"assistant", "agent", "agent_message"}


EXIT_KEYS = {"exit_code", "exitcode", "returncode", "return_code"}


def deep_parse(obj, depth=0):
    """Abre strings que contêm JSON (ex.: argumentos de chamadas de ferramenta)."""
    if depth > 6:
        return obj
    if isinstance(obj, str):
        t = obj.lstrip()
        if t[:1] in "{[" and 2 < len(t) < 50_000:
            try:
                return deep_parse(json.loads(t), depth + 1)
            except ValueError:
                return obj
        return obj
    if isinstance(obj, dict):
        return {k: deep_parse(v, depth + 1) for k, v in obj.items()}
    if isinstance(obj, list):
        return [deep_parse(v, depth + 1) for v in obj]
    return obj


def find_exit_codes(obj, depth=0):
    if depth > 8:
        return
    if isinstance(obj, dict):
        for k, v in obj.items():
            if str(k).lower() in EXIT_KEYS and isinstance(v, int) and not isinstance(v, bool) and v != 0:
                yield v
            else:
                yield from find_exit_codes(v, depth + 1)
    elif isinstance(obj, list):
        for v in obj:
            yield from find_exit_codes(v, depth + 1)


EDIT_TOOL_RE = re.compile(r"(?i)edit|write|patch|replace|apply")
PATCH_RE = re.compile(r"\*\*\* (?:Update|Add) File:\s*(\S[^\n]*)")
THRASH_MIN = 4


def find_edits(rec, depth=0):
    """Caminhos de arquivos editados por chamadas de ferramenta (edição direta ou patch em texto)."""
    if depth > 8:
        return
    if isinstance(rec, dict):
        name, inp = rec.get("name"), rec.get("input") or rec.get("arguments")
        if isinstance(name, str) and EDIT_TOOL_RE.search(name) and isinstance(inp, dict):
            for k in ("file_path", "path", "filepath", "file"):
                if isinstance(inp.get(k), str):
                    yield inp[k]
                    break
        for v in rec.values():
            yield from find_edits(v, depth + 1)
    elif isinstance(rec, list):
        for v in rec:
            yield from find_edits(v, depth + 1)
    elif isinstance(rec, str) and "*** " in rec:
        for m in PATCH_RE.finditer(rec):
            yield m.group(1).strip()


def short_path(p):
    parts = [x for x in re.split(r"[\\/]+", p) if x]
    return redact("/".join(parts[-3:]))


def has_secret(text):
    return any(r.search(text) for r in SECRET_RES)


def redact(text):
    for r in SECRET_RES:
        text = r.sub("[SEGREDO]", text)
    return IP_RE.sub("[IP]", text)


LAST_ERROR_RE = re.compile(r"[A-Za-z_.]*(?:Error|Exception)\b[^\n]*")


def signature(text):
    """Assinatura estável de um erro: troca números, ids e caminhos para agrupar repetições.

    Em tracebacks usa a última linha de exceção, que é a que identifica o erro.
    """
    if "Traceback (most recent" in text:
        last = LAST_ERROR_RE.findall(text)
        if last:
            text = last[-1]
    t = " ".join(text.split()).lower()[:400]
    t = re.sub(r"[0-9a-f]{8}-[0-9a-f-]{27}|0x[0-9a-f]+|\d+", "N", t)
    t = re.sub(r"([a-z]:)?[\\/][\w.\-~\\/ ]{3,}", "<path>", t)
    return t.strip()[:100]


WRAPPER_RE = re.compile(r"^(?:(?:bash|sh|zsh)\s+-l?c\s+|cmd(?:\.exe)?\s+/c\s+|powershell(?:\.exe)?\s+(?:-\w+\s+)*-command\s+)", re.I)
CD_RE = re.compile(r"^cd\s+(?:\"[^\"]+\"|'[^']+'|\S+)\s*&&\s*")


def norm_cmd(cmd):
    """Tira invólucros de shell e `cd ... &&` para agrupar o mesmo comando em sessões diferentes."""
    cmd = " ".join(cmd.split())
    for _ in range(3):
        new = CD_RE.sub("", WRAPPER_RE.sub("", cmd)).strip("\"' ")
        if new == cmd:
            break
        cmd = new
    return cmd


def sample_around(s, pos):
    """Trecho em volta de `pos`, mascarado antes de cortar para não deixar pedaço de segredo."""
    w = redact(" ".join(s[max(0, pos - 400):pos + 400].split()))
    m = ERROR_RE.search(w)
    return (w[max(0, m.start() - 60):] if m else w)[:SNIPPET]


def snippet(text):
    return redact(" ".join(text.split()))[:SNIPPET]


def iter_strings(obj, key="", depth=0):
    if depth > 8:
        return
    if isinstance(obj, str):
        yield key, obj
    elif isinstance(obj, dict):
        for k, v in obj.items():
            yield from iter_strings(v, str(k), depth + 1)
    elif isinstance(obj, list):
        for v in obj:
            yield from iter_strings(v, key, depth + 1)


def find_role(rec):
    if not isinstance(rec, dict):
        return None
    for holder in (rec, rec.get("message"), rec.get("payload")):
        if isinstance(holder, dict):
            for k in ("role", "type"):
                v = holder.get(k)
                if isinstance(v, str) and v.lower() in USER_ROLES | ASSISTANT_ROLES:
                    return v.lower()
    for k in rec:
        if k in USER_ROLES | ASSISTANT_ROLES:
            return k
    return None


def find_commands(rec, depth=0):
    if depth > 8:
        return
    if isinstance(rec, dict):
        for k, v in rec.items():
            if k in ("command", "cmd") and isinstance(v, (str, list)):
                yield v if isinstance(v, str) else " ".join(map(str, v))
            else:
                yield from find_commands(v, depth + 1)
    elif isinstance(rec, list):
        for v in rec:
            yield from find_commands(v, depth + 1)


def find_models(rec, depth=0):
    if depth > 8:
        return
    if isinstance(rec, dict):
        for k, v in rec.items():
            if k == "model" and isinstance(v, str):
                yield v
            else:
                yield from find_models(v, depth + 1)
    elif isinstance(rec, list):
        for v in rec:
            yield from find_models(v, depth + 1)


@dataclass
class Stats:
    records: int = 0
    models: Counter = field(default_factory=Counter)
    commands: Counter = field(default_factory=Counter)
    errors: int = 0
    error_samples: list = field(default_factory=list)
    no_verify: int = 0
    secrets: int = 0
    corrections: list = field(default_factory=list)
    ends_with_question: bool = False
    error_sigs: set = field(default_factory=set)
    edits: Counter = field(default_factory=Counter)

    def merge(self, other):
        self.records += other.records
        self.models.update(other.models)
        self.commands.update(other.commands)
        self.errors += other.errors
        self.error_samples += other.error_samples
        self.no_verify += other.no_verify
        self.secrets += other.secrets
        self.corrections += other.corrections
        self.ends_with_question = self.ends_with_question or other.ends_with_question
        self.error_sigs |= other.error_sigs
        self.edits.update(other.edits)


def scan_records(records, max_samples=5):
    st = Stats()
    last_assistant = ""
    seen_sigs = set()
    for rec in records:
        st.records += 1
        rec = deep_parse(rec)
        role = find_role(rec)
        for code in find_exit_codes(rec):
            st.errors += 1
            st.error_sigs.add(f"exit code {code}")
        st.models.update(find_models(rec))
        st.edits.update(short_path(p) for p in set(find_edits(rec)))
        for cmd in find_commands(rec):
            st.commands[snippet(norm_cmd(cmd))[:120]] += 1
            if "--no-verify" in cmd:
                st.no_verify += 1
        for _, s in iter_strings(rec):
            if has_secret(s):
                st.secrets += 1
            if ERROR_RE.search(s):
                st.errors += 1
                m = ERROR_RE.search(s)
                sig = signature(s[m.start():m.start() + 4000])
                st.error_sigs.add(sig)
                if len(st.error_samples) < max_samples and sig not in seen_sigs:
                    seen_sigs.add(sig)
                    st.error_samples.append(sample_around(s, m.start()))
            if role in USER_ROLES and CORRECTION_RE.search(s) and len(s) < 400:
                if len(st.corrections) < max_samples:
                    st.corrections.append(snippet(s))
            if role in ASSISTANT_ROLES and len(s) >= 10 and " " in s:
                last_assistant = s
    st.ends_with_question = last_assistant.strip().endswith("?")
    return st


def read_jsonl(path):
    with open(path, encoding="utf-8", errors="replace") as fh:
        for line in fh:
            line = line.strip()
            if line:
                try:
                    yield json.loads(line)
                except ValueError:
                    continue


def load_dump(path):
    """Lê um JSON solto ou a resposta salva de uma ferramenta (com prefixo antes do JSON)."""
    with open(path, encoding="utf-8", errors="replace") as fh:
        text = fh.read()
    start = text.find('{"ccr"')
    if start < 0:
        start = min((i for i in (text.find("{"), text.find("[")) if i >= 0), default=-1)
    if start < 0:
        return []
    data, _ = json.JSONDecoder().raw_decode(text[start:])
    if isinstance(data, dict) and "ccr" in data:
        data = data["ccr"].get("data", [])
    return data if isinstance(data, list) else [data]


def read_records(path):
    return read_jsonl(path) if str(path).endswith(".jsonl") else iter(load_dump(path))


def render(name, st, repeat_min=3):
    lines = [f"### {name}", f"- registros: {st.records}"]
    if st.models:
        lines.append("- modelos: " + ", ".join(f"{m} ({n})" for m, n in st.models.most_common(3)))
    lines.append(f"- sinais de erro: {st.errors}; --no-verify: {st.no_verify}; segredos em texto: {st.secrets}")
    if st.ends_with_question:
        lines.append("- terminou com uma pergunta do assistente (provável espera por resposta)")
    rep = [(c, n) for c, n in st.commands.most_common(5) if n >= repeat_min]
    if rep:
        lines.append("- comandos repetidos: " + "; ".join(f"`{c}` x{n}" for c, n in rep))
    thrash = thrashed(st)
    if thrash:
        lines.append("- arquivos editados várias vezes na sessão: " + "; ".join(f"`{p}` x{n}" for p, n in thrash[:3]))
    for s in st.error_samples[:3]:
        lines.append(f"  - erro: {s}")
    for s in st.corrections[:3]:
        lines.append(f"  - correção do usuário: {s}")
    return "\n".join(lines)


def thrashed(st):
    return [(p, n) for p, n in st.edits.most_common(5) if n >= THRASH_MIN]


def score(st):
    """Pontuação de gravidade de uma sessão: decide quais abrir inteiras (filtro barato antes do caro)."""
    return (min(st.errors, 10) + 3 * len(st.corrections) + 2 * (st.no_verify > 0)
            + 2 * st.ends_with_question + 2 * len(thrashed(st)) + 3 * (st.secrets > 0))


CATEGORIES = (
    ("pergunta", "terminaram com pergunta do assistente", lambda st: st.ends_with_question),
    ("no_verify", "usaram --no-verify", lambda st: st.no_verify > 0),
    ("correcao", "tiveram correção ou irritação do usuário", lambda st: bool(st.corrections)),
    ("segredos", "tiveram segredos em texto", lambda st: st.secrets > 0),
    ("edicoes", "tiveram um arquivo editado 4 vezes ou mais", lambda st: bool(thrashed(st))),
)


def summary(per_file):
    """Contagens da semana, para guardar e comparar com a próxima."""
    sigs, hot = Counter(), Counter()
    for _, st in per_file:
        sigs.update(st.error_sigs)
        hot.update(p for p, _ in thrashed(st))
    return {
        "arquivos": len(per_file),
        "categorias": {k: sum(1 for _, st in per_file if pred(st)) for k, _, pred in CATEGORIES},
        "erros": {sig[:100]: k for sig, k in sigs.most_common(20) if k >= 2},
        "arquivos_quentes": dict(hot.most_common(10)),
    }


def compare(old, new):
    """Diferença entre a semana anterior e a atual: o que continua, o que é novo e o que sumiu."""
    lines = ["## Comparação com a semana anterior"]
    labels = {k: label for k, label, _ in CATEGORIES}
    for k, label in labels.items():
        a, b = old.get("categorias", {}).get(k, 0), new["categorias"].get(k, 0)
        if a or b:
            lines.append(f"- {label}: {a} de {old.get('arquivos', 0)} antes, {b} de {new['arquivos']} agora")
    for title, key in (("erro", "erros"), ("arquivo muito editado", "arquivos_quentes")):
        before, now = set(old.get(key, {})), set(new[key])
        for item in sorted(now - before):
            lines.append(f"- novo ({title}): {item}")
        for item in sorted(now & before):
            lines.append(f"- continua ({title}): {item}")
        for item in sorted(before - now):
            lines.append(f"- sumiu ({title}): {item}")
    return "\n".join(lines) if len(lines) > 1 else ""


def render_cross(per_file, min_files=2):
    """Padrões que se repetem em vários arquivos (sessões): é o que vira sugestão de processo."""
    n = len(per_file)
    lines = [f"## Entre os {n} arquivos"]
    sm = summary(per_file)
    for k, label, _ in CATEGORIES:
        if sm["categorias"][k]:
            lines.append(f"- {sm['categorias'][k]} de {n} {label}")
    for sig, k in sm["erros"].items():
        lines.append(f"- erro recorrente em {k} arquivos: {sig}")
    for path, k in sm["arquivos_quentes"].items():
        if k >= min_files:
            lines.append(f"- arquivo muito editado em {k} sessões: `{path}`")
    cmds = Counter()
    for _, st in per_file:
        cmds.update(c for c, k in st.commands.items() if k >= 3)
    for cmd, k in cmds.most_common(5):
        if k >= min_files:
            lines.append(f"- comando repetido 3 vezes ou mais na mesma sessão, em {k} arquivos: `{cmd[:100]}`")
    ranked = sorted(((score(st), name) for name, st in per_file), reverse=True)[:3]
    ranked = [f"{name} ({sc})" for sc, name in ranked if sc >= 5]
    if ranked and len(lines) > 1:
        lines.append("- para leitura completa (maior pontuação): " + ", ".join(ranked))
    return "\n".join(lines) if len(lines) > 1 else ""
