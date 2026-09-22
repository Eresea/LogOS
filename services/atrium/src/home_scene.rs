use logos_abi::GuiRect;
use logos_ui::{UiBlueprint, UiComponentTree, UiIcon, UiNodeKind, UiStyle, UiStyleList, UiText};

use crate::{Atrium, COMMAND_MENU_ITEMS, COMMAND_MENU_LABELS};

/// Updates the retained home-scene tree from the current Atrium state.
pub fn build_home_scene(tree: &mut UiComponentTree, atrium: &Atrium, now: u64) -> bool {
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
    let mount = tree.tree().is_empty();
    if mount {
        let Ok(new_tree) = UiComponentTree::from_blueprint(&blueprint) else {
            return false;
        };
        *tree = new_tree;
    }
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

    use logos_abi::{GuiNodeOperation, InputMessage, PointerState, SurfaceHandle};
    use logos_ui_graphics::{UiSceneTheme, emit};

    use super::*;

    #[test]
    fn home_scene_states_stay_within_the_display_node_budget() {
        let surface = SurfaceHandle::new(1, 1, 13).unwrap();
        let mut tree = UiComponentTree::new();
        let mut check = |atrium: &Atrium, state: &str| {
            assert!(build_home_scene(&mut tree, atrium, 0));
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
    }
}
