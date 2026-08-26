use std::{env, fs, path::PathBuf, process};

fn main() {
    let output_directory = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is required"));
    for (name, source_path) in [
        ("login_ui", "../../ui-compiler/examples/login.ui"),
        ("register_ui", "../../ui-compiler/examples/register.ui"),
    ] {
        println!("cargo:rerun-if-changed={source_path}");
        let source = fs::read_to_string(source_path).unwrap_or_else(|error| {
            panic!("cannot read {source_path}: {error}");
        });
        let build = logos_ui_compiler::compile(&source);
        if !build.is_valid() {
            for index in 0..build.diagnostics.len() {
                let Some(diagnostic) = build.diagnostics.get(index) else { continue };
                let (line, column) = diagnostic.line_column(&source);
                eprintln!(
                    "{source_path}:{line}:{column}: error[{}]: {}",
                    diagnostic.code(),
                    diagnostic.message()
                );
            }
            process::exit(1);
        }
        let mut generated = String::new();
        logos_ui_compiler::write_rust(&build, &mut generated)
            .expect("validated UI must be codegen-compatible");
        fs::write(output_directory.join(format!("{name}.rs")), generated)
            .unwrap_or_else(|error| panic!("cannot write generated {name} UI: {error}"));
    }
}
