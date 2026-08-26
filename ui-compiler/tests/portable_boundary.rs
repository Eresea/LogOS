use logos_ui::{MAX_UI_NODES, UiTree};
use logos_ui_compiler::compile_login_page;

#[test]
fn compiled_page_crosses_the_portable_framework_boundary() {
    let build = compile_login_page();
    assert!(build.is_valid());

    let document: logos_ui::UiDocument = build.document;
    assert!(document.node_count() <= MAX_UI_NODES);

    let blueprint = document.to_blueprint().unwrap();
    let tree = UiTree::from_blueprint(&blueprint).unwrap();
    assert_eq!(tree.len(), document.node_count());
}
