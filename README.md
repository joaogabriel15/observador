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

Python 3.9 ou mais novo, só biblioteca padrão. Testes: `python -m unittest discover -s tests` (37 casos, incluindo semana com gabarito, entradas hostis e variantes de segredo).

```bash
# 1. Ver o que existe na sua máquina (só nomes, tamanhos e nomes de chaves, nunca conteúdo)
python -m observador discover

# 2. Resumir os últimos 7 dias de logs locais (Codex, Kimi, Claude Code)
python -m observador scan ~/.codex ~/.kimi ~/.kimi-code ~/.claude/projects --days 7 \
  --rules CLAUDE.md --state digests/semana.json

# 3. Estatísticas das sessões da nuvem, a partir da resposta salva de list_sessions
python -m observador sessions resposta_list_sessions.txt --days 7
```

No Windows use `%USERPROFILE%` no lugar de `~` se o shell não expandir. A saída é limitada a 6000 caracteres por padrão (`--max-chars`).

## O que ele procura

| Sinal | Como detecta |
| --- | --- |
| Erros, por tipo (ambiente, http, teste, código) | `Traceback`, `Error:`, `command not found`, `permission denied`, `HTTP 4xx/5xx`, `killed`, `timed out`, códigos de saída diferentes de 0 |
| Erro que se repete entre sessões | assinatura do erro (sem números, ids e caminhos); em tracebacks vale a última linha de exceção |
| Comandos repetidos | mesmo comando 3 vezes ou mais na sessão, com `cd ... &&` e `bash -lc` removidos |
| Arquivo editado 4 vezes ou mais | chamadas de edição e patches em texto |
| `--no-verify` | texto do comando |
| Correções do usuário | "errado", "de novo", "não funcionou", "still failing", palavrões; só fala do usuário, nunca saída de ferramenta |
| Conclusão contestada | a IA diz "pronto/feito/done" e a próxima fala do usuário é uma correção |
| Usuário repetindo a instrução | a mesma mensagem do usuário duas vezes na sessão |
| Compactação de contexto | marcadores de resumo/compactação |
| Espera por resposta | última mensagem do assistente termina com `?` |
| Segredos em texto | Bearer, JWT, chaves `sk-`/`ghp_`/AWS, chave privada, URL com senha, `password=`; só conta, nunca imprime o valor |
| Regras desrespeitadas | `--rules CLAUDE.md`: comandos que o arquivo manda não usar ("nunca use `x`") e que apareceram nas sessões |
| Modelos e tokens | chave `model` e soma de `input_tokens`/`output_tokens` |
| Custo e status das sessões da nuvem | `sessions`: custo estimado por status e as 3 mais caras (só sessões que informam custo) |
| Mudança desde a semana passada | `--state arquivo.json`: novo, continua e sumiu |

Cada sessão recebe uma pontuação; as maiores aparecem em "para leitura completa", que é o filtro barato antes do caro: só essas merecem ser abertas inteiras.

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
