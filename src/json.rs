//! JSON mínimo, só com a biblioteca padrão. Mantém a ordem das chaves, como o `dict` do Python.

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Arr(Vec<Value>),
    Obj(Vec<(String, Value)>),
}

const MAX_DEPTH: usize = 200;

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Obj(items) => items.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    /// Verdade ou falsidade no sentido do Python (`x or y`).
    pub fn truthy(&self) -> bool {
        match self {
            Value::Null => false,
            Value::Bool(b) => *b,
            Value::Int(i) => *i != 0,
            Value::Float(f) => *f != 0.0,
            Value::Str(s) => !s.is_empty(),
            Value::Arr(a) => !a.is_empty(),
            Value::Obj(o) => !o.is_empty(),
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Int(i) => Some(*i as f64),
            Value::Float(f) => Some(*f),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            _ => None,
        }
    }

    /// `str(valor)` do Python para escalares; contêineres usam o `repr` aproximado.
    pub fn py_str(&self) -> String {
        match self {
            Value::Str(s) => s.clone(),
            other => other.py_repr(),
        }
    }

    pub fn py_repr(&self) -> String {
        match self {
            Value::Null => "None".into(),
            Value::Bool(true) => "True".into(),
            Value::Bool(false) => "False".into(),
            Value::Int(i) => i.to_string(),
            Value::Float(f) => py_float(*f),
            Value::Str(s) => py_repr_str(s),
            Value::Arr(a) => format!("[{}]", a.iter().map(Value::py_repr).collect::<Vec<_>>().join(", ")),
            Value::Obj(o) => format!(
                "{{{}}}",
                o.iter().map(|(k, v)| format!("{}: {}", py_repr_str(k), v.py_repr())).collect::<Vec<_>>().join(", ")
            ),
        }
    }
}

pub fn py_float(f: f64) -> String {
    if f.is_nan() {
        return "nan".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "inf".into() } else { "-inf".into() };
    }
    let s = format!("{}", f);
    if s.contains('.') || s.contains('e') {
        s
    } else {
        format!("{}.0", s)
    }
}

pub fn py_repr_str(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') { '"' } else { '\'' };
    let mut out = String::new();
    out.push(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20 || c as u32 == 0x7f => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

/// Lê um valor JSON do começo do texto. Devolve o valor e quantos bytes consumiu.
pub fn raw_decode(text: &str) -> Option<(Value, usize)> {
    let mut p = Parser { b: text.as_bytes(), i: 0 };
    let v = p.value(0)?;
    Some((v, p.i))
}

/// Como `json.loads`: o texto inteiro precisa ser um único valor (espaços nas pontas são aceitos).
pub fn parse(text: &str) -> Option<Value> {
    let mut p = Parser { b: text.as_bytes(), i: 0 };
    p.ws();
    let v = p.value(0)?;
    p.ws();
    if p.i == p.b.len() {
        Some(v)
    } else {
        None
    }
}

impl<'a> Parser<'a> {
    fn ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn lit(&mut self, word: &str) -> bool {
        if self.b[self.i..].starts_with(word.as_bytes()) {
            self.i += word.len();
            true
        } else {
            false
        }
    }

    fn value(&mut self, depth: usize) -> Option<Value> {
        if depth > MAX_DEPTH || self.i >= self.b.len() {
            return None;
        }
        let c = self.b[self.i];
        match c {
            b'{' => {
                self.i += 1;
                let mut items: Vec<(String, Value)> = Vec::new();
                self.ws();
                if self.peek() == Some(b'}') {
                    self.i += 1;
                    return Some(Value::Obj(items));
                }
                loop {
                    self.ws();
                    if self.peek() != Some(b'"') {
                        return None;
                    }
                    let k = self.string()?;
                    self.ws();
                    if self.peek() != Some(b':') {
                        return None;
                    }
                    self.i += 1;
                    self.ws();
                    let v = self.value(depth + 1)?;
                    // chave repetida: o valor novo vence, a posição é a da primeira (como o dict)
                    match items.iter_mut().find(|(ek, _)| *ek == k) {
                        Some(slot) => slot.1 = v,
                        None => items.push((k, v)),
                    }
                    self.ws();
                    match self.peek()? {
                        b',' => self.i += 1,
                        b'}' => {
                            self.i += 1;
                            return Some(Value::Obj(items));
                        }
                        _ => return None,
                    }
                }
            }
            b'[' => {
                self.i += 1;
                let mut items = Vec::new();
                self.ws();
                if self.peek() == Some(b']') {
                    self.i += 1;
                    return Some(Value::Arr(items));
                }
                loop {
                    self.ws();
                    items.push(self.value(depth + 1)?);
                    self.ws();
                    match self.peek()? {
                        b',' => self.i += 1,
                        b']' => {
                            self.i += 1;
                            return Some(Value::Arr(items));
                        }
                        _ => return None,
                    }
                }
            }
            b'"' => self.string().map(Value::Str),
            b't' if self.lit("true") => Some(Value::Bool(true)),
            b'f' if self.lit("false") => Some(Value::Bool(false)),
            b'n' if self.lit("null") => Some(Value::Null),
            b'N' if self.lit("NaN") => Some(Value::Float(f64::NAN)),
            b'I' if self.lit("Infinity") => Some(Value::Float(f64::INFINITY)),
            b'-' if self.b[self.i..].starts_with(b"-Infinity") => {
                self.i += 9;
                Some(Value::Float(f64::NEG_INFINITY))
            }
            b'-' | b'0'..=b'9' => self.number(),
            _ => None,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    fn number(&mut self) -> Option<Value> {
        let start = self.i;
        if self.peek() == Some(b'-') {
            self.i += 1;
        }
        match self.peek()? {
            b'0' => self.i += 1,
            b'1'..=b'9' => self.digits(),
            _ => return None,
        }
        let mut is_float = false;
        if self.peek() == Some(b'.') && self.b.get(self.i + 1).map_or(false, u8::is_ascii_digit) {
            is_float = true;
            self.i += 1;
            self.digits();
        }
        if matches!(self.peek(), Some(b'e') | Some(b'E')) {
            let save = self.i;
            self.i += 1;
            if matches!(self.peek(), Some(b'+') | Some(b'-')) {
                self.i += 1;
            }
            if self.peek().map_or(false, |c| c.is_ascii_digit()) {
                is_float = true;
                self.digits();
            } else {
                self.i = save;
            }
        }
        let text = std::str::from_utf8(&self.b[start..self.i]).ok()?;
        if !is_float {
            if let Ok(i) = text.parse::<i64>() {
                return Some(Value::Int(i));
            }
        }
        text.parse::<f64>().ok().map(Value::Float)
    }

    fn digits(&mut self) {
        while self.peek().map_or(false, |c| c.is_ascii_digit()) {
            self.i += 1;
        }
    }

    fn hex4(&mut self) -> Option<u32> {
        let s = self.b.get(self.i..self.i + 4)?;
        let t = std::str::from_utf8(s).ok()?;
        if !t.bytes().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        self.i += 4;
        u32::from_str_radix(t, 16).ok()
    }

    fn string(&mut self) -> Option<String> {
        self.i += 1; // aspas de abertura
        let mut out = String::new();
        loop {
            let start = self.i;
            while self.i < self.b.len() && self.b[self.i] != b'"' && self.b[self.i] != b'\\' {
                if self.b[self.i] < 0x20 {
                    return None; // caractere de controle: o Python estrito também recusa
                }
                self.i += 1;
            }
            out.push_str(&String::from_utf8_lossy(&self.b[start..self.i]));
            match self.peek()? {
                b'"' => {
                    self.i += 1;
                    return Some(out);
                }
                _ => {
                    self.i += 1;
                    let c = self.peek()?;
                    self.i += 1;
                    match c {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hi = self.hex4()?;
                            let code = if (0xD800..0xDC00).contains(&hi) && self.b[self.i..].starts_with(b"\\u") {
                                let save = self.i;
                                self.i += 2;
                                match self.hex4() {
                                    Some(lo) if (0xDC00..0xE000).contains(&lo) => 0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00),
                                    _ => {
                                        self.i = save;
                                        hi
                                    }
                                }
                            } else {
                                hi
                            };
                            out.push(char::from_u32(code).unwrap_or('\u{fffd}'));
                        }
                        _ => return None,
                    }
                }
            }
        }
    }
}

/// `json.dumps(valor, ensure_ascii=False, indent=1)`.
pub fn dumps_indent1(v: &Value) -> String {
    let mut out = String::new();
    dump(v, 0, &mut out);
    out
}

fn dump_str(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn dump(v: &Value, level: usize, out: &mut String) {
    let pad = |n: usize| " ".repeat(n);
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Int(i) => out.push_str(&i.to_string()),
        Value::Float(f) => out.push_str(&py_float(*f)),
        Value::Str(s) => dump_str(s, out),
        Value::Arr(a) if a.is_empty() => out.push_str("[]"),
        Value::Arr(a) => {
            out.push_str("[\n");
            for (n, x) in a.iter().enumerate() {
                out.push_str(&pad(level + 1));
                dump(x, level + 1, out);
                out.push_str(if n + 1 < a.len() { ",\n" } else { "\n" });
            }
            out.push_str(&pad(level));
            out.push(']');
        }
        Value::Obj(o) if o.is_empty() => out.push_str("{}"),
        Value::Obj(o) => {
            out.push_str("{\n");
            for (n, (k, x)) in o.iter().enumerate() {
                out.push_str(&pad(level + 1));
                dump_str(k, out);
                out.push_str(": ");
                dump(x, level + 1, out);
                out.push_str(if n + 1 < o.len() { ",\n" } else { "\n" });
            }
            out.push_str(&pad(level));
            out.push('}');
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basico() {
        let v = parse(r#" {"a": [1, 2.5, "x\n\u00e7", null, true], "a": 9, "b": {}} "#).unwrap();
        assert_eq!(v.get("a"), Some(&Value::Int(9)));
        assert_eq!(v.get("b"), Some(&Value::Obj(vec![])));
        assert!(parse("{").is_none());
        assert!(parse("[1,]").is_none());
        assert!(parse("1 2").is_none());
        assert!(parse("\"a\nb\"").is_none());
    }

    #[test]
    fn raw_decode_para_no_fim_do_valor() {
        let (v, n) = raw_decode(r#"{"a": 1} lixo"#).unwrap();
        assert_eq!(v.get("a"), Some(&Value::Int(1)));
        assert_eq!(n, 8);
    }

    #[test]
    fn aninhamento_profundo_nao_estoura() {
        let s = "[".repeat(100_000);
        assert!(parse(&s).is_none());
    }

    #[test]
    fn dumps_igual_ao_python() {
        let v = parse(r#"{"x": 1, "l": {"a": 2, "ç": "é\"\n"}, "e": {}, "f": 1.5}"#).unwrap();
        assert_eq!(
            dumps_indent1(&v),
            "{\n \"x\": 1,\n \"l\": {\n  \"a\": 2,\n  \"ç\": \"é\\\"\\n\"\n },\n \"e\": {},\n \"f\": 1.5\n}"
        );
    }
}
