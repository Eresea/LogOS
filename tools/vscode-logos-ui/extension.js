const fs = require("node:fs");
const path = require("node:path");
const vscode = require("vscode");
const {
  LanguageClient,
  TransportKind
} = require("vscode-languageclient/node");

let client;

function activate(context) {
  const serverPath = configuredServerPath(context);
  if (!serverPath) {
    vscode.window.showErrorMessage(
      "LogOS UI language server not found. Run `cargo build -p logos-ui-lsp` or configure logosUi.serverPath."
    );
    return;
  }

  const serverOptions = {
    run: { command: serverPath, transport: TransportKind.stdio },
    debug: { command: serverPath, transport: TransportKind.stdio }
  };
  const clientOptions = {
    documentSelector: [{ scheme: "file", language: "logos-ui" }]
  };

  client = new LanguageClient(
    "logosUiLanguageServer",
    "LogOS UI Language Server",
    serverOptions,
    clientOptions
  );
  context.subscriptions.push(client.start());
}

function configuredServerPath(context) {
  const configured = vscode.workspace
    .getConfiguration("logosUi")
    .get("serverPath");
  if (configured) {
    return fs.existsSync(configured) ? configured : undefined;
  }

  const workspaceRoot = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
  const candidates = [];
  if (workspaceRoot) {
    candidates.push(path.join(workspaceRoot, "target", "debug", executableName()));
  }
  candidates.push(
    path.resolve(context.extensionPath, "..", "..", "target", "debug", executableName())
  );
  return candidates.find((candidate) => fs.existsSync(candidate));
}

function executableName() {
  return process.platform === "win32" ? "logos-ui-lsp.exe" : "logos-ui-lsp";
}

function deactivate() {
  return client?.stop();
}

module.exports = { activate, deactivate };
