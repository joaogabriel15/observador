//! Heurísticas sem IA para condensar logs de agentes de código em um resumo curto.
//!
//! Não conhece o formato exato de nenhuma ferramenta: percorre qualquer JSON, procura sinais
//! genéricos (erros, comandos repetidos, correções do usuário, --no-verify, segredos) e devolve só
//! contagens e trechos curtos já mascarados. Port fiel de `observador/extract.py`.

use crate::json::{self, Value};
use crate::regex::{is_space, Regex};
use std::collections::HashMap;
use std::sync::OnceLock;

const SNIPPET: usize = 200;
pub const THRASH_MIN: i64 = 4;
const COMPACT_START: &str = "this session is being continued from a previous conversation";
const USER_ROLES: [&str; 3] = ["user", "human", "user_message"];
const ASSISTANT_ROLES: [&str; 3] = ["assistant", "agent", "agent_message"];
const EXIT_KEYS: [&str; 4] = ["exit_code", "exitcode", "returncode", "return_code"];
const TOKEN_KEYS: [&str; 2] = ["input_tokens", "output_tokens"];
const TOOL_RESULT_TYPES: [&str; 4] = ["tool_result", "function_call_output", "tool_output", "tool"];

struct Res {
    error: Regex,
    correction: Regex,
    secrets: Vec<Regex>,
    ip: Regex,
    claim: Regex,
    env: Regex,
    http: Regex,
    test: Regex,
    edit_tool: Regex,
    patch: Regex,
    last_error: Regex,
    sig_ids: Regex,
    sig_path: Regex,
    wrapper: Regex,
    cd: Regex,
    rule_line: Regex,
    backticked: Regex,
}

fn res() -> &'static Res {
    static R: OnceLock<Res> = OnceLock::new();
    R.get_or_init(|| Res {
        error: Regex::new(
            r"(?i)(traceback \(most recent|(?<![A-Za-z_])(?<!except )(?:[A-Za-z]*Error|[A-Za-z]*Exception|error|fatal):|command not found|permission denied|exit code [1-9]|\bhttp[ /]*[45]\d\d\b|(?<!\b0 )\bfailed\b|\bkilled\b|timed out)",
        ),
        correction: Regex::new(
            r"(?i)(\b(errado|de novo|já falei|ja falei|não era isso|nao era isso|como eu disse|isso não|wrong|not what i|stop (doing|that|it|this)|revert|undo|desfaz|i said|that'?s not)\b|não funcion|nao funcion|continua (dando|com|errado|quebrad)|ainda (não|nao|dá|da|está|esta)\b|still (fail|not|broken|wrong|happen)|(doesn'?t|didn'?t|not) work|merda|porra|caralho|\bdroga\b|\bwtf\b|\bdamn\b|\bfuck|\bshit)",
        ),
        secrets: vec![
            Regex::new(r"(?i)bearer\s+[a-z0-9._\-]{16,}"),
            Regex::new(r"\beyJ[a-zA-Z0-9_\-]{10,}\.[a-zA-Z0-9_\-]{10,}\.[a-zA-Z0-9_\-]*"),
            Regex::new(r"\b(sk|ghp|gho|xox[bp])[-_][a-zA-Z0-9\-_]{16,}"),
            Regex::new(r"(?i)(password|senha|passwd|secret|api[_-]?key|token)\s*[=:]\s*\S{6,}"),
            Regex::new(r"\b(AKIA|ASIA)[A-Z0-9]{16}\b"),
            Regex::new(r"-----BEGIN [A-Z ]*PRIVATE KEY-----"),
            Regex::new(r"(?i)\b[a-z][a-z0-9+.\-]*://[^\s:/@]+:[^\s@/]{4,}@\S*"),
        ],
        ip: Regex::new(r"\b\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}\b"),
        claim: Regex::new(
            r"(?i)\b(pronto|feito|conclu[ií]d[oa]|corrigi|resolvi|implementei|tudo (certo|ok|verde)|done|fixed|finished|implemented|all tests pass(ed)?)\b",
        ),
        env: Regex::new(
            r"(?i)command not found|permission denied|no such file|enoent|econn|timed? ?out|timeout|killed|exit code 137|proxy|ssl|certificate|eacces|disk|memory|connection (refused|reset)",
        ),
        http: Regex::new(r"(?i)http[ /]*[45]\d\d|status.?code.?[45]\d\d|\b[45]\d\d (error|bad|not found|internal)"),
        test: Regex::new(r"(?i)assert|pytest|jest|vitest|test.*fail|fail.*test|expected .* (got|received)|lint|ruff|eslint|mypy"),
        edit_tool: Regex::new(r"(?i)edit|write|patch|replace|apply"),
        patch: Regex::new(r"\*\*\* (?:Update|Add) File:\s*(\S[^\n]*)"),
        last_error: Regex::new(r"[A-Za-z_.]*(?:Error|Exception)\b[^\n]*"),
        sig_ids: Regex::new(r"[0-9a-f]{8}-[0-9a-f-]{27}|0x[0-9a-f]+|\d+"),
        sig_path: Regex::new(r"([a-z]:)?[\\/][\w.\-~\\/ ]{3,}"),
        wrapper: Regex::new(
            r"(?i)^(?:(?:bash|sh|zsh)\s+-l?c\s+|cmd(?:\.exe)?\s+/c\s+|powershell(?:\.exe)?\s+(?:-\w+\s+)*-command\s+)",
        ),
        cd: Regex::new(r#"^cd\s+(?:"[^"]+"|'[^']+'|\S+)\s*&&\s*"#),
        rule_line: Regex::new(r"(?i)\b(nunca|never|não use|nao use|don'?t|do not|proibido|evite|avoid)\b"),
        backticked: Regex::new(r"`([^`]{3,60})`"),
    })
}

// ---------- utilidades de texto no estilo do Python ----------

pub fn chars(s: &str) -> Vec<char> {
    s.chars().collect()
}

pub fn slice(s: &[char], a: usize, b: usize) -> String {
    let b = b.min(s.len());
    if a >= b {
        String::new()
    } else {
        s[a..b].iter().collect()
    }
}

/// `s[:n]`
pub fn head(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// `" ".join(s.split())`
pub fn squash(s: &str) -> String {
    s.split(is_space).filter(|p| !p.is_empty()).collect::<Vec<_>>().join(" ")
}

pub fn strip(s: &str) -> &str {
    s.trim_matches(is_space)
}

// ---------- contador que lembra a ordem de inserção (como o Counter do Python) ----------

#[derive(Default, Clone, Debug)]
pub struct Counter {
    keys: Vec<String>,
    map: HashMap<String, i64>,
}

impl Counter {
    pub fn add(&mut self, key: &str, n: i64) {
        match self.map.get_mut(key) {
            Some(v) => *v += n,
            None => {
                self.keys.push(key.to_string());
                self.map.insert(key.to_string(), n);
            }
        }
    }

    pub fn insert_once(&mut self, key: &str) {
        if !self.map.contains_key(key) {
            self.add(key, 1);
        }
    }

    #[cfg(test)]
    pub fn get(&self, key: &str) -> i64 {
        self.map.get(key).copied().unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    pub fn items(&self) -> impl Iterator<Item = (&str, i64)> {
        self.keys.iter().map(move |k| (k.as_str(), self.map[k]))
    }

    /// `dict(counter)` do Python: ordem de inserção, não de contagem.
    pub fn in_order(&self) -> Vec<(String, i64)> {
        self.items().map(|(k, n)| (k.to_string(), n)).collect()
    }

    pub fn update(&mut self, other: &Counter) {
        for (k, n) in other.items() {
            self.add(k, n);
        }
    }

    pub fn update_keys<'a>(&mut self, keys: impl Iterator<Item = &'a str>) {
        for k in keys {
            self.add(k, 1);
        }
    }

    /// Maiores primeiro; empates na ordem de inserção.
    pub fn most_common(&self, n: usize) -> Vec<(String, i64)> {
        let mut v: Vec<(String, i64)> = self.items().map(|(k, c)| (k.to_string(), c)).collect();
        v.sort_by(|a, b| b.1.cmp(&a.1));
        v.truncate(n);
        v
    }
}

// ---------- navegação no JSON ----------

pub fn deep_parse(v: &Value, depth: usize) -> Value {
    if depth > 6 {
        return v.clone();
    }
    match v {
        Value::Str(s) => {
            let t = s.trim_start_matches(is_space);
            let n = t.chars().count();
            if (t.starts_with('{') || t.starts_with('[')) && n > 2 && n < 50_000 {
                if let Some(parsed) = json::parse(t) {
                    return deep_parse(&parsed, depth + 1);
                }
            }
            v.clone()
        }
        Value::Obj(items) => Value::Obj(items.iter().map(|(k, x)| (k.clone(), deep_parse(x, depth + 1))).collect()),
        Value::Arr(items) => Value::Arr(items.iter().map(|x| deep_parse(x, depth + 1)).collect()),
        _ => v.clone(),
    }
}

fn find_exit_codes(v: &Value, depth: usize, out: &mut Vec<i64>) {
    if depth > 8 {
        return;
    }
    match v {
        Value::Obj(items) => {
            for (k, x) in items {
                match x {
                    Value::Int(code) if EXIT_KEYS.contains(&k.to_lowercase().as_str()) && *code != 0 => out.push(*code),
                    _ => find_exit_codes(x, depth + 1, out),
                }
            }
        }
        Value::Arr(items) => items.iter().for_each(|x| find_exit_codes(x, depth + 1, out)),
        _ => {}
    }
}

fn has_key(v: &Value, names: &[&str], depth: usize) -> bool {
    if depth > 8 {
        return false;
    }
    match v {
        Value::Obj(items) => items.iter().any(|(k, x)| names.contains(&k.as_str()) || has_key(x, names, depth + 1)),
        Value::Arr(items) => items.iter().any(|x| has_key(x, names, depth + 1)),
        _ => false,
    }
}

fn count_tokens(v: &Value, depth: usize) -> i64 {
    if depth > 8 {
        return 0;
    }
    match v {
        Value::Obj(items) => items
            .iter()
            .map(|(k, x)| match x {
                Value::Int(n) if TOKEN_KEYS.contains(&k.as_str()) => *n,
                _ => count_tokens(x, depth + 1),
            })
            .fold(0i64, |a, b| a.saturating_add(b)),
        Value::Arr(items) => items.iter().map(|x| count_tokens(x, depth + 1)).fold(0i64, |a, b| a.saturating_add(b)),
        _ => 0,
    }
}

pub type Strings<'a> = Vec<(&'a str, &'a str)>;

fn collect_strings<'a>(v: &'a Value, key: &'a str, depth: usize, out: &mut Strings<'a>) {
    if depth > 8 {
        return;
    }
    match v {
        Value::Str(s) => out.push((key, s.as_str())),
        Value::Obj(items) => items.iter().for_each(|(k, x)| collect_strings(x, k.as_str(), depth + 1, out)),
        Value::Arr(items) => items.iter().for_each(|x| collect_strings(x, key, depth + 1, out)),
        _ => {}
    }
}

pub fn iter_strings(v: &Value) -> Strings<'_> {
    let mut out = Vec::new();
    collect_strings(v, "", 0, &mut out);
    out
}

fn is_compaction(rec: &Value, strings: &Strings) -> bool {
    if has_key(rec, &["isCompactSummary", "is_compact_summary"], 0) {
        return true;
    }
    if strings.iter().any(|(_, v)| head(v, 80).to_lowercase().starts_with(COMPACT_START)) {
        return true;
    }
    matches!(rec, Value::Obj(_))
        && ["type", "subtype"]
            .iter()
            .any(|k| rec.get(k).and_then(Value::as_str).map_or(false, |s| s.to_lowercase().contains("compact")))
}

fn lower_str(v: Option<&Value>) -> String {
    match v {
        Some(Value::Str(s)) => s.to_lowercase(),
        Some(other) => other.py_str().to_lowercase(),
        None => String::new(),
    }
}

pub fn is_tool_result(rec: &Value, depth: usize) -> bool {
    if depth > 8 {
        return false;
    }
    match rec {
        Value::Obj(items) => {
            if rec.get("tool_use_id").is_some()
                || TOOL_RESULT_TYPES.contains(&lower_str(rec.get("type")).as_str())
                || lower_str(rec.get("role")) == "tool"
            {
                return true;
            }
            items.iter().any(|(_, v)| is_tool_result(v, depth + 1))
        }
        Value::Arr(items) => items.iter().any(|v| is_tool_result(v, depth + 1)),
        _ => false,
    }
}

/// Texto digitado pelo usuário (ignora resultados de ferramentas, que também chegam como "user").
fn user_texts<'a>(rec: &Value, strings: &Strings<'a>) -> Vec<&'a str> {
    if is_tool_result(rec, 0) {
        return vec![];
    }
    strings
        .iter()
        .filter(|(k, v)| (*k == "content" || *k == "text") && (15..=400).contains(&v.chars().count()))
        .map(|(_, v)| *v)
        .collect()
}

fn find_edits(rec: &Value, depth: usize, out: &mut Vec<String>) {
    if depth > 8 {
        return;
    }
    match rec {
        Value::Obj(items) => {
            let name = rec.get("name");
            let input = match rec.get("input") {
                Some(v) if v.truthy() => Some(v),
                _ => rec.get("arguments"),
            };
            if let (Some(Value::Str(n)), Some(inp @ Value::Obj(_))) = (name, input) {
                if res().edit_tool.is_match_str(n) {
                    for k in ["file_path", "path", "filepath", "file"] {
                        if let Some(Value::Str(p)) = inp.get(k) {
                            out.push(p.clone());
                            break;
                        }
                    }
                }
            }
            items.iter().for_each(|(_, v)| find_edits(v, depth + 1, out));
        }
        Value::Arr(items) => items.iter().for_each(|v| find_edits(v, depth + 1, out)),
        Value::Str(s) if s.contains("*** ") => {
            let c = chars(s);
            for m in res().patch.find_all(&c) {
                if let Some((a, b)) = m.group(1) {
                    out.push(strip(&slice(&c, a, b)).to_string());
                }
            }
        }
        _ => {}
    }
}

fn short_path(p: &str) -> String {
    let parts: Vec<&str> = p.split(|c| c == '\\' || c == '/').filter(|x| !x.is_empty()).collect();
    let tail = &parts[parts.len().saturating_sub(3)..];
    redact(&tail.join("/"))
}

pub fn has_secret(text: &[char]) -> bool {
    res().secrets.iter().any(|r| r.is_match(text))
}

pub fn redact(text: &str) -> String {
    let mut t = text.to_string();
    for r in &res().secrets {
        t = r.replace_all(&t, "[SEGREDO]");
    }
    res().ip.replace_all(&t, "[IP]")
}

/// Assinatura estável de um erro: troca números, ids e caminhos para agrupar repetições.
/// Em tracebacks usa a última linha de exceção, que é a que identifica o erro.
pub fn signature(text: &str) -> String {
    let mut text = text.to_string();
    if text.contains("Traceback (most recent") {
        let c = chars(&text);
        if let Some(m) = res().last_error.find_all(&c).last() {
            text = slice(&c, m.start, m.end);
        }
    }
    let t = head(&squash(&text).to_lowercase(), 400);
    let t = res().sig_ids.replace_all(&t, "N");
    let t = res().sig_path.replace_all(&t, "<path>");
    head(strip(&t), 100)
}

/// Tira invólucros de shell e `cd ... &&` para agrupar o mesmo comando em sessões diferentes.
pub fn norm_cmd(cmd: &str) -> String {
    let mut cmd = squash(cmd);
    for _ in 0..3 {
        let a = res().wrapper.replace_all(&cmd, "");
        let b = res().cd.replace_all(&a, "");
        let new = strip(&b).trim_matches(|c| c == '"' || c == '\'' || c == ' ').to_string();
        if new == cmd {
            break;
        }
        cmd = new;
    }
    cmd
}

/// Trecho em volta de `pos`, mascarado antes de cortar para não deixar pedaço de segredo.
fn sample_around(s: &[char], pos: usize) -> String {
    let w = redact(&squash(&slice(s, pos.saturating_sub(400), pos + 400)));
    let wc = chars(&w);
    match res().error.find(&wc) {
        Some(m) => head(&slice(&wc, m.start.saturating_sub(60), wc.len()), SNIPPET),
        None => head(&w, SNIPPET),
    }
}

fn snippet(text: &str) -> String {
    head(&redact(&squash(text)), SNIPPET)
}

fn role_of(rec: &Value) -> Option<String> {
    let Value::Obj(items) = rec else { return None };
    let all = |s: &str| USER_ROLES.contains(&s) || ASSISTANT_ROLES.contains(&s);
    for holder in [Some(rec), rec.get("message"), rec.get("payload")].into_iter().flatten() {
        if matches!(holder, Value::Obj(_)) {
            for k in ["role", "type"] {
                if let Some(Value::Str(v)) = holder.get(k) {
                    let l = v.to_lowercase();
                    if all(&l) {
                        return Some(l);
                    }
                }
            }
        }
    }
    items.iter().map(|(k, _)| k.as_str()).find(|k| all(k)).map(str::to_string)
}

fn find_commands(rec: &Value, depth: usize, out: &mut Vec<String>) {
    if depth > 8 {
        return;
    }
    match rec {
        Value::Obj(items) => {
            for (k, v) in items {
                match v {
                    Value::Str(s) if k == "command" || k == "cmd" => out.push(s.clone()),
                    Value::Arr(a) if k == "command" || k == "cmd" => {
                        out.push(a.iter().map(Value::py_str).collect::<Vec<_>>().join(" "))
                    }
                    _ => find_commands(v, depth + 1, out),
                }
            }
        }
        Value::Arr(items) => items.iter().for_each(|v| find_commands(v, depth + 1, out)),
        _ => {}
    }
}

fn find_models(rec: &Value, depth: usize, out: &mut Vec<String>) {
    if depth > 8 {
        return;
    }
    match rec {
        Value::Obj(items) => {
            for (k, v) in items {
                match v {
                    Value::Str(s) if k == "model" => out.push(s.clone()),
                    _ => find_models(v, depth + 1, out),
                }
            }
        }
        Value::Arr(items) => items.iter().for_each(|v| find_models(v, depth + 1, out)),
        _ => {}
    }
}

pub fn error_kind(sig: &str) -> &'static str {
    let r = res();
    if r.env.is_match_str(sig) {
        "ambiente"
    } else if r.http.is_match_str(sig) {
        "http"
    } else if r.test.is_match_str(sig) {
        "teste"
    } else {
        "codigo"
    }
}

// ---------- varredura ----------

#[derive(Default, Clone, Debug)]
pub struct Stats {
    pub records: i64,
    pub models: Counter,
    pub commands: Counter,
    pub errors: i64,
    pub error_samples: Vec<String>,
    pub no_verify: i64,
    pub secrets: i64,
    pub corrections: Vec<String>,
    pub ends_with_question: bool,
    pub error_sigs: Counter,
    pub edits: Counter,
    pub tokens: i64,
    pub compactions: i64,
    pub repeats: Vec<String>,
    pub contested: Vec<String>,
    pub err_kinds: Counter,
}

impl Stats {
    pub fn merge(&mut self, o: &Stats) {
        self.records += o.records;
        self.models.update(&o.models);
        self.commands.update(&o.commands);
        self.errors += o.errors;
        self.error_samples.extend(o.error_samples.iter().cloned());
        self.no_verify += o.no_verify;
        self.secrets += o.secrets;
        self.corrections.extend(o.corrections.iter().cloned());
        self.ends_with_question = self.ends_with_question || o.ends_with_question;
        for (k, _) in o.error_sigs.items() {
            self.error_sigs.insert_once(k);
        }
        self.edits.update(&o.edits);
        self.tokens += o.tokens;
        self.compactions += o.compactions;
        self.repeats.extend(o.repeats.iter().cloned());
        self.contested.extend(o.contested.iter().cloned());
        self.err_kinds.update(&o.err_kinds);
    }
}

pub fn scan_records(records: impl Iterator<Item = Value>) -> Stats {
    scan_records_max(records, 5)
}

pub fn scan_records_max(records: impl Iterator<Item = Value>, max_samples: usize) -> Stats {
    let r = res();
    let mut st = Stats::default();
    let mut last_assistant = String::new();
    let mut seen_sigs: Vec<String> = Vec::new();
    let mut said: HashMap<String, i64> = HashMap::new();
    let mut pending_claim: Option<String> = None;
    for raw in records {
        st.records += 1;
        let rec = deep_parse(&raw, 0);
        drop(raw);
        let role = role_of(&rec);
        let is_user = role.as_deref().map_or(false, |x| USER_ROLES.contains(&x));
        let is_assistant = role.as_deref().map_or(false, |x| ASSISTANT_ROLES.contains(&x));
        let mut codes = Vec::new();
        find_exit_codes(&rec, 0, &mut codes);
        for code in codes {
            st.errors += 1;
            st.error_sigs.insert_once(&format!("exit code {code}"));
            st.err_kinds.add("codigo", 1);
        }
        let mut models = Vec::new();
        find_models(&rec, 0, &mut models);
        st.models.update_keys(models.iter().map(String::as_str));
        let mut edited = Vec::new();
        find_edits(&rec, 0, &mut edited);
        let mut uniq: Vec<&String> = Vec::new();
        for p in &edited {
            if !uniq.contains(&p) {
                uniq.push(p);
            }
        }
        for p in uniq {
            st.edits.add(&short_path(p), 1);
        }
        st.tokens = st.tokens.saturating_add(count_tokens(&rec, 0));
        let strings = iter_strings(&rec);
        if is_compaction(&rec, &strings) {
            st.compactions += 1;
        }
        let texts = user_texts(&rec, &strings);
        if is_user {
            if let Some(t) = texts.first() {
                let key = squash(&t.to_lowercase());
                let n = said.entry(key).or_insert(0);
                *n += 1;
                if *n == 2 && st.repeats.len() < 5 {
                    st.repeats.push(snippet(t));
                }
                if let Some(claim) = &pending_claim {
                    if r.correction.is_match_str(t) {
                        st.contested.push(format!("{} => {}", snippet(claim), snippet(t)));
                    }
                }
            }
            if !texts.is_empty() {
                pending_claim = None;
            }
        }
        let mut cmds = Vec::new();
        find_commands(&rec, 0, &mut cmds);
        for cmd in &cmds {
            st.commands.add(&head(&snippet(&norm_cmd(cmd)), 120), 1);
            if cmd.contains("--no-verify") {
                st.no_verify += 1;
            }
        }
        let tool_result = is_tool_result(&rec, 0);
        for (_, s) in &strings {
            let c = chars(s);
            if has_secret(&c) {
                st.secrets += 1;
            }
            if let Some(m) = r.error.find(&c) {
                st.errors += 1;
                let sig = signature(&slice(&c, m.start, m.start + 4000));
                st.error_sigs.insert_once(&sig);
                st.err_kinds.add(error_kind(&slice(&c, m.start, m.start + 300)), 1);
                if st.error_samples.len() < max_samples && !seen_sigs.contains(&sig) {
                    seen_sigs.push(sig);
                    st.error_samples.push(sample_around(&c, m.start));
                }
            }
            if is_user && c.len() < 400 && !tool_result && r.correction.is_match(&c) && st.corrections.len() < max_samples {
                st.corrections.push(snippet(s));
            }
            if is_assistant && c.len() >= 10 && s.contains(' ') {
                last_assistant = s.to_string();
                if c.len() < 1500 && r.claim.is_match(&c) {
                    pending_claim = Some(s.to_string());
                }
            }
        }
    }
    st.ends_with_question = strip(&last_assistant).ends_with('?');
    st
}

// ---------- leitura de arquivos ----------

fn read_text(path: &std::path::Path) -> Option<String> {
    std::fs::read(path).ok().map(|b| String::from_utf8_lossy(&b).into_owned())
}

struct JsonlIter {
    reader: std::io::BufReader<std::fs::File>,
    pending: std::collections::VecDeque<String>,
}

impl Iterator for JsonlIter {
    type Item = Value;

    fn next(&mut self) -> Option<Value> {
        use std::io::BufRead;
        loop {
            while let Some(line) = self.pending.pop_front() {
                let line = strip(&line);
                if !line.is_empty() {
                    if let Some(v) = json::parse(line) {
                        return Some(v);
                    }
                }
            }
            let mut buf = Vec::new();
            match self.reader.read_until(b'\n', &mut buf) {
                Ok(0) | Err(_) => return None,
                Ok(_) => {
                    // fim de linha universal, como o modo texto do Python: \r\n e \r também separam registros
                    let text = String::from_utf8_lossy(&buf);
                    self.pending.extend(text.replace("\r\n", "\n").split(|c| c == '\n' || c == '\r').map(str::to_string));
                }
            }
        }
    }
}

/// Registros de um arquivo .jsonl, um por vez (arquivos grandes não ficam inteiros na memória).
pub fn read_jsonl(path: &std::path::Path) -> Box<dyn Iterator<Item = Value>> {
    match std::fs::File::open(path) {
        Ok(f) => Box::new(JsonlIter { reader: std::io::BufReader::new(f), pending: Default::default() }),
        Err(_) => Box::new(std::iter::empty()),
    }
}

/// Lê um JSON solto ou a resposta salva de uma ferramenta (com prefixo antes do JSON).
pub fn load_dump(path: &std::path::Path) -> Vec<Value> {
    let Some(text) = read_text(path) else { return vec![] };
    let start = text.find("{\"ccr\"").or_else(|| match (text.find('{'), text.find('[')) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    });
    let Some(start) = start else { return vec![] };
    let Some((data, _)) = json::raw_decode(&text[start..]) else { return vec![] };
    let data = match &data {
        Value::Obj(_) if data.get("ccr").is_some() => match data.get("ccr").and_then(|c| c.get("data")) {
            Some(d) => d.clone(),
            None => Value::Arr(vec![]),
        },
        _ => data,
    };
    match data {
        Value::Arr(items) => items,
        other => vec![other],
    }
}

pub fn read_records(path: &std::path::Path) -> Box<dyn Iterator<Item = Value>> {
    if path.to_string_lossy().ends_with(".jsonl") {
        read_jsonl(path)
    } else {
        Box::new(load_dump(path).into_iter())
    }
}

// ---------- relatório ----------

pub fn render(name: &str, st: &Stats) -> String {
    let mut lines = vec![format!("### {name}"), format!("- registros: {}", st.records)];
    if !st.models.is_empty() {
        let m: Vec<String> = st.models.most_common(3).iter().map(|(m, n)| format!("{m} ({n})")).collect();
        lines.push(format!("- modelos: {}", m.join(", ")));
    }
    lines.push(format!(
        "- sinais de erro: {}; --no-verify: {}; segredos em texto: {}",
        st.errors, st.no_verify, st.secrets
    ));
    if st.ends_with_question {
        lines.push("- terminou com uma pergunta do assistente (provável espera por resposta)".into());
    }
    let rep: Vec<String> = st
        .commands
        .most_common(5)
        .into_iter()
        .filter(|(_, n)| *n >= 3)
        .map(|(c, n)| format!("`{c}` x{n}"))
        .collect();
    if !rep.is_empty() {
        lines.push(format!("- comandos repetidos: {}", rep.join("; ")));
    }
    if !st.err_kinds.is_empty() {
        let k: Vec<String> = st.err_kinds.most_common(usize::MAX).iter().map(|(k, n)| format!("{k} {n}")).collect();
        lines.push(format!("- erros por tipo: {}", k.join(", ")));
    }
    if st.tokens != 0 {
        lines.push(format!("- tokens de entrada+saída (soma dos registros): {}", st.tokens));
    }
    if st.compactions != 0 {
        lines.push(format!("- compactações de contexto: {}", st.compactions));
    }
    for t in st.repeats.iter().take(2) {
        lines.push(format!("  - usuário repetiu a instrução: {t}"));
    }
    for t in st.contested.iter().take(2) {
        lines.push(format!("  - conclusão contestada: {t}"));
    }
    let thrash = thrashed(st);
    if !thrash.is_empty() {
        let t: Vec<String> = thrash.iter().take(3).map(|(p, n)| format!("`{p}` x{n}")).collect();
        lines.push(format!("- arquivos editados várias vezes na sessão: {}", t.join("; ")));
    }
    for s in st.error_samples.iter().take(3) {
        lines.push(format!("  - erro: {s}"));
    }
    for s in st.corrections.iter().take(3) {
        lines.push(format!("  - correção do usuário: {s}"));
    }
    lines.join("\n")
}

pub fn thrashed(st: &Stats) -> Vec<(String, i64)> {
    st.edits.most_common(5).into_iter().filter(|(_, n)| *n >= THRASH_MIN).collect()
}

/// Pontuação de gravidade de uma sessão: decide quais abrir inteiras (filtro barato antes do caro).
pub fn score(st: &Stats) -> i64 {
    st.errors.min(10)
        + 3 * st.corrections.len() as i64
        + 2 * (st.no_verify > 0) as i64
        + 2 * st.ends_with_question as i64
        + 2 * thrashed(st).len() as i64
        + 3 * (st.secrets > 0) as i64
        + 2 * !st.contested.is_empty() as i64
        + 2 * !st.repeats.is_empty() as i64
        + (st.compactions >= 2) as i64
}

type Category = (&'static str, &'static str, fn(&Stats) -> bool);

const CATEGORIES: [Category; 8] = [
    ("pergunta", "terminaram com pergunta do assistente", |st| st.ends_with_question),
    ("no_verify", "usaram --no-verify", |st| st.no_verify > 0),
    ("correcao", "tiveram correção ou irritação do usuário", |st| !st.corrections.is_empty()),
    ("segredos", "tiveram segredos em texto", |st| st.secrets > 0),
    ("edicoes", "tiveram um arquivo editado 4 vezes ou mais", |st| !thrashed(st).is_empty()),
    ("contestada", "tiveram uma conclusão afirmada pela IA e contestada pelo usuário", |st| !st.contested.is_empty()),
    ("repetiu", "tiveram o usuário repetindo a mesma instrução", |st| !st.repeats.is_empty()),
    ("compactacao", "passaram por compactação de contexto", |st| st.compactions > 0),
];

pub type PerFile = Vec<(String, Stats)>;

/// Contagens da semana, para guardar e comparar com a próxima.
pub struct Summary {
    pub arquivos: i64,
    pub categorias: Vec<(String, i64)>,
    pub erros: Vec<(String, i64)>,
    pub quentes: Vec<(String, i64)>,
}

pub fn summary(per_file: &PerFile) -> Summary {
    let mut sigs = Counter::default();
    let mut hot = Counter::default();
    for (_, st) in per_file {
        for (s, _) in st.error_sigs.items() {
            sigs.add(s, 1);
        }
        for (p, _) in thrashed(st) {
            hot.add(&p, 1);
        }
    }
    Summary {
        arquivos: per_file.len() as i64,
        categorias: CATEGORIES
            .iter()
            .map(|(k, _, pred)| (k.to_string(), per_file.iter().filter(|(_, st)| pred(st)).count() as i64))
            .collect(),
        erros: sigs.most_common(20).into_iter().filter(|(_, k)| *k >= 2).map(|(s, k)| (head(&s, 100), k)).collect(),
        quentes: hot.most_common(10),
    }
}

impl Summary {
    pub fn to_value(&self) -> Value {
        let obj = |v: &Vec<(String, i64)>| Value::Obj(v.iter().map(|(k, n)| (k.clone(), Value::Int(*n))).collect());
        Value::Obj(vec![
            ("arquivos".into(), Value::Int(self.arquivos)),
            ("categorias".into(), obj(&self.categorias)),
            ("erros".into(), obj(&self.erros)),
            ("arquivos_quentes".into(), obj(&self.quentes)),
        ])
    }
}

fn int_at(v: Option<&Value>) -> i64 {
    v.and_then(Value::as_i64).unwrap_or(0)
}

fn keys_of(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Obj(items)) => items.iter().map(|(k, _)| k.clone()).collect(),
        _ => vec![],
    }
}

/// Diferença entre a semana anterior e a atual: o que continua, o que é novo e o que sumiu.
pub fn compare(old: &Value, new: &Summary) -> String {
    let mut lines = vec!["## Comparação com a semana anterior".to_string()];
    for (k, label, _) in CATEGORIES.iter() {
        let a = int_at(old.get("categorias").and_then(|c| c.get(k)));
        let b = new.categorias.iter().find(|(x, _)| x == k).map_or(0, |(_, n)| *n);
        if a != 0 || b != 0 {
            lines.push(format!(
                "- {label}: {a} de {} antes, {b} de {} agora",
                int_at(old.get("arquivos")),
                new.arquivos
            ));
        }
    }
    let sets: [(&str, &str, Vec<String>); 2] = [
        ("erro", "erros", new.erros.iter().map(|(k, _)| k.clone()).collect()),
        ("arquivo muito editado", "arquivos_quentes", new.quentes.iter().map(|(k, _)| k.clone()).collect()),
    ];
    for (title, key, now_keys) in sets {
        let before = keys_of(old.get(key));
        let mut added: Vec<&String> = now_keys.iter().filter(|k| !before.contains(k)).collect();
        let mut kept: Vec<&String> = now_keys.iter().filter(|k| before.contains(k)).collect();
        let mut gone: Vec<&String> = before.iter().filter(|k| !now_keys.contains(k)).collect();
        added.sort();
        kept.sort();
        gone.sort();
        lines.extend(added.iter().map(|i| format!("- novo ({title}): {i}")));
        lines.extend(kept.iter().map(|i| format!("- continua ({title}): {i}")));
        lines.extend(gone.iter().map(|i| format!("- sumiu ({title}): {i}")));
    }
    if lines.len() > 1 {
        lines.join("\n")
    } else {
        String::new()
    }
}

/// Padrões que se repetem em vários arquivos (sessões): é o que vira sugestão de processo.
pub fn render_cross(per_file: &PerFile) -> String {
    let min_files = 2;
    let n = per_file.len();
    let mut lines = vec![format!("## Entre os {n} arquivos")];
    let sm = summary(per_file);
    for ((k, label, _), (_, count)) in CATEGORIES.iter().zip(&sm.categorias) {
        let _ = k;
        if *count != 0 {
            lines.push(format!("- {count} de {n} {label}"));
        }
    }
    for (sig, k) in &sm.erros {
        lines.push(format!("- erro recorrente em {k} arquivos: {sig}"));
    }
    for (path, k) in &sm.quentes {
        if *k >= min_files {
            lines.push(format!("- arquivo muito editado em {k} sessões: `{path}`"));
        }
    }
    let mut cmds = Counter::default();
    for (_, st) in per_file {
        cmds.update_keys(st.commands.items().filter(|(_, k)| *k >= 3).map(|(c, _)| c));
    }
    for (cmd, k) in cmds.most_common(5) {
        if k >= min_files {
            lines.push(format!("- comando repetido 3 vezes ou mais na mesma sessão, em {k} arquivos: `{}`", head(&cmd, 100)));
        }
    }
    let mut kinds = Counter::default();
    for (_, st) in per_file {
        kinds.update(&st.err_kinds);
    }
    if !kinds.is_empty() {
        let k: Vec<String> = kinds.most_common(usize::MAX).iter().map(|(k, c)| format!("{k} {c}")).collect();
        lines.push(format!("- erros por tipo: {}", k.join(", ")));
    }
    let mut ranked: Vec<(i64, &str)> = per_file.iter().map(|(name, st)| (score(st), name.as_str())).collect();
    ranked.sort_by(|a, b| b.cmp(a));
    let ranked: Vec<String> = ranked.into_iter().take(3).filter(|(sc, _)| *sc >= 5).map(|(sc, name)| format!("{name} ({sc})")).collect();
    if !ranked.is_empty() && lines.len() > 1 {
        lines.push(format!("- para leitura completa (maior pontuação): {}", ranked.join(", ")));
    }
    if lines.len() > 1 {
        lines.join("\n")
    } else {
        String::new()
    }
}

/// Cruza regras escritas no CLAUDE.md/AGENTS.md ("nunca use `x`") com os comandos que as sessões rodaram.
pub fn rule_violations(rules_text: &str, per_file: &PerFile) -> Vec<String> {
    let r = res();
    let max_rules = 5;
    let mut found: Vec<(usize, i64, String, String)> = Vec::new();
    for line in rules_text.lines() {
        if !r.rule_line.is_match_str(line) {
            continue;
        }
        let c = chars(line);
        for m in r.backticked.find_all(&c) {
            let Some((a, b)) = m.group(1) else { continue };
            let tok = slice(&c, a, b);
            let files = per_file.iter().filter(|(_, st)| st.commands.items().any(|(cmd, _)| cmd.contains(&tok))).count();
            let times: i64 = per_file
                .iter()
                .flat_map(|(_, st)| st.commands.items().filter(|(cmd, _)| cmd.contains(&tok)).map(|(_, n)| n).collect::<Vec<_>>())
                .sum();
            if files > 0 {
                found.push((files, times, tok, head(&squash(line), 90)));
            }
        }
    }
    found.sort_by(|a, b| b.cmp(a));
    if found.is_empty() {
        return vec![];
    }
    let mut out = vec!["## Regras do projeto desrespeitadas".to_string()];
    out.extend(found.into_iter().take(max_rules).map(|(k, times, tok, rule)| format!("- `{tok}` em {k} sessões ({times} vezes). Regra: {rule}")));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(s: &str) -> Value {
        json::parse(s).unwrap()
    }

    fn sample() -> Vec<Value> {
        vec![
            rec(r#"{"role": "user", "content": "roda os testes"}"#),
            rec(r#"{"role": "assistant", "content": "ok", "model": "gpt-x"}"#),
            rec(r#"{"type": "tool", "command": "pytest -q", "output": "Traceback (most recent call last): boom"}"#),
            rec(r#"{"type": "tool", "command": "pytest -q", "output": "FAILED tests/a.py"}"#),
            rec(r#"{"type": "tool", "command": "pytest -q", "output": "FAILED tests/a.py"}"#),
            rec(r#"{"type": "tool", "command": "git push --no-verify origin x", "output": "ok"}"#),
            rec(r#"{"role": "user", "content": "errado, de novo isso"}"#),
            rec(r#"{"role": "user", "content": "use Authorization: Bearer abcdefghijklmnop1234567890 no curl em 192.168.1.10"}"#),
            rec(r#"{"role": "assistant", "content": "Quer que eu aplique a correção agora?"}"#),
        ]
    }

    #[test]
    fn scan_encontra_os_sinais() {
        let st = scan_records(sample().into_iter());
        assert_eq!(st.records, 9);
        assert!(st.errors >= 3);
        assert_eq!(st.no_verify, 1);
        assert!(st.secrets >= 1);
        assert!(st.ends_with_question);
        assert_eq!(st.commands.get("pytest -q"), 3);
        assert!(st.corrections.iter().any(|c| c.contains("errado")));
        assert_eq!(st.models.get("gpt-x"), 1);
    }

    #[test]
    fn saida_nunca_vaza_segredo_nem_ip() {
        let text = render("x", &scan_records(sample().into_iter()));
        assert!(!text.contains("abcdefghijklmnop1234567890"));
        assert!(!text.contains("192.168.1.10"));
    }

    #[test]
    fn nome_de_parametro_nao_e_erro() {
        let st = scan_records(
            vec![rec(r#"{"role": "assistant", "content": "def f(raise_on_error: bool = False):\n    try: pass\n    except Exception: pass"}"#)]
                .into_iter(),
        );
        assert_eq!(st.errors, 0);
    }

    #[test]
    fn erro_real_e_detectado() {
        for txt in ["ValueError: x", "error: linking failed", "fatal: not a git repository", "Killed"] {
            let st = scan_records(vec![Value::Obj(vec![("output".into(), Value::Str(txt.into()))])].into_iter());
            assert_eq!(st.errors, 1, "{txt}");
        }
    }

    #[test]
    fn segredos_em_varios_formatos_sao_mascarados() {
        for s in [
            "password=SuperSecreta123",
            "Bearer abcdefghijklmnopqrstuv",
            "ghp_abcdefghijklmnopqrstuv1234",
            "postgres://user:passw0rd@10.0.0.5:5432/db",
            "AKIAABCDEFGHIJKLMNOP",
            "-----BEGIN RSA PRIVATE KEY-----",
            "eyJhbGciOiJIUzI1.eyJzdWIiOiIxMjM0NTY.assinatura",
        ] {
            let out = redact(&format!("antes {s} depois"));
            assert!(out.contains("[SEGREDO]"), "{s} -> {out}");
        }
        assert_eq!(redact("host 10.1.2.3 ok"), "host [IP] ok");
    }

    #[test]
    fn assinatura_agrupa_ruido() {
        let a = signature("Traceback (most recent call last): timeout after 3s em /tmp/run3/x.py");
        let b = signature("Traceback (most recent call last): timeout after 500s em /tmp/run500/x.py");
        assert_eq!(a, b);
    }

    #[test]
    fn comando_normalizado_tira_involucro() {
        assert_eq!(norm_cmd("bash -lc 'cd /w && cargo test'"), "cargo test");
        assert_eq!(norm_cmd("cmd /c npm test"), "npm test");
    }

    #[test]
    fn jsonl_ignora_linhas_ruins() {
        let dir = std::env::temp_dir().join(format!("obs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.jsonl");
        std::fs::write(&p, b"{\"a\": 1}\nnot json\r\n{\"b\": 2}\n\xff\xfe{{{\n").unwrap();
        assert_eq!(read_jsonl(&p).count(), 2);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn registro_gigante_e_aninhado_nao_derrubam() {
        let big = format!("{} Traceback (most recent call last) {} password=Segredo12345", "x".repeat(2_000_000), "y".repeat(1000));
        let st = scan_records(vec![Value::Obj(vec![("output".into(), Value::Str(big))])].into_iter());
        assert_eq!(st.errors, 1);
        assert_eq!(st.secrets, 1);
        assert!(!render("g", &st).contains("Segredo12345"));
    }
}
