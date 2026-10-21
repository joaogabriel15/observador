//! Estatísticas das sessões do Claude Code a partir da resposta salva de `list_sessions`.

use crate::extract::{head, load_dump, Counter};
use crate::json::{py_float, Value};
use std::path::Path;

const BIG_CONTEXT: i64 = 300_000;

/// Segundos desde 1970 de um timestamp ISO 8601 (aceita `Z`, `±HH:MM`, `±HHMM`; sem fuso vale UTC).
pub fn parse_ts(value: &str) -> Option<f64> {
    let b = value.as_bytes();
    let num = |a: usize, n: usize| -> Option<i64> {
        let s = value.get(a..a + n)?;
        s.bytes().all(|c| c.is_ascii_digit()).then(|| s.parse().ok()).flatten()
    };
    let (y, mo, d) = (num(0, 4)?, num(5, 2)?, num(8, 2)?);
    if b.get(4) != Some(&b'-') || b.get(7) != Some(&b'-') || !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return None;
    }
    let (mut h, mut mi, mut s, mut frac) = (0, 0, 0, 0.0);
    let mut i = 10;
    if i < b.len() && (b[i] == b'T' || b[i] == b't' || b[i] == b' ') {
        h = num(i + 1, 2)?;
        i += 3;
        if b.get(i) == Some(&b':') {
            mi = num(i + 1, 2)?;
            i += 3;
            if b.get(i) == Some(&b':') {
                s = num(i + 1, 2)?;
                i += 3;
                if matches!(b.get(i), Some(b'.') | Some(b',')) {
                    let st = i + 1;
                    i = st;
                    while i < b.len() && b[i].is_ascii_digit() {
                        i += 1;
                    }
                    frac = format!("0.{}", &value[st..i]).parse().ok()?;
                }
            }
        }
    }
    let mut offset = 0i64;
    if i < b.len() {
        match b[i] {
            b'Z' | b'z' => i += 1,
            b'+' | b'-' => {
                let sign = if b[i] == b'-' { -1 } else { 1 };
                let oh = num(i + 1, 2)?;
                let mut j = i + 3;
                let mut om = 0;
                if b.get(j) == Some(&b':') {
                    j += 1;
                }
                if j + 2 <= b.len() {
                    om = num(j, 2).unwrap_or(0);
                    j += 2;
                }
                offset = sign * (oh * 3600 + om * 60);
                i = j;
            }
            _ => return None,
        }
    }
    if i != b.len() || h > 24 || mi > 59 || s > 59 {
        return None;
    }
    // dias desde 1970 (algoritmo de Howard Hinnant)
    let yy = if mo <= 2 { y - 1 } else { y };
    let era = yy.div_euclid(400);
    let yoe = yy - era * 400;
    let doy = (153 * (if mo > 2 { mo - 3 } else { mo + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some((days * 86400 + h * 3600 + mi * 60 + s - offset) as f64 + frac)
}

pub struct Info {
    pub days: i64,
    pub sessions: usize,
    pub status: Counter,
    pub models: Counter,
    pub repos: Vec<(String, i64)>,
    pub to_open: Vec<(String, String, String, i64)>,
    pub cost: f64,
    pub cost_by_status: Vec<(String, f64)>,
    pub costly: Vec<(f64, String, String)>,
}

fn round2(x: f64) -> f64 {
    format!("{x:.2}").parse().unwrap_or(x)
}

fn path_f64(s: &Value, keys: &[&str]) -> f64 {
    let mut cur = Some(s);
    for k in keys {
        cur = cur.and_then(|v| v.get(k));
    }
    cur.and_then(Value::as_f64).unwrap_or(0.0)
}

pub fn summarize(path: &Path, days: i64, now: f64, max_open: usize) -> Info {
    let cutoff = now - days as f64 * 86400.0;
    let all = load_dump(path);
    let sessions: Vec<&Value> = all
        .iter()
        .filter(|s| matches!(s, Value::Obj(_)))
        .filter(|s| s.get("updated_at").and_then(Value::as_str).and_then(parse_ts).map_or(false, |t| t >= cutoff))
        .collect();
    let mut status = Counter::default();
    let mut models = Counter::default();
    let mut repos = Counter::default();
    let mut cost = 0.0;
    let mut cost_by_status: Vec<(String, f64)> = Vec::new();
    let mut costly: Vec<(f64, String, String)> = Vec::new();
    let mut to_open: Vec<(i64, String, String, String, i64)> = Vec::new();
    for s in &sessions {
        let cat = s
            .get("post_turn_summary")
            .and_then(|p| p.get("status_category"))
            .and_then(Value::as_str)
            .filter(|c| !c.is_empty())
            .unwrap_or("sem_resumo")
            .to_string();
        status.add(&cat, 1);
        let ctx = s.get("session_context");
        let model = ctx.and_then(|c| c.get("model")).and_then(Value::as_str).filter(|m| !m.is_empty()).unwrap_or("?");
        models.add(model, 1);
        if let Some(Value::Arr(sources)) = ctx.and_then(|c| c.get("sources")) {
            for src in sources {
                let url = src.get("git_repository").and_then(|g| g.get("url")).and_then(Value::as_str).unwrap_or("");
                if !url.is_empty() {
                    repos.add(url.split("github.com/").last().unwrap_or(""), 1);
                }
            }
        }
        let usd = path_f64(s, &["external_metadata", "usage", "cost_usd"]);
        cost += usd;
        match cost_by_status.iter_mut().find(|(k, _)| *k == cat) {
            Some(slot) => slot.1 += usd,
            None => cost_by_status.push((cat.clone(), usd)),
        }
        let title = s.get("title").and_then(Value::as_str).unwrap_or("");
        costly.push((usd, head(title, 50), cat.clone()));
        let used = path_f64(s, &["external_metadata", "context_usage", "used_tokens"]) as i64;
        let stuck = cat == "need_input" || cat == "failed";
        if stuck || used > BIG_CONTEXT {
            let id = s.get("id").and_then(Value::as_str).unwrap_or("").to_string();
            to_open.push((if stuck { 0 } else { 1 }, id, head(title, 60), cat.clone(), used));
        }
    }
    to_open.sort();
    costly.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    Info {
        days,
        sessions: sessions.len(),
        status,
        models,
        repos: repos.most_common(5),
        to_open: to_open.into_iter().take(max_open).map(|t| (t.1, t.2, t.3, t.4)).collect(),
        cost: round2(cost),
        cost_by_status: cost_by_status.into_iter().filter(|(_, v)| *v != 0.0).map(|(k, v)| (k, round2(v))).collect(),
        costly: costly.into_iter().take(3).filter(|(u, _, _)| *u != 0.0).map(|(u, t, c)| (round2(u), t, c)).collect(),
    }
}

pub fn render(info: &Info) -> String {
    let join = |items: Vec<(String, i64)>| items.iter().map(|(k, v)| format!("{k} {v}")).collect::<Vec<_>>().join(", ");
    let mut lines = vec![
        format!("## Sessões dos últimos {} dias: {}", info.days, info.sessions),
        format!("- status: {}", join(info.status.in_order())),
        format!("- modelos: {}", join(info.models.in_order())),
        format!("- repositórios: {}", join(info.repos.clone())),
    ];
    if info.cost != 0.0 {
        let by: Vec<String> = info.cost_by_status.iter().map(|(k, v)| format!("{k} {}", py_float(*v))).collect();
        lines.push(format!(
            "- custo estimado das sessões que informam custo (equivalente em API, não é cobrança): US$ {}; por status: {}",
            py_float(info.cost),
            by.join(", ")
        ));
        for (u, t, c) in &info.costly {
            lines.push(format!("  - mais cara: US$ {} | {t} | {c}", py_float(*u)));
        }
    }
    lines.push("- abrir a transcrição de:".into());
    for (sid, title, cat, used) in &info.to_open {
        lines.push(format!("  - {sid} | {title} | {cat} | contexto {used}"));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_iso() {
        assert_eq!(parse_ts("1970-01-02T00:00:00Z"), Some(86400.0));
        assert_eq!(parse_ts("1970-01-01T01:00:00+01:00"), Some(0.0));
        assert_eq!(parse_ts("2026-10-20T10:00:00.5Z"), Some(1792490400.5));
        assert_eq!(parse_ts("lixo"), None);
    }

    #[test]
    fn filtra_por_dias_e_escolhe_as_travadas() {
        let dir = std::env::temp_dir().join(format!("obs-s-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("x.txt");
        std::fs::write(
            &p,
            r#"prefixo {"ccr": {"data": [
              {"id": "s1", "title": "nova", "updated_at": "2026-10-19T10:00:00Z", "post_turn_summary": {"status_category": "need_input"},
               "external_metadata": {"usage": {"cost_usd": 12.5}}, "session_context": {"model": "m1"}},
              {"id": "s2", "title": "velha", "updated_at": "2026-09-01T10:00:00Z", "post_turn_summary": {"status_category": "completed"}}
            ]}}"#,
        )
        .unwrap();
        let now = parse_ts("2026-10-20T00:00:00Z").unwrap();
        let info = summarize(&p, 7, now, 8);
        assert_eq!(info.sessions, 1);
        assert_eq!(info.to_open[0].0, "s1");
        assert_eq!(info.cost, 12.5);
        assert!(render(&info).contains("US$ 12.5"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
