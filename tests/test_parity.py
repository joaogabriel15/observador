"""Paridade entre o Python (referência) e o binário em Rust, com registros gerados ao acaso.

Só roda com `OBSERVADOR_BIN=caminho/do/binario python -m unittest discover -s tests`.
"""

import json
import os
import random
import tempfile
import unittest
from pathlib import Path

from test_scenarios import scan, write_session

PIECES = [
    "Traceback (most recent call last):\n  File \"/home/u/app.py\", line 12, in main\nValueError: valor 123 invalido",
    "error: linking failed", "FAILED tests/test_a.py::test_x - AssertionError", "exit code 2", "HTTP 503 Service Unavailable",
    "Killed", "command timed out after 30s", "0 failed, 12 passed", "3 failed, 9 passed", "def f(raise_on_error: bool): pass",
    "except Error: pass", "fatal: not a git repository", "permission denied: /etc/x", "ECONNRESET connection reset",
    "password=Hunter22222", "Bearer abcdefghijklmnop123456", "ghp_abcdefghijklmnopqrstuv1234", "sk-abcdefghijklmnopqrstuvwx",
    "postgres://user:passw0rd@10.0.0.5:5432/db", "servidor 192.168.0.1 e 10.1.2.3", "AKIAABCDEFGHIJKLMNOP",
    "-----BEGIN RSA PRIVATE KEY-----", "eyJhbGciOiJIUzI1.eyJzdWIiOiIxMjM0NTY.assinatura_x",
    "errado, de novo isso", "já falei que não era isso", "wtf isso não funciona", "still failing after the change",
    "that's not what I asked", "continua dando erro", "ainda não funciona aqui", "pronto, tudo certo", "Done. All tests passed",
    "implementei e corrigi", "Quer que eu continue?", "Posso aplicar a migração agora?", "ação 😡 çãõ", "texto comum sem nada",
    "git push --no-verify origin main", "bash -lc 'cd /x/y && pytest -q tests/unit'", "cmd /c npm test", "make build",
    "/home/user/proj/src/main.rs", "C:\\Users\\joao\\proj\\src\\lib.rs", "*** Update File: src/a.rs\n@@\n-x\n+y",
    "This session is being continued from a previous conversation that ran out of context.",
    "uma frase longa o bastante para virar texto de usuário sem ser curta demais",
    "a" * 30, "token: abc", "api_key = sk_live_0123456789abcdef", "HTTP/1.1 404 Not Found", "status code 500",
    "ERROR: BOOM", "Permission Denied", "ERRADO!", "Não Era Isso", "İstanbul error: x", "\u212a error: kelvin", "ＦＡＩＬＥＤ",
    "٣ failed", "e\u0301rror: combinado", "\u00a0nbsp\u00a0failed\u2003", "tab\tfailed\x1c", "WTF!!", "Ação: Feito.", "CONCLUÍDO",
    "Error: ENOENT no such file", "TypeError: undefined is not a function at 0xdeadbeef", "KeyError: 'abc'",
]
COMMANDS = ["pytest -q", "bash -lc 'cd /w && cargo test'", "git status", "npm run build", "powershell -NoProfile -Command ls",
            "ls -la /tmp/x", "git push --no-verify", "make test"]
MODELS = ["claude-x", "gpt-y", "k3"]


def text(rng):
    return " ".join(rng.choice(PIECES) for _ in range(rng.randint(1, 3)))


def record(rng):
    kind = rng.randint(0, 11)
    if kind == 0:
        return {"role": "user", "content": text(rng)}
    if kind == 1:
        return {"type": "assistant", "message": {"role": "assistant", "model": rng.choice(MODELS),
                                                 "content": [{"type": "text", "text": text(rng)}]}}
    if kind == 2:
        return {"type": "assistant", "message": {"role": "assistant", "content": [
            {"type": "tool_use", "name": "Bash", "input": {"command": rng.choice(COMMANDS)}}]}}
    if kind == 3:
        return {"type": "user", "message": {"role": "user", "content": [{"type": "tool_result", "content": text(rng)}]}}
    if kind == 4:
        return {"type": "assistant", "message": {"role": "assistant", "content": [
            {"type": "tool_use", "name": rng.choice(["Edit", "Write", "apply_patch"]),
             "input": {"file_path": rng.choice(["/a/b/c.py", "C:\\x\\y\\z.rs", "src/m.rs"])}}]}}
    if kind == 5:
        return {"type": "response_item", "payload": {"type": "function_call", "name": "shell", "arguments": json.dumps(
            {"command": ["bash", "-lc", rng.choice(COMMANDS)]})}}
    if kind == 6:
        return {"type": "response_item", "payload": {"type": "function_call_output", "output": json.dumps(
            {"output": text(rng), "metadata": {"exit_code": rng.choice([0, 0, 1, 2, 137])}})}}
    if kind == 7:
        return {"role": rng.choice(["user", "assistant", "tool", "agent", "human"]), "content": text(rng),
                "usage": {"input_tokens": rng.randint(0, 5000), "output_tokens": rng.randint(0, 900)}}
    if kind == 8:
        return {"type": "response_item", "payload": {"type": "function_call", "name": "apply_patch", "arguments": json.dumps(
            {"input": f"*** Begin Patch\n*** Update File: {rng.choice(['a.rs', 'src/b.rs'])}\n@@\n-x\n+y\n*** End Patch"})}}
    if kind == 9:
        return {"isCompactSummary": True, "type": "user", "message": {"role": "user", "content": text(rng)}}
    if kind == 10:
        return rng.choice([[1, 2], 42, "texto solto " + text(rng), None, True, {"role": 7}])
    return {"x": {"y": [{"output": text(rng)}, {"cmd": rng.choice(COMMANDS)}]}, "model": rng.choice(MODELS)}


@unittest.skipUnless(os.environ.get("OBSERVADOR_BIN"), "defina OBSERVADOR_BIN para comparar com o Rust")
class Paridade(unittest.TestCase):
    def test_registros_aleatorios(self):
        rng = random.Random(int(os.environ.get("OBSERVADOR_FUZZ_SEED", 20261021)))
        for round_ in range(int(os.environ.get("OBSERVADOR_FUZZ_ROUNDS", 40))):
            with tempfile.TemporaryDirectory() as d:
                root = Path(d)
                for i in range(rng.randint(1, 7)):
                    write_session(root, f"s{i}", [record(rng) for _ in range(rng.randint(1, 60))])
                rules = root / "CLAUDE.md"
                rules.write_text("- Nunca use `--no-verify`.\n- Não use `make build` aqui.\n- evite `git status`\n")
                out = scan(root, rules=str(rules), max_chars=1_000_000)
                self.assertIn("TOTAL", out, round_)

    def test_estado_semanal_aleatorio(self):
        rng = random.Random(7)
        with tempfile.TemporaryDirectory() as d:
            state = str(Path(d) / "s" / "estado.json")
            (Path(d) / "s").mkdir()
            for week in range(4):
                root = Path(d) / f"w{week}"
                root.mkdir()
                for i in range(rng.randint(2, 6)):
                    write_session(root, f"s{i}", [record(rng) for _ in range(rng.randint(5, 50))])
                scan(root, state=state, max_chars=1_000_000)

    def test_sessoes_aleatorias(self):
        import subprocess
        from datetime import datetime, timedelta, timezone
        from observador import sessions
        rng = random.Random(3)
        now = datetime.now(timezone.utc)
        stamps = [lambda d: d.isoformat().replace("+00:00", "Z"), lambda d: d.isoformat(),
                  lambda d: d.strftime("%Y-%m-%dT%H:%M:%S.%f")[:-3] + "+00:00", lambda d: d.strftime("%Y-%m-%d %H:%M:%S+0000")]
        data = []
        for i in range(60):
            when = now - timedelta(days=rng.uniform(0, 12), hours=rng.randint(0, 23))
            s = {"id": f"id{i:02d}", "title": rng.choice(["corrige bug", "refatora módulo longo " * 4, "ação ç"]),
                 "updated_at": rng.choice(stamps)(when),
                 "session_context": {"model": rng.choice(["m1", "m2", None]),
                                     "sources": [{"git_repository": {"url": f"https://github.com/o/r{rng.randint(1, 8)}"}}]}}
            if rng.random() < 0.8:
                s["post_turn_summary"] = {"status_category": rng.choice(["need_input", "completed", "failed", "working"])}
            if rng.random() < 0.6:
                s["external_metadata"] = {"usage": {"cost_usd": round(rng.uniform(0, 30), rng.randint(0, 4))},
                                          "context_usage": {"used_tokens": rng.randint(0, 600_000)}}
            data.append(s)
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "dump.txt"
            p.write_text("prefixo qualquer\n" + json.dumps({"ccr": {"data": data}}))
            py = sessions.render(sessions.summarize(p, days=7))
            rust = subprocess.run([os.environ["OBSERVADOR_BIN"], "sessions", str(p), "--days", "7"],
                                  capture_output=True, encoding="utf-8", check=True).stdout
        self.assertEqual(rust, py + "\n")


if __name__ == "__main__":
    unittest.main()
