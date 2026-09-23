#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]
#![cfg_attr(not(target_os = "none"), allow(dead_code, unused_imports, unused_variables))]

mod common;

use logos_abi::{
    AtriumApp, AtriumControl, AtriumControlOperation, AtriumSurfaceInput, AtriumSurfaceRequest,
    AtriumSurfaceResponse, GuiDrawBatch, GuiDrawCommand, GuiHook, GuiHookKind, GuiRect, GuiSceneOp,
    GuiSessionContext, GuiSurfaceOperation, GuiSurfaceRequest, GuiSurfaceResponse, InputMessage,
    IpcStatus, KeyCode, KeyState, MessageKind, PointerState, RenderMessage, SurfaceHandle,
};

const INPUT_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_GUI_INPUT,
    b"input",
    core::mem::size_of::<InputMessage>(),
    logos_abi::IpcRights::Receive,
);
const INPUT_SETTINGS_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_INPUT_SETTINGS,
    b"input",
    core::mem::size_of::<logos_abi::InputSettings>(),
    logos_abi::IpcRights::Send,
);
const DISPLAY_DRAW_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_GUI_DRAW,
    b"display",
    core::mem::size_of::<logos_abi::GuiSceneOp>(),
    logos_abi::IpcRights::Send,
);
const TERMINAL_RENDER_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_RENDER,
    b"terminal",
    core::mem::size_of::<RenderMessage>(),
    logos_abi::IpcRights::Receive,
);
const DISPLAY_RENDER_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_RENDER,
    b"display",
    core::mem::size_of::<RenderMessage>(),
    logos_abi::IpcRights::Send,
);
const DISPLAY_CONTROL_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_GUI_SURFACE,
    b"display",
    core::mem::size_of::<GuiSurfaceRequest>(),
    logos_abi::IpcRights::Send,
);
const DISPLAY_RESPONSE_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_GUI_SURFACE,
    b"display",
    core::mem::size_of::<GuiSurfaceResponse>(),
    logos_abi::IpcRights::Receive,
);
const TERMINAL_SURFACE_REQUEST_CAPABILITY: common::CapabilitySpec =
    common::capability_contract_named(
        logos_abi::IPC_CONTRACT_ATRIUM_SURFACE_REQUEST,
        b"terminal",
        core::mem::size_of::<AtriumSurfaceRequest>(),
        logos_abi::IpcRights::Receive,
    );
const TERMINAL_SURFACE_RESPONSE_CAPABILITY: common::CapabilitySpec =
    common::capability_contract_named(
        logos_abi::IPC_CONTRACT_ATRIUM_SURFACE_RESPONSE,
        b"terminal",
        core::mem::size_of::<AtriumSurfaceResponse>(),
        logos_abi::IpcRights::Send,
    );
const TERMINAL_SURFACE_INPUT_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_ATRIUM_SURFACE_INPUT,
    b"terminal",
    core::mem::size_of::<AtriumSurfaceInput>(),
    logos_abi::IpcRights::Send,
);
const SYSTEM_SURFACE_REQUEST_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_ATRIUM_SURFACE_REQUEST,
    b"system",
    core::mem::size_of::<AtriumSurfaceRequest>(),
    logos_abi::IpcRights::Receive,
);
const SYSTEM_SURFACE_RESPONSE_CAPABILITY: common::CapabilitySpec =
    common::capability_contract_named(
        logos_abi::IPC_CONTRACT_ATRIUM_SURFACE_RESPONSE,
        b"system",
        core::mem::size_of::<AtriumSurfaceResponse>(),
        logos_abi::IpcRights::Send,
    );
const SYSTEM_SURFACE_INPUT_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_ATRIUM_SURFACE_INPUT,
    b"system",
    core::mem::size_of::<AtriumSurfaceInput>(),
    logos_abi::IpcRights::Send,
);
const SYSTEM_SURFACE_DRAW_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_ATRIUM_SURFACE_DRAW,
    b"system",
    core::mem::size_of::<logos_abi::GuiSceneOp>(),
    logos_abi::IpcRights::Receive,
);
const SHELL_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_ATRIUM_CONTROL,
    b"shell",
    core::mem::size_of::<AtriumControl>(),
    logos_abi::IpcRights::Send,
);
const SHELL_CONTEXT_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_GUI_SESSION,
    b"shell",
    core::mem::size_of::<GuiSessionContext>(),
    logos_abi::IpcRights::Receive,
);
const LOCKSCREEN_INPUT_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_GUI_INPUT,
    b"lockscreen",
    core::mem::size_of::<InputMessage>(),
    logos_abi::IpcRights::Send,
);
const LOCKSCREEN_CONTROL_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_GUI_HOOK,
    b"lockscreen",
    core::mem::size_of::<GuiHook>(),
    logos_abi::IpcRights::Send,
);
const MAX_PENDING_SURFACE_COMMANDS: usize = logos_atrium::MAX_ATRIUM_SURFACES * 2;
const CURSOR_BOUNDS: GuiRect = GuiRect::new(
    0,
    0,
    logos_abi::DEFAULT_SCREEN_WIDTH as u32,
    logos_abi::DEFAULT_SCREEN_HEIGHT as u32,
);

#[derive(Clone, Copy)]
struct ProgramSurfaceCapabilities {
    client: logos_abi::ServiceHandle,
    input: logos_abi::CapabilityHandle,
    render: logos_abi::CapabilityHandle,
    draw: logos_abi::CapabilityHandle,
}

#[derive(Clone, Copy)]
enum PendingSettingsRender {
    Controls(SurfaceHandle),
    SelectHover(SurfaceHandle),
}

static mut ATRIUM: logos_atrium::Atrium = logos_atrium::Atrium::new();
static mut CALCULATOR: logos_atrium::Calculator = logos_atrium::Calculator::new();
static mut COMMAND_MENU_TREE: logos_ui::UiComponentTree = logos_ui::UiComponentTree::new();
static mut SETTINGS_TREE: logos_ui::UiComponentTree = logos_ui::UiComponentTree::new();
static mut SETTINGS_ROUTER: logos_ui::UiEventRouter = logos_ui::UiEventRouter::new();
static mut SETTINGS_MOUNT: logos_ui::UiRouteMount = logos_ui::UiRouteMount::EMPTY;
static mut SETTINGS_ROUTE: u8 = u8::MAX;
static mut PENDING_HOME_SCENE: logos_ui_graphics::UiSceneFrame =
    logos_ui_graphics::UiSceneFrame::new();
static mut PENDING_HOME_SCENE_INDEX: usize = 0;
static mut LAST_HOME_SCENE: logos_ui_graphics::UiSceneFrame =
    logos_ui_graphics::UiSceneFrame::new();
static mut LAST_HOME_SURFACE: SurfaceHandle = SurfaceHandle::EMPTY;
static mut LAST_HOME_SCENE_READY: bool = false;
const SETTINGS_SELECT_NODE_BASE: u32 = 200;
const SETTINGS_POPOVER_NODE_BASE: u32 = 204;

fn home_scene_pending() -> bool {
    unsafe {
        *core::ptr::addr_of!(PENDING_HOME_SCENE_INDEX)
            < (*core::ptr::addr_of!(PENDING_HOME_SCENE)).len()
    }
}

fn push_text(batch: &mut GuiDrawBatch, x: i32, y: i32, color: u32, text: &[u8]) {
    if let Some(command) = GuiDrawCommand::glyph_run(x, y, color, text) {
        let _ = batch.push(command);
    }
}

#[cfg(feature = "qemu-proof")]
fn proof_line(message: &[u8]) {
    common::proof_line(message);
}

#[cfg(not(feature = "qemu-proof"))]
fn proof_line(_message: &[u8]) {}

#[cfg(feature = "qemu-proof")]
fn proof_home_surface_ready(surface: SurfaceHandle) {
    use core::fmt::Write as _;

    struct ProofLine {
        bytes: [u8; 96],
        length: usize,
    }

    impl core::fmt::Write for ProofLine {
        fn write_str(&mut self, value: &str) -> core::fmt::Result {
            let length = value.len().min(self.bytes.len().saturating_sub(self.length));
            self.bytes[self.length..self.length + length]
                .copy_from_slice(&value.as_bytes()[..length]);
            self.length += length;
            if length == value.len() { Ok(()) } else { Err(core::fmt::Error) }
        }
    }

    let mut line = ProofLine { bytes: [0; 96], length: 0 };
    let _ = write!(
        line,
        "LogOS vNext: Atrium home surface ready surface={}/{}",
        surface.slot, surface.generation,
    );
    common::proof_line(&line.bytes[..line.length]);
}

#[cfg(not(feature = "qemu-proof"))]
fn proof_home_surface_ready(_surface: SurfaceHandle) {}

fn push_surface_text(
    batch: &mut GuiDrawBatch,
    bounds: GuiRect,
    x: i32,
    y: i32,
    color: u32,
    text: &[u8],
) {
    push_text(batch, bounds.x.saturating_add(x), bounds.y.saturating_add(y), color, text);
}

fn settings_route(page: logos_atrium::SettingsPage) -> u8 {
    match page {
        logos_atrium::SettingsPage::Overview => 0,
        logos_atrium::SettingsPage::Keyboard => 1,
        logos_atrium::SettingsPage::Mouse => 2,
    }
}

fn settings_route_document(page: logos_atrium::SettingsPage) -> Option<logos_ui::UiDocument> {
    let mut document = logos_ui::UiDocument::EMPTY;
    let mut root_styles = logos_ui::UiStyleList::EMPTY;
    if !root_styles.push(logos_ui::UiStyle::Transparent) {
        return None;
    }
    let root = document.push_node(logos_ui::UiNodeTemplate {
        kind: logos_ui::UiNodeKind::Root,
        styles: root_styles,
        ..logos_ui::UiNodeTemplate::EMPTY
    })?;
    let label = match page {
        logos_atrium::SettingsPage::Keyboard => Some(b"Keyboard layout".as_slice()),
        logos_atrium::SettingsPage::Mouse => Some(b"Mouse acceleration".as_slice()),
        logos_atrium::SettingsPage::Overview => None,
    };
    if label.is_some() {
        document.push_node(logos_ui::UiNodeTemplate {
            kind: logos_ui::UiNodeKind::Label,
            parent: root,
            text: logos_ui::UiText::from_bytes(b"< Settings")?,
            ..logos_ui::UiNodeTemplate::EMPTY
        })?;
    }
    if let Some(label) = label {
        document.push_node(logos_ui::UiNodeTemplate {
            kind: logos_ui::UiNodeKind::Label,
            parent: root,
            text: logos_ui::UiText::from_bytes(label)?,
            ..logos_ui::UiNodeTemplate::EMPTY
        });
    }
    Some(document)
}

fn set_settings_tree_bounds(
    tree: &mut logos_ui::UiComponentTree,
    index: usize,
    bounds: GuiRect,
) -> bool {
    let Ok(handle) = tree.tree().handle_at(index) else { return false };
    tree.tree_mut()
        .set_bounds(handle, logos_ui::UiRect::new(bounds.x, bounds.y, bounds.width, bounds.height))
        .is_ok()
}

fn draw_settings_tree(
    display: logos_abi::CapabilityHandle,
    surface: logos_atrium::Surface,
    atrium: &logos_atrium::Atrium,
    sequence: u32,
) -> bool {
    let tree = unsafe { &mut *core::ptr::addr_of_mut!(SETTINGS_TREE) };
    if tree.tree().is_empty() {
        let mut blueprint = logos_ui::UiBlueprint::new();
        let Some(root) = blueprint.push_root(logos_ui::UiNodeKind::Root, 1).ok() else {
            return false;
        };
        let Some(sidebar) = blueprint.push_child(logos_ui::UiNodeKind::Panel, root, 2).ok() else {
            return false;
        };
        let Some(frame) = blueprint.push_child(logos_ui::UiNodeKind::RouteFrame, root, 3).ok()
        else {
            return false;
        };
        let Some(search) = blueprint.push_child(logos_ui::UiNodeKind::TextInput, root, 4).ok()
        else {
            return false;
        };
        let Some(keyboard) = blueprint.push_child(logos_ui::UiNodeKind::Button, root, 5).ok()
        else {
            return false;
        };
        let Some(mouse) = blueprint.push_child(logos_ui::UiNodeKind::Button, root, 6).ok() else {
            return false;
        };
        let Some(settings_label) = blueprint.push_child(logos_ui::UiNodeKind::Label, root, 7).ok()
        else {
            return false;
        };
        let Some(keyboard_description) =
            blueprint.push_child(logos_ui::UiNodeKind::Label, root, 8).ok()
        else {
            return false;
        };
        let Some(mouse_description) =
            blueprint.push_child(logos_ui::UiNodeKind::Label, root, 9).ok()
        else {
            return false;
        };
        let mut root_styles = logos_ui::UiStyleList::EMPTY;
        if !root_styles.push(logos_ui::UiStyle::Transparent)
            || blueprint.set_styles(root, root_styles).is_err()
        {
            return false;
        }
        let mut panel_styles = logos_ui::UiStyleList::EMPTY;
        if !panel_styles.push(logos_ui::UiStyle::RoundedLarge)
            || blueprint.set_styles(sidebar, panel_styles).is_err()
            || blueprint.set_styles(frame, panel_styles).is_err()
        {
            return false;
        }
        let mut control_styles = logos_ui::UiStyleList::EMPTY;
        let mut description_styles = logos_ui::UiStyleList::EMPTY;
        if !description_styles.push(logos_ui::UiStyle::TextMuted) {
            return false;
        }
        if !control_styles.push(logos_ui::UiStyle::RoundedLarge)
            || blueprint.set_styles(search, control_styles).is_err()
            || blueprint.set_styles(keyboard, control_styles).is_err()
            || blueprint.set_styles(mouse, control_styles).is_err()
            || blueprint
                .set_text(search, logos_ui::UiText::from_bytes(b"Search settings...").unwrap())
                .is_err()
            || blueprint
                .set_text(keyboard, logos_ui::UiText::from_bytes(b"Keyboard").unwrap())
                .is_err()
            || blueprint.set_text(mouse, logos_ui::UiText::from_bytes(b"Mouse").unwrap()).is_err()
            || blueprint
                .set_text(settings_label, logos_ui::UiText::from_bytes(b"Settings").unwrap())
                .is_err()
            || blueprint
                .set_text(
                    keyboard_description,
                    logos_ui::UiText::from_bytes(logos_atrium::SETTINGS_CARD_DESCRIPTIONS[0])
                        .unwrap(),
                )
                .is_err()
            || blueprint
                .set_text(
                    mouse_description,
                    logos_ui::UiText::from_bytes(logos_atrium::SETTINGS_CARD_DESCRIPTIONS[1])
                        .unwrap(),
                )
                .is_err()
            || blueprint.set_styles(keyboard_description, description_styles).is_err()
            || blueprint.set_styles(mouse_description, description_styles).is_err()
        {
            return false;
        }
        let Ok(new_tree) = logos_ui::UiComponentTree::from_blueprint(&blueprint) else {
            return false;
        };
        *tree = new_tree;
    }

    let page = atrium.settings_page();
    let route = settings_route(page);
    if unsafe { *core::ptr::addr_of!(SETTINGS_ROUTE) } != route {
        let Some(document) = settings_route_document(page) else { return false };
        let Ok(frame) = tree.tree().handle_at(2) else { return false };
        let router = unsafe { &mut *core::ptr::addr_of_mut!(SETTINGS_ROUTER) };
        let Ok(mount) = tree.switch_route_frame(frame, route, &document, router) else {
            return false;
        };
        unsafe { *core::ptr::addr_of_mut!(SETTINGS_MOUNT) = mount };
        unsafe { *core::ptr::addr_of_mut!(SETTINGS_ROUTE) = route };
    }

    let bounds = surface.bounds;
    let sidebar_bounds =
        GuiRect::new(bounds.x + 20, bounds.y + 58, 220, bounds.height.saturating_sub(76));
    let frame_bounds = GuiRect::new(
        bounds.x + 260,
        bounds.y + 58,
        bounds.width.saturating_sub(280),
        bounds.height.saturating_sub(76),
    );
    let overview = page == logos_atrium::SettingsPage::Overview;
    let route_bounds = if overview { GuiRect::EMPTY } else { frame_bounds };
    if !set_settings_tree_bounds(tree, 0, bounds)
        || !set_settings_tree_bounds(tree, 1, sidebar_bounds)
        || !set_settings_tree_bounds(tree, 2, route_bounds)
        || !set_settings_tree_bounds(tree, 6, GuiRect::new(bounds.x + 28, bounds.y + 72, 180, 28))
    {
        return false;
    }
    let search = logos_atrium::SETTINGS_SEARCH_BOUNDS;
    if !set_settings_tree_bounds(
        tree,
        3,
        GuiRect::new(bounds.x + search.x, bounds.y + search.y, search.width, search.height),
    ) {
        return false;
    }
    let query = atrium.settings_search_query();
    let Ok(search_handle) = tree.tree().handle_at(3) else { return false };
    if tree.set_value(search_handle, query).is_err() {
        return false;
    }
    if tree.tree_mut().set_focused(search_handle, atrium.settings_search_active()).is_err() {
        return false;
    }
    let mut visible_index = 0;
    for (index, page) in logos_atrium::SETTINGS_CARD_PAGES.into_iter().enumerate() {
        let card = logos_atrium::settings_card_bounds(visible_index);
        let card_bounds = if atrium.settings_card_visible(page) {
            visible_index += 1;
            GuiRect::new(bounds.x + card.x, bounds.y + card.y, card.width, card.height)
        } else {
            GuiRect::EMPTY
        };
        if !set_settings_tree_bounds(tree, 4 + index, card_bounds) {
            return false;
        }
        let description_bounds = if card_bounds.is_empty() {
            GuiRect::EMPTY
        } else {
            GuiRect::new(
                card_bounds.x.saturating_add(8),
                card_bounds.y.saturating_add(52),
                card_bounds.width.saturating_sub(16),
                24,
            )
        };
        if !set_settings_tree_bounds(tree, 7 + index, description_bounds) {
            return false;
        }
        let Ok(handle) = tree.tree().handle_at(4 + index) else { return false };
        let mut styles = logos_ui::UiStyleList::EMPTY;
        if !styles.push(logos_ui::UiStyle::RoundedLarge)
            || ((atrium.settings_page() == page
                || atrium.settings_card_hover()
                    == if page == logos_atrium::SettingsPage::Keyboard { 1 } else { 2 })
                && !styles.push(logos_ui::UiStyle::BackgroundAccent))
            || tree.set_styles(handle, styles).is_err()
            || tree
                .tree_mut()
                .set_hovered(
                    handle,
                    atrium.settings_card_hover()
                        == if page == logos_atrium::SettingsPage::Keyboard { 1 } else { 2 },
                )
                .is_err()
        {
            return false;
        }
    }
    let mount = unsafe { *core::ptr::addr_of!(SETTINGS_MOUNT) };
    if mount.root() != logos_ui::UiNodeHandle::EMPTY {
        let _ = tree.tree_mut().set_bounds(
            mount.root(),
            logos_ui::UiRect::new(
                frame_bounds.x,
                frame_bounds.y,
                frame_bounds.width,
                frame_bounds.height,
            ),
        );
        if page != logos_atrium::SettingsPage::Overview {
            if let Some(back) = mount.handle_at(1) {
                let _ = tree.tree_mut().set_bounds(
                    back,
                    logos_ui::UiRect::new(
                        bounds.x + 280,
                        bounds.y + 84,
                        frame_bounds.width.saturating_sub(20),
                        28,
                    ),
                );
            }
            if let Some(label) = mount.handle_at(2) {
                let _ = tree.tree_mut().set_bounds(
                    label,
                    logos_ui::UiRect::new(
                        bounds.x + 280,
                        bounds.y + 112,
                        frame_bounds.width.saturating_sub(20),
                        28,
                    ),
                );
            }
        }
    }
    let Ok(scene) = logos_ui_graphics::emit(
        surface.reference,
        sequence,
        tree,
        logos_ui_graphics::UiSceneTheme::DEFAULT,
    ) else {
        return false;
    };
    scene
        .as_slice()
        .iter()
        .any(|operation| common::ipc_send_handle(display, operation) == IpcStatus::Full)
}

fn draw_home(
    display: logos_abi::CapabilityHandle,
    surface: SurfaceHandle,
    atrium: &logos_atrium::Atrium,
    sequence: u32,
) {
    let tree = unsafe { &mut *core::ptr::addr_of_mut!(COMMAND_MENU_TREE) };
    if !logos_atrium::build_home_scene(tree, atrium, common::current_ticks()) {
        return;
    }
    let scene = match logos_ui_graphics::emit(
        surface,
        sequence,
        tree,
        logos_ui_graphics::UiSceneTheme::DEFAULT,
    ) {
        Ok(scene) => scene,
        Err(_) => return,
    };
    let delta = unsafe {
        if *core::ptr::addr_of!(LAST_HOME_SCENE_READY)
            && *core::ptr::addr_of!(LAST_HOME_SURFACE) == surface
        {
            scene.diff_from(&*core::ptr::addr_of!(LAST_HOME_SCENE)).unwrap_or(scene)
        } else {
            scene
        }
    };
    unsafe {
        *core::ptr::addr_of_mut!(LAST_HOME_SCENE) = scene;
        *core::ptr::addr_of_mut!(LAST_HOME_SURFACE) = surface;
        *core::ptr::addr_of_mut!(LAST_HOME_SCENE_READY) = true;
        *core::ptr::addr_of_mut!(PENDING_HOME_SCENE) = delta;
        *core::ptr::addr_of_mut!(PENDING_HOME_SCENE_INDEX) = 0;
    }
    let _ = flush_pending_home_scene(display);
}

fn flush_pending_home_scene(display: logos_abi::CapabilityHandle) -> IpcStatus {
    let mut index = unsafe { *core::ptr::addr_of!(PENDING_HOME_SCENE_INDEX) };
    let len = unsafe { (*core::ptr::addr_of!(PENDING_HOME_SCENE)).len() };
    while index < len {
        let Some(operation) =
            (unsafe { *core::ptr::addr_of!(PENDING_HOME_SCENE) }).as_slice().get(index).copied()
        else {
            break;
        };
        match common::ipc_send_handle(display, &operation) {
            IpcStatus::Ok => index += 1,
            IpcStatus::Full => {
                unsafe { *core::ptr::addr_of_mut!(PENDING_HOME_SCENE_INDEX) = index };
                return IpcStatus::Full;
            }
            status => {
                unsafe {
                    *core::ptr::addr_of_mut!(PENDING_HOME_SCENE_INDEX) = len;
                    *core::ptr::addr_of_mut!(LAST_HOME_SCENE_READY) = false;
                }
                return status;
            }
        }
    }
    unsafe { *core::ptr::addr_of_mut!(PENDING_HOME_SCENE_INDEX) = len };
    IpcStatus::Ok
}

fn send_settings_menu_node(
    display: logos_abi::CapabilityHandle,
    surface: SurfaceHandle,
    sequence: u32,
    node_id: u32,
    command: GuiDrawCommand,
    more: bool,
) -> IpcStatus {
    let mut operation = GuiSceneOp::upsert(surface, sequence, node_id, command);
    operation.flags = if more { logos_abi::GUI_DRAW_FLAG_MORE } else { 0 };
    common::ipc_send_handle(display, &operation)
}

fn render_settings_select_hover(
    display: logos_abi::CapabilityHandle,
    surface: logos_atrium::Surface,
    atrium: &logos_atrium::Atrium,
    sequence: &mut u32,
) -> bool {
    let layout = if atrium.settings_page() == logos_atrium::SettingsPage::Keyboard {
        atrium.keyboard_select_popover(GuiRect::new(
            0,
            0,
            surface.bounds.width,
            surface.bounds.height,
        ))
    } else {
        atrium.mouse_select_popover(GuiRect::new(0, 0, surface.bounds.width, surface.bounds.height))
    };
    let hovered_option = if atrium.settings_page() == logos_atrium::SettingsPage::Keyboard {
        atrium.keyboard_select_hovered_option()
    } else {
        atrium.mouse_select_hovered_option()
    };
    let highlighted = hovered_option
        .and_then(|index| {
            let option = layout.option_bounds(index);
            (!option.is_empty()).then(|| {
                GuiRect::new(
                    surface.bounds.x.saturating_add(option.x),
                    surface.bounds.y.saturating_add(option.y),
                    option.width,
                    option.height,
                )
            })
        })
        .unwrap_or_else(|| {
            GuiRect::new(
                surface.bounds.x.saturating_add(layout.bounds.x),
                surface.bounds.y.saturating_add(layout.bounds.y),
                layout.bounds.width,
                layout.bounds.height,
            )
        });
    *sequence = sequence.wrapping_add(1).max(1);
    let color = if hovered_option.is_some() { 0x356bd8 } else { 0x182535 };
    let operation = logos_abi::GuiSceneOp::upsert(
        surface.reference,
        *sequence,
        SETTINGS_POPOVER_NODE_BASE + 1,
        GuiDrawCommand::fill_rounded_rect(highlighted, color, 6),
    );
    common::ipc_send_handle(display, &operation) == IpcStatus::Full
}

fn render_settings_card_hover(
    display: logos_abi::CapabilityHandle,
    surface: logos_atrium::Surface,
    previous: u8,
    current: u8,
    selected_page: logos_atrium::SettingsPage,
    sequence: &mut u32,
) -> bool {
    let mut updates = [0u8; 2];
    let mut update_count = 0;
    for hover in [previous, current] {
        if hover == 0 || (hover == previous && hover == current) {
            continue;
        }
        updates[update_count] = hover;
        update_count += 1;
    }
    if update_count == 0 {
        return false;
    }
    *sequence = sequence.wrapping_add(1).max(1);
    let frame = *sequence;
    for (update_index, hover) in updates[..update_count].iter().enumerate() {
        let index = usize::from(hover.saturating_sub(1));
        let card = logos_atrium::settings_card_bounds(index);
        let bounds = GuiRect::new(
            surface.bounds.x.saturating_add(card.x),
            surface.bounds.y.saturating_add(card.y),
            card.width,
            card.height,
        );
        let page = logos_atrium::SETTINGS_CARD_PAGES[index];
        let mut operation = GuiSceneOp::upsert(
            surface.reference,
            frame,
            (4 + index as u32).saturating_mul(3).saturating_add(2),
            GuiDrawCommand::fill_rounded_rect(
                bounds,
                if *hover == current || page == selected_page { 0x356bd8 } else { 0x263548 },
                12,
            ),
        );
        if update_index + 1 < update_count {
            operation.flags = logos_abi::GUI_DRAW_FLAG_MORE;
        }
        if common::ipc_send_handle(display, &operation) == IpcStatus::Full {
            return true;
        }
    }
    false
}

fn draw_calculator_ui(
    display: logos_abi::CapabilityHandle,
    surface: logos_atrium::Surface,
    calculator: &logos_atrium::Calculator,
    sequence: u32,
) -> bool {
    let bounds = surface.bounds;
    let panel_bounds = GuiRect::new(
        bounds.x.saturating_add(12),
        bounds.y.saturating_add(40),
        bounds.width.saturating_sub(24),
        bounds.height.saturating_sub(52),
    );
    let mut base = GuiDrawBatch::new(surface.reference, sequence, bounds);
    base.flags = logos_abi::GUI_DRAW_FLAG_MORE;
    let _ = base.push(GuiDrawCommand::fill_rounded_rect(panel_bounds, 0x182535, 16));
    let display_bounds =
        GuiRect::new(bounds.x.saturating_add(20), bounds.y.saturating_add(52), 260, 40);
    let _ = base.push(GuiDrawCommand::fill_rounded_rect(display_bounds, 0x263548, 8));
    push_surface_text(&mut base, bounds, 32, 64, 0xffffff, calculator.display());
    if common::ipc_send_scene_batch(display, &base, 6) == IpcStatus::Full {
        return true;
    }

    let rows: [&[u8]; 4] = [
        b"[ 7 ]   [ 8 ]   [ 9 ]   [ / ]",
        b"[ 4 ]   [ 5 ]   [ 6 ]   [ * ]",
        b"[ 1 ]   [ 2 ]   [ 3 ]   [ - ]",
        b"[ 0 ]   [ . ]   [ = ]   [ + ]",
    ];
    for (row, labels) in rows.into_iter().enumerate() {
        let mut keypad = GuiDrawBatch::new(surface.reference, sequence, bounds);
        if row < 3 {
            keypad.flags = logos_abi::GUI_DRAW_FLAG_MORE;
        }
        push_surface_text(
            &mut keypad,
            bounds,
            20,
            logos_atrium::CALCULATOR_BUTTON_TOP + row as i32 * 28,
            0xffffff,
            labels,
        );
        if common::ipc_send_scene_batch(display, &keypad, 9 + row as u32) == IpcStatus::Full {
            return true;
        }
    }
    false
}

fn draw_settings_ui(
    display: logos_abi::CapabilityHandle,
    surface: logos_atrium::Surface,
    atrium: &logos_atrium::Atrium,
    sequence: u32,
) -> bool {
    if draw_settings_tree(display, surface, atrium, sequence) {
        return true;
    }
    draw_settings_controls(display, surface, atrium, sequence)
}

fn clear_settings_popover(
    display: logos_abi::CapabilityHandle,
    surface: SurfaceHandle,
    sequence: u32,
) -> bool {
    let nodes = [
        SETTINGS_POPOVER_NODE_BASE,
        SETTINGS_POPOVER_NODE_BASE + 1,
        SETTINGS_POPOVER_NODE_BASE + 2,
        SETTINGS_POPOVER_NODE_BASE + 3,
        SETTINGS_POPOVER_NODE_BASE + 4,
        SETTINGS_POPOVER_NODE_BASE + 5,
        SETTINGS_POPOVER_NODE_BASE + 10,
        SETTINGS_POPOVER_NODE_BASE + 11,
    ];
    for (index, node_id) in nodes.into_iter().enumerate() {
        let mut operation = GuiSceneOp::remove(surface, sequence, node_id);
        if index + 1 < nodes.len() {
            operation.flags = logos_abi::GUI_DRAW_FLAG_MORE;
        }
        if common::ipc_send_handle(display, &operation) == IpcStatus::Full {
            return true;
        }
    }
    false
}

fn draw_settings_controls(
    display: logos_abi::CapabilityHandle,
    surface: logos_atrium::Surface,
    atrium: &logos_atrium::Atrium,
    sequence: u32,
) -> bool {
    let bounds = surface.bounds;
    if !matches!(
        atrium.settings_page(),
        logos_atrium::SettingsPage::Keyboard | logos_atrium::SettingsPage::Mouse
    ) {
        return false;
    }

    let mut select = GuiDrawBatch::new(surface.reference, sequence, bounds);
    let select_open = if atrium.settings_page() == logos_atrium::SettingsPage::Keyboard {
        atrium.keyboard_select_open()
    } else {
        atrium.mouse_select_open()
    };
    select.flags = if select_open { logos_abi::GUI_DRAW_FLAG_MORE } else { 0 };
    let select_bounds = GuiRect::new(
        bounds.x.saturating_add(logos_atrium::SETTINGS_SELECT_BOUNDS.x),
        bounds.y.saturating_add(logos_atrium::SETTINGS_SELECT_BOUNDS.y),
        logos_atrium::SETTINGS_SELECT_BOUNDS.width,
        logos_atrium::SETTINGS_SELECT_BOUNDS.height,
    );
    let hovered = if atrium.settings_page() == logos_atrium::SettingsPage::Keyboard {
        atrium.keyboard_select_hovered()
    } else {
        atrium.mouse_select_hovered()
    };
    let select_color = if hovered { 0x356bd8 } else { 0x263548 };
    let _ = select.push(GuiDrawCommand::fill_rounded_rect(select_bounds, select_color, 10));
    push_surface_text(
        &mut select,
        bounds,
        logos_atrium::SETTINGS_SELECT_BOUNDS.x.saturating_add(16),
        logos_atrium::SETTINGS_SELECT_BOUNDS.y.saturating_add(16),
        0xffffff,
        if atrium.settings_page() == logos_atrium::SettingsPage::Keyboard {
            if atrium.keyboard_layout() == logos_atrium::KeyboardLayout::Azerty {
                b"AZERTY"
            } else {
                b"QWERTY"
            }
        } else {
            match atrium.mouse_acceleration() {
                logos_atrium::MouseAcceleration::Off => b"Off",
                logos_atrium::MouseAcceleration::Low => b"Low",
                logos_atrium::MouseAcceleration::Medium => b"Medium",
                logos_atrium::MouseAcceleration::High => b"High",
            }
        },
    );
    push_surface_text(
        &mut select,
        bounds,
        logos_atrium::SETTINGS_SELECT_BOUNDS
            .x
            .saturating_add(logos_atrium::SETTINGS_SELECT_BOUNDS.width as i32)
            .saturating_sub(28),
        logos_atrium::SETTINGS_SELECT_BOUNDS.y.saturating_add(16),
        0xb8c7da,
        b"v",
    );
    if common::ipc_send_scene_batch(display, &select, SETTINGS_SELECT_NODE_BASE) == IpcStatus::Full
    {
        return true;
    }
    if !select_open {
        return clear_settings_popover(display, surface.reference, sequence);
    }

    let layout = if atrium.settings_page() == logos_atrium::SettingsPage::Keyboard {
        atrium.keyboard_select_popover(GuiRect::new(0, 0, bounds.width, bounds.height))
    } else {
        atrium.mouse_select_popover(GuiRect::new(0, 0, bounds.width, bounds.height))
    };
    if layout.bounds.is_empty() {
        return false;
    }
    let popover_bounds = GuiRect::new(
        bounds.x.saturating_add(layout.bounds.x),
        bounds.y.saturating_add(layout.bounds.y),
        layout.bounds.width,
        layout.bounds.height,
    );
    let mut popover = GuiDrawBatch::new(surface.reference, sequence, bounds);
    popover.flags = logos_abi::GUI_DRAW_FLAG_MORE;
    let _ = popover.push(GuiDrawCommand::fill_rounded_rect(popover_bounds, 0x182535, 10));
    if common::ipc_send_scene_batch(display, &popover, SETTINGS_POPOVER_NODE_BASE)
        == IpcStatus::Full
    {
        return true;
    }
    let hovered_option = if atrium.settings_page() == logos_atrium::SettingsPage::Keyboard {
        atrium.keyboard_select_hovered_option()
    } else {
        atrium.mouse_select_hovered_option()
    };
    let hovered_bounds = hovered_option
        .map(|index| layout.option_bounds(index))
        .filter(|option_bounds| !option_bounds.is_empty())
        .map(|option_bounds| {
            GuiRect::new(
                bounds.x.saturating_add(option_bounds.x),
                bounds.y.saturating_add(option_bounds.y),
                option_bounds.width,
                option_bounds.height,
            )
        })
        .unwrap_or(popover_bounds);
    let hovered_color = if hovered_option.is_some() { 0x356bd8 } else { 0x182535 };
    if send_settings_menu_node(
        display,
        surface.reference,
        sequence,
        SETTINGS_POPOVER_NODE_BASE + 1,
        GuiDrawCommand::fill_rounded_rect(hovered_bounds, hovered_color, 6),
        true,
    ) == IpcStatus::Full
    {
        return true;
    }
    for index in layout.first_option..layout.first_option.saturating_add(layout.visible_options) {
        let option: &[u8] = if atrium.settings_page() == logos_atrium::SettingsPage::Keyboard {
            if index == 0 { b"AZERTY" } else { b"QWERTY" }
        } else {
            match index {
                0 => b"Off",
                1 => b"Low",
                2 => b"Medium",
                _ => b"High",
            }
        };
        let option_bounds = layout.option_bounds(index);
        let Some(command) = GuiDrawCommand::glyph_run(
            bounds.x.saturating_add(option_bounds.x).saturating_add(16),
            bounds.y.saturating_add(option_bounds.y).saturating_add(16),
            0xffffff,
            option,
        ) else {
            continue;
        };
        if send_settings_menu_node(
            display,
            surface.reference,
            sequence,
            SETTINGS_POPOVER_NODE_BASE.saturating_add(2).saturating_add(u32::from(index)),
            command,
            true,
        ) == IpcStatus::Full
        {
            return true;
        }
    }
    for index in 0..4u32 {
        let visible = index >= u32::from(layout.first_option)
            && index
                < u32::from(layout.first_option).saturating_add(u32::from(layout.visible_options));
        if !visible {
            let mut operation = GuiSceneOp::remove(
                surface.reference,
                sequence,
                SETTINGS_POPOVER_NODE_BASE + 2 + index,
            );
            operation.flags = logos_abi::GUI_DRAW_FLAG_MORE;
            if common::ipc_send_handle(display, &operation) == IpcStatus::Full {
                return true;
            }
        }
    }
    if !layout.scrollbar.is_empty() {
        let scrollbar = GuiRect::new(
            bounds.x.saturating_add(layout.scrollbar.x),
            bounds.y.saturating_add(layout.scrollbar.y),
            layout.scrollbar.width,
            layout.scrollbar.height,
        );
        if send_settings_menu_node(
            display,
            surface.reference,
            sequence,
            SETTINGS_POPOVER_NODE_BASE + 10,
            GuiDrawCommand::fill_rounded_rect(scrollbar, 0x4b82f2, 2),
            true,
        ) == IpcStatus::Full
        {
            return true;
        }
    } else {
        let mut operation =
            GuiSceneOp::remove(surface.reference, sequence, SETTINGS_POPOVER_NODE_BASE + 10);
        operation.flags = logos_abi::GUI_DRAW_FLAG_MORE;
        if common::ipc_send_handle(display, &operation) == IpcStatus::Full {
            return true;
        }
    }
    send_settings_menu_node(
        display,
        surface.reference,
        sequence,
        SETTINGS_POPOVER_NODE_BASE + 11,
        GuiDrawCommand::stroke_rounded_rect(
            popover_bounds,
            logos_ui_graphics::UiSceneTheme::DEFAULT.border,
            10,
            1,
        ),
        false,
    ) == IpcStatus::Full
}

fn draw_surface_chrome(
    display: logos_abi::CapabilityHandle,
    surface: logos_atrium::Surface,
    sequence: u32,
    title: &[u8],
    more: bool,
    opaque: bool,
) -> bool {
    let bounds = surface.bounds;
    let mut base = GuiDrawBatch::new(surface.reference, sequence, bounds);
    base.flags = logos_abi::GUI_DRAW_FLAG_MORE;
    let status_bar =
        GuiRect::new(bounds.x, bounds.y, bounds.width, logos_atrium::STATUS_BAR_BOUNDS.height);
    if opaque {
        let _ = base.push(GuiDrawCommand::fill_surface(0x101820));
    }
    let _ = base.push(GuiDrawCommand::fill_rect(status_bar, 0x182535));
    push_surface_text(&mut base, bounds, 16, 10, 0xffffff, title);
    if common::ipc_send_scene_batch(display, &base, 1) == IpcStatus::Full {
        return true;
    }

    let mut close = GuiDrawBatch::new(surface.reference, sequence, bounds);
    if more {
        close.flags = logos_abi::GUI_DRAW_FLAG_MORE;
    }
    let close_local = logos_atrium::surface_close_bounds(bounds);
    let close_bounds = GuiRect::new(
        bounds.x.saturating_add(close_local.x),
        bounds.y.saturating_add(close_local.y),
        close_local.width,
        close_local.height,
    );
    let _ = close.push(GuiDrawCommand::fill_rounded_rect(close_bounds, 0x9f3b3b, 6));
    push_surface_text(&mut close, bounds, close_local.x.saturating_add(16), 10, 0xffffff, b"X");
    common::ipc_send_scene_batch(display, &close, 4) == IpcStatus::Full
}

fn draw_app(
    display: logos_abi::CapabilityHandle,
    surface: logos_atrium::Surface,
    atrium: &logos_atrium::Atrium,
    calculator: &logos_atrium::Calculator,
    atrium_client: logos_abi::ServiceHandle,
    sequence: u32,
) -> bool {
    if matches!(
        surface.app,
        logos_atrium::AppId::Calculator
            | logos_atrium::AppId::Files
            | logos_atrium::AppId::Settings
    ) && !atrium.owns_surface(surface.reference, atrium_client)
    {
        return false;
    }
    let title: &[u8] = match surface.app {
        logos_atrium::AppId::Calculator => b"Calculator",
        logos_atrium::AppId::Files => b"Files",
        logos_atrium::AppId::Terminal => b"Terminal",
        logos_atrium::AppId::System => b"System",
        logos_atrium::AppId::Settings => b"Settings",
    };
    match surface.app {
        logos_atrium::AppId::Calculator => {
            if draw_surface_chrome(display, surface, sequence, title, true, true) {
                true
            } else {
                draw_calculator_ui(display, surface, calculator, sequence)
            }
        }
        logos_atrium::AppId::Files => {
            if draw_surface_chrome(display, surface, sequence, title, true, true) {
                return true;
            }
            let mut panel = GuiDrawBatch::new(surface.reference, sequence, surface.bounds);
            panel.flags = logos_abi::GUI_DRAW_FLAG_MORE;
            let _ = panel.push(GuiDrawCommand::fill_rect(
                GuiRect::new(
                    surface.bounds.x.saturating_add(20),
                    surface.bounds.y.saturating_add(52),
                    260,
                    48,
                ),
                0x263548,
            ));
            if common::ipc_send_scene_batch(display, &panel, 6) == IpcStatus::Full {
                return true;
            }
            let mut detail = GuiDrawBatch::new(
                surface.reference,
                sequence,
                GuiRect::new(
                    surface.bounds.x,
                    surface.bounds.y,
                    surface.bounds.width,
                    surface.bounds.height,
                ),
            );
            push_surface_text(&mut detail, surface.bounds, 32, 82, 0xffffff, b"No files found");
            push_surface_text(
                &mut detail,
                surface.bounds,
                24,
                132,
                0xb8c7da,
                b"Storage browser is not available yet",
            );
            common::ipc_send_scene_batch(display, &detail, 7) == IpcStatus::Full
        }
        logos_atrium::AppId::Terminal => {
            draw_surface_chrome(display, surface, sequence, title, false, true)
        }
        logos_atrium::AppId::System => {
            // The System service owns this retained scene, including its chrome.
            false
        }
        logos_atrium::AppId::Settings => {
            if draw_surface_chrome(display, surface, sequence, title, true, true) {
                true
            } else {
                draw_settings_ui(display, surface, atrium, sequence)
            }
        }
    }
}

fn next_request_id(next: &mut u32) -> u32 {
    let value = *next;
    *next = next.wrapping_add(1).max(1);
    value
}

struct SurfaceCommandQueue {
    requests: [Option<GuiSurfaceRequest>; MAX_PENDING_SURFACE_COMMANDS],
    head: usize,
    len: usize,
}

impl SurfaceCommandQueue {
    const fn new() -> Self {
        Self { requests: [None; MAX_PENDING_SURFACE_COMMANDS], head: 0, len: 0 }
    }

    fn push(&mut self, request: GuiSurfaceRequest) -> bool {
        for offset in 0..self.len {
            let index = (self.head + offset) % self.requests.len();
            let Some(queued) = self.requests[index] else { continue };
            if queued.surface == request.surface
                && (queued.operation == request.operation
                    || request.operation == GuiSurfaceOperation::Destroy)
            {
                self.requests[index] = Some(request);
                return true;
            }
        }
        if self.len == self.requests.len() {
            return false;
        }
        let index = (self.head + self.len) % self.requests.len();
        self.requests[index] = Some(request);
        self.len += 1;
        true
    }

    fn flush(&mut self, display: logos_abi::CapabilityHandle) {
        while self.len != 0 {
            let Some(request) = self.requests[self.head] else {
                self.len = 0;
                break;
            };
            match common::ipc_send_handle(display, &request) {
                IpcStatus::Ok => self.pop(),
                IpcStatus::Full => break,
                IpcStatus::Stale
                | IpcStatus::Disconnected
                | IpcStatus::Unauthorized
                | IpcStatus::Malformed
                | IpcStatus::Empty => self.pop(),
            }
        }
    }

    fn pop(&mut self) {
        self.requests[self.head] = None;
        self.head = (self.head + 1) % self.requests.len();
        self.len -= 1;
    }
}

fn send_surface_command(
    display: logos_abi::CapabilityHandle,
    queue: &mut SurfaceCommandQueue,
    operation: GuiSurfaceOperation,
    surface: SurfaceHandle,
    bounds: GuiRect,
    next: &mut u32,
) {
    let mut request = GuiSurfaceRequest::new(operation, next_request_id(next));
    request.surface = surface;
    request.bounds = bounds;
    if queue.push(request) {
        queue.flush(display);
    }
}

fn queue_surface_updates(
    display: logos_abi::CapabilityHandle,
    queue: &mut SurfaceCommandQueue,
    atrium: &logos_atrium::Atrium,
    next: &mut u32,
    pending_terminal_update: &mut Option<AtriumSurfaceResponse>,
    last_terminal_bounds: &mut GuiRect,
) {
    for surface in atrium.surfaces() {
        send_surface_command(
            display,
            queue,
            GuiSurfaceOperation::Update,
            surface.reference,
            surface.bounds,
            next,
        );
    }
    queue_terminal_surface_update(pending_terminal_update, last_terminal_bounds, atrium, next);
}

fn queue_terminal_surface_update(
    pending: &mut Option<AtriumSurfaceResponse>,
    last_bounds: &mut GuiRect,
    atrium: &logos_atrium::Atrium,
    next: &mut u32,
) {
    let Some(surface) = atrium.surface_for_app(logos_atrium::AppId::Terminal) else {
        *pending = None;
        *last_bounds = GuiRect::EMPTY;
        return;
    };
    if surface.bounds == *last_bounds {
        return;
    }
    *last_bounds = surface.bounds;
    *pending = Some(AtriumSurfaceResponse::update(
        next_request_id(next),
        surface.reference,
        surface.bounds,
    ));
}

fn is_fps_toggle(input: &InputMessage) -> bool {
    input.kind == MessageKind::Key
        && input.state == KeyState::Pressed
        && input.code == KeyCode::function(12).raw()
        && input.modifiers & logos_abi::MOD_CTRL != 0
}

fn queue_fps_toggle(queue: &mut SurfaceCommandQueue, next: &mut u32) {
    let request = GuiSurfaceRequest::new(GuiSurfaceOperation::ToggleFps, next_request_id(next));
    let _ = queue.push(request);
}

fn send_lockscreen_section(lockscreen: logos_abi::CapabilityHandle, visible: bool, next: &mut u32) {
    let mut hook = GuiHook::new(GuiHookKind::Section, next_request_id(next));
    hook.deadline = u64::from(visible);
    let _ = common::ipc_send_handle(lockscreen, &hook);
}

fn queue_terminal_response(
    pending: &mut Option<AtriumSurfaceResponse>,
    request: AtriumSurfaceRequest,
    status: logos_abi::GuiStatus,
    surface: SurfaceHandle,
) {
    if pending.is_some() {
        return;
    }
    let mut response = AtriumSurfaceResponse::new(request, status);
    response.surface = surface;
    *pending = Some(response);
}

fn queue_terminal_revoke(
    pending: &mut Option<AtriumSurfaceResponse>,
    deferred: &mut Option<SurfaceHandle>,
    next: &mut u32,
    surface: SurfaceHandle,
) {
    if !surface.is_valid() {
        return;
    }
    if pending.is_none() {
        *pending = Some(AtriumSurfaceResponse::revoke(next_request_id(next), surface));
    } else {
        *deferred = Some(surface);
    }
}

fn queue_system_revoke(
    pending: &mut Option<AtriumSurfaceResponse>,
    deferred: &mut Option<SurfaceHandle>,
    response_capability: logos_abi::CapabilityHandle,
    pending_capability: &mut logos_abi::CapabilityHandle,
    next: &mut u32,
    surface: SurfaceHandle,
) {
    if !surface.is_valid() {
        return;
    }
    if pending.is_none() {
        *pending = Some(AtriumSurfaceResponse::revoke(next_request_id(next), surface));
        *pending_capability = response_capability;
    } else {
        *deferred = Some(surface);
    }
}

fn atrium_status(error: logos_atrium::AtriumError) -> logos_abi::GuiStatus {
    match error {
        logos_atrium::AtriumError::Capacity => logos_abi::GuiStatus::Capacity,
        logos_atrium::AtriumError::AlreadyRegistered | logos_atrium::AtriumError::NotFound => {
            logos_abi::GuiStatus::NotFound
        }
        logos_atrium::AtriumError::Locked => logos_abi::GuiStatus::Unauthorized,
        logos_atrium::AtriumError::InvalidSurface => logos_abi::GuiStatus::Malformed,
    }
}

fn hide_surfaces(
    display: logos_abi::CapabilityHandle,
    commands: &mut SurfaceCommandQueue,
    atrium: &mut logos_atrium::Atrium,
    next: &mut u32,
) {
    let mut handles = [SurfaceHandle::EMPTY; logos_atrium::MAX_ATRIUM_SURFACES];
    let mut count = 0;
    for surface in atrium.surfaces() {
        handles[count] = surface.reference;
        count += 1;
    }
    for surface in handles[..count].iter().copied() {
        send_surface_command(
            display,
            commands,
            GuiSurfaceOperation::Destroy,
            surface,
            GuiRect::EMPTY,
            next,
        );
    }
    if atrium.home_surface().is_valid() {
        send_surface_command(
            display,
            commands,
            GuiSurfaceOperation::Destroy,
            atrium.home_surface(),
            GuiRect::EMPTY,
            next,
        );
    }
    atrium.lock();
    atrium.clear_surfaces();
    unsafe {
        *core::ptr::addr_of_mut!(LAST_HOME_SCENE_READY) = false;
        *core::ptr::addr_of_mut!(LAST_HOME_SURFACE) = SurfaceHandle::EMPTY;
    }
}

fn render_home_surface(
    display: logos_abi::CapabilityHandle,
    atrium: &logos_atrium::Atrium,
    sequence: &mut u32,
) -> bool {
    if home_scene_pending() {
        let status = flush_pending_home_scene(display);
        if home_scene_pending() {
            return status == IpcStatus::Full;
        }
    }
    let Some(home) = atrium.home_surface().is_valid().then_some(atrium.home_surface()) else {
        return false;
    };
    *sequence = sequence.wrapping_add(1).max(1);
    draw_home(display, home, atrium, *sequence);
    home_scene_pending()
}

fn render_settings_surface(
    display: logos_abi::CapabilityHandle,
    surface: logos_atrium::Surface,
    atrium: &logos_atrium::Atrium,
    sequence: &mut u32,
) -> bool {
    *sequence = sequence.wrapping_add(1).max(1);
    if draw_surface_chrome(display, surface, *sequence, b"Settings", true, true) {
        return true;
    }
    draw_settings_ui(display, surface, atrium, *sequence)
}

fn render_settings_controls_surface(
    display: logos_abi::CapabilityHandle,
    surface: logos_atrium::Surface,
    atrium: &logos_atrium::Atrium,
    sequence: &mut u32,
) -> bool {
    *sequence = sequence.wrapping_add(1).max(1);
    draw_settings_controls(display, surface, atrium, *sequence)
}

fn retry_settings_render(
    display: logos_abi::CapabilityHandle,
    atrium: &logos_atrium::Atrium,
    sequence: &mut u32,
    pending: PendingSettingsRender,
) -> bool {
    let reference = match pending {
        PendingSettingsRender::Controls(reference)
        | PendingSettingsRender::SelectHover(reference) => reference,
    };
    let Some(surface) = atrium.surface_by_reference(reference) else {
        return false;
    };
    if surface.app != logos_atrium::AppId::Settings {
        return false;
    }
    match pending {
        PendingSettingsRender::Controls(_) => {
            render_settings_controls_surface(display, surface, atrium, sequence)
        }
        PendingSettingsRender::SelectHover(_) => {
            render_settings_select_hover(display, surface, atrium, sequence)
        }
    }
}

fn render(
    display: logos_abi::CapabilityHandle,
    atrium: &logos_atrium::Atrium,
    calculator: &logos_atrium::Calculator,
    atrium_client: logos_abi::ServiceHandle,
    sequence: &mut u32,
) -> bool {
    if render_home_surface(display, atrium, sequence) {
        return true;
    }
    for surface in atrium.surfaces() {
        if draw_app(display, surface, atrium, calculator, atrium_client, *sequence) {
            return true;
        }
    }
    false
}

fn queue_home_surface(
    display_control: logos_abi::CapabilityHandle,
    pending_surface: &mut Option<(GuiSurfaceRequest, Option<logos_atrium::SurfaceRequest>)>,
    pending_surface_for_client: &mut bool,
    next: &mut u32,
) {
    if pending_surface.is_some() {
        return;
    }
    let mut request =
        GuiSurfaceRequest::new(GuiSurfaceOperation::CreateModal, next_request_id(next));
    request.bounds = logos_atrium::FULLSCREEN_SURFACE_BOUNDS;
    request.z_order = 3;
    if common::ipc_send_handle(display_control, &request) == IpcStatus::Ok {
        *pending_surface = Some((request, None));
        *pending_surface_for_client = false;
    }
}

fn queue_cursor_surface(
    display_control: logos_abi::CapabilityHandle,
    next: &mut u32,
) -> Option<GuiSurfaceRequest> {
    let mut request =
        GuiSurfaceRequest::new(GuiSurfaceOperation::CreateModal, next_request_id(next));
    request.flags = logos_abi::GUI_SURFACE_FLAG_CURSOR;
    request.bounds = CURSOR_BOUNDS;
    request.z_order = 3;
    let sent = common::ipc_send_handle(display_control, &request) == IpcStatus::Ok;
    sent.then_some(request)
}

fn cursor_op(
    surface: SurfaceHandle,
    x: i16,
    y: i16,
    pressed: bool,
    sequence: &mut u32,
) -> GuiSceneOp {
    let mut command =
        GuiDrawCommand::fill_rect(GuiRect::new(i32::from(x), i32::from(y), 3, 14), 0xffffff);
    command.auxiliary = pressed as u32;
    GuiSceneOp::upsert(surface, next_request_id(sequence), 1, command)
}

fn discover_program_capability(
    client: logos_abi::ServiceHandle,
    rights: logos_abi::IpcRights,
    contract_id: u16,
    message_bytes: usize,
) -> Option<logos_abi::CapabilityHandle> {
    common::discover_capabilities_contract(rights, contract_id, message_bytes)
        .ok()?
        .into_iter()
        .find_map(|(peer, capability)| (peer == client).then_some(capability))
}

fn app_id(app: AtriumApp) -> logos_atrium::AppId {
    match app {
        AtriumApp::Calculator => logos_atrium::AppId::Calculator,
        AtriumApp::Files => logos_atrium::AppId::Files,
        AtriumApp::Terminal => logos_atrium::AppId::Terminal,
        AtriumApp::System => logos_atrium::AppId::System,
    }
}

fn program_client_live(client: logos_abi::ServiceHandle) -> bool {
    common::discover_capabilities_contract(
        logos_abi::IpcRights::Receive,
        logos_abi::IPC_CONTRACT_ATRIUM_SURFACE_REQUEST,
        core::mem::size_of::<AtriumSurfaceRequest>(),
    )
    .is_ok_and(|clients| clients.into_iter().any(|(peer, _)| peer == client))
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    common::init_service_allocator();
    let input = common::capability_handle(INPUT_CAPABILITY).unwrap_or_else(|_| common::idle());
    let input_settings =
        common::capability_handle(INPUT_SETTINGS_CAPABILITY).unwrap_or_else(|_| common::idle());
    let display =
        common::capability_handle(DISPLAY_DRAW_CAPABILITY).unwrap_or_else(|_| common::idle());
    let terminal_render =
        common::capability_handle(TERMINAL_RENDER_CAPABILITY).unwrap_or_else(|_| common::idle());
    let display_render =
        common::capability_handle(DISPLAY_RENDER_CAPABILITY).unwrap_or_else(|_| common::idle());
    let display_control =
        common::capability_handle(DISPLAY_CONTROL_CAPABILITY).unwrap_or_else(|_| common::idle());
    let display_response =
        common::capability_handle(DISPLAY_RESPONSE_CAPABILITY).unwrap_or_else(|_| common::idle());
    let terminal_surface_request = common::capability_handle(TERMINAL_SURFACE_REQUEST_CAPABILITY)
        .unwrap_or_else(|_| common::idle());
    let terminal_surface_response = common::capability_handle(TERMINAL_SURFACE_RESPONSE_CAPABILITY)
        .unwrap_or_else(|_| common::idle());
    let terminal = common::capability_handle(TERMINAL_SURFACE_INPUT_CAPABILITY)
        .unwrap_or_else(|_| common::idle());
    let system_surface_request = common::capability_handle(SYSTEM_SURFACE_REQUEST_CAPABILITY)
        .unwrap_or_else(|_| common::idle());
    let system_surface_response = common::capability_handle(SYSTEM_SURFACE_RESPONSE_CAPABILITY)
        .unwrap_or_else(|_| common::idle());
    let system_surface_input = common::capability_handle(SYSTEM_SURFACE_INPUT_CAPABILITY)
        .unwrap_or_else(|_| common::idle());
    let system_surface_draw = common::capability_handle(SYSTEM_SURFACE_DRAW_CAPABILITY)
        .unwrap_or_else(|_| common::idle());
    let shell = common::capability_handle(SHELL_CAPABILITY).unwrap_or_else(|_| common::idle());
    let shell_context =
        common::capability_handle(SHELL_CONTEXT_CAPABILITY).unwrap_or_else(|_| common::idle());
    let lockscreen_input =
        common::capability_handle(LOCKSCREEN_INPUT_CAPABILITY).unwrap_or_else(|_| common::idle());
    let lockscreen_control =
        common::capability_handle(LOCKSCREEN_CONTROL_CAPABILITY).unwrap_or_else(|_| common::idle());

    let atrium = unsafe { &mut *core::ptr::addr_of_mut!(ATRIUM) };
    let calculator = unsafe { &mut *core::ptr::addr_of_mut!(CALCULATOR) };
    let atrium_client = common::bootstrap_page().service;
    let mut next_request = 1u32;
    let mut sequence = 0u32;
    let mut pending_surface: Option<(GuiSurfaceRequest, Option<logos_atrium::SurfaceRequest>)> =
        None;
    let mut pending_surface_for_client = false;
    let mut pending_client_request: Option<AtriumSurfaceRequest> = None;
    let mut pending_client_response_capability = logos_abi::CapabilityHandle::EMPTY;
    let mut program_surface_capabilities: [Option<ProgramSurfaceCapabilities>;
        logos_atrium::MAX_ATRIUM_SURFACES] = [None; logos_atrium::MAX_ATRIUM_SURFACES];
    let mut last_terminal_request: Option<AtriumSurfaceRequest> = None;
    let mut last_system_request: Option<AtriumSurfaceRequest> = None;
    let mut terminal_client = logos_abi::ServiceHandle::EMPTY;
    // The System service may publish its request capability after Atrium starts.
    // Bind the client from the first valid request instead of rejecting that request
    // when the startup directory snapshot was not ready yet.
    let mut system_client = logos_abi::ServiceHandle::EMPTY;
    let mut pending_client_response: Option<AtriumSurfaceResponse> = None;
    let mut pending_terminal_update: Option<AtriumSurfaceResponse> = None;
    let mut last_terminal_bounds = GuiRect::EMPTY;
    let mut deferred_terminal_revoke: Option<SurfaceHandle> = None;
    let mut deferred_system_revoke: Option<SurfaceHandle> = None;
    let mut pending_render: Option<RenderMessage> = None;
    let mut pending_draw: Option<GuiSceneOp> = None;
    let mut pending_app_render = false;
    let mut pending_settings_render = None;
    let mut cursor_surface = SurfaceHandle::EMPTY;
    let mut pending_cursor_surface = queue_cursor_surface(display_control, &mut next_request);
    let mut cursor_x = (logos_abi::DEFAULT_SCREEN_WIDTH / 2) as i16;
    let mut cursor_y = (logos_abi::DEFAULT_SCREEN_HEIGHT / 2) as i16;
    let mut cursor_sequence = 1u32;
    let mut pending_cursor_draw: Option<GuiSceneOp> = None;
    let mut pending_input_settings: Option<logos_abi::InputSettings> =
        Some(atrium.input_settings());
    let mut surface_commands = SurfaceCommandQueue::new();
    let mut authenticated = false;
    let mut heartbeat_ticks = 0u16;
    let mut event = InputMessage::key(KeyCode::Unknown, KeyState::Released, 0);
    let mut deferred_event = None;
    let mut response = GuiSurfaceResponse::new(
        GuiSurfaceRequest::new(GuiSurfaceOperation::CreateModal, 1),
        logos_abi::GuiStatus::Malformed,
    );
    atrium.lock();
    proof_line(b"LogOS vNext: Atrium locked route ready");
    send_lockscreen_section(lockscreen_control, true, &mut next_request);

    loop {
        common::heartbeat_tick(&mut heartbeat_ticks);
        if let Some(pending) = pending_settings_render {
            if !retry_settings_render(display, atrium, &mut sequence, pending) {
                pending_settings_render = None;
            }
        }
        if let Some(settings) = pending_input_settings {
            match common::ipc_send_handle(input_settings, &settings) {
                IpcStatus::Ok => pending_input_settings = None,
                IpcStatus::Full => {}
                _ => pending_input_settings = None,
            }
        }
        if pending_app_render {
            pending_app_render = render(display, atrium, calculator, atrium_client, &mut sequence);
            if pending_app_render {
                common::heartbeat();
                continue;
            }
        }
        surface_commands.flush(display_control);
        if !cursor_surface.is_valid() && pending_cursor_surface.is_none() {
            pending_cursor_surface = queue_cursor_surface(display_control, &mut next_request);
        }
        if let Some(batch) = pending_cursor_draw {
            match common::ipc_send_handle(display, &batch) {
                IpcStatus::Ok => pending_cursor_draw = None,
                IpcStatus::Full => {}
                _ => {
                    pending_cursor_draw = None;
                    cursor_surface = SurfaceHandle::EMPTY;
                }
            }
        }
        if pending_client_response.is_none() {
            if let Some(surface) = deferred_terminal_revoke.take() {
                pending_client_response = Some(AtriumSurfaceResponse::revoke(
                    next_request_id(&mut next_request),
                    surface,
                ));
            } else if let Some(surface) = deferred_system_revoke.take() {
                pending_client_response = Some(AtriumSurfaceResponse::revoke(
                    next_request_id(&mut next_request),
                    surface,
                ));
                pending_client_response_capability = system_surface_response;
            }
        }
        if let Some(response) = pending_client_response {
            let response_capability = if pending_client_response_capability.is_valid() {
                pending_client_response_capability
            } else {
                terminal_surface_response
            };
            match common::ipc_send_handle(response_capability, &response) {
                IpcStatus::Ok => {
                    pending_client_response = None;
                    pending_client_response_capability = logos_abi::CapabilityHandle::EMPTY;
                }
                IpcStatus::Full => {}
                IpcStatus::Stale
                | IpcStatus::Disconnected
                | IpcStatus::Unauthorized
                | IpcStatus::Malformed
                | IpcStatus::Empty => {
                    pending_client_response = None;
                    pending_client_response_capability = logos_abi::CapabilityHandle::EMPTY;
                }
            }
        }
        if pending_client_response.is_none() {
            if let Some(update) = pending_terminal_update {
                match common::ipc_send_handle(terminal_surface_response, &update) {
                    IpcStatus::Ok => pending_terminal_update = None,
                    IpcStatus::Full => {}
                    IpcStatus::Stale
                    | IpcStatus::Disconnected
                    | IpcStatus::Unauthorized
                    | IpcStatus::Malformed
                    | IpcStatus::Empty => {
                        pending_terminal_update = None;
                        last_terminal_bounds = GuiRect::EMPTY;
                    }
                }
            }
        }
        if let Some(batch) = pending_draw {
            let live = atrium.surface_by_reference(batch.surface).is_some_and(|surface| {
                surface.app == logos_atrium::AppId::System
                    && atrium.owns_surface(batch.surface, system_client)
            }) || program_surface_capabilities
                .iter()
                .flatten()
                .any(|caps| atrium.owns_surface(batch.surface, caps.client));
            if !live {
                pending_draw = None;
            } else {
                match common::ipc_send_handle(display, &batch) {
                    IpcStatus::Ok => pending_draw = None,
                    IpcStatus::Full => {}
                    IpcStatus::Stale
                    | IpcStatus::Disconnected
                    | IpcStatus::Unauthorized
                    | IpcStatus::Malformed
                    | IpcStatus::Empty => pending_draw = None,
                }
            }
        }
        if let Some(message) = pending_render {
            let live = atrium.surface_by_reference(message.surface).is_some_and(|surface| {
                surface.app == logos_atrium::AppId::Terminal
                    && atrium.owns_surface(message.surface, terminal_client)
            }) || program_surface_capabilities
                .iter()
                .flatten()
                .any(|caps| atrium.owns_surface(message.surface, caps.client));
            if !live {
                pending_render = None;
            } else {
                match common::ipc_send_handle(display_render, &message) {
                    IpcStatus::Ok => pending_render = None,
                    IpcStatus::Full => {}
                    IpcStatus::Stale
                    | IpcStatus::Disconnected
                    | IpcStatus::Unauthorized
                    | IpcStatus::Malformed
                    | IpcStatus::Empty => pending_render = None,
                }
            }
        }
        let mut terminal_request = AtriumSurfaceRequest::new(AtriumApp::Terminal, atrium_client, 1);
        while common::ipc_receive_handle(terminal_surface_request, &mut terminal_request)
            == IpcStatus::Ok
        {
            if !terminal_request.is_valid() || terminal_request.app() != Some(AtriumApp::Terminal) {
                queue_terminal_response(
                    &mut pending_client_response,
                    terminal_request,
                    logos_abi::GuiStatus::Malformed,
                    SurfaceHandle::EMPTY,
                );
            } else {
                terminal_client = terminal_request.client();
                last_terminal_request = Some(terminal_request);
                if let Some(surface) = atrium
                    .surface_for_client(terminal_request.client(), logos_atrium::AppId::Terminal)
                {
                    queue_terminal_response(
                        &mut pending_client_response,
                        terminal_request,
                        logos_abi::GuiStatus::Ok,
                        surface.reference,
                    );
                    if let Some(response) = pending_client_response.as_mut() {
                        response.bounds = surface.bounds;
                    }
                    last_terminal_bounds = surface.bounds;
                } else if let Some(surface) = atrium.surface_for_app(logos_atrium::AppId::Terminal)
                {
                    let _ = atrium.close_reference(surface.reference);
                    send_surface_command(
                        display_control,
                        &mut surface_commands,
                        GuiSurfaceOperation::Destroy,
                        surface.reference,
                        GuiRect::EMPTY,
                        &mut next_request,
                    );
                    if pending_client_request.is_none() {
                        pending_client_request = Some(terminal_request);
                    } else {
                        queue_terminal_response(
                            &mut pending_client_response,
                            terminal_request,
                            logos_abi::GuiStatus::Backpressure,
                            SurfaceHandle::EMPTY,
                        );
                    }
                } else if atrium.phase() == logos_atrium::AtriumPhase::Home
                    && pending_client_request.is_none()
                {
                    pending_client_request = Some(terminal_request);
                } else {
                    queue_terminal_response(
                        &mut pending_client_response,
                        terminal_request,
                        logos_abi::GuiStatus::Backpressure,
                        SurfaceHandle::EMPTY,
                    );
                }
            }
        }
        let mut system_request = AtriumSurfaceRequest::new(AtriumApp::System, system_client, 1);
        while common::ipc_receive_handle(system_surface_request, &mut system_request)
            == IpcStatus::Ok
        {
            let client_matches =
                !system_client.is_valid() || system_request.client() == system_client;
            if !system_request.is_valid()
                || system_request.app() != Some(AtriumApp::System)
                || !client_matches
            {
                let response_was_empty = pending_client_response.is_none();
                queue_terminal_response(
                    &mut pending_client_response,
                    system_request,
                    logos_abi::GuiStatus::Malformed,
                    SurfaceHandle::EMPTY,
                );
                if response_was_empty {
                    pending_client_response_capability = system_surface_response;
                }
            } else {
                system_client = system_request.client();
                last_system_request = Some(system_request);
                if let Some(surface) =
                    atrium.surface_for_client(system_client, logos_atrium::AppId::System)
                {
                    let response_was_empty = pending_client_response.is_none();
                    queue_terminal_response(
                        &mut pending_client_response,
                        system_request,
                        logos_abi::GuiStatus::Ok,
                        surface.reference,
                    );
                    if response_was_empty {
                        if let Some(response) = pending_client_response.as_mut() {
                            response.bounds = surface.bounds;
                        }
                        pending_client_response_capability = system_surface_response;
                    }
                }
            }
        }
        if pending_surface.is_none()
            && pending_client_request.is_none()
            && pending_client_response.is_none()
        {
            let requests = common::discover_capabilities_contract(
                logos_abi::IpcRights::Receive,
                logos_abi::IPC_CONTRACT_ATRIUM_SURFACE_REQUEST,
                core::mem::size_of::<AtriumSurfaceRequest>(),
            )
            .unwrap_or_default();
            for (client, request_capability) in requests {
                if client == atrium_client || client == terminal_client {
                    continue;
                }
                let mut request = AtriumSurfaceRequest::new(AtriumApp::Calculator, client, 1);
                if common::ipc_receive_handle(request_capability, &mut request) != IpcStatus::Ok {
                    continue;
                }
                if !request.is_valid() || request.client() != client {
                    queue_terminal_response(
                        &mut pending_client_response,
                        request,
                        logos_abi::GuiStatus::Malformed,
                        SurfaceHandle::EMPTY,
                    );
                    pending_client_response_capability = discover_program_capability(
                        client,
                        logos_abi::IpcRights::Send,
                        logos_abi::IPC_CONTRACT_ATRIUM_SURFACE_RESPONSE,
                        core::mem::size_of::<AtriumSurfaceResponse>(),
                    )
                    .unwrap_or(logos_abi::CapabilityHandle::EMPTY);
                    break;
                }
                let Some(response_capability) = discover_program_capability(
                    client,
                    logos_abi::IpcRights::Send,
                    logos_abi::IPC_CONTRACT_ATRIUM_SURFACE_RESPONSE,
                    core::mem::size_of::<AtriumSurfaceResponse>(),
                ) else {
                    continue;
                };
                let Some(input_capability) = discover_program_capability(
                    client,
                    logos_abi::IpcRights::Send,
                    logos_abi::IPC_CONTRACT_ATRIUM_SURFACE_INPUT,
                    core::mem::size_of::<AtriumSurfaceInput>(),
                ) else {
                    continue;
                };
                let Some(render_capability) = discover_program_capability(
                    client,
                    logos_abi::IpcRights::Receive,
                    logos_abi::IPC_CONTRACT_RENDER,
                    core::mem::size_of::<RenderMessage>(),
                ) else {
                    continue;
                };
                let Some(draw_capability) = discover_program_capability(
                    client,
                    logos_abi::IpcRights::Receive,
                    logos_abi::IPC_CONTRACT_ATRIUM_SURFACE_DRAW,
                    core::mem::size_of::<logos_abi::GuiSceneOp>(),
                ) else {
                    continue;
                };
                let app = request.app().unwrap_or(AtriumApp::Calculator);
                let Ok(surface_request) = atrium.request_surface(app_id(app), client) else {
                    let response =
                        AtriumSurfaceResponse::new(request, logos_abi::GuiStatus::Capacity);
                    pending_client_response = Some(response);
                    pending_client_response_capability = response_capability;
                    break;
                };
                let mut display_request = GuiSurfaceRequest::new(
                    GuiSurfaceOperation::CreateModal,
                    next_request_id(&mut next_request),
                );
                display_request.bounds = surface_request.bounds();
                display_request.z_order = 2;
                if common::ipc_send_handle(display_control, &display_request) == IpcStatus::Ok {
                    pending_surface = Some((display_request, Some(surface_request)));
                    pending_surface_for_client = true;
                    pending_client_request = Some(request);
                    pending_client_response_capability = response_capability;
                    let caps = ProgramSurfaceCapabilities {
                        client,
                        input: input_capability,
                        render: render_capability,
                        draw: draw_capability,
                    };
                    if let Some(slot) = program_surface_capabilities
                        .iter()
                        .position(|entry| entry.is_none_or(|entry| entry.client == client))
                    {
                        program_surface_capabilities[slot] = Some(caps);
                    }
                    break;
                }
            }
        }
        let mut context = GuiSessionContext::EMPTY;
        while common::ipc_receive_handle(shell_context, &mut context) == IpcStatus::Ok {
            if context.is_authenticated() {
                authenticated = true;
                proof_line(b"LogOS vNext: Atrium authenticated");
                send_lockscreen_section(lockscreen_control, false, &mut next_request);
                atrium.authenticate();
                if !atrium.home_surface().is_valid() && pending_surface.is_none() {
                    queue_home_surface(
                        display_control,
                        &mut pending_surface,
                        &mut pending_surface_for_client,
                        &mut next_request,
                    );
                }
            } else if authenticated {
                authenticated = false;
                pending_surface_for_client = false;
                pending_client_request = None;
                let terminal_surface =
                    atrium.surface_for_app(logos_atrium::AppId::Terminal).map(|s| s.reference);
                let system_surface =
                    atrium.surface_for_app(logos_atrium::AppId::System).map(|s| s.reference);
                hide_surfaces(display_control, &mut surface_commands, atrium, &mut next_request);
                if let Some(surface) = terminal_surface {
                    queue_terminal_revoke(
                        &mut pending_client_response,
                        &mut deferred_terminal_revoke,
                        &mut next_request,
                        surface,
                    );
                }
                if let Some(surface) = system_surface {
                    queue_system_revoke(
                        &mut pending_client_response,
                        &mut deferred_system_revoke,
                        system_surface_response,
                        &mut pending_client_response_capability,
                        &mut next_request,
                        surface,
                    );
                }
                send_lockscreen_section(lockscreen_control, true, &mut next_request);
            }
        }

        let mut stale_program_surfaces = [SurfaceHandle::EMPTY; logos_atrium::MAX_ATRIUM_SURFACES];
        let mut stale_count = 0;
        for surface in atrium.surfaces() {
            if surface.client == atrium_client
                || surface.client == terminal_client
                || (surface.client == system_client && surface.app == logos_atrium::AppId::System)
            {
                continue;
            }
            if !program_client_live(surface.client) {
                stale_program_surfaces[stale_count] = surface.reference;
                stale_count += 1;
            }
        }
        for surface in stale_program_surfaces[..stale_count].iter().copied() {
            if let Ok(closed) = atrium.close_reference(surface) {
                send_surface_command(
                    display_control,
                    &mut surface_commands,
                    GuiSurfaceOperation::Destroy,
                    closed.reference,
                    GuiRect::EMPTY,
                    &mut next_request,
                );
                if let Some(caps) = program_surface_capabilities
                    .iter_mut()
                    .flatten()
                    .find(|caps| caps.client == closed.client)
                {
                    *caps = ProgramSurfaceCapabilities {
                        client: logos_abi::ServiceHandle::EMPTY,
                        input: logos_abi::CapabilityHandle::EMPTY,
                        render: logos_abi::CapabilityHandle::EMPTY,
                        draw: logos_abi::CapabilityHandle::EMPTY,
                    };
                }
            }
        }
        if stale_count != 0 {
            queue_surface_updates(
                display_control,
                &mut surface_commands,
                atrium,
                &mut next_request,
                &mut pending_terminal_update,
                &mut last_terminal_bounds,
            );
        }

        if pending_surface.is_none()
            && pending_client_request.is_some()
            && atrium.home_surface().is_valid()
        {
            if let Some(client_request) = pending_client_request {
                let app = client_request.app().map(app_id).unwrap_or(logos_atrium::AppId::Terminal);
                if let Ok(surface_request) = atrium.request_surface(app, client_request.client()) {
                    let mut request = GuiSurfaceRequest::new(
                        GuiSurfaceOperation::CreateModal,
                        next_request_id(&mut next_request),
                    );
                    request.bounds = surface_request.bounds();
                    request.z_order = 2;
                    if app == logos_atrium::AppId::Terminal {
                        request.flags = logos_abi::GUI_SURFACE_FLAG_TERMINAL;
                    }
                    if common::ipc_send_handle(display_control, &request) == IpcStatus::Ok {
                        pending_surface = Some((request, Some(surface_request)));
                        pending_surface_for_client = true;
                    }
                }
            }
        }

        while common::ipc_receive_handle(display_response, &mut response) == IpcStatus::Ok {
            if pending_cursor_surface.is_some_and(|request| response.is_valid_for(request)) {
                pending_cursor_surface = None;
                if response.status == logos_abi::GuiStatus::Ok && response.surface.is_valid() {
                    cursor_surface = response.surface;
                    pending_cursor_draw = Some(cursor_op(
                        cursor_surface,
                        cursor_x,
                        cursor_y,
                        false,
                        &mut cursor_sequence,
                    ));
                }
                continue;
            }
            let Some((request, app)) = pending_surface.take() else { continue };
            let for_client = pending_surface_for_client;
            if !response.is_valid_for(request) || response.request_id != request.request_id {
                pending_surface = Some((request, app));
                continue;
            }
            pending_surface_for_client = false;
            if response.status != logos_abi::GuiStatus::Ok || !response.surface.is_valid() {
                if for_client {
                    if let Some(client_request) = pending_client_request.take() {
                        queue_terminal_response(
                            &mut pending_client_response,
                            client_request,
                            response.status,
                            SurfaceHandle::EMPTY,
                        );
                    }
                }
                continue;
            }
            if !authenticated {
                send_surface_command(
                    display_control,
                    &mut surface_commands,
                    GuiSurfaceOperation::Destroy,
                    response.surface,
                    GuiRect::EMPTY,
                    &mut next_request,
                );
                continue;
            }
            let home_surface = app.is_none();
            let admitted = if let Some(request) = app {
                match atrium.spawn_surface(request, response.surface) {
                    Ok(surface) if for_client => {
                        if let Some(client_request) = pending_client_request.take() {
                            queue_terminal_response(
                                &mut pending_client_response,
                                client_request,
                                logos_abi::GuiStatus::Ok,
                                surface.reference,
                            );
                            if client_request.app() == Some(AtriumApp::Terminal) {
                                if let Some(response) = pending_client_response.as_mut() {
                                    response.bounds = surface.bounds;
                                }
                                last_terminal_bounds = surface.bounds;
                            } else if client_request.app() == Some(AtriumApp::System) {
                                if let Some(response) =
                                    pending_client_response.as_mut().filter(|response| {
                                        response.request_id == client_request.request_id
                                    })
                                {
                                    response.bounds = surface.bounds;
                                }
                            }
                        }
                        true
                    }
                    Ok(_) => true,
                    Err(error) => {
                        send_surface_command(
                            display_control,
                            &mut surface_commands,
                            GuiSurfaceOperation::Destroy,
                            response.surface,
                            GuiRect::EMPTY,
                            &mut next_request,
                        );
                        if for_client {
                            if let Some(client_request) = pending_client_request.take() {
                                queue_terminal_response(
                                    &mut pending_client_response,
                                    client_request,
                                    atrium_status(error),
                                    SurfaceHandle::EMPTY,
                                );
                            }
                        }
                        false
                    }
                }
            } else if atrium.set_home_surface(response.surface).is_ok() {
                true
            } else {
                send_surface_command(
                    display_control,
                    &mut surface_commands,
                    GuiSurfaceOperation::Destroy,
                    response.surface,
                    GuiRect::EMPTY,
                    &mut next_request,
                );
                false
            };
            if admitted {
                queue_surface_updates(
                    display_control,
                    &mut surface_commands,
                    atrium,
                    &mut next_request,
                    &mut pending_terminal_update,
                    &mut last_terminal_bounds,
                );
                if !home_surface {
                    if let Some(surface) = atrium.focused_surface() {
                        send_surface_command(
                            display_control,
                            &mut surface_commands,
                            GuiSurfaceOperation::Focus,
                            surface.reference,
                            GuiRect::EMPTY,
                            &mut next_request,
                        );
                    }
                }
                if home_surface {
                    proof_home_surface_ready(response.surface);
                }
                pending_app_render =
                    render(display, atrium, calculator, atrium_client, &mut sequence);
            }
            if authenticated
                && atrium.phase() == logos_atrium::AtriumPhase::Home
                && !atrium.home_surface().is_valid()
            {
                queue_home_surface(
                    display_control,
                    &mut pending_surface,
                    &mut pending_surface_for_client,
                    &mut next_request,
                );
            }
        }

        if pending_render.is_none() {
            let mut render = RenderMessage::empty(MessageKind::RenderCells);
            while common::ipc_receive_handle(terminal_render, &mut render) == IpcStatus::Ok {
                let terminal_surface_is_live = render.surface.is_valid()
                    && matches!(render.kind, MessageKind::RenderCells | MessageKind::FullRedraw)
                    && atrium.surface_by_reference(render.surface).is_some_and(|surface| {
                        surface.app == logos_atrium::AppId::Terminal
                            && atrium.owns_surface(render.surface, terminal_client)
                    });
                if terminal_surface_is_live {
                    pending_render = Some(render);
                    break;
                }
            }
        }
        if pending_draw.is_none() {
            let mut op = GuiSceneOp::clear(SurfaceHandle::new(0, 1, 13).unwrap(), 1);
            while common::ipc_receive_handle(system_surface_draw, &mut op) == IpcStatus::Ok {
                let live = op.is_valid()
                    && atrium.surface_by_reference(op.surface).is_some_and(|surface| {
                        surface.app == logos_atrium::AppId::System
                            && atrium.owns_surface(op.surface, system_client)
                    });
                if live {
                    pending_draw = Some(op);
                    break;
                }
                op = GuiSceneOp::clear(SurfaceHandle::new(0, 1, 13).unwrap(), 1);
            }
        }
        if pending_draw.is_none() {
            for caps in program_surface_capabilities.iter().flatten().copied() {
                let mut op = GuiSceneOp::clear(SurfaceHandle::new(0, 1, 13).unwrap(), 1);
                while common::ipc_receive_handle(caps.draw, &mut op) == IpcStatus::Ok {
                    let live = op.is_valid() && atrium.owns_surface(op.surface, caps.client);
                    if live {
                        pending_draw = Some(op);
                        break;
                    }
                    op = GuiSceneOp::clear(SurfaceHandle::new(0, 1, 13).unwrap(), 1);
                }
                if pending_draw.is_some() {
                    break;
                }
            }
        }
        if pending_render.is_none() {
            for caps in program_surface_capabilities.iter().flatten().copied() {
                let mut render = RenderMessage::empty(MessageKind::RenderCells);
                while common::ipc_receive_handle(caps.render, &mut render) == IpcStatus::Ok {
                    let live =
                        matches!(render.kind, MessageKind::RenderCells | MessageKind::FullRedraw)
                            && atrium.owns_surface(render.surface, caps.client);
                    if live {
                        pending_render = Some(render);
                        break;
                    }
                }
                if pending_render.is_some() {
                    break;
                }
            }
        }

        let mut cursor_sent_in_input = false;
        loop {
            if let Some(next) = deferred_event.take() {
                event = next;
            } else if common::ipc_receive_handle(input, &mut event) != IpcStatus::Ok {
                break;
            }
            if event.pointer_event().is_some_and(|pointer| pointer.state == PointerState::Move) {
                let (latest, deferred) = logos_atrium::coalesce_pointer_move(event, &mut |next| {
                    common::ipc_receive_handle(input, next) == IpcStatus::Ok
                });
                event = latest;
                deferred_event = deferred;
            }
            if let Some(pointer) = event.pointer_event() {
                cursor_x = pointer.x.clamp(0, (logos_abi::DEFAULT_SCREEN_WIDTH - 1) as i16);
                cursor_y = pointer.y.clamp(0, (logos_abi::DEFAULT_SCREEN_HEIGHT - 1) as i16);
                if cursor_surface.is_valid() {
                    let cursor = cursor_op(
                        cursor_surface,
                        cursor_x,
                        cursor_y,
                        pointer.buttons & 1 != 0,
                        &mut cursor_sequence,
                    );
                    if cursor_sent_in_input || pending_cursor_draw.is_some() {
                        pending_cursor_draw = Some(cursor);
                    } else {
                        match common::ipc_send_handle(display, &cursor) {
                            IpcStatus::Ok => cursor_sent_in_input = true,
                            IpcStatus::Full => pending_cursor_draw = Some(cursor),
                            _ => {
                                pending_cursor_draw = None;
                                cursor_surface = SurfaceHandle::EMPTY;
                            }
                        }
                    }
                }
            }
            if is_fps_toggle(&event) {
                queue_fps_toggle(&mut surface_commands, &mut next_request);
                surface_commands.flush(display_control);
                continue;
            }
            if !authenticated || atrium.phase() != logos_atrium::AtriumPhase::Home {
                if common::ipc_send_handle(lockscreen_input, &event) == IpcStatus::Ok {
                    proof_line(b"LogOS vNext: Atrium sent LockScreen input");
                } else {
                    proof_line(b"LogOS vNext: Atrium dropped LockScreen input");
                }
                continue;
            }
            let menu_selected = atrium.command_menu_open()
                && event
                    .pointer_event()
                    .and_then(|pointer| {
                        (pointer.state == PointerState::Down && pointer.buttons & 1 != 0)
                            .then(|| {
                                atrium.command_menu_item_at(
                                    i32::from(pointer.x),
                                    i32::from(pointer.y),
                                )
                            })
                            .flatten()
                    })
                    .is_some();
            let settings_menu_was_open = atrium.settings_menu_open();
            let account_menu_was_open = atrium.account_menu_open();
            let settings_menu_hovered_option = atrium.settings_menu_hovered_option();
            let account_menu_hovered_option = atrium.account_menu_hovered_option();
            let sidebar_hover = atrium.sidebar_hover();
            let settings_menu_pointer = !atrium.command_menu_open()
                && event.pointer_event().is_some_and(|pointer| {
                    atrium.settings_menu_open()
                        || atrium.account_menu_open()
                        || atrium.sidebar_hover() != 0
                        || logos_atrium::Atrium::sidebar_contains(
                            i32::from(pointer.x),
                            i32::from(pointer.y),
                        )
                });
            let sidebar_action =
                settings_menu_pointer.then(|| atrium.settings_menu_input(&event)).flatten();
            let previous_launcher_app = atrium.launcher_app();
            let command_menu_hover_changed = if atrium.command_menu_open() {
                event.pointer_event().is_some_and(|pointer| {
                    pointer.state == PointerState::Move
                        && atrium
                            .command_menu_item_at(i32::from(pointer.x), i32::from(pointer.y))
                            .is_some()
                        && previous_launcher_app != atrium.launcher_app()
                })
            } else {
                false
            };
            let settings_menu_changed = settings_menu_was_open != atrium.settings_menu_open()
                || account_menu_was_open != atrium.account_menu_open()
                || settings_menu_hovered_option != atrium.settings_menu_hovered_option()
                || account_menu_hovered_option != atrium.account_menu_hovered_option()
                || sidebar_hover != atrium.sidebar_hover();
            let sidebar_pointer = !atrium.command_menu_open()
                && event.pointer_event().is_some_and(|pointer| {
                    logos_atrium::Atrium::sidebar_contains(
                        i32::from(pointer.x),
                        i32::from(pointer.y),
                    )
                });
            if menu_selected {
                event = InputMessage::key(KeyCode::ENTER, KeyState::Pressed, 0);
            } else if atrium.command_menu_open() && event.pointer_event().is_some() {
                if command_menu_hover_changed {
                    pending_app_render = render_home_surface(display, atrium, &mut sequence);
                }
                continue;
            } else if settings_menu_pointer {
                if settings_menu_changed {
                    pending_app_render = render_home_surface(display, atrium, &mut sequence);
                }
                if sidebar_action.is_none() {
                    continue;
                }
            } else if sidebar_pointer {
                continue;
            } else if atrium.handle_splitter_pointer(&event) {
                queue_surface_updates(
                    display_control,
                    &mut surface_commands,
                    atrium,
                    &mut next_request,
                    &mut pending_terminal_update,
                    &mut last_terminal_bounds,
                );
                pending_app_render =
                    render(display, atrium, calculator, atrium_client, &mut sequence);
                continue;
            } else if event.pointer_event().is_none() {
                if let Some(surface) = atrium
                    .focused_surface()
                    .filter(|surface| surface.app == logos_atrium::AppId::Settings)
                {
                    if atrium.settings_input(&event) {
                        pending_app_render =
                            render_settings_surface(display, surface, atrium, &mut sequence);
                        continue;
                    }
                }
            } else if let Some(pointer) = event.pointer_event() {
                if let Some(surface) = atrium.pointer_target(&event) {
                    if pointer.state == PointerState::Down {
                        send_surface_command(
                            display_control,
                            &mut surface_commands,
                            GuiSurfaceOperation::Focus,
                            surface.reference,
                            GuiRect::EMPTY,
                            &mut next_request,
                        );
                    }
                    let local_x = i32::from(pointer.x).saturating_sub(surface.bounds.x);
                    let local_y = i32::from(pointer.y).saturating_sub(surface.bounds.y);
                    let close_bounds = if surface.app == logos_atrium::AppId::System {
                        logos_atrium::system_surface_close_bounds(surface.bounds)
                    } else {
                        logos_atrium::surface_close_bounds(surface.bounds)
                    };
                    let close_clicked = pointer.state == PointerState::Down
                        && close_bounds.contains(local_x, local_y);
                    let local = InputMessage::pointer(
                        local_x.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16,
                        local_y.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16,
                        pointer.buttons,
                        pointer.state,
                    )
                    .unwrap_or(event);
                    if close_clicked {
                        event = InputMessage::key(KeyCode::ESCAPE, KeyState::Pressed, 0);
                    } else {
                        let routed = AtriumSurfaceInput::new(surface.reference, local);
                        if surface.app == logos_atrium::AppId::Settings {
                            let previous_input_settings = atrium.input_settings();
                            let previous_settings_page = atrium.settings_page();
                            let previous_settings_card_hover = atrium.settings_card_hover();
                            let previous_settings_search = atrium.settings_search_query();
                            let previous_settings_search_active = atrium.settings_search_active();
                            let previous_keyboard_select_open = atrium.keyboard_select_open();
                            let previous_mouse_select_open = atrium.mouse_select_open();
                            let previous_keyboard_layout = atrium.keyboard_layout();
                            let previous_mouse_acceleration = atrium.mouse_acceleration();
                            let fast_hover = pointer.state == PointerState::Move
                                && ((atrium.settings_page()
                                    == logos_atrium::SettingsPage::Keyboard
                                    && atrium.keyboard_select_open())
                                    || (atrium.settings_page()
                                        == logos_atrium::SettingsPage::Mouse
                                        && atrium.mouse_select_open()));
                            if atrium.settings_input(&local) {
                                let current_input_settings = atrium.input_settings();
                                if current_input_settings != previous_input_settings {
                                    pending_input_settings = Some(current_input_settings);
                                }
                                let fast_card_hover = pointer.state == PointerState::Move
                                    && previous_settings_page == atrium.settings_page()
                                    && previous_settings_card_hover != atrium.settings_card_hover()
                                    && atrium.settings_search_query().as_bytes().is_empty()
                                    && !fast_hover;
                                let settings_controls_changed = previous_settings_page
                                    == atrium.settings_page()
                                    && previous_settings_search.as_bytes()
                                        == atrium.settings_search_query().as_bytes()
                                    && previous_settings_search_active
                                        == atrium.settings_search_active()
                                    && (previous_keyboard_select_open
                                        != atrium.keyboard_select_open()
                                        || previous_mouse_select_open
                                            != atrium.mouse_select_open()
                                        || previous_keyboard_layout != atrium.keyboard_layout()
                                        || previous_mouse_acceleration
                                            != atrium.mouse_acceleration());
                                pending_app_render = if fast_card_hover {
                                    render_settings_card_hover(
                                        display,
                                        surface,
                                        previous_settings_card_hover,
                                        atrium.settings_card_hover(),
                                        atrium.settings_page(),
                                        &mut sequence,
                                    )
                                } else if fast_hover {
                                    if matches!(
                                        pending_settings_render,
                                        Some(PendingSettingsRender::Controls(_))
                                    ) {
                                        false
                                    } else {
                                        let retry = render_settings_select_hover(
                                            display,
                                            surface,
                                            atrium,
                                            &mut sequence,
                                        );
                                        if retry {
                                            pending_settings_render =
                                                Some(PendingSettingsRender::SelectHover(
                                                    surface.reference,
                                                ));
                                        } else {
                                            pending_settings_render = None;
                                        }
                                        false
                                    }
                                } else if settings_controls_changed {
                                    let retry = render_settings_controls_surface(
                                        display,
                                        surface,
                                        atrium,
                                        &mut sequence,
                                    );
                                    pending_settings_render = retry.then_some(
                                        PendingSettingsRender::Controls(surface.reference),
                                    );
                                    false
                                } else {
                                    render_settings_surface(display, surface, atrium, &mut sequence)
                                };
                            }
                        } else if routed.is_valid() {
                            if surface.app == logos_atrium::AppId::Terminal {
                                if atrium.owns_surface(surface.reference, terminal_client) {
                                    let _ = common::ipc_send_handle(terminal, &routed);
                                }
                            } else if surface.app == logos_atrium::AppId::System {
                                if atrium.owns_surface(surface.reference, system_client) {
                                    let _ = common::ipc_send_handle(system_surface_input, &routed);
                                }
                            } else if surface.app == logos_atrium::AppId::Calculator
                                && atrium.owns_surface(surface.reference, atrium_client)
                                && calculator.input(&local)
                            {
                                pending_app_render = render(
                                    display,
                                    atrium,
                                    calculator,
                                    atrium_client,
                                    &mut sequence,
                                );
                            } else if let Some(caps) =
                                program_surface_capabilities.iter().flatten().copied().find(
                                    |caps| atrium.owns_surface(surface.reference, caps.client),
                                )
                            {
                                let _ = common::ipc_send_handle(caps.input, &routed);
                            }
                        }
                        continue;
                    }
                }
                if event.pointer_event().is_some() {
                    continue;
                }
            }
            let action = sidebar_action.unwrap_or_else(|| atrium.input(&event));
            match action {
                logos_atrium::AtriumAction::Launch(app) if pending_surface.is_none() => {
                    atrium.close_command_menu();
                    if let Some(surface) = atrium.surface_for_app(app) {
                        if atrium.focus(surface.id).is_ok() {
                            send_surface_command(
                                display_control,
                                &mut surface_commands,
                                GuiSurfaceOperation::Focus,
                                surface.reference,
                                GuiRect::EMPTY,
                                &mut next_request,
                            );
                            pending_app_render =
                                render(display, atrium, calculator, atrium_client, &mut sequence);
                        }
                        continue;
                    }
                    if app == logos_atrium::AppId::Terminal {
                        let Some(client_request) = last_terminal_request else { continue };
                        if pending_client_request.is_none() {
                            pending_client_request = Some(client_request);
                        }
                    } else if app == logos_atrium::AppId::System {
                        let Some(client_request) = last_system_request else { continue };
                        if pending_client_request.is_none() {
                            pending_client_request = Some(client_request);
                            pending_client_response_capability = system_surface_response;
                        }
                    }
                    let client = match app {
                        logos_atrium::AppId::Terminal => terminal_client,
                        logos_atrium::AppId::System => system_client,
                        _ => atrium_client,
                    };
                    if !client.is_valid() {
                        continue;
                    }
                    let Ok(surface_request) = atrium.request_surface(app, client) else { continue };
                    let mut request = GuiSurfaceRequest::new(
                        GuiSurfaceOperation::CreateModal,
                        next_request_id(&mut next_request),
                    );
                    request.bounds = surface_request.bounds();
                    request.z_order = 2;
                    if app == logos_atrium::AppId::Terminal {
                        request.flags = logos_abi::GUI_SURFACE_FLAG_TERMINAL;
                    }
                    if common::ipc_send_handle(display_control, &request) == IpcStatus::Ok {
                        pending_surface = Some((request, Some(surface_request)));
                        pending_surface_for_client = matches!(
                            app,
                            logos_atrium::AppId::Terminal | logos_atrium::AppId::System
                        );
                    }
                }
                logos_atrium::AtriumAction::Logout => {
                    let home_surface = atrium.home_surface();
                    let terminal_surface =
                        atrium.surface_for_app(logos_atrium::AppId::Terminal).map(|s| s.reference);
                    let system_surface =
                        atrium.surface_for_app(logos_atrium::AppId::System).map(|s| s.reference);
                    let _ = atrium.apply_action(action);
                    authenticated = false;
                    pending_surface_for_client = false;
                    pending_client_request = None;
                    hide_surfaces(
                        display_control,
                        &mut surface_commands,
                        atrium,
                        &mut next_request,
                    );
                    if home_surface.is_valid() {
                        send_surface_command(
                            display_control,
                            &mut surface_commands,
                            GuiSurfaceOperation::Destroy,
                            home_surface,
                            GuiRect::EMPTY,
                            &mut next_request,
                        );
                    }
                    if let Some(surface) = terminal_surface {
                        queue_terminal_revoke(
                            &mut pending_client_response,
                            &mut deferred_terminal_revoke,
                            &mut next_request,
                            surface,
                        );
                    }
                    pending_terminal_update = None;
                    last_terminal_bounds = GuiRect::EMPTY;
                    if let Some(surface) = system_surface {
                        queue_system_revoke(
                            &mut pending_client_response,
                            &mut deferred_system_revoke,
                            system_surface_response,
                            &mut pending_client_response_capability,
                            &mut next_request,
                            surface,
                        );
                    }
                    send_lockscreen_section(lockscreen_control, true, &mut next_request);
                    let command = AtriumControl::new(AtriumControlOperation::Logout, 1);
                    let _ = common::ipc_send_handle(shell, &command);
                }
                logos_atrium::AtriumAction::LauncherChanged => {
                    pending_app_render =
                        render(display, atrium, calculator, atrium_client, &mut sequence);
                }
                logos_atrium::AtriumAction::OpenCommandMenu
                | logos_atrium::AtriumAction::CloseCommandMenu
                | logos_atrium::AtriumAction::CloseSettingsMenu => {
                    let _ = atrium.apply_action(action);
                    pending_app_render =
                        render(display, atrium, calculator, atrium_client, &mut sequence);
                }
                logos_atrium::AtriumAction::Shutdown => {
                    let _ = common::power(logos_abi::POWER_SHUTDOWN);
                    pending_app_render =
                        render(display, atrium, calculator, atrium_client, &mut sequence);
                }
                logos_atrium::AtriumAction::Restart => {
                    let _ = common::power(logos_abi::POWER_REBOOT);
                    pending_app_render =
                        render(display, atrium, calculator, atrium_client, &mut sequence);
                }
                logos_atrium::AtriumAction::CloseFocused => {
                    let old = atrium.focused_surface();
                    if atrium.apply_action(action).is_ok() {
                        if let Some(surface) = old {
                            if surface.app == logos_atrium::AppId::Terminal {
                                queue_terminal_revoke(
                                    &mut pending_client_response,
                                    &mut deferred_terminal_revoke,
                                    &mut next_request,
                                    surface.reference,
                                );
                            } else if surface.app == logos_atrium::AppId::System {
                                queue_system_revoke(
                                    &mut pending_client_response,
                                    &mut deferred_system_revoke,
                                    system_surface_response,
                                    &mut pending_client_response_capability,
                                    &mut next_request,
                                    surface.reference,
                                );
                            }
                            send_surface_command(
                                display_control,
                                &mut surface_commands,
                                GuiSurfaceOperation::Destroy,
                                surface.reference,
                                GuiRect::EMPTY,
                                &mut next_request,
                            );
                        }
                        queue_surface_updates(
                            display_control,
                            &mut surface_commands,
                            atrium,
                            &mut next_request,
                            &mut pending_terminal_update,
                            &mut last_terminal_bounds,
                        );
                        pending_app_render =
                            render(display, atrium, calculator, atrium_client, &mut sequence);
                    }
                }
                logos_atrium::AtriumAction::FocusNext
                | logos_atrium::AtriumAction::FocusPrevious
                | logos_atrium::AtriumAction::MoveFocused(_, _)
                | logos_atrium::AtriumAction::MoveFocusedInDirection(_) => {
                    if atrium.apply_action(action).is_ok() {
                        if let Some(surface) = atrium.focused_surface() {
                            if matches!(
                                action,
                                logos_atrium::AtriumAction::MoveFocused(_, _)
                                    | logos_atrium::AtriumAction::MoveFocusedInDirection(_)
                            ) {
                                queue_surface_updates(
                                    display_control,
                                    &mut surface_commands,
                                    atrium,
                                    &mut next_request,
                                    &mut pending_terminal_update,
                                    &mut last_terminal_bounds,
                                );
                            } else {
                                send_surface_command(
                                    display_control,
                                    &mut surface_commands,
                                    GuiSurfaceOperation::Focus,
                                    surface.reference,
                                    GuiRect::EMPTY,
                                    &mut next_request,
                                );
                            }
                        }
                        pending_app_render =
                            render(display, atrium, calculator, atrium_client, &mut sequence);
                    }
                }
                logos_atrium::AtriumAction::Split(_) => {
                    if atrium.apply_action(action).is_ok() {
                        queue_surface_updates(
                            display_control,
                            &mut surface_commands,
                            atrium,
                            &mut next_request,
                            &mut pending_terminal_update,
                            &mut last_terminal_bounds,
                        );
                        pending_app_render =
                            render(display, atrium, calculator, atrium_client, &mut sequence);
                    }
                }
                _ => {}
            }
            if action.routes_to_surface() {
                if let Some(surface) = atrium.focused_surface() {
                    if surface.app == logos_atrium::AppId::Terminal {
                        let routed = AtriumSurfaceInput::new(surface.reference, event);
                        if routed.is_valid()
                            && atrium.owns_surface(surface.reference, terminal_client)
                        {
                            let _ = common::ipc_send_handle(terminal, &routed);
                        }
                    } else if surface.app == logos_atrium::AppId::System {
                        let routed = AtriumSurfaceInput::new(surface.reference, event);
                        if routed.is_valid()
                            && atrium.owns_surface(surface.reference, system_client)
                        {
                            let _ = common::ipc_send_handle(system_surface_input, &routed);
                        }
                    } else if surface.app == logos_atrium::AppId::Calculator
                        && atrium.owns_surface(surface.reference, atrium_client)
                        && calculator.input(&event)
                    {
                        pending_app_render =
                            render(display, atrium, calculator, atrium_client, &mut sequence);
                    } else if let Some(caps) = program_surface_capabilities
                        .iter()
                        .flatten()
                        .copied()
                        .find(|caps| atrium.owns_surface(surface.reference, caps.client))
                    {
                        let routed = AtriumSurfaceInput::new(surface.reference, event);
                        if routed.is_valid() {
                            let _ = common::ipc_send_handle(caps.input, &routed);
                        }
                    }
                }
            }
        }
        if let Some(op) = pending_cursor_draw {
            match common::ipc_send_handle(display, &op) {
                IpcStatus::Ok => pending_cursor_draw = None,
                IpcStatus::Full => {}
                _ => {
                    pending_cursor_draw = None;
                    cursor_surface = SurfaceHandle::EMPTY;
                }
            }
        }
        let now_ticks = common::current_ticks();
        let menu_motion_active = unsafe {
            (&*core::ptr::addr_of!(COMMAND_MENU_TREE)).next_deadline(now_ticks).is_some()
        };
        if menu_motion_active {
            pending_app_render = render(display, atrium, calculator, atrium_client, &mut sequence);
        }
        if home_scene_pending() {
            let _ = flush_pending_home_scene(display);
        }
        let mut wait_capabilities = [logos_abi::CapabilityHandle::EMPTY; 24];
        let mut wait_count = 0;
        for capability in [
            input,
            display,
            display_control,
            display_response,
            shell_context,
            terminal_surface_request,
            terminal_surface_response,
            system_surface_request,
            system_surface_draw,
            terminal_render,
            display_render,
        ] {
            wait_capabilities[wait_count] = capability;
            wait_count += 1;
        }
        if let Ok(requests) = common::discover_capabilities_contract(
            logos_abi::IpcRights::Receive,
            logos_abi::IPC_CONTRACT_ATRIUM_SURFACE_REQUEST,
            core::mem::size_of::<AtriumSurfaceRequest>(),
        ) {
            for (client, capability) in requests {
                if client == atrium_client || client == terminal_client {
                    continue;
                }
                if wait_count < wait_capabilities.len() {
                    wait_capabilities[wait_count] = capability;
                    wait_count += 1;
                }
            }
        }
        for caps in program_surface_capabilities.iter().flatten().copied() {
            if wait_count < wait_capabilities.len() {
                wait_capabilities[wait_count] = caps.render;
                wait_count += 1;
            }
            if wait_count < wait_capabilities.len() {
                wait_capabilities[wait_count] = caps.draw;
                wait_count += 1;
            }
        }
        common::wait_on_capabilities(&wait_capabilities[..wait_count]);
    }
}

#[cfg(target_os = "none")]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    common::idle()
}

#[cfg(not(target_os = "none"))]
fn main() {}
