# Observador

Agente que observa o que as IAs de código fizeram durante a semana (Claude Code, Codex, Kimi CLI) e entrega um relatório com erros recorrentes, retrabalho e melhorias de processo. Feito para gastar pouco: um script sem IA condensa os logs e o modelo lê só o resumo.

## Como funciona

1. **Coleta sem IA.** `python -m observador` lê logs e devolve contagens e trechos curtos, com segredos e IPs mascarados.
2. **Análise.** O modelo lê o resumo (meta de uns 15 mil tokens), compara com o relatório anterior e escreve o relatório.
3. **Entrega.** A tarefa agendada roda toda segunda 08:31 (Brasília) e manda o resumo por email e push. Para pedir fora de hora, é só pedir na thread do projeto.

```
logs locais (Codex, Kimi, Claude Code) ─┐
sessões na nuvem (list_sessions)        ├─> observador (script) ─> resumo ─> modelo ─> relatório
telemetria OpenTelemetry (opcional)    ─┘
```

## Uso

Python 3.9 ou mais novo, só biblioteca padrão.

```bash
# 1. Ver o que existe na sua máquina (só nomes, tamanhos e nomes de chaves, nunca conteúdo)
python -m observador discover

# 2. Resumir os últimos 7 dias de logs locais (Codex, Kimi, Claude Code)
python -m observador scan ~/.codex ~/.kimi ~/.kimi-code ~/.claude/projects --days 7

# 3. Estatísticas das sessões da nuvem, a partir da resposta salva de list_sessions
python -m observador sessions resposta_list_sessions.txt --days 7
```

No Windows use `%USERPROFILE%` no lugar de `~` se o shell não expandir. A saída é limitada a 6000 caracteres por padrão (`--max-chars`).

## O que ele procura

| Sinal | Como detecta |
| --- | --- |
| Erros | `Traceback`, `Algo Error:`, `command not found`, `permission denied`, `exit code` diferente de 0, `failed` |
| Comandos repetidos | mesmo valor de `command`/`cmd` 3 vezes ou mais |
| `--no-verify` | texto do comando |
| Correções do usuário | mensagens do usuário com "errado", "de novo", "já falei", "wrong", "revert", "stop"... |
| Espera por resposta | última mensagem do assistente termina com `?` |
| Segredos em texto | Bearer, JWT, `sk-`/`ghp_`, `password=`; só conta, nunca imprime o valor |
| Modelos | valores da chave `model` |

## Limites conhecidos

- **Formato dos logs não verificado.** Foi testado com dados sintéticos e com transcrições do Claude Code salvas pela API, não com arquivos reais do Codex ou do Kimi CLI. O percurso é genérico (qualquer JSON/JSONL), mas rode `discover` primeiro e ajuste `extract.py` se o formato real fugir do esperado. Locais prováveis, a confirmar: `~/.codex`, `~/.kimi` ou `~/.kimi-code` (depende da versão).
- **Heurísticas ruidosas.** Trechos de código do assistente podem ser contados como erro. O resumo serve para triagem; a análise final é do modelo.
- **Mascaramento por regex** pega os formatos comuns de segredo, não todos. Não publique os resumos sem olhar.

## Estrutura

```
observador/extract.py    heurísticas, mascaramento e resumo
observador/sessions.py   estatísticas das sessões da nuvem
observador/__main__.py   linha de comando
prompts/weekly-report.md prompt da tarefa agendada
docs/pesquisa.md         ferramentas e artigos pesquisados
tests/                   python -m unittest discover -s tests
```

## Próximos passos

- Rodar `discover` na máquina do João e ajustar para o formato real do Codex e do Kimi.
- Opcional: ativar a telemetria OpenTelemetry do Claude Code (conteúdo de prompts desligado) para custo por semana sem ler transcrição.
- Guardar o resumo de cada semana em `digests/` (ignorado pelo git) para comparar semana a semana.
