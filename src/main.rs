//! Uso:
//!   observador sessions ARQUIVO [--days 7]
//!   observador scan CAMINHO... [--days 7] [--max-chars 6000] [--rules CLAUDE.md] [--state estado.json]
//!   observador discover [CAMINHO...]

mod extract;
mod json;
mod regex;
mod sessions;

use json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const KNOWN_ROOTS: [&str; 4] = ["~/.codex", "~/.kimi", "~/.kimi-code", "~/.claude/projects"];
const USAGE: &str = "uso:
  observador sessions ARQUIVO [--days 7]
  observador scan CAMINHO... [--days 7] [--max-chars 6000] [--rules CLAUDE.md] [--state estado.json]
  observador discover [CAMINHO...]";

fn now_secs() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

fn expand_user(p: &str) -> PathBuf {
    let home = || std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from);
    if p == "~" {
        return home().unwrap_or_else(|| PathBuf::from(p));
    }
    if let Some(rest) = p.strip_prefix("~/").or_else(|| p.strip_prefix("~\\")) {
        if let Some(h) = home() {
            return h.join(rest);
        }
    }
    PathBuf::from(p)
}

/// `Path.suffix` do Python.
fn suffix(name: &str) -> &str {
    match name.rfind('.') {
        Some(i) if i > 0 && i + 1 < name.len() => &name[i..],
        _ => "",
    }
}

/// Todos os arquivos de uma pasta, em ordem por componentes do caminho (como `sorted(Path.rglob)`).
fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let path = entry.path();
        match entry.file_type() {
            Ok(t) if t.is_dir() => walk(&path, out),
            _ => {
                if fs::metadata(&path).map_or(false, |m| m.is_file()) {
                    out.push(path);
                }
            }
        }
    }
}

fn files_under(root: &Path) -> Vec<PathBuf> {
    let mut v = Vec::new();
    walk(root, &mut v);
    v.sort_by(|a, b| a.components().cmp(b.components()));
    v
}

fn expand(paths: &[String]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for p in paths {
        let p = expand_user(p);
        if p.is_dir() {
            out.extend(files_under(&p).into_iter().filter(|f| {
                matches!(suffix(&f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()), ".jsonl" | ".json")
            }));
        } else if p.is_file() {
            out.push(p);
        }
    }
    out
}

fn mtime(p: &Path) -> f64 {
    fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0.0, |d| d.as_secs_f64())
}

struct Args {
    paths: Vec<String>,
    days: i64,
    max_chars: usize,
    rules: Option<String>,
    state: Option<String>,
}

fn parse_args(args: &[String], takes_paths: bool) -> Result<Args, String> {
    let mut a = Args { paths: vec![], days: 7, max_chars: 6000, rules: None, state: None };
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        let (name, inline) = match arg.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n.to_string(), Some(v.to_string())),
            _ => (arg.clone(), None),
        };
        let value = |i: &mut usize| -> Result<String, String> {
            if let Some(v) = inline.clone() {
                return Ok(v);
            }
            *i += 1;
            args.get(*i).cloned().ok_or_else(|| format!("{name} precisa de um valor"))
        };
        match name.as_str() {
            "--days" => a.days = value(&mut i)?.parse().map_err(|_| "--days precisa de um número inteiro".to_string())?,
            "--max-chars" => a.max_chars = value(&mut i)?.parse().map_err(|_| "--max-chars precisa de um número inteiro".to_string())?,
            "--rules" => a.rules = Some(value(&mut i)?),
            "--state" => a.state = Some(value(&mut i)?),
            n if n.starts_with("--") => return Err(format!("opção desconhecida: {n}")),
            _ if takes_paths => a.paths.push(arg.clone()),
            _ => return Err(format!("argumento inesperado: {arg}")),
        }
        i += 1;
    }
    Ok(a)
}

fn cmd_sessions(file: &str, days: i64) {
    let info = sessions::summarize(&expand_user(file), days, now_secs(), 8);
    println!("{}", sessions::render(&info));
}

fn cmd_scan(a: &Args) {
    let cutoff = now_secs() - a.days as f64 * 86400.0;
    let mut total = extract::Stats::default();
    let mut parts = Vec::new();
    let mut per_file: extract::PerFile = Vec::new();
    for f in expand(&a.paths) {
        if mtime(&f) < cutoff {
            continue;
        }
        let st = extract::scan_records(extract::read_records(&f));
        if st.records > 0 {
            let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            total.merge(&st);
            if st.errors > 0 || st.no_verify > 0 || st.secrets > 0 || !st.corrections.is_empty() || st.ends_with_question {
                parts.push(extract::render(&name, &st));
            }
            per_file.push((name, st));
        }
    }
    let mut out = vec![extract::render("TOTAL", &total)];
    if per_file.len() > 1 {
        out.push(extract::render_cross(&per_file));
        out.extend(parts);
    }
    if let (Some(rules), false) = (&a.rules, per_file.is_empty()) {
        if let Ok(bytes) = fs::read(expand_user(rules)) {
            let viol = extract::rule_violations(&String::from_utf8_lossy(&bytes), &per_file);
            if !viol.is_empty() {
                out.insert(2.min(out.len()), viol.join("\n"));
            }
        }
    }
    if let (Some(state), false) = (&a.state, per_file.is_empty()) {
        let new = extract::summary(&per_file);
        let state = expand_user(state);
        if state.exists() {
            let old = fs::read(&state).ok().and_then(|b| json::parse(&String::from_utf8_lossy(&b)));
            if let Some(old) = old {
                out.insert(2.min(out.len()), extract::compare(&old, &new));
            }
        }
        if let Err(e) = fs::write(&state, json::dumps_indent1(&new.to_value())) {
            eprintln!("não consegui gravar o estado em {}: {e}", state.display());
        }
    }
    let text = out.into_iter().filter(|o| !o.is_empty()).collect::<Vec<_>>().join("\n\n");
    if text.chars().count() > a.max_chars {
        println!("{}\n[cortado]", extract::head(&text, a.max_chars));
    } else {
        println!("{text}");
    }
}

/// Só nomes, tamanhos e nomes de chaves. Nunca imprime conteúdo.
fn cmd_discover(paths: &[String]) {
    let roots: Vec<String> = if paths.is_empty() { KNOWN_ROOTS.iter().map(|s| s.to_string()).collect() } else { paths.to_vec() };
    for root in roots {
        let r = expand_user(&root);
        if !r.exists() {
            println!("{root}: não existe");
            continue;
        }
        let files = files_under(&r);
        println!("{root}: {} arquivos", files.len());
        let mut by_time: Vec<&PathBuf> = files.iter().collect();
        by_time.sort_by(|a, b| mtime(b).partial_cmp(&mtime(a)).unwrap_or(std::cmp::Ordering::Equal));
        for f in by_time.iter().take(15) {
            let rel = f.strip_prefix(&r).unwrap_or(f);
            println!("  {}  {} bytes", rel.display(), fs::metadata(f).map_or(0, |m| m.len()));
        }
        let sample = files.iter().find(|f| f.extension().map_or(false, |e| e == "jsonl"));
        if let Some(sample) = sample {
            if let Some(Value::Obj(items)) = extract::read_jsonl(sample).next() {
                let mut keys: Vec<String> = items.iter().map(|(k, _)| json::py_repr_str(k)).collect();
                keys.sort();
                println!(
                    "  chaves do 1º registro de {}: [{}]",
                    sample.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                    keys.join(", ")
                );
            }
        }
    }
}

fn run() -> Result<(), String> {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let Some((cmd, rest)) = argv.split_first() else { return Err(USAGE.into()) };
    match cmd.as_str() {
        "-h" | "--help" | "help" => {
            println!("{USAGE}");
            Ok(())
        }
        "-V" | "--version" | "version" => {
            println!("observador {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        "sessions" => {
            let a = parse_args(rest, true)?;
            match a.paths.as_slice() {
                [file] => {
                    cmd_sessions(file, a.days);
                    Ok(())
                }
                _ => Err("sessions precisa de exatamente um ARQUIVO".into()),
            }
        }
        "scan" => {
            let a = parse_args(rest, true)?;
            if a.paths.is_empty() {
                return Err("scan precisa de pelo menos um CAMINHO".into());
            }
            cmd_scan(&a);
            Ok(())
        }
        "discover" => {
            let a = parse_args(rest, true)?;
            cmd_discover(&a.paths);
            Ok(())
        }
        other => Err(format!("comando desconhecido: {other}\n{USAGE}")),
    }
}

fn main() {
    // pilha grande: registros aninhados e padrões com backtracking não podem derrubar o processo
    let worker = std::thread::Builder::new().stack_size(256 * 1024 * 1024).spawn(run).expect("thread");
    match worker.join() {
        Ok(Ok(())) => {}
        Ok(Err(msg)) => {
            eprintln!("{msg}");
            std::process::exit(2);
        }
        Err(_) => std::process::exit(1),
    }
}
