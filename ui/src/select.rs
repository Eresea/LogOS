use crate::events::{UiInputEvent, UiOutput, UiOutputError};
use crate::runtime::{UiInteraction, UiInteractive};
use crate::template::UiText;
use crate::{UiComponent, UiComponentContract, UiEventDisposition, UiRect};

pub const UI_KEY_ESCAPE: u16 = 1;
pub const MAX_UI_SELECT_OPTIONS: usize = 4;
pub const UI_SELECT_NO_SELECTION: u8 = u8::MAX;
pub const UI_SELECT_GAP: i32 = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiPopoverPlacement {
    Above,
    Below,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiPopoverLayout {
    pub bounds: UiRect,
    pub option_height: u32,
    pub first_option: u8,
    pub visible_options: u8,
    pub scrollbar_track: UiRect,
    pub scrollbar: UiRect,
    pub placement: UiPopoverPlacement,
}

impl UiPopoverLayout {
    pub const EMPTY: Self = Self {
        bounds: UiRect::EMPTY,
        option_height: 0,
        first_option: 0,
        visible_options: 0,
        scrollbar_track: UiRect::EMPTY,
        scrollbar: UiRect::EMPTY,
        placement: UiPopoverPlacement::Below,
    };

    pub const fn option_bounds(self, index: u8) -> UiRect {
        if index < self.first_option
            || index >= self.first_option.saturating_add(self.visible_options)
        {
            return UiRect::EMPTY;
        }
        UiRect::new(
            self.bounds.x,
            self.bounds.y.saturating_add(
                (index.saturating_sub(self.first_option) as i32)
                    .saturating_mul(self.option_height as i32),
            ),
            self.bounds.width,
            self.option_height,
        )
    }

    pub const fn option_at(self, x: i32, y: i32) -> Option<u8> {
        if !self.bounds.contains(x, y) || self.option_height == 0 {
            return None;
        }
        let row = ((y - self.bounds.y) as u32) / self.option_height;
        if row < self.visible_options as u32 {
            Some(self.first_option.saturating_add(row as u8))
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiPopover {
    open: bool,
    scroll_offset: u8,
    hovered_option: u8,
}

impl UiPopover {
    pub const fn new() -> Self {
        Self { open: false, scroll_offset: 0, hovered_option: UI_SELECT_NO_SELECTION }
    }

    pub const fn is_open(self) -> bool {
        self.open
    }

    pub const fn scroll_offset(self) -> u8 {
        self.scroll_offset
    }

    pub const fn hovered_option(self) -> Option<u8> {
        if self.hovered_option == UI_SELECT_NO_SELECTION { None } else { Some(self.hovered_option) }
    }

    pub fn open(&mut self) -> bool {
        if self.open {
            return false;
        }
        self.open = true;
        true
    }

    pub fn close(&mut self) -> bool {
        if !self.open {
            return false;
        }
        self.open = false;
        self.scroll_offset = 0;
        self.hovered_option = UI_SELECT_NO_SELECTION;
        true
    }

    pub fn toggle(&mut self) -> bool {
        if self.open { self.close() } else { self.open() }
    }

    pub fn scroll_to(&mut self, offset: u8, option_count: u8, visible_options: u8) -> bool {
        let offset = offset.min(option_count.saturating_sub(visible_options));
        if self.scroll_offset == offset {
            return false;
        }
        self.scroll_offset = offset;
        true
    }

    pub fn set_hovered_option(&mut self, option: Option<u8>, option_count: u8) -> bool {
        let hovered_option =
            option.filter(|index| *index < option_count).unwrap_or(UI_SELECT_NO_SELECTION);
        if self.hovered_option == hovered_option {
            return false;
        }
        self.hovered_option = hovered_option;
        true
    }

    pub fn layout(
        self,
        anchor: UiRect,
        viewport: UiRect,
        option_count: u8,
        option_height: u32,
        max_height: u32,
    ) -> UiPopoverLayout {
        if anchor.is_empty() || viewport.is_empty() || option_count == 0 || option_height == 0 {
            return UiPopoverLayout::EMPTY;
        }
        let below = viewport
            .y
            .saturating_add(viewport.height as i32)
            .saturating_sub(
                anchor.y.saturating_add(anchor.height as i32).saturating_add(UI_SELECT_GAP),
            )
            .max(0) as u32;
        let above = anchor.y.saturating_sub(viewport.y).saturating_sub(UI_SELECT_GAP).max(0) as u32;
        let desired = u32::from(option_count).saturating_mul(option_height);
        let max_height = max_height.max(option_height).min(viewport.height);
        let (placement, available) =
            if desired <= below || (below >= above && above < option_height) {
                (UiPopoverPlacement::Below, below)
            } else {
                (UiPopoverPlacement::Above, above)
            };
        let visible_options =
            (available.min(max_height) / option_height).min(u32::from(option_count));
        if visible_options == 0 {
            return UiPopoverLayout::EMPTY;
        }
        let height = visible_options.saturating_mul(option_height);
        let width = anchor.width.min(viewport.width);
        let max_x = viewport.x.saturating_add(viewport.width.saturating_sub(width) as i32);
        let x = anchor.x.clamp(viewport.x, max_x);
        let unclamped_y = match placement {
            UiPopoverPlacement::Below => {
                anchor.y.saturating_add(anchor.height as i32).saturating_add(UI_SELECT_GAP)
            }
            UiPopoverPlacement::Above => {
                anchor.y.saturating_sub(height as i32).saturating_sub(UI_SELECT_GAP)
            }
        };
        let max_y = viewport.y.saturating_add(viewport.height.saturating_sub(height) as i32);
        let y = unclamped_y.clamp(viewport.y, max_y);
        let first_option =
            self.scroll_offset.min(option_count.saturating_sub(visible_options as u8));
        let (scrollbar_track, scrollbar) = if visible_options < u32::from(option_count) {
            let track = UiRect::new(
                x.saturating_add(width as i32).saturating_sub(10),
                y + 6,
                4,
                height.saturating_sub(12),
            );
            let thumb_height =
                (track.height.saturating_mul(visible_options) / u32::from(option_count)).max(8);
            let max_thumb_y = track.height.saturating_sub(thumb_height);
            let max_offset = option_count.saturating_sub(visible_options as u8);
            let thumb_y = if max_offset == 0 {
                0
            } else {
                max_thumb_y.saturating_mul(u32::from(first_option)) / u32::from(max_offset)
            };
            (
                track,
                UiRect::new(
                    track.x,
                    track.y.saturating_add(thumb_y as i32),
                    track.width,
                    thumb_height,
                ),
            )
        } else {
            (UiRect::EMPTY, UiRect::EMPTY)
        };
        UiPopoverLayout {
            bounds: UiRect::new(x, y, width, height),
            option_height,
            first_option,
            visible_options: visible_options as u8,
            scrollbar_track,
            scrollbar,
            placement,
        }
    }
}

impl Default for UiPopover {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiSelectEvent {
    Opened,
    Closed,
    Changed { index: u8 },
}

#[derive(Clone, Copy)]
pub struct UiSelect {
    interaction: UiInteraction,
    options: [UiText; MAX_UI_SELECT_OPTIONS],
    option_count: u8,
    selected: u8,
    popover: UiPopover,
}

impl UiSelect {
    pub const fn new(option_count: u8) -> Self {
        Self::with_selection(option_count, None)
    }

    pub const fn with_selection(option_count: u8, selected: Option<u8>) -> Self {
        let option_count = if option_count > MAX_UI_SELECT_OPTIONS as u8 {
            MAX_UI_SELECT_OPTIONS as u8
        } else {
            option_count
        };
        let selected = match selected {
            Some(index) if index < option_count => index,
            _ => UI_SELECT_NO_SELECTION,
        };
        Self {
            interaction: UiInteraction::for_kind(crate::UiNodeKind::Button),
            options: [UiText::EMPTY; MAX_UI_SELECT_OPTIONS],
            option_count,
            selected,
            popover: UiPopover::new(),
        }
    }

    pub const fn option_count(self) -> u8 {
        self.option_count
    }

    pub const fn selected(self) -> Option<u8> {
        if self.selected == UI_SELECT_NO_SELECTION { None } else { Some(self.selected) }
    }

    pub const fn selected_label(self) -> Option<UiText> {
        let Some(index) = self.selected() else { return None };
        Some(self.options[index as usize])
    }

    pub const fn option_label(self, index: u8) -> Option<UiText> {
        if index < self.option_count { Some(self.options[index as usize]) } else { None }
    }

    pub const fn is_open(self) -> bool {
        self.popover.is_open()
    }

    pub const fn is_hovered(self) -> bool {
        self.interaction.is_hovered()
    }

    pub const fn scroll_offset(self) -> u8 {
        self.popover.scroll_offset()
    }

    pub const fn hovered_option(self) -> Option<u8> {
        self.popover.hovered_option()
    }

    pub fn set_hovered(&mut self, hovered: bool) -> bool {
        if self.interaction.is_hovered() == hovered {
            return false;
        }
        self.interaction.set_hovered(hovered);
        true
    }

    pub fn set_option_hovered(&mut self, option: Option<u8>) -> bool {
        self.popover.set_hovered_option(option, self.option_count)
    }

    pub fn set_options(&mut self, options: &[UiText]) -> bool {
        let count = options.len().min(MAX_UI_SELECT_OPTIONS);
        let changed = self.option_count != count as u8 || self.options[..count] != options[..count];
        self.options.fill(UiText::EMPTY);
        self.options[..count].copy_from_slice(&options[..count]);
        self.option_count = count as u8;
        if self.selected >= self.option_count && self.selected != UI_SELECT_NO_SELECTION {
            self.selected = UI_SELECT_NO_SELECTION;
        }
        changed
    }

    pub fn set_selected(&mut self, selected: Option<u8>) -> bool {
        let selected =
            selected.filter(|index| *index < self.option_count).unwrap_or(UI_SELECT_NO_SELECTION);
        if self.selected == selected {
            return false;
        }
        self.selected = selected;
        true
    }

    pub fn open(&mut self) -> bool {
        self.popover.open()
    }

    pub fn close(&mut self) -> bool {
        self.popover.close()
    }

    pub fn toggle(&mut self) -> bool {
        self.popover.toggle()
    }

    pub fn select(&mut self, index: u8) -> bool {
        if index >= self.option_count {
            return false;
        }
        let changed = self.selected != index;
        self.selected = index;
        self.popover.close();
        changed
    }

    pub fn scroll_to(&mut self, offset: u8, visible_options: u8) -> bool {
        self.popover.scroll_to(offset, self.option_count, visible_options)
    }

    pub fn scroll_by(&mut self, delta: i8, visible_options: u8) -> bool {
        let offset = if delta.is_negative() {
            self.scroll_offset().saturating_sub(delta.unsigned_abs())
        } else {
            self.scroll_offset().saturating_add(delta as u8)
        };
        self.scroll_to(offset, visible_options)
    }

    pub fn popover_layout(
        self,
        anchor: UiRect,
        viewport: UiRect,
        option_height: u32,
        max_height: u32,
    ) -> UiPopoverLayout {
        self.popover.layout(anchor, viewport, self.option_count, option_height, max_height)
    }

    pub fn handle_event(
        &mut self,
        event: UiInputEvent,
        output: &mut UiOutput<UiSelectEvent>,
    ) -> Result<UiEventDisposition, UiOutputError> {
        if self.interaction.is_disabled() {
            return Ok(UiEventDisposition::Ignored);
        }
        match event {
            UiInputEvent::Focus => {
                self.interaction.set_focused(true);
                Ok(UiEventDisposition::Consumed)
            }
            UiInputEvent::Blur => {
                self.interaction.set_focused(false);
                if self.close() {
                    output.emit(UiSelectEvent::Closed)?;
                }
                Ok(UiEventDisposition::Consumed)
            }
            UiInputEvent::Click | UiInputEvent::PointerUp { .. } => {
                if self.toggle() {
                    output.emit(if self.is_open() {
                        UiSelectEvent::Opened
                    } else {
                        UiSelectEvent::Closed
                    })?;
                }
                Ok(UiEventDisposition::Consumed)
            }
            UiInputEvent::KeyDown { code: UI_KEY_ESCAPE, .. } if self.is_open() => {
                self.close();
                output.emit(UiSelectEvent::Closed)?;
                Ok(UiEventDisposition::Consumed)
            }
            UiInputEvent::KeyDown { code: crate::UI_KEY_ENTER, .. } => {
                if self.toggle() {
                    output.emit(if self.is_open() {
                        UiSelectEvent::Opened
                    } else {
                        UiSelectEvent::Closed
                    })?;
                }
                Ok(UiEventDisposition::Consumed)
            }
            _ => Ok(UiEventDisposition::Ignored),
        }
    }
}

impl Default for UiSelect {
    fn default() -> Self {
        Self::new(0)
    }
}

impl UiInteractive for UiSelect {
    fn interaction(&self) -> &UiInteraction {
        &self.interaction
    }

    fn interaction_mut(&mut self) -> &mut UiInteraction {
        &mut self.interaction
    }
}

impl UiComponent for UiSelect {
    type Output = UiSelectEvent;
    const CONTRACT: UiComponentContract = UiComponentContract::for_kind(crate::UiNodeKind::Button);

    fn handle_event(
        &mut self,
        event: UiInputEvent,
        output: &mut UiOutput<Self::Output>,
    ) -> Result<UiEventDisposition, UiOutputError> {
        Self::handle_event(self, event, output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options() -> [UiText; 4] {
        [
            UiText::from_bytes(b"One").unwrap(),
            UiText::from_bytes(b"Two").unwrap(),
            UiText::from_bytes(b"Three").unwrap(),
            UiText::from_bytes(b"Four").unwrap(),
        ]
    }

    #[test]
    fn select_shows_selected_label_and_closes_after_selection() {
        let mut select = UiSelect::new(4);
        assert_eq!(select.selected_label(), None);
        select.set_options(&options());
        assert_eq!(select.selected(), None);
        select.open();
        assert!(select.is_open());
        assert!(select.select(2));
        assert_eq!(select.selected_label(), Some(UiText::from_bytes(b"Three").unwrap()));
        assert!(!select.is_open());
    }

    #[test]
    fn popover_clamps_above_and_reports_scrollbar_for_overflow() {
        let mut popover = UiPopover::new();
        popover.open();
        let layout =
            popover.layout(UiRect::new(20, 720, 200, 40), UiRect::new(0, 0, 240, 800), 8, 40, 160);
        assert_eq!(layout.placement, UiPopoverPlacement::Above);
        assert!(layout.bounds.y >= 0);
        assert!(layout.bounds.y + layout.bounds.height as i32 <= 800);
        assert!(!layout.scrollbar_track.is_empty());
        assert!(!layout.scrollbar.is_empty());
        assert_eq!(layout.option_at(30, layout.bounds.y + 1), Some(0));
    }

    #[test]
    fn popover_option_hover_is_bounded_and_clears_on_close() {
        let mut popover = UiPopover::new();
        popover.open();
        assert!(popover.set_hovered_option(Some(1), 2));
        assert_eq!(popover.hovered_option(), Some(1));
        assert!(popover.set_hovered_option(Some(3), 2));
        assert_eq!(popover.hovered_option(), None);
        popover.close();
        assert_eq!(popover.hovered_option(), None);
    }
}
