"""Cenários sintéticos: a coleta precisa notar problemas repetidos e não inventar problemas."""

import contextlib
import io
import json
import os
import subprocess
import tempfile
import time
import unittest
from pathlib import Path

from observador import __main__ as cli


def claude(msgs):
    """Registros no estilo das transcrições do Claude Code (mensagem + blocos de conteúdo)."""
    out = []
    for kind, body in msgs:
        if kind == "user":
            out.append({"type": "user", "message": {"role": "user", "content": body}})
        elif kind == "assistant":
            out.append({"type": "assistant", "message": {"role": "assistant", "model": "claude-x",
                                                         "content": [{"type": "text", "text": body}]}})
        elif kind == "bash":
            cmd, result = body
            out.append({"type": "assistant", "message": {"role": "assistant", "content": [
                {"type": "tool_use", "name": "Bash", "input": {"command": cmd}}]}})
            out.append({"type": "user", "message": {"role": "user", "content": [
                {"type": "tool_result", "content": result}]}})
    return out


def codex(msgs):
    """Estilo Codex: argumentos como texto JSON e códigos de saída em metadados (suposição, não verificada)."""
    out = []
    for kind, body in msgs:
        if kind == "user":
            out.append({"type": "response_item", "payload": {
                "type": "message", "role": "user", "content": [{"type": "input_text", "text": body}]}})
        elif kind == "assistant":
            out.append({"type": "response_item", "payload": {
                "type": "message", "role": "assistant", "content": [{"type": "output_text", "text": body}]}})
        elif kind == "bash":
            cmd, result = body
            code = 1 if any(w in result.lower() for w in ("error", "failed", "killed")) else 0
            out.append({"type": "response_item", "payload": {
                "type": "function_call", "name": "shell",
                "arguments": json.dumps({"command": ["bash", "-lc", cmd]})}})
            out.append({"type": "response_item", "payload": {
                "type": "function_call_output",
                "output": json.dumps({"output": result, "metadata": {"exit_code": code}})}})
    return out


def kimi(msgs):
    """Estilo Kimi CLI: eventos de fio com role e chamadas de ferramenta (suposição, não verificada)."""
    out = []
    for kind, body in msgs:
        if kind in ("user", "assistant"):
            out.append({"role": kind, "content": body})
        elif kind == "bash":
            cmd, result = body
            out.append({"role": "assistant", "tool_calls": [
                {"function": {"name": "Shell", "arguments": json.dumps({"command": cmd})}}]})
            out.append({"role": "tool", "content": result})
    return out


def write_session(root, name, records):
    path = Path(root) / f"{name}.jsonl"
    path.write_text("\n".join(json.dumps(r) for r in records), encoding="utf-8")


def scan(root, state=None, rules=None, max_chars=8000):
    args = type("A", (), {"paths": [str(root)], "days": 7, "max_chars": max_chars, "state": state, "rules": rules})()
    before = Path(state).read_bytes() if state and Path(state).exists() else None
    buf = io.StringIO()
    with contextlib.redirect_stdout(buf):
        cli.cmd_scan(args)
    out = buf.getvalue()
    binary = os.environ.get("OBSERVADOR_BIN")
    if binary:
        # com o binário em Rust compilado, todo cenário vira também um teste de paridade com o Python
        with tempfile.TemporaryDirectory() as d:
            cmd = [binary, "scan", str(root), "--days", "7", "--max-chars", str(max_chars)]
            rust_state = Path(d) / "estado.json"
            if state:
                if before is not None:
                    rust_state.write_bytes(before)
                cmd += ["--state", str(rust_state)]
            if rules:
                cmd += ["--rules", str(rules)]
            rust = subprocess.run(cmd, capture_output=True, encoding="utf-8", check=True).stdout
            assert rust == out, f"Rust diferente do Python\n--- python\n{out}\n--- rust\n{rust}"
            if state and Path(state).exists():
                assert rust_state.read_bytes() == Path(state).read_bytes(), "estado gravado difere entre Rust e Python"
    return out


CLEAN = [("user", "adicione um teste"), ("bash", ("pytest -q", "3 passed")), ("assistant", "Pronto, testes passando.")]


class ScenarioTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)

    def tearDown(self):
        self._tmp.cleanup()

    def fill(self, builders):
        for i, (build, msgs) in enumerate(builders):
            write_session(self.root, f"s{i}", build(msgs))

    def test_sessoes_travadas_em_pergunta(self):
        stuck = [("user", "corrige o bug"), ("assistant", "Corrigi. Quer que eu aplique no banco?")]
        self.fill([(claude, stuck)] * 6 + [(claude, CLEAN)] * 2)
        self.assertIn("6 de 8 terminaram com pergunta", scan(self.root))

    def test_testes_mortos_repetidos(self):
        killed = [("user", "roda tudo"), ("bash", ("pytest tests/unit -q", "Killed")),
                  ("bash", ("pytest tests/unit -q", "Killed")), ("bash", ("pytest tests/unit -q", "Killed"))]
        self.fill([(claude, killed)] * 3 + [(codex, killed)])
        out = scan(self.root)
        self.assertRegex(out, r"erro recorrente em 4 arquivos")
        self.assertIn("na mesma sessão, em 4 arquivos", out)
        self.assertIn("pytest tests/unit -q", out)

    def test_no_verify_em_varias_ferramentas(self):
        push = [("user", "sobe"), ("bash", ("git push --no-verify origin x", "ok"))]
        self.fill([(claude, push), (codex, push), (kimi, push)])
        self.assertIn("3 de 3 usaram --no-verify", scan(self.root))

    def test_segredos_nunca_aparecem(self):
        leak = [("user", "usa a senha: password=SuperSecreta123 e o token Bearer abcdefghijklmnopqrstuv no 10.1.2.3")]
        self.fill([(claude, leak), (codex, leak), (kimi, CLEAN)])
        out = scan(self.root)
        self.assertIn("2 de 3 tiveram segredos", out)
        for forbidden in ("SuperSecreta123", "abcdefghijklmnopqrstuv", "10.1.2.3"):
            self.assertNotIn(forbidden, out)

    def test_correcoes_e_irritacao(self):
        angry = [("user", "faz o deploy"), ("assistant", "feito"), ("user", "errado, de novo isso, já falei")]
        self.fill([(claude, angry), (kimi, angry), (codex, CLEAN)])
        self.assertIn("2 de 3 tiveram correção", scan(self.root))

    def test_cenario_limpo_nao_gera_alarme(self):
        self.fill([(claude, CLEAN)] * 3 + [(codex, CLEAN), (kimi, CLEAN)])
        out = scan(self.root)
        for alarm in ("terminaram com pergunta", "usaram --no-verify", "tiveram correção",
                      "tiveram segredos", "erro recorrente", "na mesma sessão"):
            self.assertNotIn(alarm, out)

    def test_erro_com_numeros_diferentes_agrupa(self):
        def boom(n):
            return [("user", "roda"), ("bash", ("make", f"Traceback (most recent call last): timeout after {n}s em /tmp/run{n}/x.py"))]
        self.fill([(claude, boom(3)), (claude, boom(40)), (codex, boom(500))])
        self.assertIn("erro recorrente em 3 arquivos", scan(self.root))

    def test_exit_code_nos_metadados_do_codex(self):
        fail = [("user", "build"), ("bash", ("npm run build", "build error: boom"))]
        self.fill([(codex, fail), (codex, fail)])
        self.assertIn("erro recorrente em 2 arquivos", scan(self.root))

    def test_arquivo_grande_e_rapido(self):
        records = claude(CLEAN) * 20_000
        write_session(self.root, "grande", records)
        t0 = time.time()
        scan(self.root)
        self.assertLess(time.time() - t0, 20)

    def test_saida_curta_com_muitos_arquivos(self):
        noisy = [("user", "x"), ("bash", (f"cmd", "Traceback (most recent call last): erro unico"))]
        self.fill([(claude, noisy)] * 60)
        out = scan(self.root)
        self.assertLessEqual(len(out), 8100)


if __name__ == "__main__":
    unittest.main()
