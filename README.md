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

The interpreter supports a deliberately small subset: `app`, zero-argument `fn main()`, `let`/`var` declarations, mutable reassignment, text/integer/boolean literals, variable references, built-in `print`, `when`/`else` with boolean conditions, and `loop <integer> { ... }` counted loops. It rejects undefined variables, duplicate bindings and unknown calls. `check` validates **syntax only**, not runtime semantics.

It does **not** compile native code, provide a type checker, execute user-defined functions beyond `main`, or implement the full `loop`/`when` language design. Counted loops have a 100000 iteration limit, and execution has a 100000 statement budget.

## Project docs

- [Versioning policy](docs/VERSIONING.md)
- [Compatibility](docs/COMPATIBILITY.md)
- [Changelog](CHANGELOG.md)

Licensed under Apache-2.0.
