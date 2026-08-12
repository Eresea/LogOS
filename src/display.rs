//! Display service state: cells in, pixels out.

use crate::terminal_abi::{Cell, MAX_COLUMNS, MAX_ROWS, MessageKind, RenderMessage};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DisplayError {
    InvalidMessage,
    StaleGeneration,
}

pub struct Display {
    generation: u16,
    columns: usize,
    rows: usize,
    cursor_column: usize,
    cursor_row: usize,
    cells: [Cell; MAX_COLUMNS * MAX_ROWS],
    applied: usize,
}

impl Display {
    pub const fn new(generation: u16) -> Self {
        Self {
            generation,
            columns: 0,
            rows: 0,
            cursor_column: 0,
            cursor_row: 0,
            cells: [Cell::EMPTY; MAX_COLUMNS * MAX_ROWS],
            applied: 0,
        }
    }

    pub const fn generation(&self) -> u16 {
        self.generation
    }
    pub const fn size(&self) -> (usize, usize) {
        (self.columns, self.rows)
    }
    pub const fn cursor(&self) -> (usize, usize) {
        (self.cursor_column, self.cursor_row)
    }
    pub const fn applied_cells(&self) -> usize {
        self.applied
    }

    pub fn replace_generation(&mut self, generation: u16) {
        self.generation = generation;
        self.cells.fill(Cell::EMPTY);
        self.applied = 0;
    }

    pub fn apply(&mut self, generation: u16, message: &RenderMessage) -> Result<(), DisplayError> {
        if generation != self.generation {
            return Err(DisplayError::StaleGeneration);
        }
        if !matches!(message.kind, MessageKind::RenderCells | MessageKind::FullRedraw) {
            return Err(DisplayError::InvalidMessage);
        }
        let columns = usize::from(message.columns);
        let rows = usize::from(message.rows);
        if columns == 0
            || columns > MAX_COLUMNS
            || rows == 0
            || rows > MAX_ROWS
            || message.count as usize > message.cells.len()
        {
            return Err(DisplayError::InvalidMessage);
        }
        for index in 0..message.count as usize {
            let position = usize::from(message.positions[index]);
            if position >= rows * MAX_COLUMNS || position % MAX_COLUMNS >= columns {
                return Err(DisplayError::InvalidMessage);
            }
        }
        if message.kind == MessageKind::FullRedraw {
            self.cells.fill(Cell::EMPTY);
        }
        for index in 0..message.count as usize {
            let position = usize::from(message.positions[index]);
            self.cells[position] = message.cells[index];
        }
        self.columns = columns;
        self.rows = rows;
        self.cursor_column = usize::from(message.cursor_column).min(columns - 1);
        self.cursor_row = usize::from(message.cursor_row).min(rows - 1);
        self.applied += message.count as usize;
        Ok(())
    }

    pub fn cell(&self, column: usize, row: usize) -> Option<Cell> {
        (column < self.columns && row < self.rows).then(|| self.cells[row * MAX_COLUMNS + column])
    }
}

impl Default for Display {
    fn default() -> Self {
        Self::new(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::Terminal;

    #[test]
    fn display_applies_terminal_diffs_and_rejects_stale_pages() {
        let mut terminal = Terminal::new();
        let mut display = Display::new(7);
        while let Some(message) = terminal.next_render() {
            display.apply(7, &message).unwrap();
        }
        assert_eq!(display.size(), (80, 25));
        terminal.feed(b"A");
        while let Some(message) = terminal.next_render() {
            display.apply(7, &message).unwrap();
        }
        assert_eq!(display.cell(0, 0).unwrap().codepoint, b'A' as u32);
        assert_eq!(
            display.apply(6, &RenderMessage::empty(MessageKind::RenderCells)),
            Err(DisplayError::StaleGeneration)
        );
    }

    #[test]
    fn malformed_render_is_atomic() {
        let mut display = Display::new(7);
        let mut valid = RenderMessage::empty(MessageKind::RenderCells);
        valid.columns = 80;
        valid.rows = 25;
        valid.count = 1;
        valid.positions[0] = 0;
        valid.cells[0].codepoint = b'A' as u32;
        display.apply(7, &valid).unwrap();
        let applied = display.applied_cells();

        let mut invalid = RenderMessage::empty(MessageKind::FullRedraw);
        invalid.columns = 80;
        invalid.rows = 25;
        invalid.count = 2;
        invalid.positions[0] = 0;
        invalid.positions[1] = (25 * MAX_COLUMNS) as u16;
        invalid.cells[0].codepoint = b'X' as u32;

        assert_eq!(display.apply(7, &invalid), Err(DisplayError::InvalidMessage));
        assert_eq!(display.cell(0, 0).unwrap().codepoint, b'A' as u32);
        assert_eq!(display.applied_cells(), applied);
    }
}
