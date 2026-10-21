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
    r"command not found|permission denied|exit code [1-9]|\bfailed\b|\bkilled\b|timed out)"
)
CORRECTION_RE = re.compile(
    r"(?i)\b(errado|de novo|já falei|ja falei|não era isso|nao era isso|"
    r"wrong|not what i|stop|revert|undo|desfaz)\b"
)
SECRET_RES = [
    re.compile(r"(?i)bearer\s+[a-z0-9._\-]{16,}"),
    re.compile(r"\beyJ[a-zA-Z0-9_\-]{10,}\.[a-zA-Z0-9_\-]{10,}\.[a-zA-Z0-9_\-]*"),
    re.compile(r"\b(sk|ghp|gho|xox[bp])[-_][a-zA-Z0-9\-_]{16,}"),
    re.compile(r"(?i)(password|senha|passwd|secret|api[_-]?key|token)\s*[=:]\s*\S{6,}"),
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


def has_secret(text):
    return any(r.search(text) for r in SECRET_RES)


def redact(text):
    for r in SECRET_RES:
        text = r.sub("[SEGREDO]", text)
    return IP_RE.sub("[IP]", text)


def signature(text):
    """Assinatura estável de um erro: troca números, ids e caminhos para agrupar repetições."""
    t = " ".join(text.split()).lower()[:100]
    t = re.sub(r"[0-9a-f]{8}-[0-9a-f-]{27}|0x[0-9a-f]+|\d+", "N", t)
    t = re.sub(r"([a-z]:)?[\\/][\w.\-~\\/ ]{3,}", "<path>", t)
    return t.strip()


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
                sig = signature(s[m.start():m.start() + 100])
                st.error_sigs.add(sig)
                if len(st.error_samples) < max_samples and sig not in seen_sigs:
                    seen_sigs.add(sig)
                    st.error_samples.append(snippet(s[max(0, m.start() - 60):]))
            if role in USER_ROLES and CORRECTION_RE.search(s) and len(s) < 400:
                if len(st.corrections) < max_samples:
                    st.corrections.append(snippet(s))
            if role in ASSISTANT_ROLES and len(s) > 20:
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
    for s in st.error_samples[:3]:
        lines.append(f"  - erro: {s}")
    for s in st.corrections[:3]:
        lines.append(f"  - correção do usuário: {s}")
    return "\n".join(lines)


def render_cross(per_file, min_files=2):
    """Padrões que se repetem em vários arquivos (sessões): é o que vira sugestão de processo."""
    n = len(per_file)
    lines = [f"## Entre os {n} arquivos"]
    for label, pred in (
        ("terminaram com pergunta do assistente", lambda st: st.ends_with_question),
        ("usaram --no-verify", lambda st: st.no_verify > 0),
        ("tiveram correção ou irritação do usuário", lambda st: bool(st.corrections)),
        ("tiveram segredos em texto", lambda st: st.secrets > 0),
    ):
        k = sum(1 for _, st in per_file if pred(st))
        if k:
            lines.append(f"- {k} de {n} {label}")
    sigs, cmds = Counter(), Counter()
    for _, st in per_file:
        sigs.update(st.error_sigs)
        cmds.update(c for c, k in st.commands.items() if k >= 3)
    for sig, k in sigs.most_common(5):
        if k >= min_files:
            lines.append(f"- erro recorrente em {k} arquivos: {sig[:100]}")
    for cmd, k in cmds.most_common(5):
        if k >= min_files:
            lines.append(f"- comando repetido 3 vezes ou mais na mesma sessão, em {k} arquivos: `{cmd[:100]}`")
    return "\n".join(lines) if len(lines) > 1 else ""
