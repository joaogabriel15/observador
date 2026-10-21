# Observador

Responda em português. Este repositório condensa logs de agentes de código em resumos curtos para um relatório semanal.

## Regras

- Gastar poucos tokens: processe logs com `python -m observador`, nunca cole logs brutos no contexto.
- Nunca imprimir senhas, tokens, IPs de servidores ou outros segredos, nem em resumos, testes ou commits. O mascaramento está em `observador/extract.py`; qualquer novo formato de saída passa por `redact`.
- Só biblioteca padrão do Python. Testes: `python -m unittest discover -s tests`.
- Ao mexer nas heurísticas, adicione um caso em `tests/test_extract.py`.
- O formato dos logs do Codex e do Kimi CLI ainda não foi confirmado com arquivos reais; não afirme que funciona neles antes de testar.
