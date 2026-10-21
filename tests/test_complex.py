"""Cenários complexos: semana mista com gabarito, formatos misturados, entradas hostis e comparação semanal."""

import json
import tempfile
import time
import unittest
from pathlib import Path

from observador import extract
from test_scenarios import CLEAN, claude, codex, kimi, scan, write_session


def edit_claude(path, n):
    msgs = []
    for i in range(n):
        msgs.append({"type": "assistant", "message": {"role": "assistant", "content": [
            {"type": "tool_use", "name": "Edit", "input": {"file_path": path, "new_string": f"v{i}"}}]}})
    return msgs


def edit_codex(path, n):
    patch = f"*** Begin Patch\n*** Update File: {path}\n@@\n-x\n+y\n*** End Patch"
    return [{"type": "response_item", "payload": {"type": "function_call", "name": "apply_patch",
                                                  "arguments": json.dumps({"input": patch})}} for _ in range(n)]


class Base(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)

    def tearDown(self):
        self._tmp.cleanup()


class ThrashTests(Base):
    def test_arquivo_editado_varias_vezes_em_formatos_diferentes(self):
        write_session(self.root, "a", claude(CLEAN) + edit_claude("C:\\proj\\src\\api\\handler.py", 6))
        write_session(self.root, "b", claude(CLEAN) + edit_codex("/home/u/proj/src/api/handler.py", 5))
        write_session(self.root, "c", claude(CLEAN) + edit_claude("/home/u/proj/README.md", 2))
        out = scan(self.root)
        self.assertIn("2 de 3 tiveram um arquivo editado 4 vezes ou mais", out)
        self.assertIn("arquivo muito editado em 2 sessões: `api/handler.py`", out.replace("src/api/handler.py", "api/handler.py"))

    def test_poucas_edicoes_nao_alarmam(self):
        write_session(self.root, "a", claude(CLEAN) + edit_claude("/p/a.py", 3))
        write_session(self.root, "b", claude(CLEAN) + edit_claude("/p/b.py", 3))
        self.assertNotIn("editado", scan(self.root))


class WeekWithGroundTruth(Base):
    """20 sessões com problemas plantados; a coleta tem que reproduzir as contagens exatas."""

    def build(self):
        stuck = [("user", "corrige"), ("assistant", "Pronto. Quer que eu aplique?")]
        killed = [("user", "roda"), ("bash", ("pytest tests -q", "Killed")), ("bash", ("pytest tests -q", "Killed")),
                  ("bash", ("pytest tests -q", "Killed"))]
        push = [("user", "sobe"), ("bash", ("git push --no-verify origin x", "ok"))]
        leak = [("user", "usa Bearer abcdefghijklmnopqrstuvwxyz0123 no curl")]
        angry = [("user", "faz"), ("assistant", "feito"), ("user", "não funcionou, continua dando erro, que merda")]
        plan = ([(claude, stuck)] * 4 + [(codex, stuck)] * 3 + [(claude, killed)] * 2 + [(codex, killed)] * 2 +
                [(kimi, push)] * 3 + [(claude, leak)] * 2 + [(codex, angry)] * 2)
        for i, (b, m) in enumerate(plan):
            write_session(self.root, f"w{i:02d}", b(m))
        for i in range(3):
            write_session(self.root, f"x{i}", claude(CLEAN) + edit_claude("/p/core.py", 5))
        for i in range(6):
            write_session(self.root, f"z{i}", claude(CLEAN))
        return len(plan) + 3 + 6

    def test_contagens_batem_com_o_gabarito(self):
        total = self.build()
        out = scan(self.root)
        self.assertIn(f"Entre os {total} arquivos", out)
        self.assertIn(f"7 de {total} terminaram com pergunta", out)
        self.assertIn(f"3 de {total} usaram --no-verify", out)
        self.assertIn(f"2 de {total} tiveram segredos", out)
        self.assertIn(f"2 de {total} tiveram correção", out)
        self.assertIn(f"3 de {total} tiveram um arquivo editado", out)
        self.assertIn("erro recorrente em 4 arquivos: killed", out)
        self.assertIn("arquivo muito editado em 3 sessões", out)
        self.assertNotIn("abcdefghijklmnopqrstuvwxyz0123", out)

    def test_escala_as_piores_sessoes(self):
        self.build()
        out = scan(self.root)
        line = [l for l in out.splitlines() if "para leitura completa" in l][0]
        self.assertTrue(any(name in line for name in ("w07", "w08", "w09", "w10", "w14", "w15")), line)

    def test_saida_cabe_no_limite(self):
        self.build()
        self.assertLessEqual(len(scan(self.root)), 8100)


class Hostile(Base):
    def test_entradas_hostis_nao_derrubam(self):
        (self.root / "vazio.jsonl").write_text("")
        (self.root / "brancos.jsonl").write_text("\n\n   \n")
        (self.root / "lixo.jsonl").write_bytes(b"\x00\xff\xfe{{{ not json \x80\n" * 50)
        (self.root / "truncado.jsonl").write_text('{"role": "user", "content": "oi"}\n{"role": "assi')
        (self.root / "tipos.jsonl").write_text('[1,2,3]\n42\n"texto"\nnull\ntrue\n{"role": 7}\n')
        nested = {}
        cur = nested
        for _ in range(60):
            cur["x"] = {}
            cur = cur["x"]
        cur["output"] = "Traceback (most recent call last)"
        (self.root / "fundo.jsonl").write_text(json.dumps(nested))
        (self.root / "crlf.jsonl").write_text('{"role": "user", "content": "errado, de novo"}\r\n{"role": "assistant", "content": "ok?"}\r\n')
        out = scan(self.root)
        self.assertIn("TOTAL", out)

    def test_registro_gigante(self):
        big = "x" * 2_000_000 + " Traceback (most recent call last) " + "y" * 1000 + " password=Segredo12345"
        write_session(self.root, "gigante", [{"role": "tool", "output": big}])
        t0 = time.time()
        out = scan(self.root)
        self.assertLess(time.time() - t0, 15)
        self.assertNotIn("Segredo12345", out)
        self.assertLess(len(out), 8100)

    def test_unicode(self):
        write_session(self.root, "u", [{"role": "user", "content": "isso está errado, de novo 😡 ação"}])
        self.assertIn("correção do usuário: isso está errado", scan(self.root))


class SecretVariants(Base):
    VARIANTS = [
        "AKIAIOSFODNN7EXAMPLE",
        "-----BEGIN RSA PRIVATE KEY-----",
        "postgres://admin:SenhaForte99@db.interno:5432/app",
        "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.abcdefghijklmnop",
        "ghp_abcdefghijklmnopqrstuvwxyz0123456789",
        "sk-abcdefghijklmnopqrstuvwxyz0123456789",
        "api_key = Zx9Qw8Er7Ty6Ui5",
    ]

    def test_nenhuma_variante_vaza(self):
        for i, v in enumerate(self.VARIANTS):
            write_session(self.root, f"s{i}", [{"role": "assistant", "content": f"falha: {v} Traceback (most recent call last)"}])
        out = scan(self.root)
        self.assertIn(f"{len(self.VARIANTS)} de {len(self.VARIANTS)} tiveram segredos", out)
        for v in self.VARIANTS:
            for piece in ("AKIAIOSFODNN7EXAMPLE", "PRIVATE KEY", "SenhaForte99", "db.interno", "abcdefghijklmnop",
                          "Zx9Qw8Er7Ty6Ui5"):
                if piece in v:
                    self.assertNotIn(piece, out, v)


class FalsePositives(Base):
    def test_texto_comum_nao_alarma(self):
        texts = ["Resultado: 3 passed, 0 failed", "def f(raise_on_error: bool = False): ...",
                 "vamos usar stop words na busca", "o usuário pediu para ignorar o erro de digitação",
                 "git stash para guardar e depois git push normal"]
        for i, t in enumerate(texts):
            write_session(self.root, f"t{i}", [{"role": "user" if i % 2 else "assistant", "content": t}])
        out = scan(self.root)
        for alarm in ("tiveram correção", "erro recorrente", "usaram --no-verify", "tiveram segredos"):
            self.assertNotIn(alarm, out)
        self.assertIn("sinais de erro: 0", out)


class Languages(Base):
    def test_correcoes_em_portugues_e_ingles(self):
        for i, t in enumerate(["that's not what I asked", "it's still failing", "isso não funcionou", "continua dando erro",
                               "porra, de novo?", "wtf this is wrong"]):
            write_session(self.root, f"l{i}", [{"role": "user", "content": t}, {"role": "assistant", "content": "feito"}])
        self.assertIn("6 de 6 tiveram correção", scan(self.root))


class NoiseGrouping(Base):
    def test_mesmo_erro_com_ruido_diferente_agrupa(self):
        variants = [
            "Traceback (most recent call last): ConnectionError: port 5432 refused at 2026-10-01T10:00:01Z id 3f2a9c1e-5b7d-4e8a-9c21-7d4e5f6a8b90",
            "Traceback (most recent call last): ConnectionError: port 5433 refused at 2026-10-02T11:30:09Z id 11111111-2222-3333-4444-555555555555",
            "Traceback (most recent call last): ConnectionError: port 6000 refused at 2026-10-03T08:15:44Z id aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
        ]
        for i, v in enumerate(variants):
            write_session(self.root, f"n{i}", [{"role": "tool", "output": v}])
        self.assertIn("erro recorrente em 3 arquivos", scan(self.root))


class WeeklyState(Base):
    def test_compara_com_a_semana_anterior(self):
        state = str(self.root / "estado" / "semana.json")
        Path(state).parent.mkdir()
        week1, week2 = self.root / "w1", self.root / "w2"
        week1.mkdir(), week2.mkdir()
        stuck = [("user", "x"), ("assistant", "Quer que eu aplique?")]
        killed = [("user", "x"), ("bash", ("make test", "Killed"))]
        push = [("user", "x"), ("bash", ("git push --no-verify", "ok"))]
        for i in range(3):
            write_session(week1, f"a{i}", claude(stuck))
            write_session(week1, f"k{i}", claude(killed))
        for i in range(3):
            write_session(week2, f"p{i}", claude(push))
            write_session(week2, f"k{i}", claude(killed))
        first = scan(week1, state=state)
        self.assertNotIn("Comparação", first)
        second = scan(week2, state=state)
        self.assertIn("Comparação com a semana anterior", second)
        self.assertIn("terminaram com pergunta do assistente: 3 de 6 antes, 0 de 6 agora", second)
        self.assertIn("usaram --no-verify: 0 de 6 antes, 3 de 6 agora", second)
        self.assertIn("continua (erro): killed", second)
        self.assertTrue(Path(state).exists())


if __name__ == "__main__":
    unittest.main()


class PainPointSignals(Base):
    """Sinais ligados às dores de quem usa agentes de IA: regras esquecidas, 'pronto' falso, compactação, custo."""

    def test_conclusao_afirmada_e_contestada(self):
        talk = [("user", "corrige o login"), ("assistant", "Pronto, corrigi o login e os testes passam."),
                ("user", "não funcionou, continua dando erro")]
        ok = [("user", "corrige o login"), ("assistant", "Pronto, corrigi o login."), ("user", "ótimo, agora o logout")]
        for i in range(3):
            write_session(self.root, f"c{i}", claude(talk))
        write_session(self.root, "ok", claude(ok))
        out = scan(self.root)
        self.assertIn("3 de 4 tiveram uma conclusão afirmada pela IA e contestada", out)

    def test_usuario_repetindo_a_mesma_instrucao(self):
        msgs = [("user", "use sempre português nas respostas"), ("assistant", "certo, vou usar"),
                ("user", "use sempre português nas respostas")]
        write_session(self.root, "r1", claude(msgs))
        write_session(self.root, "r2", claude(msgs))
        write_session(self.root, "r3", claude(CLEAN))
        self.assertIn("2 de 3 tiveram o usuário repetindo a mesma instrução", scan(self.root))

    def test_resultado_de_ferramenta_nao_conta_como_fala_do_usuario(self):
        # saída de comando contém "revert" e se repete; não é o usuário falando
        msgs = [("user", "faz o commit do arquivo"), ("bash", ("git log", "revert abc123 revert def456 desfaz tudo")),
                ("bash", ("git log", "revert abc123 revert def456 desfaz tudo"))]
        write_session(self.root, "t", claude(msgs))
        write_session(self.root, "t2", claude(msgs))
        out = scan(self.root)
        self.assertNotIn("tiveram correção", out)
        self.assertNotIn("repetindo", out)

    def test_compactacao_de_contexto(self):
        compact = claude(CLEAN) + [{"type": "user", "isCompactSummary": True, "message": {"role": "user", "content": "Resumo da conversa"}}]
        cont = claude(CLEAN) + [{"type": "user", "message": {"role": "user",
              "content": "This session is being continued from a previous conversation that ran out of context."}}]
        write_session(self.root, "k1", compact)
        write_session(self.root, "k2", cont)
        write_session(self.root, "k3", claude(CLEAN))
        self.assertIn("2 de 3 passaram por compactação de contexto", scan(self.root))

    def test_tipos_de_erro(self):
        msgs = [("user", "x"), ("bash", ("a", "bash: foo: command not found")), ("bash", ("b", "HTTP 500 Internal Server Error")),
                ("bash", ("c", "AssertionError: expected 3 got 4")), ("bash", ("d", "ValueError: invalid literal"))]
        write_session(self.root, "e1", claude(msgs))
        write_session(self.root, "e2", claude(msgs))
        out = scan(self.root)
        for kind in ("ambiente", "http", "teste", "codigo"):
            self.assertIn(kind, out.split("erros por tipo:")[-1].splitlines()[0])

    def test_tokens_somados(self):
        rec = {"type": "assistant", "message": {"role": "assistant", "content": "ok pronto aqui",
                                                  "usage": {"input_tokens": 100, "output_tokens": 50, "cache_read_input_tokens": 9999}}}
        write_session(self.root, "tk", [rec, rec])
        self.assertIn("tokens de entrada+saída (soma dos registros): 300", scan(self.root))

    def test_regras_do_claude_md_desrespeitadas(self):
        rules = self.root / "CLAUDE.md"
        rules.write_text("# Regras\n- Nunca use `git push --force` na main\n- Evite `rm -rf` fora de /tmp\n- Prefira testes pequenos\n")
        bad = [("user", "x"), ("bash", ("git push --force origin main", "ok"))]
        write_session(self.root, "b1", claude(bad))
        write_session(self.root, "b2", codex(bad))
        write_session(self.root, "b3", claude(CLEAN))
        out = scan(self.root, rules=str(rules))
        self.assertIn("Regras do projeto desrespeitadas", out)
        self.assertIn("`git push --force` em 2 sessões", out)
        self.assertNotIn("rm -rf", out.split("Regras do projeto desrespeitadas")[1].split("##")[0])


class SessionsCost(unittest.TestCase):
    def test_custo_por_status(self):
        from datetime import datetime, timezone
        from observador import sessions
        data = {"ccr": {"data": [
            {"id": "a", "title": "cara", "updated_at": "2026-10-20T10:00:00Z", "post_turn_summary": {"status_category": "need_input"},
             "external_metadata": {"usage": {"cost_usd": 12.5}}, "session_context": {"model": "m"}},
            {"id": "b", "title": "barata", "updated_at": "2026-10-20T10:00:00Z", "post_turn_summary": {"status_category": "completed"},
             "external_metadata": {"usage": {"cost_usd": 0.5}}, "session_context": {"model": "m"}},
            {"id": "c", "title": "sem custo", "updated_at": "2026-10-20T10:00:00Z", "session_context": {"model": "m"}},
        ]}}
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "x.txt"
            p.write_text(json.dumps(data))
            info = sessions.summarize(p, days=7, now=datetime(2026, 10, 21, tzinfo=timezone.utc))
        self.assertEqual(info["cost"], 13.0)
        self.assertEqual(info["cost_by_status"]["need_input"], 12.5)
        self.assertEqual(info["costly"][0][1], "cara")
        self.assertIn("US$ 13.0", sessions.render(info))
