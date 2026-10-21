"""Uso:
  python -m observador sessions ARQUIVO [--days 7]
  python -m observador scan CAMINHO... [--days 7] [--max-chars 6000]
  python -m observador discover [CAMINHO...]
"""

import argparse
import json
import sys
import time
from pathlib import Path

from . import extract, sessions

KNOWN_ROOTS = ["~/.codex", "~/.kimi", "~/.kimi-code", "~/.claude/projects"]
LOG_SUFFIXES = {".jsonl", ".json"}


def expand(paths):
    for p in paths:
        p = Path(p).expanduser()
        if p.is_dir():
            yield from (f for f in sorted(p.rglob("*")) if f.is_file() and f.suffix in LOG_SUFFIXES)
        elif p.is_file():
            yield p


def cmd_sessions(args):
    print(sessions.render(sessions.summarize(args.file, days=args.days)))


def cmd_scan(args):
    cutoff = time.time() - args.days * 86400
    total, parts, per_file = extract.Stats(), [], []
    for f in expand(args.paths):
        if f.stat().st_mtime < cutoff:
            continue
        st = extract.scan_records(extract.read_records(f))
        if st.records:
            total.merge(st)
            per_file.append((f.name, st))
            if st.errors or st.no_verify or st.secrets or st.corrections or st.ends_with_question:
                parts.append(extract.render(f.name, st))
    out = [extract.render("TOTAL", total)]
    if len(per_file) > 1:
        out += [extract.render_cross(per_file)] + parts
    state_path = getattr(args, "state", None)
    rules = getattr(args, "rules", None)
    if rules and per_file:
        try:
            viol = extract.rule_violations(Path(rules).read_text(encoding="utf-8", errors="replace"), per_file)
        except OSError:
            viol = []
        if viol:
            out.insert(min(2, len(out)), "\n".join(viol))
    if state_path and per_file:
        new = extract.summary(per_file)
        state = Path(state_path)
        if state.exists():
            try:
                out.insert(min(2, len(out)), extract.compare(json.loads(state.read_text(encoding="utf-8")), new))
            except ValueError:
                pass
        state.write_text(json.dumps(new, ensure_ascii=False, indent=1), encoding="utf-8")
    out = [o for o in out if o]
    text = "\n\n".join(out)
    print(text[: args.max_chars] + ("\n[cortado]" if len(text) > args.max_chars else ""))


def cmd_discover(args):
    """Só nomes, tamanhos e nomes de chaves. Nunca imprime conteúdo."""
    roots = args.paths or KNOWN_ROOTS
    for root in roots:
        r = Path(root).expanduser()
        if not r.exists():
            print(f"{root}: não existe")
            continue
        files = [f for f in r.rglob("*") if f.is_file()]
        print(f"{root}: {len(files)} arquivos")
        for f in sorted(files, key=lambda x: x.stat().st_mtime, reverse=True)[:15]:
            print(f"  {f.relative_to(r)}  {f.stat().st_size} bytes")
        sample = next((f for f in files if f.suffix == ".jsonl"), None)
        if sample:
            first = next(extract.read_jsonl(sample), None)
            if isinstance(first, dict):
                print(f"  chaves do 1º registro de {sample.name}: {sorted(first)}")


def main(argv=None):
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8")
    ap = argparse.ArgumentParser(prog="observador")
    sub = ap.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("sessions")
    s.add_argument("file")
    s.add_argument("--days", type=int, default=7)
    s.set_defaults(fn=cmd_sessions)
    c = sub.add_parser("scan")
    c.add_argument("paths", nargs="+")
    c.add_argument("--days", type=int, default=7)
    c.add_argument("--max-chars", type=int, default=6000)
    c.add_argument("--rules", help="CLAUDE.md ou AGENTS.md: aponta comandos que as sessões rodaram contra o que a regra proíbe")
    c.add_argument("--state", help="arquivo JSON com o resumo da semana anterior; é lido e depois atualizado")
    c.set_defaults(fn=cmd_scan)
    d = sub.add_parser("discover")
    d.add_argument("paths", nargs="*")
    d.set_defaults(fn=cmd_discover)
    args = ap.parse_args(argv)
    args.fn(args)
    return 0


if __name__ == "__main__":
    sys.exit(main())
