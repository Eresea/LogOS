#![no_std]

pub mod display;
pub mod input;
pub mod terminal;
pub mod text;

#[cfg(test)]
mod tests {
    use super::{display, input, terminal, text};

    #[test]
    fn input_layouts_and_bounded_repeats() {
        assert!(input::Service::self_check());
    }

    #[test]
    fn terminal_utf8_editing_navigation_and_history() {
        assert!(terminal::Model::self_check());
    }

    #[test]
    fn display_restart_state_is_valid() {
        assert!(display::Service::self_check());
    }

    #[test]
    fn text_layout_and_font_are_valid() {
        assert!(text::Service::self_check());
    }
}
