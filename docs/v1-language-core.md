# RUX v1 — núcleo da linguagem: contrato e critérios de aceite

Este documento acompanha a estabilização do núcleo. Funcionalidades listadas como pendentes NÃO estão implementadas.

## Sintaxe atualmente disponível
- `app Nome { fn main() { ... } }` como unidade de execução.
- `let` imutável e `var` mutável; atribuição com `=`.
- Strings UTF-8 com escapes, inteiros, booleanos `true`/`false`.
- Identificadores Unicode, operadores aritméticos e comparações.
- `when`/`else`, `loop N`, `loop while`, funções, argumentos e `return`.
- `print(...)`, separadores por quebra de linha ou ponto e vírgula.
- `rux check` verifica sintaxe e validações semânticas iniciais.
- `rux run` executa programas e mantém o modo CRUD declarativo.

## Invariantes
- `main` é obrigatório, único e não aceita parâmetros.
- Funções e parâmetros duplicados são rejeitados.
- Variáveis `let` não podem ser reatribuídas.
- Operações inteiras sinalizam overflow e divisão por zero.
- Execução possui limites de instruções e profundidade de chamadas.
- Erros não devem resultar em panic nem em encerramento silencioso.

## Pendências para encerrar o Passo 1
- Tipos estáticos explícitos e verificação semântica completa.
- Escopos léxicos de blocos, coleções e valores tipados no runtime.
- Importação de módulos e resolução de nomes entre arquivos.
- Tratamento de erros da linguagem e diagnósticos com linha/coluna.
- Especificação formal de gramática e precedência.
- Testes de conformidade e compatibilidade retroativa dos exemplos.
- Formatação e documentação de referência.

## Portão de conclusão
O Passo 1 só será concluído quando cada pendência acima estiver implementada, coberta por testes e integrada à main com CI verde. O Passo 2 (runtime e desempenho) inicia após esse portão.
