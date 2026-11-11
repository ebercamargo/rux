# RUX v1 — runtime e desempenho (Passo 2)

## Objetivos
- Isolar trabalho bloqueante do executor HTTP.
- Tornar pool de conexões configurável por implantação.
- Proteger recursos com limites explícitos e métricas.
- Sustentar shutdown controlado, readiness e liveness.
- Validar comportamento sob carga e falhas.

## Configuração inicial
- `RUX_HOST`: endereço IP de bind, padrão `127.0.0.1`.
- `PORT`: porta HTTP, padrão `3000`.
- `RUX_DB_POOL_SIZE`: conexões SQLite, padrão `8`, intervalo `1..=128`.
- `RUX_DB_WAIT_MS`: espera máxima para admissão de operação SQLite, padrão `2000 ms`, intervalo `0..=60000 ms`.
- `RUX_MAX_CONCURRENT_REQUESTS`: limite de operações API simultâneas por réplica, padrão `256`, intervalo `1..=100000`.
- `RUX_MAX_QUEUED_REQUESTS`: máximo de requisições aguardando vaga, padrão `512`, intervalo `0..=100000`.
- `RUX_API_QUEUE_WAIT_MS`: espera máxima assíncrona por vaga, padrão `50 ms`, intervalo `0..=10000 ms`. Ao esgotar o prazo, responde HTTP 429.

## Limitações conhecidas
- O controle atual ainda é global para rotas API, não separado por classe de recurso. Não é um teto global do cluster: aplica-se por réplica.
- O acesso SQLite possui limite independente igual ao tamanho do pool. Ainda faltam controles específicos para CPU e serviços externos.
- A admissão ao banco possui timeout, mas uma operação SQL já iniciada não é interrompida por esse prazo.
- Métricas em `/metrics` exigem `RUX_EXPOSE_METRICS=true`; não habilitar em interfaces públicas sem proteção.
- Benchmark: `python scripts/benchmark_http.py --requests 10000 --concurrency 100`. Executar em ambiente isolado e registrar hardware, carga e resultados.
- Os valores padrão são provisórios e devem ser calibrados por benchmarks.

## Critérios de aceite do Passo 2
1. Limites de conexões e de requisições concorrentes configuráveis.
2. Timeouts e tratamento de saturação explícitos.
3. Benchmark reprodutível de latência p50/p95/p99 e throughput.
4. Testes de carga com leituras e escritas concorrentes.
5. Shutdown e reinício testados com requisições em andamento.
6. Nenhum bloqueio de I/O SQLite nas threads do executor HTTP.
7. Métricas operacionais e logs estruturados.
8. Sem regressão de CRUD, paginação ou segurança.

**Nota:** pool de conexões não elimina o limite de escritor único do SQLite. Bancos adicionais serão integrados por drivers independentes. Metas numéricas de produção dependem de hardware, banco e cenário de carga.
