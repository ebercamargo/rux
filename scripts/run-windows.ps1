$ErrorActionPreference = "Stop"
Set-Location (Resolve-Path (Join-Path $PSScriptRoot ".."))
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Error "Rust/Cargo nao encontrado. Instale pelo https://rustup.rs/ e abra um novo terminal."
    exit 1
}
cargo test
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
cargo run -- run examples/hello.rux
exit $LASTEXITCODE
