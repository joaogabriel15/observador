# Observador

Responda em português. Este repositório condensa logs de agentes de código em resumos curtos para um relatório semanal.

## Regras

- Gastar poucos tokens: processe logs com `observador` (ou `python -m observador`), nunca cole logs brutos no contexto.
- Nunca imprimir senhas, tokens, IPs de servidores ou outros segredos, nem em resumos, testes ou commits. O mascaramento está em `observador/extract.py`; qualquer novo formato de saída passa por `redact`.
- A versão principal é em Rust, sem dependências (`Cargo.toml` sem `[dependencies]`; compila offline). `observador/` (Python, só biblioteca padrão) é a referência de comportamento. Testes: `cargo test` e `OBSERVADOR_BIN=target/release/observador python -m unittest discover -s tests` (compara Rust e Python byte a byte).
- Ao mexer nas heurísticas, mude o Rust e o Python juntos e adicione um caso em `tests/test_extract.py` (e no módulo `tests` do Rust, se for o caso). Se mexer em `src/regex.rs`, rode a paridade com mais rodadas: `OBSERVADOR_FUZZ_ROUNDS=300 OBSERVADOR_FUZZ_SEED=1`.
- O formato dos logs do Codex e do Kimi CLI ainda não foi confirmado com arquivos reais; não afirme que funciona neles antes de testar.
