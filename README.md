# RUX Programming Language

RUX is an experimental programming language under active development.

**Status:** interpreter prototype (not a native compiler). Not production ready. No release has been published yet.

## Quick start

Install the Rust toolchain, then run:

```bash
cargo fmt --all -- --check
cargo test
cargo run -- --version
cargo run -- check examples/hello.rux
cargo run -- run examples/hello.rux
```

Expected output from `run`:

```text
Hello, World!
```

The interpreter supports a deliberately small subset: `app`, zero-argument `fn main()`, `let`/`var` declarations, text and integer literals, variable references, and built-in `print`. It rejects undefined variables, duplicate bindings and unknown calls. `check` validates **syntax only**, not runtime semantics.

It does **not** compile native code, provide a type checker, support reassignment, execute other user-defined functions, or generate websites. `loop` and `when` are reserved for future implementation.

## Project docs

- [Versioning policy](docs/VERSIONING.md)
- [Compatibility](docs/COMPATIBILITY.md)
- [Changelog](CHANGELOG.md)

Licensed under Apache-2.0.
