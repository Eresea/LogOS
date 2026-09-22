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
    // ponytail: the display retains 15 nodes per surface; a query label keeps the shell scene
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
    let _ = tree.advance(now);
    true
}

#[cfg(test)]
mod tests {
    use logos_abi::{GuiNodeOperation, InputMessage, PointerState, SurfaceHandle};
    use logos_ui_graphics::{UiSceneTheme, emit};

    use super::*;

    #[test]
    fn home_scene_states_stay_within_the_display_node_budget() {
        let surface = SurfaceHandle::new(1, 1, 13).unwrap();
        let mut tree = UiComponentTree::new();
        for query in [&b""[..], b"Calculator", b"Files", b"Terminal", b"System", b"none"] {
            for menu_open in [false, true] {
                let mut atrium = Atrium::new();
                atrium.authenticate();
                if !query.is_empty() {
                    atrium.input(&InputMessage::text(query).unwrap());
                }
                if !menu_open {
                    atrium.close_command_menu();
                }
                for (hover, x, y) in [(0, 300, 300), (1, 20, 20), (2, 20, 760)] {
                    atrium.settings_menu_input(
                        &InputMessage::pointer(x, y, 0, PointerState::Move).unwrap(),
                    );
                    for selected in 0..atrium.launcher_result_count().max(1) {
                        if menu_open && atrium.launcher_result_count() > 0 {
                            atrium.command_menu_item_at(
                                crate::COMMAND_MENU_ITEM_LEFT + 1,
                                crate::COMMAND_MENU_ITEM_TOP
                                    + selected as i32
                                        * (crate::COMMAND_MENU_ITEM_HEIGHT as i32
                                            + crate::COMMAND_MENU_ITEM_GAP)
                                    + 1,
                            );
                        }
                        assert!(build_home_scene(&mut tree, &atrium, 0));
                        let scene = emit(surface, 1, &tree, UiSceneTheme::DEFAULT).unwrap();
                        let upserts = scene
                            .as_slice()
                            .iter()
                            .filter(|operation| operation.operation == GuiNodeOperation::Upsert)
                            .count();
                        assert!(
                            upserts <= logos_abi::MAX_GUI_NODES,
                            "query={query:?}, menu_open={menu_open}, hover={hover}, selected={selected}: {upserts} Upserts"
                        );
                    }
                }
            }
        }
    }
}
