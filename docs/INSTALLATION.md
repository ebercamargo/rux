# Install RUX

The install scripts are **prepared for future GitHub Releases**. No official release is published yet. Until then, use the source instructions below.

## Windows (after first release)

```powershell
irm https://raw.githubusercontent.com/ebercamargo/rux/main/install.ps1 | iex
rux --version
```

The script downloads a published Windows x64 release, verifies SHA256, installs into `%LOCALAPPDATA%\RUX\bin` and updates the user PATH. Review the script before running it.

## Linux (after first release)

```bash
curl -fsSL https://raw.githubusercontent.com/ebercamargo/rux/main/install.sh | bash
rux --version
```

The Linux script currently supports x86_64 and needs curl, Python 3, sha256sum and tar. macOS arm64 packaging is also configured, but its checksum verification needs an appropriate platform-specific tool before enabling the shell installer on macOS.

## Available now: build from source

```powershell
git clone https://github.com/ebercamargo/rux.git
cd rux
git switch develop
cargo build --release
.\target\release\rux.exe run examples\hello.rux
```

The executable is a Rust-based RUX interpreter, not a native compiler for .rux programs.
