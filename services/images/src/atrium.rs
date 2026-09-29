#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]
#![cfg_attr(not(target_os = "none"), allow(dead_code, unused_imports, unused_variables))]

mod common;

use logos_abi::{
    AtriumApp, AtriumControl, AtriumControlOperation, AtriumSettingsOperation,
    AtriumSettingsRequest, AtriumSettingsResponse, AtriumSettingsStatus, AtriumSurfaceInput,
    AtriumSurfaceRequest, AtriumSurfaceResponse, GuiDrawCommand, GuiHook, GuiHookKind, GuiRect,
    GuiSceneOp, GuiSessionContext, GuiSurfaceOperation, GuiSurfaceRequest, GuiSurfaceResponse,
    GuiTextGridRow, InputMessage, IpcStatus, KeyCode, KeyState, ManagerOperation, ManagerRequest,
    ManagerResponse, ManagerStatus, MessageKind, PointerState, RenderMessage, ServiceManagerRecord,
    SurfaceHandle,
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
    core::mem::size_of::<GuiTextGridRow>(),
    logos_abi::IpcRights::Receive,
);
const DISPLAY_RENDER_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_RENDER,
    b"display",
    core::mem::size_of::<GuiTextGridRow>(),
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
// S4 (#81, ADR-0091): Atrium's settings load/save request/response to User.
// User only carries the opaque record inside its own canonical snapshot
// (ADR-0064); Atrium owns the record's layout and interpretation.
const USER_SETTINGS_SEND_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_ATRIUM_SETTINGS_REQUEST,
    b"user",
    core::mem::size_of::<AtriumSettingsRequest>(),
    logos_abi::IpcRights::Send,
);
const USER_SETTINGS_RECEIVE_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_ATRIUM_SETTINGS_RESPONSE,
    b"user",
    core::mem::size_of::<AtriumSettingsResponse>(),
    logos_abi::IpcRights::Receive,
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

static mut ATRIUM: logos_atrium::Atrium = logos_atrium::Atrium::new();
static mut CALCULATOR: logos_atrium::Calculator = logos_atrium::Calculator::new();
static mut COMMAND_MENU_TREE: logos_ui::UiComponentTree = logos_ui::UiComponentTree::new();
static mut SETTINGS_TREE: logos_ui::UiComponentTree = logos_ui::UiComponentTree::new();
static mut HOME_SCENE_PUBLISHER: logos_ui_graphics::UiScenePublisher =
    logos_ui_graphics::UiScenePublisher::new();
static mut HOME_SCENE_REPORTED: bool = false;
static mut HOME_SCENE_SEQUENCE: u32 = 0;
/// (hour, minute) last rendered on the Home header clock (H1). Compared each
/// time the main loop wakes (already bounded/no-busy-wait, see
/// `common::WAIT_TIMEOUT_TICKS`) so the clock advances without a dedicated
/// timer or extra busy-waking.
static mut HOME_CLOCK_LAST: Option<(u8, u8)> = None;
/// The home surface `HOME_CLOCK_LAST` was seeded for. A fresh session (new
/// surface handle) seeds the clock silently instead of forcing an extra
/// render that would race the login flow's own first publish for the same
/// surface/sequence and get rejected as backpressure.
static mut HOME_CLOCK_SURFACE: SurfaceHandle = SurfaceHandle::EMPTY;
static mut APP_SCENE_PUBLISHERS: [logos_ui_graphics::UiScenePublisher;
    logos_atrium::MAX_ATRIUM_SURFACES] =
    [logos_ui_graphics::UiScenePublisher::new(); logos_atrium::MAX_ATRIUM_SURFACES];
static mut APP_SCENE_SURFACES: [SurfaceHandle; logos_atrium::MAX_ATRIUM_SURFACES] =
    [SurfaceHandle::EMPTY; logos_atrium::MAX_ATRIUM_SURFACES];
static mut APP_SCENE_SEQUENCES: [u32; logos_atrium::MAX_ATRIUM_SURFACES] =
    [0; logos_atrium::MAX_ATRIUM_SURFACES];
static mut APP_SCENE_REPORTED: [bool; logos_atrium::MAX_ATRIUM_SURFACES] =
    [false; logos_atrium::MAX_ATRIUM_SURFACES];
static mut APP_SCENE_TREE: logos_ui::UiComponentTree = logos_ui::UiComponentTree::new();
/// Node id of the Terminal surface's own `TextGrid` content node, computed
/// from its position in `APP_SCENE_TREE` whenever `build_app_scene_tree`
/// rebuilds it. 0 means "not yet published" (`GuiTextGridRow::is_valid`
/// rejects `node_id == 0`), so a row arriving before the first publish is
/// simply dropped rather than mis-addressed (#74).
///
/// One per Atrium surface slot (T3b, #97): several Terminal panes can be
/// live at once, each with its own text-grid node.
static mut TERMINAL_GRID_NODE_IDS: [u32; logos_atrium::MAX_ATRIUM_SURFACES] =
    [0; logos_atrium::MAX_ATRIUM_SURFACES];
/// Packed tab-bar state Terminal last reported on each pane's surface
/// request (#76; see `pack_terminal_tab_state`), indexed by surface slot and
/// read by `build_app_scene_tree` when laying out the tab strip.
static mut TERMINAL_TAB_STATES: [u16; logos_atrium::MAX_ATRIUM_SURFACES] =
    [0; logos_atrium::MAX_ATRIUM_SURFACES];

/// Slot-indexed grid node id of a live Terminal surface; 0 if unpublished.
fn terminal_grid_node_id(surface: SurfaceHandle) -> u32 {
    app_scene_slot(surface)
        .map_or(0, |slot| unsafe { (*core::ptr::addr_of!(TERMINAL_GRID_NODE_IDS))[slot] })
}

fn terminal_tab_state(surface: SurfaceHandle) -> u16 {
    app_scene_slot(surface)
        .map_or(0, |slot| unsafe { (*core::ptr::addr_of!(TERMINAL_TAB_STATES))[slot] })
}

fn set_terminal_tab_state(surface: SurfaceHandle, state: u16) {
    if let Some(slot) = app_scene_slot(surface) {
        unsafe { (*core::ptr::addr_of_mut!(TERMINAL_TAB_STATES))[slot] = state };
    }
}

/// Session slots (bit N = slot N) currently shown as tabs across every
/// Terminal pane. Each session belongs to exactly one pane, so the popcount
/// is how many of the shared `MAX_TERMINAL_SESSIONS` are taken (T3b, #97).
fn terminal_open_session_bitmap(atrium: &logos_atrium::Atrium) -> u8 {
    atrium.surfaces().filter(|surface| surface.app == logos_atrium::AppId::Terminal).fold(
        0u8,
        |bitmap, surface| {
            bitmap | logos_abi::terminal_tab_open_bitmap(terminal_tab_state(surface.reference))
        },
    )
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

#[cfg(feature = "qemu-proof")]
fn proof_app_scene_published(app: logos_atrium::AppId, surface: SurfaceHandle, bounds: GuiRect) {
    use core::fmt::Write as _;

    struct ProofLine {
        bytes: [u8; 112],
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

    let title = match app {
        logos_atrium::AppId::Calculator => "Calculator",
        logos_atrium::AppId::Files => "Files",
        logos_atrium::AppId::Terminal => "Terminal",
        logos_atrium::AppId::Settings => "Settings",
        _ => return,
    };
    let mut line = ProofLine { bytes: [0; 112], length: 0 };
    let _ = write!(
        line,
        "LogOS vNext: Atrium app={title} scene published surface={}/{} bounds={},{},{},{}",
        surface.slot, surface.generation, bounds.x, bounds.y, bounds.width, bounds.height,
    );
    common::proof_line(&line.bytes[..line.length]);
}

#[cfg(not(feature = "qemu-proof"))]
fn proof_app_scene_published(_app: logos_atrium::AppId, _surface: SurfaceHandle, _bounds: GuiRect) {
}

struct HomeSceneSink(logos_abi::CapabilityHandle);

impl logos_ui_graphics::UiSceneSink for HomeSceneSink {
    fn send(&mut self, operation: &GuiSceneOp) -> IpcStatus {
        common::ipc_send_handle(self.0, operation)
    }
}

struct AppSceneSink(logos_abi::CapabilityHandle);

impl logos_ui_graphics::UiSceneSink for AppSceneSink {
    fn send(&mut self, operation: &GuiSceneOp) -> IpcStatus {
        common::ipc_send_handle(self.0, operation)
    }
}

fn add_app_scene_node(
    tree: &mut logos_ui::UiComponentTree,
    parent: logos_ui::UiNodeHandle,
    kind: logos_ui::UiNodeKind,
    bounds: GuiRect,
    text: &[u8],
    styles: logos_ui::UiStyleList,
) -> Option<logos_ui::UiNodeHandle> {
    let handle = tree.insert(kind, parent, tree.tree().len() as u16).ok()?;
    tree.tree_mut()
        .set_bounds(handle, logos_ui::UiRect::new(bounds.x, bounds.y, bounds.width, bounds.height))
        .ok()?;
    if !text.is_empty() {
        tree.set_text(handle, logos_ui::UiText::from_bytes(text)?).ok()?;
    }
    tree.set_styles(handle, styles).ok()?;
    Some(handle)
}

fn build_app_scene_tree(
    surface: logos_atrium::Surface,
    calculator: &logos_atrium::Calculator,
) -> bool {
    let title = match surface.app {
        logos_atrium::AppId::Calculator => b"Calculator".as_slice(),
        logos_atrium::AppId::Files => b"Files".as_slice(),
        logos_atrium::AppId::Terminal => b"Terminal".as_slice(),
        _ => return false,
    };
    let tree = unsafe { &mut *core::ptr::addr_of_mut!(APP_SCENE_TREE) };
    tree.clear();
    let bounds = surface.bounds;
    let Some(root) = add_app_scene_node(
        tree,
        logos_ui::UiNodeHandle::EMPTY,
        logos_ui::UiNodeKind::Root,
        bounds,
        b"",
        logos_ui::UiStyleList::EMPTY,
    ) else {
        return false;
    };
    let mut close_styles = logos_ui::UiStyleList::EMPTY;
    if !close_styles.push(logos_ui::UiStyle::BackgroundAccent)
        || !close_styles.push(logos_ui::UiStyle::RoundedLarge)
    {
        return false;
    }
    let close = logos_atrium::surface_close_bounds(bounds);
    if add_app_scene_node(
        tree,
        root,
        logos_ui::UiNodeKind::Panel,
        GuiRect::new(bounds.x, bounds.y, bounds.width, logos_atrium::STATUS_BAR_BOUNDS.height),
        b"",
        logos_ui::UiStyleList::EMPTY,
    )
    .is_none()
        || add_app_scene_node(
            tree,
            root,
            logos_ui::UiNodeKind::Label,
            // Full chrome-strip height, starting at the surface's own top
            // edge — matching System's title layout (`system_layout`'s
            // `title` rect) so the label centers at the same vertical
            // position instead of ~4px lower (#75).
            GuiRect::new(
                bounds.x.saturating_add(16),
                bounds.y,
                180,
                logos_atrium::STATUS_BAR_BOUNDS.height,
            ),
            title,
            logos_ui::UiStyleList::EMPTY,
        )
        .is_none()
        || add_app_scene_node(
            tree,
            root,
            logos_ui::UiNodeKind::Panel,
            GuiRect::new(
                bounds.x.saturating_add(close.x),
                bounds.y.saturating_add(close.y),
                close.width,
                close.height,
            ),
            b"",
            close_styles,
        )
        .is_none()
        || add_app_scene_node(
            tree,
            root,
            logos_ui::UiNodeKind::Label,
            GuiRect::new(
                bounds.x.saturating_add(close.x).saturating_add(16),
                bounds.y.saturating_add(10),
                20,
                20,
            ),
            b"X",
            logos_ui::UiStyleList::EMPTY,
        )
        .is_none()
    {
        return false;
    }

    let mut rounded = logos_ui::UiStyleList::EMPTY;
    let _ = rounded.push(logos_ui::UiStyle::RoundedLarge);
    match surface.app {
        logos_atrium::AppId::Calculator => {
            let panel_bounds = GuiRect::new(
                bounds.x.saturating_add(12),
                bounds.y.saturating_add(40),
                bounds.width.saturating_sub(24),
                bounds.height.saturating_sub(52),
            );
            let display_bounds =
                GuiRect::new(bounds.x.saturating_add(20), bounds.y.saturating_add(52), 260, 40);
            if add_app_scene_node(
                tree,
                root,
                logos_ui::UiNodeKind::Panel,
                panel_bounds,
                b"",
                rounded,
            )
            .is_none()
                || add_app_scene_node(
                    tree,
                    root,
                    logos_ui::UiNodeKind::Button,
                    display_bounds,
                    b"",
                    rounded,
                )
                .is_none()
                || add_app_scene_node(
                    tree,
                    root,
                    logos_ui::UiNodeKind::Label,
                    GuiRect::new(bounds.x.saturating_add(32), bounds.y.saturating_add(64), 240, 20),
                    calculator.display(),
                    logos_ui::UiStyleList::EMPTY,
                )
                .is_none()
            {
                return false;
            }
            let rows: [&[u8]; 4] = [
                b"[ 7 ]   [ 8 ]   [ 9 ]   [ / ]",
                b"[ 4 ]   [ 5 ]   [ 6 ]   [ * ]",
                b"[ 1 ]   [ 2 ]   [ 3 ]   [ - ]",
                b"[ 0 ]   [ . ]   [ = ]   [ + ]",
            ];
            for (row, labels) in rows.into_iter().enumerate() {
                if add_app_scene_node(
                    tree,
                    root,
                    logos_ui::UiNodeKind::Label,
                    GuiRect::new(
                        bounds.x.saturating_add(20),
                        bounds
                            .y
                            .saturating_add(logos_atrium::CALCULATOR_BUTTON_TOP + row as i32 * 28),
                        bounds.width.saturating_sub(40),
                        20,
                    ),
                    labels,
                    logos_ui::UiStyleList::EMPTY,
                )
                .is_none()
                {
                    return false;
                }
            }
        }
        logos_atrium::AppId::Files => {
            let mut muted = logos_ui::UiStyleList::EMPTY;
            let _ = muted.push(logos_ui::UiStyle::TextMuted);
            if add_app_scene_node(
                tree,
                root,
                logos_ui::UiNodeKind::Button,
                GuiRect::new(bounds.x.saturating_add(20), bounds.y.saturating_add(52), 260, 48),
                b"",
                rounded,
            )
            .is_none()
                || add_app_scene_node(
                    tree,
                    root,
                    logos_ui::UiNodeKind::Label,
                    GuiRect::new(
                        bounds.x.saturating_add(32),
                        bounds.y.saturating_add(82),
                        bounds.width.saturating_sub(64),
                        20,
                    ),
                    b"No files found",
                    logos_ui::UiStyleList::EMPTY,
                )
                .is_none()
                || add_app_scene_node(
                    tree,
                    root,
                    logos_ui::UiNodeKind::Label,
                    GuiRect::new(
                        bounds.x.saturating_add(24),
                        bounds.y.saturating_add(132),
                        bounds.width.saturating_sub(48),
                        20,
                    ),
                    b"Storage browser is not available yet",
                    muted,
                )
                .is_none()
            {
                return false;
            }
        }
        logos_atrium::AppId::Terminal => {
            // Tab bar (#76): one chip per open session plus a trailing
            // add-tab button, laid out from the same shared geometry
            // Terminal's own pointer hit-testing uses, so a click always
            // lands on what's actually drawn there.
            let tab_state = terminal_tab_state(surface.reference);
            let open_bitmap = logos_abi::terminal_tab_open_bitmap(tab_state);
            let active_slot = logos_abi::terminal_tab_active_slot(tab_state);
            let mut active_styles = logos_ui::UiStyleList::EMPTY;
            let _ = active_styles.push(logos_ui::UiStyle::BackgroundAccent);
            let _ = active_styles.push(logos_ui::UiStyle::RoundedLarge);
            let mut inactive_styles = logos_ui::UiStyleList::EMPTY;
            let _ = inactive_styles.push(logos_ui::UiStyle::RoundedLarge);
            let mut muted = logos_ui::UiStyleList::EMPTY;
            let _ = muted.push(logos_ui::UiStyle::TextMuted);
            for slot in 0..logos_abi::TERMINAL_MAX_TABS {
                if open_bitmap & (1 << slot) == 0 {
                    continue;
                }
                let chip = logos_abi::terminal_tab_chip_bounds(bounds, slot);
                let styles =
                    if slot as u8 == active_slot { active_styles } else { inactive_styles };
                if add_app_scene_node(tree, root, logos_ui::UiNodeKind::Panel, chip, b"", styles)
                    .is_none()
                {
                    return false;
                }
                let label = [b'1' + slot as u8];
                if add_app_scene_node(
                    tree,
                    root,
                    logos_ui::UiNodeKind::Label,
                    GuiRect::new(chip.x.saturating_add(8), chip.y.saturating_add(2), 60, 16),
                    &label[..1],
                    logos_ui::UiStyleList::EMPTY,
                )
                .is_none()
                {
                    return false;
                }
                let close = logos_abi::terminal_tab_close_bounds(bounds, slot);
                let Some(close_node) =
                    add_app_scene_node(tree, root, logos_ui::UiNodeKind::Label, close, b"", muted)
                else {
                    return false;
                };
                if tree.set_icon(close_node, logos_ui::UiIcon::Close).is_err() {
                    return false;
                }
            }
            let add_bounds = logos_abi::terminal_tab_add_button_bounds(bounds);
            let Some(add_node) = add_app_scene_node(
                tree,
                root,
                logos_ui::UiNodeKind::Button,
                add_bounds,
                b"",
                inactive_styles,
            ) else {
                return false;
            };
            if tree.set_icon(add_node, logos_ui::UiIcon::Add).is_err() {
                return false;
            }

            // Content below the title bar and tab strip, inset by the inner
            // padding (#75), sized from the surface bounds (works in tiled
            // panes and after resize) via the same shared math Terminal's
            // own `resize_to_surface` uses, so both sides always agree on
            // the grid shape (ADR-0087; a prior duplicated formula could
            // drift and rows would be clipped or rejected in narrow panes).
            let (_, _, grid_bounds) = logos_abi::terminal_grid_metrics(bounds);
            let Some(grid) = add_app_scene_node(
                tree,
                root,
                logos_ui::UiNodeKind::TextGrid,
                grid_bounds,
                b"",
                logos_ui::UiStyleList::EMPTY,
            ) else {
                return false;
            };
            let node_id = (grid.slot as u32).saturating_mul(3).saturating_add(1);
            if let Some(slot) = app_scene_slot(surface.reference) {
                unsafe {
                    (*core::ptr::addr_of_mut!(TERMINAL_GRID_NODE_IDS))[slot] = node_id;
                }
            }
        }
        _ => return false,
    }
    true
}

fn app_scene_slot(surface: SurfaceHandle) -> Option<usize> {
    let slot = usize::from(surface.slot);
    (surface.is_valid() && slot < logos_atrium::MAX_ATRIUM_SURFACES).then_some(slot)
}

fn bind_app_scene_publisher(surface: logos_atrium::Surface) {
    if !matches!(
        surface.app,
        logos_atrium::AppId::Calculator
            | logos_atrium::AppId::Files
            | logos_atrium::AppId::Terminal
            | logos_atrium::AppId::Settings
    ) {
        return;
    }
    let Some(slot) = app_scene_slot(surface.reference) else { return };
    unsafe {
        let publishers = &mut *core::ptr::addr_of_mut!(APP_SCENE_PUBLISHERS);
        let surfaces = &mut *core::ptr::addr_of_mut!(APP_SCENE_SURFACES);
        let sequences = &mut *core::ptr::addr_of_mut!(APP_SCENE_SEQUENCES);
        let reported = &mut *core::ptr::addr_of_mut!(APP_SCENE_REPORTED);
        if surfaces[slot] != surface.reference {
            (*core::ptr::addr_of_mut!(TERMINAL_GRID_NODE_IDS))[slot] = 0;
            (*core::ptr::addr_of_mut!(TERMINAL_TAB_STATES))[slot] = 0;
            publishers[slot].reset();
            surfaces[slot] = surface.reference;
            sequences[slot] = 0;
            reported[slot] = false;
        }
    }
}

fn unbind_app_scene_publisher(surface: SurfaceHandle) {
    let Some(slot) = app_scene_slot(surface) else { return };
    unsafe {
        let publishers = &mut *core::ptr::addr_of_mut!(APP_SCENE_PUBLISHERS);
        let surfaces = &mut *core::ptr::addr_of_mut!(APP_SCENE_SURFACES);
        let sequences = &mut *core::ptr::addr_of_mut!(APP_SCENE_SEQUENCES);
        let reported = &mut *core::ptr::addr_of_mut!(APP_SCENE_REPORTED);
        if surfaces[slot] == surface {
            (*core::ptr::addr_of_mut!(TERMINAL_GRID_NODE_IDS))[slot] = 0;
            (*core::ptr::addr_of_mut!(TERMINAL_TAB_STATES))[slot] = 0;
            publishers[slot].reset();
            surfaces[slot] = SurfaceHandle::EMPTY;
            sequences[slot] = 0;
            reported[slot] = false;
        }
    }
}

/// Snapshots the service list for Settings' About page via the same
/// service-manager query the System surface already uses (`system.rs`'s
/// `refresh_status`); no new ABI, no duplicated syscall path.
fn about_services_snapshot() -> [ServiceManagerRecord; logos_atrium::MAX_ABOUT_SERVICES] {
    let mut records = [ServiceManagerRecord::EMPTY; logos_atrium::MAX_ABOUT_SERVICES];
    let mut count = 0usize;
    let mut cursor = 0u64;
    let mut request_id = 1u32;
    while count < records.len() {
        let request =
            ManagerRequest { cursor, ..ManagerRequest::new(ManagerOperation::List, request_id) };
        let mut response =
            ManagerResponse::new(ManagerOperation::List, ManagerStatus::Malformed, request_id);
        if common::manager_call(&request, &mut response) != IpcStatus::Ok
            || response.status != ManagerStatus::Ok
        {
            break;
        }
        records[count] = response.record;
        count += 1;
        request_id = request_id.wrapping_add(1).max(1);
        if response.cursor == u64::MAX || response.cursor <= cursor {
            break;
        }
        cursor = response.cursor;
    }
    records
}

fn render_app_scene(
    display: logos_abi::CapabilityHandle,
    surface: logos_atrium::Surface,
    atrium: &logos_atrium::Atrium,
    calculator: &logos_atrium::Calculator,
) -> bool {
    let Some(slot) = app_scene_slot(surface.reference) else { return false };
    let (frame, resuming) = unsafe {
        let publishers = &mut *core::ptr::addr_of_mut!(APP_SCENE_PUBLISHERS);
        let surfaces = &*core::ptr::addr_of!(APP_SCENE_SURFACES);
        let sequences = &mut *core::ptr::addr_of_mut!(APP_SCENE_SEQUENCES);
        if surfaces[slot] != surface.reference {
            return false;
        }
        let resuming = publishers[slot].is_pending_for(surface.reference, sequences[slot]);
        if !resuming {
            sequences[slot] = sequences[slot].wrapping_add(1).max(1);
        }
        (sequences[slot], resuming)
    };
    let tree = if surface.app == logos_atrium::AppId::Settings {
        let tree = unsafe { &mut *core::ptr::addr_of_mut!(SETTINGS_TREE) };
        // Only query the manager while About is open; other pages never show it.
        let (about_services, system) =
            if atrium.settings_page() == logos_atrium::SettingsPage::About {
                let system = common::system_info();
                proof_about_system(system);
                (about_services_snapshot(), system)
            } else {
                (
                    [ServiceManagerRecord::EMPTY; logos_atrium::MAX_ABOUT_SERVICES],
                    logos_abi::SystemInfo { cpus: 0, mem_total_mib: 0, mem_used_mib: 0 },
                )
            };
        if !logos_atrium::build_settings_scene(
            tree,
            surface.bounds,
            atrium,
            &about_services,
            system,
        ) {
            return false;
        }
        tree
    } else {
        if !build_app_scene_tree(surface, calculator) {
            return false;
        }
        unsafe { &mut *core::ptr::addr_of_mut!(APP_SCENE_TREE) }
    };
    let publisher = unsafe { &mut (*core::ptr::addr_of_mut!(APP_SCENE_PUBLISHERS))[slot] };
    let mut sink = AppSceneSink(display);
    match publisher.publish(surface.reference, frame, tree, atrium.app_theme(), None, &mut sink) {
        Ok((IpcStatus::Ok, _)) => {
            let reported = unsafe { &mut (*core::ptr::addr_of_mut!(APP_SCENE_REPORTED))[slot] };
            if !*reported {
                *reported = true;
                proof_app_scene_published(surface.app, surface.reference, surface.bounds);
            }
            resuming
        }
        Ok((IpcStatus::Full, _)) => true,
        Ok(_) | Err(_) => false,
    }
}
fn publish_home_scene(
    display: logos_abi::CapabilityHandle,
    surface: SurfaceHandle,
    atrium: &logos_atrium::Atrium,
    sequence: u32,
) -> IpcStatus {
    let pending =
        unsafe { (*core::ptr::addr_of!(HOME_SCENE_PUBLISHER)).is_pending_for(surface, sequence) };
    let tree = unsafe { &mut *core::ptr::addr_of_mut!(COMMAND_MENU_TREE) };
    if !pending
        && !logos_atrium::build_home_scene(
            tree,
            atrium,
            common::current_ticks(),
            common::wall_time(),
        )
    {
        return IpcStatus::Malformed;
    }
    let mut sink = HomeSceneSink(display);
    match unsafe {
        (*core::ptr::addr_of_mut!(HOME_SCENE_PUBLISHER)).publish(
            surface,
            sequence,
            tree,
            atrium.home_theme(),
            None,
            &mut sink,
        )
    } {
        Ok((status, _)) => status,
        Err(_) => IpcStatus::Malformed,
    }
}

fn reset_home_scene_publisher() {
    unsafe {
        (*core::ptr::addr_of_mut!(HOME_SCENE_PUBLISHER)).reset();
        *core::ptr::addr_of_mut!(HOME_SCENE_REPORTED) = false;
    }
}

#[inline(never)]
fn draw_settings_ui(
    display: logos_abi::CapabilityHandle,
    surface: logos_atrium::Surface,
    atrium: &logos_atrium::Atrium,
    calculator: &logos_atrium::Calculator,
) -> bool {
    render_app_scene(display, surface, atrium, calculator)
}
fn draw_app(
    display: logos_abi::CapabilityHandle,
    surface: logos_atrium::Surface,
    atrium: &logos_atrium::Atrium,
    calculator: &logos_atrium::Calculator,
    atrium_client: logos_abi::ServiceHandle,
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
    match surface.app {
        logos_atrium::AppId::Calculator
        | logos_atrium::AppId::Files
        | logos_atrium::AppId::Terminal => render_app_scene(display, surface, atrium, calculator),
        logos_atrium::AppId::Settings => draw_settings_ui(display, surface, atrium, calculator),
        logos_atrium::AppId::System => {
            // The System service owns this retained scene, including its chrome.
            false
        }
    }
}

fn next_request_id(next: &mut u32) -> u32 {
    let value = *next;
    *next = next.wrapping_add(1).max(1);
    value
}

/// QEMU proof marker (S4, #81, ADR-0091): the settings state actually in
/// effect once the boot-time Load resolves (applied, or skipped because the
/// user already changed a setting first -- see `should_apply_loaded_settings`
/// in `logos-atrium`), so a change-then-reboot proof can assert persistence
/// with a log search instead of a pixel match.
fn proof_settings_loaded(light_theme: bool) {
    proof_line(if light_theme {
        b"LogOS vNext: Atrium settings loaded light_theme=1"
    } else {
        b"LogOS vNext: Atrium settings loaded light_theme=0"
    });
}

/// QEMU proof marker (S3b, #95, ADR-0093): the system-info reading shown on
/// the About page, once per boot, so the proof can compare `cpus=` to `-Cpus`.
#[cfg(feature = "qemu-proof")]
fn proof_about_system(info: logos_abi::SystemInfo) {
    use core::fmt::Write as _;
    use core::sync::atomic::{AtomicBool, Ordering};

    static LOGGED: AtomicBool = AtomicBool::new(false);
    if LOGGED.swap(true, Ordering::Relaxed) {
        return;
    }
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
        "LogOS vNext: Atrium about cpus={} mem_total={} mem_used={}",
        info.cpus, info.mem_total_mib, info.mem_used_mib,
    );
    common::proof_line(&line.bytes[..line.length]);
}

#[cfg(not(feature = "qemu-proof"))]
fn proof_about_system(_info: logos_abi::SystemInfo) {}

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
    pending_terminal_update: &mut TerminalUpdates,
    last_terminal_bounds: &mut TerminalBounds,
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

/// Per-surface-slot Terminal bounds updates waiting to be sent, and the
/// bounds last sent for each (T3b, #97: several panes can move at once when
/// a divider is dragged).
type TerminalUpdates = [Option<AtriumSurfaceResponse>; logos_atrium::MAX_ATRIUM_SURFACES];
type TerminalBounds = [GuiRect; logos_atrium::MAX_ATRIUM_SURFACES];

fn queue_terminal_surface_update(
    pending: &mut TerminalUpdates,
    last_bounds: &mut TerminalBounds,
    atrium: &logos_atrium::Atrium,
    next: &mut u32,
) {
    let mut live = [false; logos_atrium::MAX_ATRIUM_SURFACES];
    for surface in atrium.surfaces().filter(|s| s.app == logos_atrium::AppId::Terminal) {
        let Some(slot) = app_scene_slot(surface.reference) else { continue };
        live[slot] = true;
        if surface.bounds != last_bounds[slot] {
            last_bounds[slot] = surface.bounds;
            pending[slot] = Some(AtriumSurfaceResponse::update(
                next_request_id(next),
                surface.reference,
                surface.bounds,
            ));
        }
    }
    for (slot, is_live) in live.iter().enumerate() {
        if !is_live {
            pending[slot] = None;
            last_bounds[slot] = GuiRect::EMPTY;
        }
    }
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

/// Terminal revokes waiting behind an occupied `pending_client_response`
/// (T3b, #97: logout revokes every Terminal pane at once).
type DeferredRevokes = [SurfaceHandle; logos_atrium::MAX_ATRIUM_SURFACES];

fn queue_terminal_revoke(
    pending: &mut Option<AtriumSurfaceResponse>,
    deferred: &mut DeferredRevokes,
    next: &mut u32,
    surface: SurfaceHandle,
) {
    if !surface.is_valid() {
        return;
    }
    if pending.is_none() {
        *pending = Some(AtriumSurfaceResponse::revoke(next_request_id(next), surface));
    } else if let Some(free) = deferred.iter_mut().find(|slot| !slot.is_valid()) {
        *free = surface;
    }
}

/// Every live Terminal surface, for revoking them all on logout.
fn terminal_surfaces(atrium: &logos_atrium::Atrium) -> DeferredRevokes {
    let mut list = [SurfaceHandle::EMPTY; logos_atrium::MAX_ATRIUM_SURFACES];
    let terminals = atrium.surfaces().filter(|s| s.app == logos_atrium::AppId::Terminal);
    for (entry, surface) in list.iter_mut().zip(terminals) {
        *entry = surface.reference;
    }
    list
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
        unbind_app_scene_publisher(surface);
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
    reset_home_scene_publisher();
}

fn render_home_surface(
    display: logos_abi::CapabilityHandle,
    atrium: &logos_atrium::Atrium,
) -> bool {
    let Some(home) = atrium.home_surface().is_valid().then_some(atrium.home_surface()) else {
        return false;
    };
    let (frame, resuming) = unsafe {
        let sequence = &mut *core::ptr::addr_of_mut!(HOME_SCENE_SEQUENCE);
        let resuming = (*core::ptr::addr_of!(HOME_SCENE_PUBLISHER)).is_pending_for(home, *sequence);
        if !resuming {
            *sequence = sequence.wrapping_add(1).max(1);
        }
        (*sequence, resuming)
    };
    match publish_home_scene(display, home, atrium, frame) {
        IpcStatus::Ok => {
            unsafe {
                if !*core::ptr::addr_of!(HOME_SCENE_REPORTED) {
                    proof_line(b"LogOS vNext: Atrium home scene built");
                    *core::ptr::addr_of_mut!(HOME_SCENE_REPORTED) = true;
                }
            }
            resuming
        }
        IpcStatus::Full => true,
        _ => {
            reset_home_scene_publisher();
            false
        }
    }
}

fn render(
    display: logos_abi::CapabilityHandle,
    atrium: &logos_atrium::Atrium,
    calculator: &logos_atrium::Calculator,
    atrium_client: logos_abi::ServiceHandle,
) -> bool {
    if render_home_surface(display, atrium) {
        return true;
    }
    for surface in atrium.surfaces() {
        if draw_app(display, surface, atrium, calculator, atrium_client) {
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
    let user_settings_send =
        common::capability_handle(USER_SETTINGS_SEND_CAPABILITY).unwrap_or_else(|_| common::idle());
    let user_settings_receive = common::capability_handle(USER_SETTINGS_RECEIVE_CAPABILITY)
        .unwrap_or_else(|_| common::idle());

    let atrium = unsafe { &mut *core::ptr::addr_of_mut!(ATRIUM) };
    let calculator = unsafe { &mut *core::ptr::addr_of_mut!(CALCULATOR) };
    // Atrium already boots with `AtriumSettingsRecord::DEFAULT` --
    // `Atrium::new()`'s own defaults, kept in lockstep with `DEFAULT` by a
    // host test -- so nothing needs applying here. The persisted record, if
    // any, is loaded non-blockingly from the main loop below (S4, #81,
    // ADR-0091): never delay the first render, including LockScreen, on
    // User's answer.
    let atrium_client = common::bootstrap_page().service;
    let mut next_request = 1u32;
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
    let mut pending_terminal_update: TerminalUpdates = [None; logos_atrium::MAX_ATRIUM_SURFACES];
    let mut last_terminal_bounds: TerminalBounds =
        [GuiRect::EMPTY; logos_atrium::MAX_ATRIUM_SURFACES];
    let mut deferred_terminal_admit: Option<AtriumSurfaceResponse> = None;
    let mut deferred_terminal_revoke: DeferredRevokes =
        [SurfaceHandle::EMPTY; logos_atrium::MAX_ATRIUM_SURFACES];
    let mut deferred_system_revoke: Option<SurfaceHandle> = None;
    let mut pending_render: Option<GuiTextGridRow> = None;
    let mut pending_draw: Option<GuiSceneOp> = None;
    let mut pending_app_render = false;
    let mut cursor_surface = SurfaceHandle::EMPTY;
    let mut pending_cursor_surface = queue_cursor_surface(display_control, &mut next_request);
    let mut cursor_x = (logos_abi::DEFAULT_SCREEN_WIDTH / 2) as i16;
    let mut cursor_y = (logos_abi::DEFAULT_SCREEN_HEIGHT / 2) as i16;
    let mut cursor_sequence = 1u32;
    let mut pending_cursor_draw: Option<GuiSceneOp> = None;
    // LockScreen, Terminal and System start with default appearance (no
    // flags).
    let mut lockscreen_appearance = 0u16;
    let mut terminal_appearance: Option<(SurfaceHandle, u16)> = None;
    let mut system_appearance: Option<(SurfaceHandle, u16)> = None;
    // Home applies its own theme (home_theme()) directly, with no separate
    // channel to itself, so unlike LockScreen/Terminal/System it needs no
    // flags to arrive -- just a redraw forced whenever `flags` changes,
    // since otherwise it only repaints on its own triggers (input, the
    // clock's minute ticking over, menu motion), which a pure theme flip
    // is none of.
    let mut home_appearance = 0u16;
    let mut pending_input_settings: Option<logos_abi::InputSettings> =
        Some(atrium.input_settings());
    // S4 (#81, ADR-0091): the record last known to be durable. A settings
    // change only queues a save when the encoded record actually differs,
    // not on every frame. Starts as the (default) record already applied;
    // updated once a persisted Load response is applied, and on every save.
    let mut last_saved_settings = atrium.settings_record().encode();
    let mut pending_settings_save: Option<[u8; logos_abi::ATRIUM_SETTINGS_RECORD_BYTES]> = None;
    // Non-blocking boot-time settings load: `Some` until the request is
    // sent (retried on `Full`, same as every other pending-send below); the
    // response is picked up by the general receive drain further down.
    let mut pending_settings_load_request = Some(AtriumSettingsRequest::new(
        AtriumSettingsOperation::Load,
        next_request_id(&mut next_request),
    ));
    // Once the user changes a setting in this boot, their live choice wins
    // over a Load response that was already in flight (`should_apply_
    // loaded_settings`, host-tested in `logos-atrium`).
    let mut settings_changed_by_user = false;
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
        if let Some(settings) = pending_input_settings {
            match common::ipc_send_handle(input_settings, &settings) {
                IpcStatus::Ok => pending_input_settings = None,
                IpcStatus::Full => {}
                _ => pending_input_settings = None,
            }
        }
        if let Some(request) = pending_settings_load_request {
            match common::ipc_send_handle(user_settings_send, &request) {
                IpcStatus::Ok => pending_settings_load_request = None,
                IpcStatus::Full => {}
                // User's Load capability is gone; stay on the defaults
                // already in effect rather than retry forever.
                _ => {
                    pending_settings_load_request = None;
                    proof_settings_loaded(atrium.light_theme());
                }
            }
        }
        if let Some(data) = pending_settings_save {
            let settings_save_request = AtriumSettingsRequest::with_data(
                AtriumSettingsOperation::Save,
                next_request_id(&mut next_request),
                data,
            );
            match common::ipc_send_handle(user_settings_send, &settings_save_request) {
                IpcStatus::Ok => pending_settings_save = None,
                IpcStatus::Full => {}
                // No path to User to retry on; keep the in-memory state and
                // try again on the next change rather than looping forever.
                _ => pending_settings_save = None,
            }
        }
        // Drain every response on this channel (save acks and the one Load
        // answer) so the bounded response queue never fills.
        let mut settings_response = AtriumSettingsResponse::new(
            AtriumSettingsRequest::new(AtriumSettingsOperation::Load, 1),
            AtriumSettingsStatus::Invalid,
        );
        while common::ipc_receive_handle(user_settings_receive, &mut settings_response)
            == IpcStatus::Ok
        {
            match settings_response.operation {
                AtriumSettingsOperation::Save => {
                    // `persist_catalog` on the User side only returns `Ok`
                    // after its own blocking Storage round trip (ADR-0064),
                    // so this status genuinely means durable -- worth a
                    // proof marker for a change-then-reboot QEMU proof.
                    if settings_response.status == AtriumSettingsStatus::Ok {
                        proof_line(b"LogOS vNext: Atrium settings saved");
                    }
                }
                AtriumSettingsOperation::Load => {
                    if logos_atrium::should_apply_loaded_settings(settings_changed_by_user) {
                        let record = if settings_response.status == AtriumSettingsStatus::Ok {
                            logos_atrium::AtriumSettingsRecord::decode(&settings_response.data)
                        } else {
                            logos_atrium::AtriumSettingsRecord::DEFAULT
                        };
                        let fps_overlay = atrium.fps_overlay();
                        atrium.apply_settings_record(record);
                        if atrium.fps_overlay() != fps_overlay {
                            queue_fps_toggle(&mut surface_commands, &mut next_request);
                            surface_commands.flush(display_control);
                        }
                        // Already durable (it's what we just loaded); update
                        // the baseline so a later real change diffs against
                        // it instead of the boot defaults.
                        last_saved_settings = record.encode();
                        pending_input_settings = Some(atrium.input_settings());
                        if atrium.home_surface().is_valid() {
                            pending_app_render =
                                render_home_surface(display, atrium) || pending_app_render;
                        }
                    }
                    proof_settings_loaded(atrium.light_theme());
                }
            }
        }
        if pending_app_render {
            pending_app_render = render(display, atrium, calculator, atrium_client);
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
            if let Some(admitted) = deferred_terminal_admit.take() {
                pending_client_response = Some(admitted);
            } else if let Some(surface) = deferred_terminal_revoke
                .iter_mut()
                .find(|surface| surface.is_valid())
                .map(|surface| core::mem::replace(surface, SurfaceHandle::EMPTY))
            {
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
            for slot in 0..pending_terminal_update.len() {
                let Some(update) = pending_terminal_update[slot] else { continue };
                match common::ipc_send_handle(terminal_surface_response, &update) {
                    IpcStatus::Ok => pending_terminal_update[slot] = None,
                    IpcStatus::Full => break,
                    IpcStatus::Stale
                    | IpcStatus::Disconnected
                    | IpcStatus::Unauthorized
                    | IpcStatus::Malformed
                    | IpcStatus::Empty => {
                        pending_terminal_update[slot] = None;
                        last_terminal_bounds[slot] = GuiRect::EMPTY;
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
            });
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
                // Terminal re-sends this request whenever a pane's tab bar
                // changes (#76), naming that pane's surface (T3b, #97). A
                // request that names none (its first) means the client's
                // first surface, if it has one.
                let named = if terminal_request.surface.is_valid() {
                    atrium.surface_by_reference(terminal_request.surface).filter(|surface| {
                        surface.app == logos_atrium::AppId::Terminal
                            && surface.client == terminal_request.client()
                    })
                } else {
                    atrium.surface_for_client(
                        terminal_request.client(),
                        logos_atrium::AppId::Terminal,
                    )
                };
                if let Some(surface) = named {
                    // A same-surface repeat just reconfirms; capture the
                    // latest tab state for that surface unconditionally.
                    set_terminal_tab_state(surface.reference, terminal_request.tab_state);
                    queue_terminal_response(
                        &mut pending_client_response,
                        terminal_request,
                        logos_abi::GuiStatus::Ok,
                        surface.reference,
                    );
                    if let Some(response) = pending_client_response.as_mut() {
                        response.bounds = surface.bounds;
                    }
                    if let Some(slot) = app_scene_slot(surface.reference) {
                        last_terminal_bounds[slot] = surface.bounds;
                    }
                    pending_app_render = true;
                } else if terminal_request.surface.is_valid() {
                    // A pane closed while its report was in flight.
                    queue_terminal_response(
                        &mut pending_client_response,
                        terminal_request,
                        logos_abi::GuiStatus::NotFound,
                        SurfaceHandle::EMPTY,
                    );
                } else if let Some(surface) = atrium.surface_for_app(logos_atrium::AppId::Terminal)
                {
                    if atrium.close_reference(surface.reference).is_ok() {
                        unbind_app_scene_publisher(surface.reference);
                    }
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
                let terminal_surface_list = terminal_surfaces(atrium);
                let system_surface =
                    atrium.surface_for_app(logos_atrium::AppId::System).map(|s| s.reference);
                hide_surfaces(display_control, &mut surface_commands, atrium, &mut next_request);
                for surface in terminal_surface_list {
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
                unbind_app_scene_publisher(closed.reference);
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
                        bind_app_scene_publisher(surface);
                        if let Some(client_request) = pending_client_request.take() {
                            if client_request.app() == Some(AtriumApp::Terminal) {
                                // Never dropped for a busy response slot: a
                                // lost admission would leave the new pane
                                // blank, since Terminal learns of each pane
                                // only from this response (T3b, #97).
                                let mut admitted = AtriumSurfaceResponse::new(
                                    client_request,
                                    logos_abi::GuiStatus::Ok,
                                );
                                admitted.surface = surface.reference;
                                admitted.bounds = surface.bounds;
                                if pending_client_response.is_none() {
                                    pending_client_response = Some(admitted);
                                } else {
                                    deferred_terminal_admit = Some(admitted);
                                }
                                if let Some(slot) = app_scene_slot(surface.reference) {
                                    last_terminal_bounds[slot] = surface.bounds;
                                }
                            } else {
                                queue_terminal_response(
                                    &mut pending_client_response,
                                    client_request,
                                    logos_abi::GuiStatus::Ok,
                                    surface.reference,
                                );
                            }
                            if client_request.app() == Some(AtriumApp::System) {
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
                    Ok(surface) => {
                        bind_app_scene_publisher(surface);
                        true
                    }
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
                reset_home_scene_publisher();
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
                pending_app_render = render(display, atrium, calculator, atrium_client);
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
            let mut row = GuiTextGridRow::EMPTY;
            while common::ipc_receive_handle(terminal_render, &mut row) == IpcStatus::Ok {
                // Each pane's surface has its own text-grid node (T3b, #97).
                let grid_node_id = terminal_grid_node_id(row.surface);
                let terminal_surface_is_live = row.surface.is_valid()
                    && grid_node_id != 0
                    && atrium.surface_by_reference(row.surface).is_some_and(|surface| {
                        surface.app == logos_atrium::AppId::Terminal
                            && atrium.owns_surface(row.surface, terminal_client)
                    });
                if terminal_surface_is_live {
                    // Terminal only knows its own surface, not the node id
                    // Atrium assigned this surface's TextGrid content node
                    // in `build_app_scene_tree`; fill that in here (#74).
                    row.node_id = grid_node_id;
                    pending_render = Some(row);
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
        // Note: `caps.render` (the generic per-program `RenderMessage` path)
        // is not drained here. No program ever calls `send_render` — every
        // current app (Calculator/Files/Settings) draws through its scene
        // (`caps.draw`) — and once Terminal's own dedicated render channel
        // is retyped to `GuiTextGridRow` (#74), that generic path can no
        // longer share `pending_render`'s type.

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
                atrium.toggle_fps_overlay();
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
            let previous_grid_focus = atrium.home_grid_focus();
            let grid_selected = atrium.home_grid_showing()
                && event
                    .pointer_event()
                    .and_then(|pointer| {
                        (pointer.state == PointerState::Down && pointer.buttons & 1 != 0)
                            .then(|| {
                                atrium.home_grid_item_at(i32::from(pointer.x), i32::from(pointer.y))
                            })
                            .flatten()
                    })
                    .is_some();
            let grid_hover_changed = atrium.home_grid_showing()
                && event.pointer_event().is_some_and(|pointer| {
                    pointer.state == PointerState::Move
                        && atrium
                            .home_grid_item_at(i32::from(pointer.x), i32::from(pointer.y))
                            .is_some()
                        && previous_grid_focus != atrium.home_grid_focus()
                });
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
            if menu_selected || grid_selected {
                event = InputMessage::key(KeyCode::ENTER, KeyState::Pressed, 0);
            } else if atrium.command_menu_open() && event.pointer_event().is_some() {
                if command_menu_hover_changed {
                    pending_app_render = render_home_surface(display, atrium);
                }
                continue;
            } else if atrium.home_grid_showing() && event.pointer_event().is_some() {
                if grid_hover_changed {
                    pending_app_render = render_home_surface(display, atrium);
                }
                continue;
            } else if settings_menu_pointer {
                if settings_menu_changed {
                    pending_app_render = render_home_surface(display, atrium);
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
                pending_app_render = render(display, atrium, calculator, atrium_client);
                continue;
            } else if event.pointer_event().is_none() {
                if atrium
                    .focused_surface()
                    .is_some_and(|surface| surface.app == logos_atrium::AppId::Settings)
                    && atrium.settings_input(&event)
                {
                    pending_app_render = render(display, atrium, calculator, atrium_client);
                    continue;
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
                    let close_bounds = logos_atrium::surface_close_bounds(surface.bounds);
                    let close_clicked = pointer.state == PointerState::Down
                        && close_bounds.contains(local_x, local_y);
                    let local = InputMessage::pointer_wheel(
                        local_x.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16,
                        local_y.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16,
                        pointer.buttons,
                        pointer.state,
                        pointer.wheel,
                    )
                    .unwrap_or(event);
                    if close_clicked {
                        event = InputMessage::key(KeyCode::ESCAPE, KeyState::Pressed, 0);
                    } else {
                        let routed = AtriumSurfaceInput::new(surface.reference, local);
                        if surface.app == logos_atrium::AppId::Settings {
                            let fps_overlay = atrium.fps_overlay();
                            if atrium.settings_input(&local) {
                                if atrium.fps_overlay() != fps_overlay {
                                    queue_fps_toggle(&mut surface_commands, &mut next_request);
                                    surface_commands.flush(display_control);
                                }
                                let settings = atrium.input_settings();
                                if pending_input_settings != Some(settings) {
                                    pending_input_settings = Some(settings);
                                }
                                // S4 (#81, ADR-0091): persist only on an
                                // actual change to the record, not every
                                // frame the Settings surface handles input.
                                // Also wins over a same-boot Load response
                                // that hasn't landed yet (`should_apply_
                                // loaded_settings`): the user's live choice
                                // is authoritative from here on.
                                let encoded = atrium.settings_record().encode();
                                if encoded != last_saved_settings {
                                    last_saved_settings = encoded;
                                    pending_settings_save = Some(encoded);
                                    settings_changed_by_user = true;
                                }
                                pending_app_render =
                                    render(display, atrium, calculator, atrium_client);
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
                                pending_app_render =
                                    render(display, atrium, calculator, atrium_client);
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
                    // Launching Terminal into a split pane that is waiting
                    // for its app attaches a new Terminal surface (T3b,
                    // #97) while a session is free; a plain launch keeps
                    // refocusing the existing one.
                    let new_terminal_pane = app == logos_atrium::AppId::Terminal
                        && atrium.has_empty_leaf()
                        && (terminal_open_session_bitmap(atrium).count_ones() as usize)
                            < logos_abi::TERMINAL_MAX_TABS;
                    let existing = atrium.surface_for_app(app).filter(|_| !new_terminal_pane);
                    if let Some(surface) = existing {
                        if atrium.focus(surface.id).is_ok() {
                            send_surface_command(
                                display_control,
                                &mut surface_commands,
                                GuiSurfaceOperation::Focus,
                                surface.reference,
                                GuiRect::EMPTY,
                                &mut next_request,
                            );
                            pending_app_render = render(display, atrium, calculator, atrium_client);
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
                    let terminal_surface_list = terminal_surfaces(atrium);
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
                    for surface in terminal_surface_list {
                        queue_terminal_revoke(
                            &mut pending_client_response,
                            &mut deferred_terminal_revoke,
                            &mut next_request,
                            surface,
                        );
                    }
                    pending_terminal_update = [None; logos_atrium::MAX_ATRIUM_SURFACES];
                    last_terminal_bounds = [GuiRect::EMPTY; logos_atrium::MAX_ATRIUM_SURFACES];
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
                    pending_app_render = render(display, atrium, calculator, atrium_client);
                }
                logos_atrium::AtriumAction::OpenCommandMenu
                | logos_atrium::AtriumAction::CloseCommandMenu
                | logos_atrium::AtriumAction::CloseSettingsMenu => {
                    let _ = atrium.apply_action(action);
                    pending_app_render = render(display, atrium, calculator, atrium_client);
                }
                logos_atrium::AtriumAction::Shutdown => {
                    let _ = common::power(logos_abi::POWER_SHUTDOWN);
                    pending_app_render = render(display, atrium, calculator, atrium_client);
                }
                logos_atrium::AtriumAction::Restart => {
                    let _ = common::power(logos_abi::POWER_REBOOT);
                    pending_app_render = render(display, atrium, calculator, atrium_client);
                }
                logos_atrium::AtriumAction::CloseFocused => {
                    let old = atrium.focused_surface();
                    if atrium.apply_action(action).is_ok() {
                        if let Some(surface) = old {
                            unbind_app_scene_publisher(surface.reference);
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
                        pending_app_render = render(display, atrium, calculator, atrium_client);
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
                        pending_app_render = render(display, atrium, calculator, atrium_client);
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
                        pending_app_render = render(display, atrium, calculator, atrium_client);
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
                        pending_app_render = render(display, atrium, calculator, atrium_client);
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
        // ADR-0089: deliver appearance changes to LockScreen and to the
        // current Terminal and System surfaces (including a newly opened
        // one). S5 (#82) adds the light-theme flag and System as a third
        // receiver, over these same channels.
        let flags = atrium.appearance_flags();
        if lockscreen_appearance != flags
            && common::ipc_send_handle(
                lockscreen_control,
                &GuiHook::appearance(next_request_id(&mut next_request), flags),
            ) == IpcStatus::Ok
        {
            lockscreen_appearance = flags;
        }
        if let Some(surface) = atrium.surface_for_app(logos_atrium::AppId::Terminal) {
            let target = (surface.reference, flags);
            if terminal_appearance != Some(target)
                && common::ipc_send_handle(
                    terminal,
                    &AtriumSurfaceInput::new(surface.reference, InputMessage::appearance(flags)),
                ) == IpcStatus::Ok
            {
                terminal_appearance = Some(target);
            }
        }
        if let Some(surface) = atrium.surface_for_app(logos_atrium::AppId::System) {
            let target = (surface.reference, flags);
            if system_appearance != Some(target)
                && common::ipc_send_handle(
                    system_surface_input,
                    &AtriumSurfaceInput::new(surface.reference, InputMessage::appearance(flags)),
                ) == IpcStatus::Ok
            {
                system_appearance = Some(target);
            }
        }
        if home_appearance != flags && atrium.home_surface().is_valid() {
            home_appearance = flags;
            pending_app_render = render_home_surface(display, atrium) || pending_app_render;
        }
        let now_ticks = common::current_ticks();
        let menu_motion_active = unsafe {
            (&*core::ptr::addr_of!(COMMAND_MENU_TREE)).next_deadline(now_ticks).is_some()
        };
        if menu_motion_active {
            pending_app_render = render(display, atrium, calculator, atrium_client);
        }
        if unsafe { (*core::ptr::addr_of!(HOME_SCENE_PUBLISHER)).is_pending() } {
            pending_app_render = render_home_surface(display, atrium);
        }
        let home_surface = atrium.home_surface();
        if home_surface.is_valid() {
            let wall = common::wall_time();
            let minute = (wall.hour, wall.minute);
            let surface_changed =
                unsafe { *core::ptr::addr_of!(HOME_CLOCK_SURFACE) } != home_surface;
            if surface_changed {
                unsafe {
                    *core::ptr::addr_of_mut!(HOME_CLOCK_SURFACE) = home_surface;
                    *core::ptr::addr_of_mut!(HOME_CLOCK_LAST) = Some(minute);
                }
            } else if unsafe { *core::ptr::addr_of!(HOME_CLOCK_LAST) } != Some(minute) {
                unsafe { *core::ptr::addr_of_mut!(HOME_CLOCK_LAST) = Some(minute) };
                pending_app_render = render_home_surface(display, atrium) || pending_app_render;
            }
        } else {
            unsafe { *core::ptr::addr_of_mut!(HOME_CLOCK_SURFACE) = SurfaceHandle::EMPTY };
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
            user_settings_receive,
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

#[cfg(test)]
mod terminal_scene_tests {
    use super::*;

    #[derive(Default)]
    struct CollectingSceneSink {
        operations: std::vec::Vec<GuiSceneOp>,
    }

    impl logos_ui_graphics::UiSceneSink for CollectingSceneSink {
        fn send(&mut self, operation: &GuiSceneOp) -> IpcStatus {
            self.operations.push(*operation);
            IpcStatus::Ok
        }
    }
    /// Two tiled Terminal panes (a vertical split) owned by one client. The
    /// first (untiled) surface starts with `DESKTOP_SURFACE_BOUNDS`, the
    /// worst-case, largest content region; after the split each is half of
    /// that (T3b, #97).
    fn terminal_panes() -> (logos_atrium::Atrium, [logos_atrium::Surface; 2]) {
        let mut atrium = logos_atrium::Atrium::new();
        atrium.authenticate();
        let client = logos_abi::ServiceHandle::new(1, 1).unwrap();
        let request = atrium.request_surface(logos_atrium::AppId::Terminal, client).unwrap();
        let first = atrium.spawn_surface(request, SurfaceHandle::new(0, 1, 7).unwrap()).unwrap();
        atrium
            .apply_action(logos_atrium::AtriumAction::Split(logos_atrium::SplitDirection::Vertical))
            .unwrap();
        assert!(atrium.has_empty_leaf());
        let request = atrium.request_surface(logos_atrium::AppId::Terminal, client).unwrap();
        let second = atrium.spawn_surface(request, SurfaceHandle::new(1, 1, 7).unwrap()).unwrap();
        assert!(!atrium.has_empty_leaf());
        let first = atrium.surface_by_reference(first.reference).unwrap();
        (atrium, [first, second])
    }

    /// SCENE-BUDGET (#69/#74/#76/#97): the Terminal scene carries its chrome
    /// (title bar, close control), the tab strip and one `TextGrid` content
    /// node. With two panes sharing the four-session cap, the worst cases
    /// are one pane holding every session (all `TERMINAL_MAX_TABS` chips,
    /// close controls and the add button) and a 3 + 1 split; each pane's
    /// scene must fit `MAX_GUI_NODES`/`MAX_UI_SCENE_OPS` and publish within
    /// `MAX_UI_SCENE_PUBLISHER_BYTES`, matching the existing Settings/Home
    /// scene budget tests. Everything runs in one test (rather than several
    /// `#[test]`s) because `APP_SCENE_TREE` and the tab-state/grid-node
    /// tables are process-wide `static mut`s that `cargo test`'s default
    /// parallel threads would otherwise race.
    #[test]
    fn two_terminal_panes_with_worst_case_tabs_fit_the_scene_budget() {
        let (atrium, panes) = terminal_panes();
        let calculator = logos_atrium::Calculator::new();
        assert_ne!(panes[0].reference, panes[1].reference);
        assert!(panes[0].bounds.width < logos_atrium::DESKTOP_SURFACE_BOUNDS.width);

        let configs = [
            [0u16, 0],
            [logos_abi::pack_terminal_tab_state(0b1111, 3), 0],
            [
                logos_abi::pack_terminal_tab_state(0b0111, 2),
                logos_abi::pack_terminal_tab_state(0b1000, 3),
            ],
        ];
        for config in configs {
            for (surface, state) in panes.iter().zip(config) {
                set_terminal_tab_state(surface.reference, state);
            }
            // Never more sessions open than the shared cap allows.
            assert!(
                (terminal_open_session_bitmap(&atrium).count_ones() as usize)
                    <= logos_abi::TERMINAL_MAX_TABS
            );
            for surface in panes {
                assert!(build_app_scene_tree(surface, &calculator));

                let tree = unsafe { &mut *core::ptr::addr_of_mut!(APP_SCENE_TREE) };
                assert!(tree.tree().len() <= logos_ui::MAX_UI_NODES);
                assert!(tree.tree().len() <= logos_abi::MAX_GUI_NODES);

                let mut publisher = logos_ui_graphics::UiScenePublisher::new();
                let mut sink = CollectingSceneSink::default();
                let (status, sent) = publisher
                    .publish(
                        surface.reference,
                        1,
                        tree,
                        logos_atrium::Atrium::new().app_theme(),
                        None,
                        &mut sink,
                    )
                    .unwrap();
                assert_eq!(status, IpcStatus::Ok);
                assert!(sent <= logos_ui_graphics::MAX_UI_SCENE_OPS);
                assert!(
                    core::mem::size_of::<logos_ui_graphics::UiScenePublisher>()
                        <= logos_ui_graphics::MAX_UI_SCENE_PUBLISHER_BYTES
                );

                let node_id = terminal_grid_node_id(surface.reference);
                assert_ne!(node_id, 0);
                assert!(sink.operations.iter().any(|operation| operation.node_id == node_id
                    && operation.command.kind == logos_abi::GuiDrawKind::TextGrid));
            }
        }

        // Tab state is per surface: pane one's chips never leak into pane two.
        set_terminal_tab_state(panes[0].reference, logos_abi::pack_terminal_tab_state(0b0011, 0));
        set_terminal_tab_state(panes[1].reference, logos_abi::pack_terminal_tab_state(0b0100, 2));
        assert_eq!(
            logos_abi::terminal_tab_open_bitmap(terminal_tab_state(panes[0].reference)),
            0b0011
        );
        assert_eq!(
            logos_abi::terminal_tab_open_bitmap(terminal_tab_state(panes[1].reference)),
            0b0100
        );
        assert_eq!(terminal_open_session_bitmap(&atrium), 0b0111);

        for surface in panes {
            unbind_app_scene_publisher(surface.reference);
            set_terminal_tab_state(surface.reference, 0);
        }
    }
}
