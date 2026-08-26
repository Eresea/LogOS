# LogOS UI VS Code support

This development extension associates `.ui` files with the `logos-ui-lsp`
stdio language server from the repository.

From the LogOS repository root:

```powershell
cargo build -p logos-ui-lsp
cd tools/vscode-logos-ui
npm.cmd install
```

Open `tools/vscode-logos-ui` in VS Code, press `F5`, and open a `.ui` file in
the Extension Development Host window. Diagnostics, completion, and hover are
provided by the Rust server.

For a server binary in another location, set `logosUi.serverPath` in VS Code
settings to its absolute path.
