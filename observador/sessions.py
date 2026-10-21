"""Estatísticas das sessões do Claude Code a partir da resposta salva de list_sessions."""

from collections import Counter
from datetime import datetime, timedelta, timezone

from .extract import load_dump

BIG_CONTEXT = 300_000


def parse_ts(value):
    return datetime.fromisoformat(value.replace("Z", "+00:00"))


def summarize(path, days=7, now=None, max_open=8):
    now = now or datetime.now(timezone.utc)
    cutoff = now - timedelta(days=days)
    sessions = [
        s for s in load_dump(path)
        if isinstance(s, dict) and s.get("updated_at") and parse_ts(s["updated_at"]) >= cutoff
    ]
    status, models, repos = Counter(), Counter(), Counter()
    cost, cost_by_status, costly = 0.0, Counter(), []
    to_open = []
    for s in sessions:
        post = s.get("post_turn_summary") or {}
        cat = post.get("status_category") or "sem_resumo"
        status[cat] += 1
        ctx = s.get("session_context") or {}
        models[ctx.get("model") or "?"] += 1
        for src in ctx.get("sources") or []:
            url = (src.get("git_repository") or {}).get("url", "")
            if url:
                repos[url.split("github.com/")[-1]] += 1
        usd = ((s.get("external_metadata") or {}).get("usage") or {}).get("cost_usd") or 0
        cost += usd
        cost_by_status[cat] += usd
        costly.append((usd, s.get("title", "")[:50], cat))
        used = ((s.get("external_metadata") or {}).get("context_usage") or {}).get("used_tokens") or 0
        if cat in ("need_input", "failed") or used > BIG_CONTEXT:
            to_open.append((0 if cat in ("need_input", "failed") else 1, s["id"], s.get("title", "")[:60], cat, used))
    to_open.sort()
    return {
        "days": days,
        "sessions": len(sessions),
        "status": dict(status),
        "models": dict(models),
        "repos": dict(repos.most_common(5)),
        "to_open": [t[1:] for t in to_open[:max_open]],
        "cost": round(cost, 2),
        "cost_by_status": {k: round(v, 2) for k, v in cost_by_status.items() if v},
        "costly": [(round(u, 2), t, c) for u, t, c in sorted(costly, reverse=True)[:3] if u],
    }


def render(info):
    lines = [
        f"## Sessões dos últimos {info['days']} dias: {info['sessions']}",
        "- status: " + ", ".join(f"{k} {v}" for k, v in info["status"].items()),
        "- modelos: " + ", ".join(f"{k} {v}" for k, v in info["models"].items()),
        "- repositórios: " + ", ".join(f"{k} {v}" for k, v in info["repos"].items()),
    ]
    if info.get("cost"):
        lines.append(f"- custo estimado das sessões que informam custo (equivalente em API, não é cobrança): US$ {info['cost']}; por status: "
                     + ", ".join(f"{k} {v}" for k, v in info["cost_by_status"].items()))
        lines += [f"  - mais cara: US$ {u} | {t} | {c}" for u, t, c in info["costly"]]
    lines.append("- abrir a transcrição de:")
    lines += [f"  - {sid} | {title} | {cat} | contexto {used}" for sid, title, cat, used in info["to_open"]]
    return "\n".join(lines)
