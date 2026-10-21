# Observador

Agente que observa o que as IAs de código fizeram durante a semana (Claude Code, Codex, Kimi CLI) e entrega um relatório com erros recorrentes, retrabalho e melhorias de processo. Feito para gastar pouco: um programa sem IA condensa os logs e o modelo lê só o resumo.

O programa é um **binário único em Rust, sem dependências**: não precisa de Python, de versão específica de nada nem de instalação. Baixe o arquivo para o seu sistema, rode e pronto.

## Como funciona

1. **Coleta sem IA.** `observador` lê logs e devolve contagens e trechos curtos, com segredos e IPs mascarados.
2. **Análise.** O modelo lê o resumo (meta de uns 15 mil tokens), compara com o relatório anterior e escreve o relatório.
3. **Entrega.** A tarefa agendada roda toda segunda 08:31 (Brasília) e manda o resumo por email e push. Para pedir fora de hora, é só pedir na thread do projeto.

```
logs locais (Codex, Kimi, Claude Code) ─┐
sessões na nuvem (list_sessions)        ├─> observador (script) ─> resumo ─> modelo ─> relatório
telemetria OpenTelemetry (opcional)    ─┘
```

## Instalação

Baixe o binário da [última versão](https://github.com/joaogabriel15/observador/releases/latest) (as somas de verificação estão em `SHA256SUMS`):

| Sistema | Arquivo |
| --- | --- |
| Linux x86_64 (estático, roda em qualquer distribuição) | `observador-linux-x86_64` (depois `chmod +x`) |
| Windows x86_64 | `observador-windows-x86_64.exe` |

Ou compile você mesmo, só com o Rust instalado (não baixa nenhuma biblioteca, então funciona até sem internet):

```bash
cargo build --release      # gera target/release/observador (observador.exe no Windows)
cargo test                 # testes do Rust
```

## Uso

```bash
# 1. Ver o que existe na sua máquina (só nomes, tamanhos e nomes de chaves, nunca conteúdo)
observador discover

# 2. Resumir os últimos 7 dias de logs locais (Codex, Kimi, Claude Code)
observador scan ~/.codex ~/.kimi ~/.kimi-code ~/.claude/projects --days 7 \
  --rules CLAUDE.md --state digests/semana.json

# 3. Estatísticas das sessões da nuvem, a partir da resposta salva de list_sessions
observador sessions resposta_list_sessions.txt --days 7
```

No Windows (PowerShell), o `~` também funciona nos argumentos do programa: ele troca por `%USERPROFILE%`. A saída é limitada a 6000 caracteres por padrão (`--max-chars`). No Windows o comando fica `.\observador-windows-x86_64.exe discover`.

### Versão em Python (referência)

A pasta `observador/` tem a primeira versão, em Python 3.9+ só com biblioteca padrão (`python -m observador ...`, mesmos comandos). Ela é a referência de comportamento: os testes em `tests/` rodam nela, e com `OBSERVADOR_BIN=target/release/observador` os mesmos cenários e mais registros gerados ao acaso são executados também no binário em Rust e comparados byte a byte. Toda mudança de heurística vale nos dois.

```bash
python -m unittest discover -s tests                                                  # só Python
OBSERVADOR_BIN=target/release/observador python -m unittest discover -s tests         # Python x Rust
```

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
- **Windows não foi executado aqui.** O código é o mesmo e os testes rodam no Linux e no Windows pelo GitHub Actions a cada envio; o que só um Windows real confirma é o caminho da pasta pessoal e o console.
- **Heurísticas ruidosas.** Trechos de código do assistente podem ser contados como erro. O resumo serve para triagem; a análise final é do modelo.
- **Mascaramento por regex** pega os formatos comuns de segredo, não todos. Não publique os resumos sem olhar.

## Estrutura

```
src/main.rs              linha de comando
src/extract.rs           heurísticas, mascaramento e resumo
src/sessions.rs          estatísticas das sessões da nuvem
src/json.rs, regex.rs    leitor de JSON e motor de regex próprios (a biblioteca padrão do Rust não tem)
observador/              versão em Python, referência de comportamento
tests/                   cenários e paridade Rust x Python
.github/workflows/       testes (Linux e Windows) e publicação dos binários ao criar uma tag v*
prompts/weekly-report.md prompt da tarefa agendada
docs/pesquisa.md         ferramentas e artigos pesquisados
```

Por que o motor de regex é próprio: o crate `regex` do Rust não aceita lookbehind (`(?<!...)`), que a detecção de erros usa, e o projeto não tem dependências de propósito. O motor cobre só o que as heurísticas usam e tem testes próprios.

## Próximos passos

- Rodar `discover` na máquina do João e ajustar para o formato real do Codex e do Kimi.
- Opcional: ativar a telemetria OpenTelemetry do Claude Code (conteúdo de prompts desligado) para custo por semana sem ler transcrição.
- Guardar o resumo de cada semana em `digests/` (ignorado pelo git) para comparar semana a semana.
