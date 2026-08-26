use std::io;

fn main() -> io::Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    logos_ui_lsp::run_stdio(stdin.lock(), stdout.lock())
}
