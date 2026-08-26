use std::{env, fs, process};

fn main() {
    let mut arguments = env::args_os();
    let program = arguments.next().unwrap_or_else(|| "logos-ui-lint".into());
    let Some(path) = arguments.next() else {
        usage(&program);
    };
    if arguments.next().is_some() {
        usage(&program);
    }

    let source = match fs::read_to_string(&path) {
        Ok(source) => source,
        Err(error) => {
            eprintln!("{}: {}", path.to_string_lossy(), error);
            process::exit(2);
        }
    };
    let diagnostics = logos_ui_compiler::lint(&source);
    for index in 0..diagnostics.len() {
        let Some(diagnostic) = diagnostics.get(index) else { continue };
        let (line, column) = diagnostic.line_column(&source);
        eprintln!(
            "{}:{}:{}: error[{}]: {}",
            path.to_string_lossy(),
            line,
            column,
            diagnostic.code(),
            diagnostic.message()
        );
    }
    if !diagnostics.is_empty() {
        process::exit(1);
    }
}

fn usage(program: &std::ffi::OsStr) -> ! {
    eprintln!("usage: {} <file.ui>", program.to_string_lossy());
    process::exit(2);
}
