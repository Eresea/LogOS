use std::{env, fs, process};

fn main() {
    let mut arguments = env::args_os();
    let program = arguments.next().unwrap_or_else(|| "logos-ui-codegen".into());
    let Some(input) = arguments.next() else {
        usage(&program);
    };
    let Some(output) = arguments.next() else {
        usage(&program);
    };
    if arguments.next().is_some() {
        usage(&program);
    }

    let source = match fs::read_to_string(&input) {
        Ok(source) => source,
        Err(error) => fail_path(&input, error),
    };
    let build = logos_ui_compiler::compile(&source);
    if !build.is_valid() {
        for index in 0..build.diagnostics.len() {
            let Some(diagnostic) = build.diagnostics.get(index) else { continue };
            let (line, column) = diagnostic.line_column(&source);
            eprintln!(
                "{}:{}:{}: error[{}]: {}",
                input.to_string_lossy(),
                line,
                column,
                diagnostic.code(),
                diagnostic.message()
            );
        }
        process::exit(1);
    }

    let mut generated = String::new();
    if let Err(error) = logos_ui_compiler::write_rust(&build, &mut generated) {
        eprintln!("code generation failed: {error:?}");
        process::exit(1);
    }
    if let Err(error) = fs::write(&output, generated) {
        fail_path(&output, error);
    }
}

fn fail_path(path: &std::ffi::OsStr, error: std::io::Error) -> ! {
    eprintln!("{}: {}", path.to_string_lossy(), error);
    process::exit(2);
}

fn usage(program: &std::ffi::OsStr) -> ! {
    eprintln!("usage: {} <file.ui> <output.rs>", program.to_string_lossy());
    process::exit(2);
}
