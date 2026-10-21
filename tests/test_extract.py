import json
import tempfile
import unittest
from datetime import datetime, timezone
from pathlib import Path

from observador import extract, sessions


def sample_records():
    return [
        {"role": "user", "content": "roda os testes"},
        {"role": "assistant", "content": "ok", "model": "gpt-x"},
        {"type": "tool", "command": "pytest -q", "output": "Traceback (most recent call last): boom"},
        {"type": "tool", "command": "pytest -q", "output": "FAILED tests/a.py"},
        {"type": "tool", "command": "pytest -q", "output": "FAILED tests/a.py"},
        {"type": "tool", "command": "git push --no-verify origin x", "output": "ok"},
        {"role": "user", "content": "errado, de novo isso"},
        {"role": "user", "content": "use Authorization: Bearer abcdefghijklmnop1234567890 no curl em 192.168.1.10"},
        {"role": "assistant", "content": "Quer que eu aplique a correção agora?"},
    ]


class ExtractTests(unittest.TestCase):
    def test_scan_finds_signals(self):
        st = extract.scan_records(sample_records())
        self.assertEqual(st.records, 9)
        self.assertGreaterEqual(st.errors, 3)
        self.assertEqual(st.no_verify, 1)
        self.assertGreaterEqual(st.secrets, 1)
        self.assertTrue(st.ends_with_question)
        self.assertEqual(st.commands["pytest -q"], 3)
        self.assertTrue(any("errado" in c for c in st.corrections))
        self.assertEqual(st.models["gpt-x"], 1)

    def test_render_never_leaks_secrets_or_ips(self):
        text = extract.render("x", extract.scan_records(sample_records()))
        self.assertNotIn("abcdefghijklmnop1234567890", text)
        self.assertNotIn("192.168.1.10", text)

    def test_read_jsonl_skips_bad_lines(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "a.jsonl"
            p.write_text('{"a": 1}\nnot json\n{"b": 2}\n')
            self.assertEqual(len(list(extract.read_jsonl(p))), 2)


class SessionsTests(unittest.TestCase):
    def test_summarize_filters_by_days_and_picks_stuck(self):
        data = {"ccr": {"data": [
            {"id": "s1", "title": "nova", "updated_at": "2026-10-19T10:00:00Z",
             "post_turn_summary": {"status_category": "need_input"},
             "session_context": {"model": "m1", "sources": [{"git_repository": {"url": "https://github.com/a/b"}}]}},
            {"id": "s2", "title": "velha", "updated_at": "2026-09-01T10:00:00Z",
             "post_turn_summary": {"status_category": "completed"}, "session_context": {"model": "m1"}},
        ]}}
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "x.txt"
            p.write_text("prefixo qualquer\n" + json.dumps(data))
            info = sessions.summarize(p, days=7, now=datetime(2026, 10, 20, tzinfo=timezone.utc))
        self.assertEqual(info["sessions"], 1)
        self.assertEqual(info["status"], {"need_input": 1})
        self.assertEqual(info["to_open"][0][0], "s1")


if __name__ == "__main__":
    unittest.main()
