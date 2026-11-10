# RUX CRUD — prototipo funcional web + API

Este recurso ainda nao esta publicado no npm e nao altera versao nem tags.

## Definir o CRUD em RUX (novo prototipo)

O arquivo `examples/clientes_crud.rux` usa `app` e `data` com campos declarativos:

```rux
app Clientes {
    data Cliente {
        nome: Text
        email: Email
        telefone: Text
    }
}
```

Gere a aplicacao com `cargo run -- crud-from examples/clientes_crud.rux`, depois execute `cd Clientes` e `node server.js`. O resultado e acessivel em http://localhost:3000.

**Limitacao importante:** o parser declarativo e separado do interpretador executavel; tipos como `Int`, `Email` e `Date` sao aceitos como metadados, mas ainda nao produzem validacao especifica, armazenamento tipado ou migracoes SQL. Os campos gerados sao tratados como texto. Nao usar em producao.

## Teste local (codigo-fonte atualizado)

Requer Rust/Cargo para compilar o projeto RUX e Node.js para executar o servidor gerado.

```powershell
cargo test
cargo run -- crud clientes nome email telefone
cd clientes
node server.js
```

Abra http://localhost:3000 no navegador. O servidor fornece o HTML responsivo e a API REST. Os registros sao gravados em `clientes/records.json` e permanecem apos reiniciar o servidor. O CRUD suporta criar, listar, pesquisar, editar e excluir.

### API

- `GET /api/records`: lista todos
- `POST /api/records`: cria (JSON com todos os campos)
- `PUT /api/records/{uuid}`: altera
- `DELETE /api/records/{uuid}`: exclui

O HTML aberto diretamente como arquivo ainda usa `localStorage`, apenas como modo offline local; nesse modo os dados nao sao compartilhados.

## Compatibilidade e limites

A interface funciona em navegadores modernos de Windows, Linux, macOS, Android e iOS. Isso NAO equivale a aplicativos nativos: nao ha APK, IPA ou instalador desktop. O servidor escuta apenas em 127.0.0.1; dispositivos na rede nao podem acessa-lo sem configuracao adicional. Nao ha autenticacao, banco SQL, migracoes, sincronizacao entre instancias, controle de concorrencia ou deploy seguro. O arquivo JSON e adequado apenas para prototipo local, nao producao.

## Proximas entregas necessarias antes de uma nova versao

1. Definicao declarativa de entidades em `.rux`, validada pelo parser.
2. Geracao de backend e frontend a partir dessa definicao.
3. Persistencia SQL (SQLite/PostgreSQL), validacoes, autenticacao e controle de concorrencia.
4. Testes de integracao em CI para CRUD e banco.
5. Distribuicao simples pelo CLI e suporte de editor.
6. Empacotamento desktop e mobile (ou PWA documentado como web), com testes em cada plataforma.

**Nenhuma nova release deve ser publicada ate concluir e validar o CRUD multiplataforma definido como meta.**

## Executar diretamente um CRUD declarativo

```sh
cargo run -- check examples/clientes_crud.rux
cargo run -- run examples/clientes_crud.rux
```

O comando `run` gera a pasta `Clientes` caso ainda nao exista e inicia o servidor Node.js local em http://localhost:3000. Node.js deve estar instalado. Se a pasta ja existir, o servidor existente e reutilizado: alteracoes no schema nao regeneram automaticamente os arquivos. O armazenamento continua em JSON; SQLite, PostgreSQL, autenticacao e empacotamento nativo ainda nao foram implementados.
