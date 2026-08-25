pub const SOURCE: &str = include_str!("../examples/login.ui");

pub fn compile() -> crate::compiler::UiBuild {
    crate::compiler::compile(SOURCE)
}
