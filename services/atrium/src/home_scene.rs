use logos_abi::{GuiRect, InterStyle, WallTime};
use logos_ui::{UiBlueprint, UiComponentTree, UiIcon, UiNodeKind, UiStyle, UiStyleList, UiText};

#[cfg(test)]
use crate::AtriumAction;
use crate::{
    AppId, Atrium, COMMAND_MENU_ITEMS, COMMAND_MENU_LABELS, HOME_GRID_APPS, HOME_GRID_LABELS,
    HOME_HEADER_BOUNDS, home_grid_item_bounds, home_grid_label_bounds,
};

fn home_grid_icon(app: AppId) -> UiIcon {
    match app {
        AppId::Calculator => UiIcon::Calculator,
        AppId::Files => UiIcon::Folder,
        AppId::Terminal => UiIcon::Terminal,
        AppId::System => UiIcon::Monitor,
        AppId::Settings => UiIcon::Settings,
    }
}

/// Formats `hour:minute` as fixed ASCII digits with no allocation.
fn format_clock(wall: WallTime) -> [u8; 5] {
    [
        b'0' + (wall.hour / 10) % 10,
        b'0' + wall.hour % 10,
        b':',
        b'0' + (wall.minute / 10) % 10,
        b'0' + wall.minute % 10,
    ]
}

/// Updates the retained home-scene tree from the current Atrium state.
pub fn build_home_scene(
    tree: &mut UiComponentTree,
    atrium: &Atrium,
    now: u64,
    wall_time: WallTime,
) -> bool {
    let mut blueprint = UiBlueprint::new();
    let root = blueprint.push_root(UiNodeKind::Root, 1).ok();
    let Some(root) = root else { return false };
    let mut root_styles = UiStyleList::EMPTY;
    if !root_styles.push(UiStyle::Transparent) || blueprint.set_styles(root, root_styles).is_err() {
        return false;
    }
    let sidebar = blueprint.push_child(UiNodeKind::Panel, root, 2).ok();
    let Some(sidebar) = sidebar else { return false };
    let account = blueprint.push_child(UiNodeKind::Avatar, sidebar, 3).ok();
    let settings = blueprint.push_child(UiNodeKind::Button, sidebar, 4).ok();
    let Some(account) = account else { return false };
    let Some(settings) = settings else { return false };
    if blueprint.set_icon(settings, UiIcon::Settings).is_err() {
        return false;
    }
    let mut settings_styles = UiStyleList::EMPTY;
    if !settings_styles.push(UiStyle::BackgroundAccent)
        || !settings_styles.push(UiStyle::RoundedFull)
        || blueprint.set_styles(settings, settings_styles).is_err()
    {
        return false;
    }
    let panel = blueprint.push_child(UiNodeKind::Panel, root, 2).ok();
    let Some(panel) = panel else { return false };
    let title = blueprint.push_child(UiNodeKind::Label, panel, 3).ok();
    // ponytail: the display retains 24 nodes per surface; a query label keeps the shell scene
    // within that bound. Atrium still owns query editing and launcher focus.
    let input = blueprint.push_child(UiNodeKind::Label, panel, 4).ok();
    let Some(title) = title else { return false };
    let Some(input) = input else { return false };
    let mut buttons = [0u16; 4];
    for (index, button_slot) in buttons.iter_mut().enumerate() {
        let Some(button) = blueprint.push_child(UiNodeKind::Button, panel, 10 + index as u16).ok()
        else {
            return false;
        };
        *button_slot = button;
    }
    // H1: the idle Home header (greeting + clock) and icon tile grid, shown
    // in place of the search overlay above whenever it is closed.
    let greeting = blueprint.push_child(UiNodeKind::Label, root, 40).ok();
    let clock = blueprint.push_child(UiNodeKind::Label, root, 41).ok();
    let Some(greeting) = greeting else { return false };
    let Some(clock) = clock else { return false };
    let mut header_styles = UiStyleList::EMPTY;
    if !header_styles.push(UiStyle::Text4xl)
        || blueprint.set_styles(greeting, header_styles).is_err()
    {
        return false;
    }
    if blueprint.set_styles(clock, header_styles).is_err() {
        return false;
    }
    // Painted before the tiles so their damage rects merge into this one
    // (see HOME_GRID_CONTAINER_BOUNDS) instead of each counting separately
    // against Display's MAX_GUI_DAMAGE_RECTS budget.
    let grid_container = blueprint.push_child(UiNodeKind::Panel, root, 45).ok();
    let Some(grid_container) = grid_container else { return false };
    let mut container_styles = UiStyleList::EMPTY;
    if !container_styles.push(UiStyle::RoundedLarge)
        || blueprint.set_styles(grid_container, container_styles).is_err()
    {
        return false;
    }
    let mut tile_buttons = [0u16; HOME_GRID_APPS.len()];
    let mut tile_labels = [0u16; HOME_GRID_APPS.len()];
    for (index, app) in HOME_GRID_APPS.into_iter().enumerate() {
        let Some(button) = blueprint.push_child(UiNodeKind::Button, root, 50 + index as u16).ok()
        else {
            return false;
        };
        if blueprint.set_icon(button, home_grid_icon(app)).is_err() {
            return false;
        }
        let mut tile_styles = UiStyleList::EMPTY;
        if !tile_styles.push(UiStyle::RoundedLarge)
            || !tile_styles.push(UiStyle::IconLarge)
            || blueprint.set_styles(button, tile_styles).is_err()
        {
            return false;
        }
        tile_buttons[index] = button;
        let Some(label) = blueprint.push_child(UiNodeKind::Label, root, 60 + index as u16).ok()
        else {
            return false;
        };
        tile_labels[index] = label;
    }
    let settings_menu = blueprint.push_child(UiNodeKind::Panel, root, 20).ok();
    let settings_highlight = blueprint.push_child(UiNodeKind::Panel, root, 21).ok();
    let Some(settings_menu) = settings_menu else { return false };
    let Some(settings_highlight) = settings_highlight else { return false };
    let mut settings_labels = [0u16; 3];
    for (index, label_slot) in settings_labels.iter_mut().enumerate() {
        let Some(label) = blueprint.push_child(UiNodeKind::Label, root, 22 + index as u16).ok()
        else {
            return false;
        };
        *label_slot = label;
    }
    let account_menu = blueprint.push_child(UiNodeKind::Panel, root, 30).ok();
    let account_highlight = blueprint.push_child(UiNodeKind::Panel, root, 31).ok();
    let Some(account_menu) = account_menu else { return false };
    let Some(account_highlight) = account_highlight else { return false };
    let account_label = blueprint.push_child(UiNodeKind::Label, root, 32).ok();
    let Some(account_label) = account_label else { return false };
    let mut menu_styles = UiStyleList::EMPTY;
    if !menu_styles.push(UiStyle::RoundedLarge)
        || blueprint.set_styles(settings_menu, menu_styles).is_err()
        || blueprint.set_styles(account_menu, menu_styles).is_err()
    {
        return false;
    }
    let Some(title_text) = UiText::from_bytes(b"What do you want to open?") else {
        return false;
    };
    let Some(settings_text) = UiText::from_bytes(b"Settings") else { return false };
    if blueprint.set_avatar(account, logos_ui::UiAvatarContent::text(b"A").unwrap()).is_err()
        || blueprint.set_text(settings, settings_text).is_err()
        || blueprint.set_text(title, title_text).is_err()
    {
        return false;
    }
    let mut title_styles = UiStyleList::EMPTY;
    if !title_styles.push(UiStyle::Text4xl) || !title_styles.push(UiStyle::FontLight) {
        return false;
    }
    if blueprint.set_styles(title, title_styles).is_err() {
        return false;
    }
    let Some(disconnect_text) = UiText::from_bytes(b"Disconnect") else { return false };
    if blueprint.set_text(account_label, disconnect_text).is_err() {
        return false;
    }
    // No session user name is available to Atrium without a new ABI, so the
    // greeting is always "Welcome" (H1 boundary).
    let Some(greeting_text) = UiText::from_bytes(b"Welcome") else { return false };
    if blueprint.set_text(greeting, greeting_text).is_err() {
        return false;
    }
    for (index, label) in HOME_GRID_LABELS.into_iter().enumerate() {
        let Some(text) = UiText::from_bytes(label) else { return false };
        if blueprint.set_text(tile_labels[index], text).is_err() {
            return false;
        }
    }
    let mount = tree.tree().is_empty();
    if mount {
        let Ok(new_tree) = UiComponentTree::from_blueprint(&blueprint) else {
            return false;
        };
        *tree = new_tree;
    }
    tree.set_reduced_motion(atrium.reduced_motion());
    let set_bounds = |tree: &mut UiComponentTree, index: u16, bounds: GuiRect| {
        let Some(handle) = tree.tree().handle_at(usize::from(index)).ok() else { return false };
        tree.tree_mut()
            .set_bounds(
                handle,
                logos_ui::UiRect::new(bounds.x, bounds.y, bounds.width, bounds.height),
            )
            .is_ok()
    };
    let gui_rect =
        |bounds: logos_ui::UiRect| GuiRect::new(bounds.x, bounds.y, bounds.width, bounds.height);
    let menu_visible = atrium.command_menu_open();
    let menu_bounds = if menu_visible { crate::COMMAND_MENU_BOUNDS } else { GuiRect::EMPTY };
    let title_bounds = if menu_visible { GuiRect::new(384, 160, 512, 40) } else { GuiRect::EMPTY };
    let input_bounds = if menu_visible { GuiRect::new(384, 216, 512, 56) } else { GuiRect::EMPTY };
    if !set_bounds(tree, root, crate::FULLSCREEN_SURFACE_BOUNDS)
        || !set_bounds(tree, sidebar, crate::SIDEBAR_BOUNDS)
        || !set_bounds(tree, account, crate::SIDEBAR_ACCOUNT_BOUNDS)
        || !set_bounds(tree, settings, crate::SIDEBAR_SETTINGS_BOUNDS)
        || !set_bounds(tree, panel, menu_bounds)
        || !set_bounds(tree, title, title_bounds)
        || !set_bounds(tree, input, input_bounds)
    {
        return false;
    }
    let query = atrium.launcher_query();
    let Some(input_handle) = tree.tree().handle_at(usize::from(input)).ok() else { return false };
    let _ = tree.set_text(input_handle, query);
    let mut input_styles = UiStyleList::EMPTY;
    let _ = input_styles.push(UiStyle::TextMuted);
    let _ = tree.set_styles(input_handle, input_styles);
    if let Ok(handle) = tree.tree().handle_at(usize::from(account)) {
        let _ = tree.tree_mut().set_hovered(handle, atrium.sidebar_hover() == 1);
    }
    if let Ok(handle) = tree.tree().handle_at(usize::from(settings)) {
        let _ = tree.tree_mut().set_hovered(handle, atrium.sidebar_hover() == 2);
    }
    let grid_showing = atrium.home_grid_showing();
    let header_bounds = if grid_showing { HOME_HEADER_BOUNDS } else { GuiRect::EMPTY };
    let greeting_bounds = if grid_showing {
        GuiRect::new(
            header_bounds.x,
            header_bounds.y,
            header_bounds.width / 2,
            header_bounds.height,
        )
    } else {
        GuiRect::EMPTY
    };
    let clock_text = format_clock(wall_time);
    let clock_bounds = if grid_showing {
        let measured = logos_abi::inter_text_width(InterStyle::Title, &clock_text);
        let right_edge = header_bounds.x.saturating_add(header_bounds.width as i32);
        GuiRect::new(
            right_edge.saturating_sub(measured as i32).saturating_sub(12),
            header_bounds.y,
            measured.saturating_add(12),
            header_bounds.height,
        )
    } else {
        GuiRect::EMPTY
    };
    if !set_bounds(tree, greeting, greeting_bounds) || !set_bounds(tree, clock, clock_bounds) {
        return false;
    }
    if let Some(text) = UiText::from_bytes(&clock_text) {
        if let Ok(handle) = tree.tree().handle_at(usize::from(clock)) {
            let _ = tree.set_text(handle, text);
        }
    }
    let container_bounds =
        if grid_showing { crate::HOME_GRID_CONTAINER_BOUNDS } else { GuiRect::EMPTY };
    if !set_bounds(tree, grid_container, container_bounds) {
        return false;
    }
    for (index, label) in HOME_GRID_LABELS.into_iter().enumerate() {
        let icon_bounds = if grid_showing { home_grid_item_bounds(index) } else { GuiRect::EMPTY };
        if !set_bounds(tree, tile_buttons[index], icon_bounds) {
            return false;
        }
        if let Ok(handle) = tree.tree().handle_at(usize::from(tile_buttons[index])) {
            let focused = grid_showing && usize::from(atrium.home_grid_focus()) == index;
            let _ = tree.tree_mut().set_focused(handle, focused);
            let _ = tree.tree_mut().set_hovered(handle, focused);
        }
        let name_bounds = if grid_showing {
            let column = home_grid_label_bounds(index);
            let measured = logos_abi::inter_text_width(InterStyle::Body, label);
            let inset = (column.width as i32).saturating_sub(measured as i32).saturating_div(2);
            GuiRect::new(
                column.x.saturating_add(inset).saturating_sub(12).max(column.x),
                column.y,
                measured.saturating_add(24).min(column.width),
                column.height,
            )
        } else {
            GuiRect::EMPTY
        };
        if !set_bounds(tree, tile_labels[index], name_bounds) {
            return false;
        }
    }
    for (index, button) in buttons.into_iter().enumerate() {
        let visible = menu_visible && index < atrium.launcher_result_count();
        let bounds = if visible { crate::command_menu_item_bounds(index) } else { GuiRect::EMPTY };
        if !set_bounds(tree, button, bounds) {
            return false;
        }
        let Some(handle) = tree.tree().handle_at(usize::from(button)).ok() else { return false };
        let label = atrium
            .launcher_result_app(index)
            .and_then(|app| COMMAND_MENU_ITEMS.iter().position(|candidate| *candidate == app))
            .and_then(|app_index| COMMAND_MENU_LABELS.get(app_index).copied())
            .unwrap_or(&[]);
        let Some(text) = UiText::from_bytes(label) else { return false };
        if tree.set_text(handle, text).is_err() {
            return false;
        }
        let mut styles = UiStyleList::EMPTY;
        if visible
            && atrium.launcher_result_app(index) == Some(atrium.launcher_app())
            && !styles.push(UiStyle::BackgroundAccent)
        {
            return false;
        }
        if tree.set_styles(handle, styles).is_err() {
            return false;
        }
    }
    let settings_open = atrium.settings_menu_open();
    let account_open = atrium.account_menu_open();
    let settings_layout = atrium.settings_menu_popover(crate::FULLSCREEN_SURFACE_BOUNDS);
    let account_layout = atrium.account_menu_popover(crate::FULLSCREEN_SURFACE_BOUNDS);
    let settings_bounds =
        if settings_open { gui_rect(settings_layout.bounds) } else { GuiRect::EMPTY };
    let account_bounds =
        if account_open { gui_rect(account_layout.bounds) } else { GuiRect::EMPTY };
    if !set_bounds(tree, settings_menu, settings_bounds)
        || !set_bounds(tree, account_menu, account_bounds)
    {
        return false;
    }
    let settings_hover = atrium.settings_menu_hovered_option();
    let account_hover = atrium.account_menu_hovered_option();
    let settings_highlight_bounds = if settings_open {
        settings_hover
            .map(|index| settings_layout.option_bounds(index))
            .filter(|bounds| !bounds.is_empty())
            .map(gui_rect)
            .unwrap_or_else(|| gui_rect(settings_layout.bounds))
    } else {
        GuiRect::EMPTY
    };
    let account_highlight_bounds = if account_open {
        account_hover
            .map(|index| account_layout.option_bounds(index))
            .filter(|bounds| !bounds.is_empty())
            .map(gui_rect)
            .unwrap_or_else(|| gui_rect(account_layout.bounds))
    } else {
        GuiRect::EMPTY
    };
    if !set_bounds(tree, settings_highlight, settings_highlight_bounds)
        || !set_bounds(tree, account_highlight, account_highlight_bounds)
    {
        return false;
    }
    for (handle, hovered) in [
        (settings_highlight, settings_hover.is_some()),
        (account_highlight, account_hover.is_some()),
    ] {
        let Ok(handle) = tree.tree().handle_at(usize::from(handle)) else { return false };
        let mut styles = UiStyleList::EMPTY;
        if !styles.push(UiStyle::RoundedLarge)
            || (hovered && !styles.push(UiStyle::BackgroundAccent))
            || tree.set_styles(handle, styles).is_err()
        {
            return false;
        }
    }
    for (index, label) in settings_labels.into_iter().enumerate() {
        let visible = settings_open && index < usize::from(settings_layout.visible_options);
        let option = usize::from(settings_layout.first_option).saturating_add(index);
        let bounds = if visible {
            gui_rect(settings_layout.option_bounds(option as u8))
        } else {
            GuiRect::EMPTY
        };
        if !set_bounds(tree, label, bounds) {
            return false;
        }
        let Some(handle) = tree.tree().handle_at(usize::from(label)).ok() else { return false };
        let text = crate::SIDEBAR_MENU_LABELS.get(option).copied().unwrap_or(&[]);
        let Some(text) = UiText::from_bytes(text) else { return false };
        if tree.set_text(handle, text).is_err() {
            return false;
        }
    }
    if !set_bounds(
        tree,
        account_label,
        if account_open { gui_rect(account_layout.option_bounds(0)) } else { GuiRect::EMPTY },
    ) {
        return false;
    }
    let _ = tree.advance(now);
    true
}

#[cfg(test)]
mod tests {
    use std::format;

    use logos_abi::{GuiNodeOperation, InputMessage, KeyState, PointerState, SurfaceHandle};
    use logos_ui_graphics::{UiSceneTheme, emit};

    use super::*;

    #[test]
    fn home_scene_states_stay_within_the_display_node_budget() {
        let surface = SurfaceHandle::new(1, 1, 13).unwrap();
        let mut tree = UiComponentTree::new();
        let wall_time = WallTime { year: 2026, month: 9, day: 26, hour: 23, minute: 59, second: 5 };
        let mut check = |atrium: &Atrium, state: &str| {
            assert!(build_home_scene(&mut tree, atrium, 0, wall_time));
            let scene = emit(surface, 1, &tree, UiSceneTheme::DEFAULT).unwrap();
            let nodes: std::vec::Vec<_> = scene
                .as_slice()
                .iter()
                .filter(|operation| operation.operation == GuiNodeOperation::Upsert)
                .map(|operation| operation.node_id)
                .collect();
            assert!(
                nodes.len() <= logos_abi::MAX_GUI_NODES,
                "{state}: {} Upserts, node IDs={nodes:?}",
                nodes.len()
            );
        };
        for query in [&b""[..], b"Calculator", b"Files", b"Terminal", b"System", b"none"] {
            let mut command = Atrium::new();
            command.authenticate();
            check(&command, &format!("idle grid, query={query:?}"));
            command.open_command_menu();
            if !query.is_empty() {
                command.input(&InputMessage::text(query).unwrap());
            }
            for selected in 0..command.launcher_result_count().max(1) {
                if command.launcher_result_count() > 0 {
                    command.command_menu_item_at(
                        crate::COMMAND_MENU_ITEM_LEFT + 1,
                        crate::COMMAND_MENU_ITEM_TOP
                            + selected as i32
                                * (crate::COMMAND_MENU_ITEM_HEIGHT as i32
                                    + crate::COMMAND_MENU_ITEM_GAP)
                            + 1,
                    );
                }
                check(&command, &format!("command query={query:?}, selected={selected}"));
            }
            command.close_command_menu();
            for (hover, x, y) in [(0, 300, 300), (1, 20, 20), (2, 20, 760)] {
                command.settings_menu_input(
                    &InputMessage::pointer(x, y, 0, PointerState::Move).unwrap(),
                );
                check(&command, &format!("closed query={query:?}, sidebar_hover={hover}"));
            }
        }

        // Grid keyboard navigation: arrows move focus across every tile,
        // Enter launches the focused app, each state stays within budget.
        let mut grid = Atrium::new();
        grid.authenticate();
        check(&grid, "grid focus=0");
        for step in 1..HOME_GRID_APPS.len() {
            assert_eq!(
                grid.input(&InputMessage::key(logos_abi::KeyCode::RIGHT, KeyState::Pressed, 0)),
                AtriumAction::LauncherChanged
            );
            assert_eq!(usize::from(grid.home_grid_focus()), step);
            check(&grid, &format!("grid focus={step}"));
        }
        assert_eq!(
            grid.input(&InputMessage::key(logos_abi::KeyCode::ENTER, KeyState::Pressed, 0)),
            AtriumAction::Launch(*HOME_GRID_APPS.last().unwrap())
        );
        for step in (0..HOME_GRID_APPS.len() - 1).rev() {
            assert_eq!(
                grid.input(&InputMessage::key(logos_abi::KeyCode::LEFT, KeyState::Pressed, 0)),
                AtriumAction::LauncherChanged
            );
            assert_eq!(usize::from(grid.home_grid_focus()), step);
        }

        // Grid hit-testing: a pointer inside each tile resolves to that
        // tile's app and moves focus there.
        let mut pointer = Atrium::new();
        pointer.authenticate();
        for (index, app) in HOME_GRID_APPS.into_iter().enumerate() {
            let bounds = home_grid_item_bounds(index);
            assert_eq!(pointer.home_grid_item_at(bounds.x + 1, bounds.y + 1), Some(app));
            assert_eq!(usize::from(pointer.home_grid_focus()), index);
            check(&pointer, &format!("grid hit-test index={index}"));
        }
        assert_eq!(pointer.home_grid_item_at(-10, -10), None);

        for (account, anchor_x, anchor_y, options) in [
            (false, 20, 760, crate::SIDEBAR_MENU_LABELS.len()),
            (true, 20, 20, crate::SIDEBAR_ACCOUNT_MENU_LABELS.len()),
        ] {
            let mut atrium = Atrium::new();
            atrium.authenticate();
            atrium.close_command_menu();
            atrium.settings_menu_input(
                &InputMessage::pointer(anchor_x, anchor_y, 1, PointerState::Down).unwrap(),
            );
            check(&atrium, if account { "account menu open" } else { "settings menu open" });
            let layout = if account {
                atrium.account_menu_popover(crate::FULLSCREEN_SURFACE_BOUNDS)
            } else {
                atrium.settings_menu_popover(crate::FULLSCREEN_SURFACE_BOUNDS)
            };
            for option in 0..options {
                let bounds = layout.option_bounds(option as u8);
                atrium.settings_menu_input(
                    &InputMessage::pointer(
                        (bounds.x + 1) as i16,
                        (bounds.y + 1) as i16,
                        0,
                        PointerState::Move,
                    )
                    .unwrap(),
                );
                check(&atrium, &format!("account={account}, hovered_option={option}"));
            }
            if account {
                atrium.settings_menu_input(
                    &InputMessage::pointer(300, 300, 1, PointerState::Down).unwrap(),
                );
                check(&atrium, "account menu closes");
            } else {
                atrium.settings_menu_input(
                    &InputMessage::pointer(20, 20, 1, PointerState::Down).unwrap(),
                );
                check(&atrium, "settings menu closes before account menu");
                atrium.settings_menu_input(
                    &InputMessage::pointer(20, 20, 1, PointerState::Down).unwrap(),
                );
                check(&atrium, "account menu opens after settings menu");
            }
        }

        for (account, anchor_x, anchor_y, options) in [
            (false, 20, 760, crate::SIDEBAR_MENU_LABELS.len()),
            (true, 20, 20, crate::SIDEBAR_ACCOUNT_MENU_LABELS.len()),
        ] {
            let mut initial = Atrium::new();
            initial.authenticate();
            for selected in 0..initial.launcher_result_count() {
                let mut atrium = Atrium::new();
                atrium.authenticate();
                atrium.open_command_menu();
                let bounds = crate::command_menu_item_bounds(selected);
                atrium.command_menu_item_at(bounds.x + 1, bounds.y + 1);
                atrium.settings_menu_input(
                    &InputMessage::pointer(anchor_x, anchor_y, 1, PointerState::Down).unwrap(),
                );
                assert!(atrium.command_menu_open());
                assert_eq!(atrium.account_menu_open(), account);
                assert_eq!(atrium.settings_menu_open(), !account);
                check(
                    &atrium,
                    &format!(
                        "combined account={account}, selected={selected}, hovered_option=None"
                    ),
                );
                let layout = if account {
                    atrium.account_menu_popover(crate::FULLSCREEN_SURFACE_BOUNDS)
                } else {
                    atrium.settings_menu_popover(crate::FULLSCREEN_SURFACE_BOUNDS)
                };
                for option in 0..options {
                    let bounds = layout.option_bounds(option as u8);
                    atrium.settings_menu_input(
                        &InputMessage::pointer(
                            (bounds.x + 1) as i16,
                            (bounds.y + 1) as i16,
                            0,
                            PointerState::Move,
                        )
                        .unwrap(),
                    );
                    check(
                        &atrium,
                        &format!(
                            "combined account={account}, selected={selected}, hovered_option={option}"
                        ),
                    );
                }
            }
        }
    }
}
