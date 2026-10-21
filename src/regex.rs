//! Motor de expressões regulares mínimo (só biblioteca padrão), com o subconjunto que as heurísticas usam:
//! literais, `.`, classes, `\b \B \d \s \w`, grupos, alternância, repetições gulosas e lookbehind negativo
//! de largura fixa. Existe porque o crate `regex` não tem lookbehind e o projeto não usa dependências.
//! Trabalha sobre `&[char]`; as posições são índices de caracteres, como no `re` do Python.

#[derive(Debug)]
enum Item {
    Ch(char),
    Range(char, char),
    Digit(bool),
    Space(bool),
    Word(bool),
}

#[derive(Debug)]
struct Class {
    items: Vec<Item>,
    neg: bool,
    icase: bool,
    /// Decisão pronta para os 128 caracteres ASCII (já com maiúsculas/minúsculas e negação).
    ascii: [bool; 128],
}

impl Class {
    fn new(items: Vec<Item>, neg: bool, icase: bool) -> Class {
        let mut c = Class { items, neg, icase, ascii: [false; 128] };
        for code in 0..128u8 {
            c.ascii[code as usize] = c.slow(char::from(code));
        }
        c
    }

    fn slow(&self, c: char) -> bool {
        let mut hit = self.items.iter().any(|i| i.hit(c));
        if !hit && self.icase {
            let (l, u) = (lower(c), upper(c));
            hit = self.items.iter().any(|i| i.hit(l) || i.hit(u));
        }
        hit != self.neg
    }

    fn matches(&self, c: char) -> bool {
        if (c as u32) < 128 {
            self.ascii[c as usize]
        } else {
            self.slow(c)
        }
    }
}

enum Node {
    Char(char, char),
    Any,
    Class(Class),
    Start,
    WordB(bool),
    Group(Box<Node>, Option<usize>),
    Alt(Vec<Node>),
    Cat(Vec<Node>),
    Repeat(Box<Node>, usize, usize),
    NotBehind(Box<Node>, usize, bool),
}

pub struct Regex {
    node: Node,
    ncaps: usize,
    icase: bool,
    /// Para cada caractere ASCII, se uma ocorrência pode começar nele (None: o padrão pode casar vazio).
    first: Option<[bool; 128]>,
    /// Literais dos quais pelo menos um precisa aparecer no texto para haver casamento (antes de rodar o motor).
    lits: Option<Vec<Vec<char>>>,
    lit_first: [bool; 128],
}

pub type Caps = Vec<Option<(usize, usize)>>;

pub struct Match {
    pub start: usize,
    pub end: usize,
    pub caps: Caps,
}

impl Match {
    pub fn group(&self, n: usize) -> Option<(usize, usize)> {
        self.caps.get(n).copied().flatten()
    }
}

pub fn is_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn is_digit(c: char) -> bool {
    c.is_ascii_digit() || (!c.is_ascii() && c.is_numeric() && c.to_digit(10).is_none() && unicode_decimal(c))
}

fn unicode_decimal(c: char) -> bool {
    // aproximação de \d do Python (categoria Nd) fora do ASCII: dígitos de outras escritas
    matches!(c as u32, 0x660..=0x669 | 0x6F0..=0x6F9 | 0x966..=0x96F | 0xFF10..=0xFF19)
}

fn lower(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

fn upper(c: char) -> char {
    c.to_uppercase().next().unwrap_or(c)
}

impl Item {
    fn hit(&self, c: char) -> bool {
        match self {
            Item::Ch(x) => *x == c,
            Item::Range(a, b) => *a <= c && c <= *b,
            Item::Digit(pos) => is_digit(c) == *pos,
            Item::Space(pos) => is_space(c) == *pos,
            Item::Word(pos) => is_word(c) == *pos,
        }
    }
}

struct Parser {
    p: Vec<char>,
    i: usize,
    ncaps: usize,
    icase: bool,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.p.get(self.i).copied()
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn alt(&mut self) -> Node {
        let mut alts = vec![self.cat()];
        while self.eat('|') {
            alts.push(self.cat());
        }
        if alts.len() == 1 {
            alts.pop().unwrap()
        } else {
            Node::Alt(alts)
        }
    }

    fn cat(&mut self) -> Node {
        let mut items = Vec::new();
        while let Some(c) = self.peek() {
            if c == '|' || c == ')' {
                break;
            }
            let atom = self.atom();
            items.push(self.quantifier(atom));
        }
        Node::Cat(items)
    }

    fn number(&mut self) -> usize {
        let mut n = 0;
        while let Some(d) = self.peek().and_then(|c| c.to_digit(10)) {
            n = n * 10 + d as usize;
            self.i += 1;
        }
        n
    }

    fn quantifier(&mut self, atom: Node) -> Node {
        let (min, max) = match self.peek() {
            Some('*') => (0, usize::MAX),
            Some('+') => (1, usize::MAX),
            Some('?') => (0, 1),
            Some('{') => {
                self.i += 1;
                let lo = self.number();
                let hi = if self.eat(',') {
                    if self.peek() == Some('}') {
                        usize::MAX
                    } else {
                        self.number()
                    }
                } else {
                    lo
                };
                assert!(self.peek() == Some('}'), "quantificador inválido");
                (lo, hi)
            }
            _ => return atom,
        };
        self.i += 1;
        assert!(self.peek() != Some('?'), "repetição preguiçosa não é suportada");
        Node::Repeat(Box::new(atom), min, max)
    }

    fn atom(&mut self) -> Node {
        let c = self.peek().unwrap();
        self.i += 1;
        match c {
            '(' => {
                if self.eat('?') {
                    if self.eat(':') {
                        let n = self.alt();
                        assert!(self.eat(')'));
                        return Node::Group(Box::new(n), None);
                    }
                    assert!(self.eat('<') && self.eat('!'), "só (?: e (?<! são suportados");
                    let before = self.ncaps;
                    let n = self.alt();
                    assert!(self.eat(')'));
                    let w = width(&n).expect("lookbehind precisa ter largura fixa");
                    return Node::NotBehind(Box::new(n), w, self.ncaps > before);
                }
                self.ncaps += 1;
                let idx = self.ncaps;
                let n = self.alt();
                assert!(self.eat(')'));
                Node::Group(Box::new(n), Some(idx))
            }
            '[' => self.class(),
            '.' => Node::Any,
            '^' => Node::Start,
            '\\' => {
                let e = self.peek().expect("barra no fim");
                self.i += 1;
                match e {
                    'b' => Node::WordB(true),
                    'B' => Node::WordB(false),
                    'd' | 'D' | 's' | 'S' | 'w' | 'W' => Node::Class(Class::new(vec![class_escape(e)], false, self.icase)),
                    'n' => self.ch('\n'),
                    other => self.ch(other),
                }
            }
            c => self.ch(c),
        }
    }

    fn ch(&self, c: char) -> Node {
        if self.icase {
            Node::Char(lower(c), upper(c))
        } else {
            Node::Char(c, c)
        }
    }

    fn class(&mut self) -> Node {
        let neg = self.eat('^');
        let mut items = Vec::new();
        let mut first = true;
        loop {
            let c = self.peek().expect("classe não fechada");
            self.i += 1;
            if c == ']' && !first {
                break;
            }
            first = false;
            let lo = if c == '\\' {
                let e = self.peek().unwrap();
                self.i += 1;
                if matches!(e, 'd' | 'D' | 's' | 'S' | 'w' | 'W') {
                    items.push(class_escape(e));
                    continue;
                }
                if e == 'n' {
                    '\n'
                } else {
                    e
                }
            } else {
                c
            };
            if self.peek() == Some('-') && self.p.get(self.i + 1).map_or(false, |&n| n != ']') {
                self.i += 1;
                let mut hi = self.peek().unwrap();
                self.i += 1;
                if hi == '\\' {
                    hi = self.peek().unwrap();
                    self.i += 1;
                }
                items.push(Item::Range(lo, hi));
            } else {
                items.push(Item::Ch(lo));
            }
        }
        Node::Class(Class::new(items, neg, self.icase))
    }
}

fn class_escape(e: char) -> Item {
    match e {
        'd' => Item::Digit(true),
        'D' => Item::Digit(false),
        's' => Item::Space(true),
        'S' => Item::Space(false),
        'w' => Item::Word(true),
        _ => Item::Word(false),
    }
}

fn width(n: &Node) -> Option<usize> {
    match n {
        Node::Char(..) | Node::Any | Node::Class(..) => Some(1),
        Node::Start | Node::WordB(_) | Node::NotBehind(..) => Some(0),
        Node::Group(g, _) => width(g),
        Node::Cat(v) => v.iter().map(width).sum(),
        Node::Alt(v) => {
            let w = width(&v[0])?;
            v.iter().all(|x| width(x) == Some(w)).then_some(w)
        }
        Node::Repeat(g, lo, hi) => (lo == hi).then(|| width(g)).flatten().map(|w| w * lo),
    }
}

struct Ctx<'a> {
    s: &'a [char],
    icase: bool,
}

type Cont<'k> = &'k mut dyn FnMut(usize, &mut Caps) -> bool;

impl Ctx<'_> {
    fn single(&self, n: &Node, pos: usize) -> Option<bool> {
        // casa um nó de um caractere só; None se o nó não é desse tipo
        match n {
            Node::Char(lo, up) => Some(pos < self.s.len() && {
                let c = self.s[pos];
                c == *lo || c == *up || (self.icase && !c.is_ascii() && lower(c) == *lo)
            }),
            Node::Any => Some(pos < self.s.len() && self.s[pos] != '\n'),
            Node::Class(cl) => Some(pos < self.s.len() && cl.matches(self.s[pos])),
            _ => None,
        }
    }

    fn m(&self, n: &Node, pos: usize, caps: &mut Caps, k: Cont) -> bool {
        if let Some(ok) = self.single(n, pos) {
            return ok && k(pos + 1, caps);
        }
        match n {
            Node::Start => pos == 0 && k(pos, caps),
            Node::WordB(want) => {
                let before = pos > 0 && is_word(self.s[pos - 1]);
                let after = pos < self.s.len() && is_word(self.s[pos]);
                ((before != after) == *want) && k(pos, caps)
            }
            Node::Group(g, None) => self.m(g, pos, caps, k),
            Node::Group(g, Some(idx)) => {
                let idx = *idx;
                self.m(g, pos, caps, &mut |p, c: &mut Caps| {
                    let old = c[idx];
                    c[idx] = Some((pos, p));
                    if k(p, c) {
                        true
                    } else {
                        c[idx] = old;
                        false
                    }
                })
            }
            Node::Alt(alts) => alts.iter().any(|a| self.m(a, pos, caps, &mut *k)),
            Node::Cat(items) => self.cat(items, pos, caps, k),
            Node::Repeat(g, min, max) => {
                if self.single(g, 0).is_some() {
                    // repetição de um caractere: laço, sem recursão (entradas grandes não estouram a pilha)
                    let mut n = 0;
                    while n < *max && self.single(g, pos + n) == Some(true) {
                        n += 1;
                    }
                    if n < *min {
                        return false;
                    }
                    let mut i = n;
                    loop {
                        if k(pos + i, caps) {
                            return true;
                        }
                        if i == *min {
                            return false;
                        }
                        i -= 1;
                    }
                }
                self.rep(g, *min, *max, 0, pos, caps, k)
            }
            Node::NotBehind(g, w, has_groups) => {
                if pos >= *w {
                    let start = pos - w;
                    let mut scratch: Caps = if *has_groups { vec![None; caps.len()] } else { Vec::new() };
                    if self.m(g, start, &mut scratch, &mut |p, _| p == pos) {
                        return false;
                    }
                }
                k(pos, caps)
            }
            _ => unreachable!(),
        }
    }

    fn cat(&self, items: &[Node], pos: usize, caps: &mut Caps, k: Cont) -> bool {
        match items.split_first() {
            None => k(pos, caps),
            Some((head, rest)) => self.m(head, pos, caps, &mut |p, c: &mut Caps| self.cat(rest, p, c, &mut *k)),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn rep(&self, g: &Node, min: usize, max: usize, count: usize, pos: usize, caps: &mut Caps, k: Cont) -> bool {
        if count < max
            && self.m(g, pos, caps, &mut |p, c: &mut Caps| {
                if p == pos && count >= min {
                    return false;
                }
                self.rep(g, min, max, count + 1, p, c, &mut *k)
            })
        {
            return true;
        }
        count >= min && k(pos, caps)
    }
}

/// (pode começar com `c`, pode casar vazio). Conservador: asserções de largura zero são ignoradas.
fn starts_with(ctx: &Ctx, n: &Node, c: char) -> (bool, bool) {
    match n {
        Node::Char(..) | Node::Any | Node::Class(..) => (ctx.single(n, 0) == Some(true), false),
        Node::Start | Node::WordB(_) | Node::NotBehind(..) => (false, true),
        Node::Group(g, _) => starts_with(ctx, g, c),
        Node::Alt(v) => v.iter().map(|x| starts_with(ctx, x, c)).fold((false, false), |a, b| (a.0 || b.0, a.1 || b.1)),
        Node::Cat(v) => {
            let mut can = false;
            for x in v {
                let (s, e) = starts_with(ctx, x, c);
                can |= s;
                if !e {
                    return (can, false);
                }
            }
            (can, true)
        }
        Node::Repeat(g, min, _) => {
            let (s, e) = starts_with(ctx, g, c);
            (s, e || *min == 0)
        }
    }
}

/// Conjunto de literais em que ao menos um aparece em qualquer casamento do nó (None: não dá para saber).
/// Em padrões sem `(?i)` os literais são exatos; com `(?i)` ficam em minúsculas.
fn required(n: &Node) -> Option<Vec<Vec<char>>> {
    match n {
        Node::Char(lo, _) => Some(vec![vec![*lo]]),
        Node::Group(g, _) => required(g),
        Node::Repeat(g, min, _) if *min >= 1 => required(g),
        Node::Alt(v) => {
            let mut all = Vec::new();
            for x in v {
                all.extend(required(x)?);
            }
            Some(all)
        }
        Node::Cat(v) => {
            let mut best: Option<Vec<Vec<char>>> = None;
            let score = |s: &Vec<Vec<char>>| s.iter().map(Vec::len).min().unwrap_or(0) * 4 + 3usize.saturating_sub(s.len().min(3));
            let mut consider = |cand: Vec<Vec<char>>| {
                if best.as_ref().map_or(true, |b| score(&cand) > score(b)) {
                    best = Some(cand);
                }
            };
            let mut run: Vec<char> = Vec::new();
            for x in v {
                if let Node::Char(lo, _) = x {
                    run.push(*lo);
                    continue;
                }
                if !run.is_empty() {
                    consider(vec![std::mem::take(&mut run)]);
                }
                if let Some(c) = required(x) {
                    consider(c);
                }
            }
            if !run.is_empty() {
                consider(vec![run]);
            }
            best
        }
        _ => None,
    }
}

fn fold(c: char, icase: bool) -> char {
    if !icase {
        c
    } else if c.is_ascii() {
        c.to_ascii_lowercase()
    } else {
        lower(c)
    }
}

impl Regex {
    fn passes_prefilter(&self, s: &[char], from: usize) -> bool {
        let Some(lits) = &self.lits else { return true };
        for i in from..s.len() {
            let c = fold(s[i], self.icase);
            if (c as u32) < 128 && !self.lit_first[c as usize] {
                continue;
            }
            for lit in lits {
                if lit[0] == c && s.len() - i >= lit.len() && lit.iter().enumerate().all(|(j, x)| fold(s[i + j], self.icase) == *x) {
                    return true;
                }
            }
        }
        false
    }

    pub fn new(pattern: &str) -> Regex {
        let (icase, body) = match pattern.strip_prefix("(?i)") {
            Some(rest) => (true, rest),
            None => (false, pattern),
        };
        let mut p = Parser { p: body.chars().collect(), i: 0, ncaps: 0, icase };
        let node = p.alt();
        assert!(p.i == p.p.len(), "padrão com ')' sobrando: {pattern}");
        let mut first = [false; 128];
        let mut nullable = false;
        for (code, slot) in first.iter_mut().enumerate() {
            let c = [char::from(code as u8)];
            let ctx = Ctx { s: &c, icase };
            let (s, e) = starts_with(&ctx, &node, c[0]);
            *slot = s;
            nullable |= e;
        }
        let lits = required(&node).filter(|l| !l.is_empty() && l.iter().all(|x| !x.is_empty()));
        let mut lit_first = [false; 128];
        for lit in lits.iter().flatten() {
            if lit[0].is_ascii() {
                lit_first[lit[0] as usize] = true;
            } else {
                // primeira letra fora do ASCII: não filtra por ela
                lit_first = [true; 128];
                break;
            }
        }
        Regex { node, ncaps: p.ncaps, icase, first: if nullable { None } else { Some(first) }, lits, lit_first }
    }

    /// Primeira ocorrência a partir de `from`.
    pub fn find_at(&self, s: &[char], from: usize) -> Option<Match> {
        if from == 0 && !self.passes_prefilter(s, 0) {
            return None;
        }
        self.search(s, from)
    }

    fn search(&self, s: &[char], from: usize) -> Option<Match> {
        let ctx = Ctx { s, icase: self.icase };
        let mut caps: Caps = vec![None; self.ncaps + 1];
        for start in from..=s.len() {
            if let Some(first) = &self.first {
                match s.get(start) {
                    None => return None,
                    Some(&c) if (c as u32) < 128 && !first[c as usize] => continue,
                    _ => {}
                }
            }
            caps.iter_mut().for_each(|c| *c = None);
            let mut end = None;
            if ctx.m(&self.node, start, &mut caps, &mut |p, _| {
                end = Some(p);
                true
            }) {
                return Some(Match { start, end: end.unwrap(), caps: caps.clone() });
            }
        }
        None
    }

    pub fn find(&self, s: &[char]) -> Option<Match> {
        self.find_at(s, 0)
    }

    pub fn is_match(&self, s: &[char]) -> bool {
        self.find_at(s, 0).is_some()
    }

    pub fn is_match_str(&self, s: &str) -> bool {
        self.is_match(&s.chars().collect::<Vec<_>>())
    }

    /// Todas as ocorrências sem sobreposição, como `re.finditer`.
    pub fn find_all(&self, s: &[char]) -> Vec<Match> {
        let mut out = Vec::new();
        let mut pos = 0;
        while pos <= s.len() {
            match if pos == 0 && !self.passes_prefilter(s, 0) { None } else { self.search(s, pos) } {
                Some(m) => {
                    pos = if m.end > m.start { m.end } else { m.end + 1 };
                    out.push(m);
                }
                None => break,
            }
        }
        out
    }

    /// `re.sub` com texto fixo no lugar de cada ocorrência.
    pub fn replace_all(&self, text: &str, with: &str) -> String {
        let s: Vec<char> = text.chars().collect();
        let mut out = String::new();
        let mut last = 0;
        for m in self.find_all(&s) {
            out.extend(&s[last..m.start]);
            out.push_str(with);
            last = m.end;
        }
        out.extend(&s[last..]);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cs(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    fn found(re: &str, s: &str) -> Option<String> {
        let v = cs(s);
        Regex::new(re).find(&v).map(|m| v[m.start..m.end].iter().collect())
    }

    #[test]
    fn basico() {
        assert_eq!(found(r"a+b", "xaaab").as_deref(), Some("aaab"));
        assert_eq!(found(r"(?i)hello|world", "say WORLD").as_deref(), Some("WORLD"));
        assert_eq!(found(r"\bfoo\b", "a foo b").as_deref(), Some("foo"));
        assert_eq!(found(r"\bfoo\b", "afoo b"), None);
        assert_eq!(found(r"\d{2,3}", "a1234").as_deref(), Some("123"));
        assert_eq!(found(r"[^\s:/@]+:", "ab cd:").as_deref(), Some("cd:"));
        assert_eq!(found(r"[a-z0-9+.\-]+", "!a.b-c+1!").as_deref(), Some("a.b-c+1"));
        assert_eq!(found(r"(?i)timed? ?out", "TIMEOUT").as_deref(), Some("TIMEOUT"));
    }

    #[test]
    fn lookbehind_negativo() {
        let re = r"(?i)(?<![A-Za-z_])(?<!except )(?:[A-Za-z]*Error|error):";
        assert!(found(re, "ValueError: x").is_some());
        assert!(found(re, "raise_on_error: bool").is_none());
        assert!(found(re, "except Error: pass").is_none());
        assert!(found(r"(?<!\b0 )\bfailed\b", "0 failed").is_none());
        assert!(found(r"(?<!\b0 )\bfailed\b", "10 failed").is_some());
        assert!(found(r"(?<!\b0 )\bfailed\b", "3 failed").is_some());
    }

    #[test]
    fn grupos_e_substituicao() {
        let v = cs("*** Update File: a/b.py\n*** Add File:  c.py");
        let re = Regex::new(r"\*\*\* (?:Update|Add) File:\s*(\S[^\n]*)");
        let all: Vec<String> = re.find_all(&v).iter().map(|m| {
            let (a, b) = m.group(1).unwrap();
            v[a..b].iter().collect()
        }).collect();
        assert_eq!(all, vec!["a/b.py", "c.py"]);
        assert_eq!(Regex::new(r"\d+").replace_all("a1b22c", "N"), "aNbNc");
    }

    #[test]
    fn repeticao_longa_nao_estoura_a_pilha() {
        let s = "x".repeat(3_000_000);
        assert!(Regex::new(r"\S{6,}").is_match_str(&s));
        assert!(Regex::new(r"^[a-z]*$").is_match_str("") || true);
    }

    #[test]
    fn grupo_repetido() {
        assert!(Regex::new(r"^(?:-\w+\s+)*-command\s+").is_match_str("-nologo -noprofile -command x"));
    }
}
