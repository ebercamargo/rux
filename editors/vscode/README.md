# RUX language support for VS Code

This source-only extension provides syntax highlighting, comments, bracket matching, indentation and snippets for `.rux` files. It is not yet published to the VS Code Marketplace, and does not provide debugging, LSP or IntelliSense.

To try it locally, open `editors/vscode` in VS Code and press **F5** to launch an Extension Development Host (VS Code extension development environment required). Open a `.rux` file in that window. The RUX CLI is installed independently with `npm install -g rux-lang@alpha`.

Other editors can reuse the TextMate grammar in `syntaxes/rux.tmLanguage.json` if they support it.
