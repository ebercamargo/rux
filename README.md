# RUX Programming Language

RUX is an experimental programming language under active development.

**Status:** compiler frontend prototype. Not production ready. The language specification is evolving; there is no published release yet.

## Quick start

Install the Rust toolchain, then run:

```bash
cargo test
cargo run -- --version
cargo run -- check examples/hello.rux
```

The current `check` command performs **syntax validation only** for a small subset: `app`, zero-argument `fn`, `let`/`var`, simple expressions and function calls. It does **not** execute RUX, perform type checking, compile native code or generate websites yet. Keywords such as `loop` and `when` are reserved for future implementation.

## Project docs

- [Versioning policy](docs/VERSIONING.md)
- [Compatibility](docs/COMPATIBILITY.md)
- [Changelog](CHANGELOG.md)

Licensed under Apache-2.0.
