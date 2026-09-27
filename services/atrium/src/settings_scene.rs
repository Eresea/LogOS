use logos_abi::GuiRect;
use logos_ui::{UiBlueprint, UiComponentTree, UiIcon, UiNodeKind, UiStyle, UiStyleList, UiText};

use crate::{
    Atrium, KeyboardLayout, MouseAcceleration, SETTINGS_NAV_LEFT, SETTINGS_NAV_WIDTH,
    SETTINGS_PAGE_LEFT, SETTINGS_PANE_TOP, SETTINGS_SEARCH_BOUNDS, SETTINGS_SELECT_BOUNDS,
    STATUS_BAR_BOUNDS, SettingsPage, settings_category_bounds, surface_close_bounds,
};

const PAGES: usize = SettingsPage::ALL.len();
const MAX_OPTIONS: usize = 4;

// Node indices follow blueprint push order in `mount`.
const ROOT: usize = 0;
const TITLE_BAR: usize = 1;
const TITLE: usize = 2;
const CLOSE: usize = 3;
const CLOSE_LABEL: usize = 4;
const NAV: usize = 5;
const NAV_TITLE: usize = 6;
const SEARCH: usize = 7;
/// Each category is a row (selection fill), its icon, and its label.
const CATEGORY_BASE: usize = 8;
const PAGE: usize = CATEGORY_BASE + PAGES * 3;
const PAGE_TITLE: usize = PAGE + 1;
const FIELD: usize = PAGE + 2;
const SELECT: usize = PAGE + 3;
const SELECT_VALUE: usize = PAGE + 4;
const SELECT_CHEVRON: usize = PAGE + 5;
const POPOVER: usize = PAGE + 6;
const HIGHLIGHT: usize = PAGE + 7;
const OPTION_BASE: usize = PAGE + 8;
const SCROLLBAR: usize = OPTION_BASE + MAX_OPTIONS;
const NODE_COUNT: usize = SCROLLBAR + 1;

/// What a Settings page shows in the page pane: a title and one labelled
/// select. Adding a page adds its arm here.
struct PageContent {
    field: &'static [u8],
    value: &'static [u8],
    options: [&'static [u8]; MAX_OPTIONS],
}

fn page_content(page: SettingsPage, atrium: &Atrium) -> PageContent {
    match page {
        SettingsPage::Keyboard => PageContent {
            field: b"Keyboard layout",
            value: match atrium.keyboard_layout() {
                KeyboardLayout::Azerty => b"AZERTY",
                KeyboardLayout::Qwerty => b"QWERTY",
            },
            options: [b"AZERTY", b"QWERTY", b"", b""],
        },
        SettingsPage::Mouse => PageContent {
            field: b"Pointer acceleration",
            value: match atrium.mouse_acceleration() {
                MouseAcceleration::Off => b"Off",
                MouseAcceleration::Low => b"Low",
                MouseAcceleration::Medium => b"Medium",
                MouseAcceleration::High => b"High",
            },
            options: [b"Off", b"Low", b"Medium", b"High"],
        },
    }
}

fn styles(tokens: &[UiStyle]) -> UiStyleList {
    let mut list = UiStyleList::EMPTY;
    for token in tokens {
        let _ = list.push(*token);
    }
    list
}

#[inline(never)]
fn mount(tree: &mut UiComponentTree) -> bool {
    let mut blueprint = UiBlueprint::new();
    let Ok(root) = blueprint.push_root(UiNodeKind::Root, 1) else { return false };
    let mut nodes = [(UiNodeKind::Root, UiStyleList::EMPTY, &b""[..], UiIcon::None); NODE_COUNT];
    let rounded = styles(&[UiStyle::RoundedLarge]);
    let muted = styles(&[UiStyle::TextMuted]);
    let title = styles(&[UiStyle::Text4xl]);
    nodes[TITLE_BAR] = (UiNodeKind::Panel, UiStyleList::EMPTY, b"", UiIcon::None);
    nodes[TITLE] = (UiNodeKind::Label, UiStyleList::EMPTY, b"Settings", UiIcon::None);
    nodes[CLOSE] = (
        UiNodeKind::Panel,
        styles(&[UiStyle::BackgroundAccent, UiStyle::RoundedLarge]),
        b"",
        UiIcon::None,
    );
    nodes[CLOSE_LABEL] = (UiNodeKind::Label, UiStyleList::EMPTY, b"X", UiIcon::None);
    nodes[NAV] = (UiNodeKind::Panel, rounded, b"", UiIcon::None);
    nodes[NAV_TITLE] = (UiNodeKind::Label, title, b"Settings", UiIcon::None);
    nodes[SEARCH] = (UiNodeKind::TextInput, rounded, b"Search settings...", UiIcon::None);
    for (index, page) in SettingsPage::ALL.into_iter().enumerate() {
        let base = CATEGORY_BASE + index * 3;
        nodes[base] = (
            UiNodeKind::Button,
            styles(&[UiStyle::RoundedLarge, UiStyle::Transparent]),
            b"",
            UiIcon::None,
        );
        nodes[base + 1] = (UiNodeKind::Button, styles(&[UiStyle::Transparent]), b"", page.icon());
        nodes[base + 2] = (UiNodeKind::Label, UiStyleList::EMPTY, page.label(), UiIcon::None);
    }
    nodes[PAGE] = (UiNodeKind::Panel, rounded, b"", UiIcon::None);
    nodes[PAGE_TITLE] = (UiNodeKind::Label, title, b"", UiIcon::None);
    nodes[FIELD] = (UiNodeKind::Label, muted, b"", UiIcon::None);
    nodes[SELECT] = (UiNodeKind::Button, rounded, b"", UiIcon::None);
    nodes[SELECT_VALUE] = (UiNodeKind::Label, UiStyleList::EMPTY, b"", UiIcon::None);
    nodes[SELECT_CHEVRON] = (UiNodeKind::Label, muted, b"v", UiIcon::None);
    nodes[POPOVER] = (UiNodeKind::Panel, rounded, b"", UiIcon::None);
    nodes[HIGHLIGHT] = (UiNodeKind::Panel, rounded, b"", UiIcon::None);
    for node in &mut nodes[OPTION_BASE..SCROLLBAR] {
        *node = (UiNodeKind::Label, UiStyleList::EMPTY, b"", UiIcon::None);
    }
    nodes[SCROLLBAR] = (UiNodeKind::Panel, rounded, b"", UiIcon::None);
    for (index, (kind, node_styles, text, icon)) in nodes.into_iter().enumerate().skip(1) {
        let Ok(node) = blueprint.push_child(kind, root, 1 + index as u16) else { return false };
        if usize::from(node) != index
            || blueprint.set_styles(node, node_styles).is_err()
            || blueprint.set_icon(node, icon).is_err()
        {
            return false;
        }
        if !text.is_empty() {
            let Some(text) = UiText::from_bytes(text) else { return false };
            if blueprint.set_text(node, text).is_err() {
                return false;
            }
        }
    }
    let Ok(mounted) = UiComponentTree::from_blueprint(&blueprint) else { return false };
    *tree = mounted;
    true
}

fn set_bounds(tree: &mut UiComponentTree, index: usize, bounds: GuiRect) -> bool {
    let Ok(handle) = tree.tree().handle_at(index) else { return false };
    tree.tree_mut()
        .set_bounds(handle, logos_ui::UiRect::new(bounds.x, bounds.y, bounds.width, bounds.height))
        .is_ok()
}

fn set_text(tree: &mut UiComponentTree, index: usize, text: &[u8]) -> bool {
    let Ok(handle) = tree.tree().handle_at(index) else { return false };
    let Some(text) = UiText::from_bytes(text) else { return false };
    tree.set_text(handle, text).is_ok()
}

fn set_styles(tree: &mut UiComponentTree, index: usize, list: UiStyleList) -> bool {
    let Ok(handle) = tree.tree().handle_at(index) else { return false };
    tree.set_styles(handle, list).is_ok()
}

fn offset(bounds: GuiRect, rect: GuiRect) -> GuiRect {
    GuiRect::new(bounds.x + rect.x, bounds.y + rect.y, rect.width, rect.height)
}

/// Updates the retained Settings tree for a surface at `bounds`: a category
/// list on the left and the selected page on the right.
pub fn build_settings_scene(tree: &mut UiComponentTree, bounds: GuiRect, atrium: &Atrium) -> bool {
    if tree.tree().len() != NODE_COUNT && !mount(tree) {
        return false;
    }
    let pane_height = bounds.height.saturating_sub(SETTINGS_PANE_TOP as u32 + 20);
    let close = surface_close_bounds(bounds);
    if !set_bounds(tree, ROOT, bounds)
        || !set_bounds(
            tree,
            TITLE_BAR,
            GuiRect::new(bounds.x, bounds.y, bounds.width, STATUS_BAR_BOUNDS.height),
        )
        || !set_bounds(
            tree,
            TITLE,
            GuiRect::new(bounds.x + 16, bounds.y, 180, STATUS_BAR_BOUNDS.height),
        )
        || !set_bounds(tree, CLOSE, offset(bounds, close))
        || !set_bounds(
            tree,
            CLOSE_LABEL,
            GuiRect::new(bounds.x + close.x + 16, bounds.y + 10, 20, 20),
        )
        || !set_bounds(
            tree,
            NAV,
            offset(
                bounds,
                GuiRect::new(SETTINGS_NAV_LEFT, SETTINGS_PANE_TOP, SETTINGS_NAV_WIDTH, pane_height),
            ),
        )
        || !set_bounds(tree, NAV_TITLE, offset(bounds, GuiRect::new(36, 64, 196, 32)))
        || !set_bounds(tree, SEARCH, offset(bounds, SETTINGS_SEARCH_BOUNDS))
    {
        return false;
    }
    let Ok(search) = tree.tree().handle_at(SEARCH) else { return false };
    if tree.set_value(search, atrium.settings_search_query()).is_err()
        || tree.tree_mut().set_focused(search, atrium.settings_search_active()).is_err()
    {
        return false;
    }

    let current = atrium.settings_page();
    let mut visible_index = 0;
    for (index, page) in SettingsPage::ALL.into_iter().enumerate() {
        let base = CATEGORY_BASE + index * 3;
        let row = if atrium.settings_category_visible(page) {
            visible_index += 1;
            offset(bounds, settings_category_bounds(visible_index - 1))
        } else {
            GuiRect::EMPTY
        };
        let (icon, label) = if row.is_empty() {
            (GuiRect::EMPTY, GuiRect::EMPTY)
        } else {
            (
                GuiRect::new(row.x + 12, row.y + (row.height as i32 - 24) / 2, 24, 24),
                GuiRect::new(row.x + 48, row.y, row.width.saturating_sub(56), row.height),
            )
        };
        // Rows are list items: focus fill when selected, control fill when
        // hovered, and the pane colour otherwise. The icon mirrors its row's
        // state so its own fill matches the row underneath.
        let selected = page == current;
        let hovered = atrium.settings_category_hover() == Some(page);
        for node in [base, base + 1] {
            let Ok(handle) = tree.tree().handle_at(node) else { return false };
            if tree.tree_mut().set_focused(handle, selected).is_err()
                || tree.tree_mut().set_hovered(handle, hovered).is_err()
            {
                return false;
            }
        }
        if !set_bounds(tree, base, row)
            || !set_bounds(tree, base + 1, icon)
            || !set_bounds(tree, base + 2, label)
        {
            return false;
        }
    }

    let content = page_content(current, atrium);
    let page_width = bounds.width.saturating_sub(SETTINGS_PAGE_LEFT as u32 + 20);
    let select = offset(bounds, SETTINGS_SELECT_BOUNDS);
    if !set_bounds(
        tree,
        PAGE,
        offset(
            bounds,
            GuiRect::new(SETTINGS_PAGE_LEFT, SETTINGS_PANE_TOP, page_width, pane_height),
        ),
    ) || !set_bounds(
        tree,
        PAGE_TITLE,
        offset(bounds, GuiRect::new(SETTINGS_SELECT_BOUNDS.x, 72, page_width - 48, 32)),
    ) || !set_text(tree, PAGE_TITLE, current.label())
        || !set_bounds(
            tree,
            FIELD,
            offset(
                bounds,
                GuiRect::new(SETTINGS_SELECT_BOUNDS.x, SETTINGS_SELECT_BOUNDS.y - 32, 360, 24),
            ),
        )
        || !set_text(tree, FIELD, content.field)
        || !set_bounds(tree, SELECT, select)
        || !set_bounds(
            tree,
            SELECT_VALUE,
            GuiRect::new(select.x + 16, select.y, select.width.saturating_sub(48), select.height),
        )
        || !set_text(tree, SELECT_VALUE, content.value)
        || !set_bounds(
            tree,
            SELECT_CHEVRON,
            GuiRect::new(select.x + select.width as i32 - 28, select.y, 16, select.height),
        )
    {
        return false;
    }
    let select_open = match current {
        SettingsPage::Keyboard => atrium.keyboard_select_open(),
        SettingsPage::Mouse => atrium.mouse_select_open(),
    };
    let select_hovered = match current {
        SettingsPage::Keyboard => atrium.keyboard_select_hovered(),
        SettingsPage::Mouse => atrium.mouse_select_hovered(),
    };
    let hovered_option = match current {
        SettingsPage::Keyboard => atrium.keyboard_select_hovered_option(),
        SettingsPage::Mouse => atrium.mouse_select_hovered_option(),
    };
    let select_styles = if select_hovered || select_open {
        styles(&[UiStyle::RoundedLarge, UiStyle::BackgroundAccent])
    } else {
        styles(&[UiStyle::RoundedLarge])
    };
    if !set_styles(tree, SELECT, select_styles) {
        return false;
    }

    // Popover layouts are surface-relative; the scene is in screen space.
    let layout = atrium.settings_select_popover(GuiRect::new(0, 0, bounds.width, bounds.height));
    let local = |rect: logos_ui::UiRect| {
        if rect.is_empty() {
            GuiRect::EMPTY
        } else {
            GuiRect::new(bounds.x + rect.x, bounds.y + rect.y, rect.width, rect.height)
        }
    };
    let open_rect = |rect: GuiRect| if select_open { rect } else { GuiRect::EMPTY };
    let highlight = hovered_option.map(|index| local(layout.option_bounds(index)));
    let highlight_styles = if highlight.is_some() {
        styles(&[UiStyle::RoundedLarge, UiStyle::BackgroundAccent])
    } else {
        styles(&[UiStyle::RoundedLarge])
    };
    if !set_bounds(tree, POPOVER, open_rect(local(layout.bounds)))
        || !set_bounds(tree, HIGHLIGHT, open_rect(highlight.unwrap_or(GuiRect::EMPTY)))
        || !set_styles(tree, HIGHLIGHT, highlight_styles)
        || !set_bounds(tree, SCROLLBAR, open_rect(local(layout.scrollbar)))
    {
        return false;
    }
    for (index, option) in content.options.into_iter().enumerate() {
        let shown = usize::from(layout.first_option)
            ..usize::from(layout.first_option.saturating_add(layout.visible_options));
        let rect = if shown.contains(&index) && !option.is_empty() {
            let option_bounds = local(layout.option_bounds(index as u8));
            GuiRect::new(
                option_bounds.x + 16,
                option_bounds.y,
                option_bounds.width.saturating_sub(32),
                option_bounds.height,
            )
        } else {
            GuiRect::EMPTY
        };
        if !set_bounds(tree, OPTION_BASE + index, open_rect(rect))
            || !set_text(tree, OPTION_BASE + index, option)
        {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use std::format;

    use logos_abi::{GuiNodeOperation, GuiSceneOp, InputMessage, IpcStatus, PointerState};
    use logos_ui_graphics::{MAX_UI_SCENE_OPS, UiScenePublisher, UiSceneSink, UiSceneTheme, emit};

    use super::*;
    use crate::{FULLSCREEN_SURFACE_BOUNDS, SETTINGS_SELECT_BOUNDS};

    /// Counts `Clear` operations; the publisher sends one on the first frame
    /// only, so page and popover changes must never remount the scene.
    struct ClearCountingSink(usize);

    impl UiSceneSink for ClearCountingSink {
        fn send(&mut self, operation: &GuiSceneOp) -> IpcStatus {
            if operation.operation == GuiNodeOperation::Clear {
                self.0 += 1;
            }
            IpcStatus::Ok
        }
    }

    fn pointer(atrium: &mut Atrium, x: i32, y: i32, down: bool) {
        let (buttons, state) = if down { (1, PointerState::Down) } else { (0, PointerState::Move) };
        let _ = atrium
            .settings_input(&InputMessage::pointer(x as i16, y as i16, buttons, state).unwrap());
    }

    #[test]
    fn settings_scene_states_stay_within_the_scene_budget() {
        let surface = logos_abi::SurfaceHandle::new(1, 1, 13).unwrap();
        let bounds = FULLSCREEN_SURFACE_BOUNDS;
        let mut tree = UiComponentTree::new();
        let mut publisher = UiScenePublisher::new();
        let mut frame = 0;
        let mut sink = ClearCountingSink(0);
        let mut check = |atrium: &Atrium, state: &str| {
            assert!(build_settings_scene(&mut tree, bounds, atrium), "{state}: build");
            let scene = emit(surface, 1, &tree, UiSceneTheme::DEFAULT)
                .unwrap_or_else(|error| panic!("{state}: {error:?}"));
            let upserts = scene
                .as_slice()
                .iter()
                .filter(|operation| operation.operation == GuiNodeOperation::Upsert)
                .count();
            assert!(upserts <= logos_abi::MAX_GUI_NODES, "{state}: {upserts} upserts");
            frame += 1;
            let (status, sent) = publisher
                .publish(surface, frame, &tree, UiSceneTheme::DEFAULT, None, &mut sink)
                .unwrap_or_else(|error| panic!("{state}: {error:?}"));
            assert_eq!(status, IpcStatus::Ok, "{state}");
            assert!(sent <= MAX_UI_SCENE_OPS, "{state}: {sent} ops");
        };

        for (index, page) in SettingsPage::ALL.into_iter().enumerate() {
            let mut atrium = Atrium::new();
            atrium.authenticate();
            let row = settings_category_bounds(index);
            pointer(&mut atrium, row.x + 4, row.y + 4, true);
            assert_eq!(atrium.settings_page(), page);
            check(&atrium, &format!("{page:?} idle"));
            for hovered in 0..SettingsPage::ALL.len() {
                let row = settings_category_bounds(hovered);
                pointer(&mut atrium, row.x + 4, row.y + 4, false);
                check(&atrium, &format!("{page:?} category hover={hovered}"));
            }
            let select = SETTINGS_SELECT_BOUNDS;
            pointer(&mut atrium, select.x + 4, select.y + 4, false);
            check(&atrium, &format!("{page:?} select hovered"));
            pointer(&mut atrium, select.x + 4, select.y + 4, true);
            check(&atrium, &format!("{page:?} select open"));
            let layout = atrium.settings_select_popover(FULLSCREEN_SURFACE_BOUNDS);
            for option in 0..layout.visible_options {
                let rect = layout.option_bounds(layout.first_option + option);
                pointer(&mut atrium, rect.x + 4, rect.y + 4, false);
                check(&atrium, &format!("{page:?} option hover={option}"));
            }
        }

        let mut filtered = Atrium::new();
        filtered.authenticate();
        let search = crate::SETTINGS_SEARCH_BOUNDS;
        pointer(&mut filtered, search.x + 4, search.y + 4, true);
        for query in [&b"mouse"[..], b"zzz"] {
            assert!(filtered.settings_input(&InputMessage::text(query).unwrap()));
            check(&filtered, &format!("search={query:?}"));
        }
        assert_eq!(sink.0, 1, "only the first publish clears the surface");
    }

    #[test]
    fn selected_and_hovered_categories_are_styled_apart() {
        let mut tree = UiComponentTree::new();
        let mut atrium = Atrium::new();
        atrium.authenticate();
        let mouse = settings_category_bounds(1);
        pointer(&mut atrium, mouse.x + 4, mouse.y + 4, false);
        assert!(build_settings_scene(&mut tree, FULLSCREEN_SURFACE_BOUNDS, &atrium));
        let state = |tree: &UiComponentTree, index: usize| {
            let node = tree.tree().node(tree.tree().handle_at(index).unwrap()).unwrap();
            (node.interaction.is_focused(), node.interaction.is_hovered())
        };
        let keyboard_row = CATEGORY_BASE;
        let mouse_row = CATEGORY_BASE + 3;
        assert_eq!(state(&tree, keyboard_row), (true, false));
        assert_eq!(state(&tree, keyboard_row + 1), (true, false), "icon mirrors its row");
        assert_eq!(state(&tree, mouse_row), (false, true));

        pointer(&mut atrium, 900, 600, false);
        assert!(build_settings_scene(&mut tree, FULLSCREEN_SURFACE_BOUNDS, &atrium));
        assert_eq!(state(&tree, mouse_row), (false, false));
    }
}
