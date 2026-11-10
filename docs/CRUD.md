# CRUD web responsivo (primeira etapa)

Sem gerar nova versao npm ou tag, compile o codigo atual do repositorio com `cargo build` e execute:

```powershell
cargo run -- crud clientes nome email telefone
start .\clientes\index.html
```

O gerador cria uma interface responsiva com criar, listar, pesquisar, editar e excluir registros, usando armazenamento local do navegador. O HTML abre em Windows, Linux, macOS e navegadores moveis modernos. **Os dados nao sao compartilhados entre dispositivos** e o HTML ainda nao e um aplicativo nativo. A geracao de API e banco de dados, sincronizacao, autenticacao, PWA instalavel e empacotamento desktop/mobile sao etapas posteriores. Esta implementacao nao altera VERSION, tags Git nem o pacote npm publicado.
