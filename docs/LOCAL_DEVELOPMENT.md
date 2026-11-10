# Run RUX locally

RUX currently uses a Rust-based experimental interpreter. There are no published RUX binaries yet.

## Windows 10/11

1. Install Git from https://git-scm.com/downloads/win and Rust (rustup) from https://rustup.rs/.
2. For the MSVC Rust toolchain, install Microsoft C++ Build Tools when requested by rustup.
3. Open a new PowerShell terminal.

```powershell
git clone https://github.com/ebercamargo/rux.git
cd rux
git switch develop
cargo test
cargo run -- run examples/hello.rux
```

Alternatively, run `powershell -ExecutionPolicy Bypass -File scripts/run-windows.ps1` from the repository root.

## Linux/macOS

Install Git and Rust, then:

```bash
git clone https://github.com/ebercamargo/rux.git
cd rux
git switch develop
cargo test
cargo run -- run examples/hello.rux
```

Expected output: `Hello, World!`

## Commands

- `cargo run -- --version`: show prototype version
- `cargo run -- check examples/hello.rux`: validate syntax
- `cargo run -- run examples/hello.rux`: interpret the example
- `cargo build --release`: create the native **Rust interpreter binary** (not a compiled RUX program)

Windows binary: `target/release/rux.exe`. Linux/macOS binary: `target/release/rux`.

## Limitations

Only a minimal RUX subset runs. There is no static type checking, native compilation of .rux programs, website generation or complete language specification support yet.
