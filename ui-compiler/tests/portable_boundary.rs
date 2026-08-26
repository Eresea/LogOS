use logos_ui::{
    MAX_UI_NODES, UiComponentTree, UiEventRouter, UiHandlerId, UiInputEvent, UiOutput,
    UiRoutedEvent, UiTree,
};
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

#[test]
fn compiled_page_events_install_and_route_by_document_node() {
    let build = compile_login_page();
    let mut router = UiEventRouter::new();
    let mut components = UiComponentTree::from_document(&build.document, &mut router).unwrap();
    let form = components.tree().handle_at(1).unwrap();
    let submit = components.tree().handle_at(5).unwrap();
    let mut component_output = UiOutput::new();
    let mut routed_output = UiOutput::<UiRoutedEvent>::new();

    components
        .dispatch_with_hooks(
            form,
            UiInputEvent::Submit,
            &router,
            &mut component_output,
            &mut routed_output,
        )
        .unwrap();
    assert_eq!(routed_output.pop().unwrap().handler, UiHandlerId::new(1));

    components
        .dispatch_with_hooks(
            submit,
            UiInputEvent::Submit,
            &router,
            &mut component_output,
            &mut routed_output,
        )
        .unwrap();
    assert_eq!(routed_output.pop().unwrap().handler, UiHandlerId::new(5));
}
