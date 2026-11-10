# Getting started with RUX

The released CLI currently supports `rux check <file.rux>` and `rux run <file.rux>`. New commands below are **source changes awaiting the next release**.

After a future release containing the scaffold feature:

```powershell
npm install -g rux-lang@alpha
rux new hello-rux
cd hello-rux
rux run
```

This creates `main.rux` and runs it. No Rust or Cargo installation is needed for npm consumers on supported platforms.

Editor support: `editors/vscode` contains an early VS Code language extension, not yet published to the Marketplace. Any editor can edit `.rux` files as plain text.
