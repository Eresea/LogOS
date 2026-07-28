use crate::{display, input, text};

const ACCENT: [u8; 3] = [255; 3];
const BACKGROUND: [u8; 3] = [0; 3];
const ORIGIN: (usize, usize) = (32, 32);
const CELLS: usize = 64;
const SCROLLBACK: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    start: usize,
    end: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Submission {
    cells: [u8; CELLS],
    length: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SearchMatch {
    pub scrollback_offset: Option<usize>,
    pub start: usize,
    pub end: usize,
}

struct Output {
    lines: [Submission; SCROLLBACK],
    head: usize,
    length: usize,
}

impl Output {
    const fn new() -> Self {
        Self { lines: [Submission::EMPTY; SCROLLBACK], head: 0, length: 0 }
    }

    fn push(&mut self, line: Submission) -> bool {
        if self.length == SCROLLBACK {
            self.lines[self.head] = line;
            self.head = (self.head + 1) % SCROLLBACK;
            return true;
        }
        self.lines[self.head] = line;
        self.head = (self.head + 1) % SCROLLBACK;
        self.length += 1;
        true
    }

    fn line(&self, offset: usize) -> Submission {
        self.lines[(self.head + SCROLLBACK - self.length + offset) % SCROLLBACK]
    }

    fn clear(&mut self) {
        self.head = 0;
        self.length = 0;
    }
}

impl Submission {
    const EMPTY: Self = Self { cells: [0; CELLS], length: 0 };
    const fn new(cells: [u8; CELLS], length: usize) -> Self {
        Self { cells, length }
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() > CELLS || core::str::from_utf8(bytes).is_err() {
            return None;
        }
        let mut cells = [0; CELLS];
        cells[..bytes.len()].copy_from_slice(bytes);
        Some(Self::new(cells, bytes.len()))
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.cells[..self.length]
    }
}

pub struct Model {
    cells: [u8; CELLS],
    length: usize,
    cursor: usize,
    caret_visible: bool,
    output: Output,
    scrollback: [Submission; SCROLLBACK],
    scrollback_head: usize,
    scrollback_len: usize,
    history_offset: Option<usize>,
    selection: Option<Selection>,
}

impl Default for Model {
    fn default() -> Self {
        Self::new()
    }
}

impl Model {
    pub const fn new() -> Self {
        Self {
            cells: [0; CELLS],
            length: 0,
            cursor: 0,
            caret_visible: true,
            output: Output::new(),
            scrollback: [Submission::EMPTY; SCROLLBACK],
            scrollback_head: 0,
            scrollback_len: 0,
            history_offset: None,
            selection: None,
        }
    }

    pub fn apply(&mut self, event: input::Event) -> bool {
        match event.pressed() {
            Some((input::LogicalKey::Text(text), _)) => self.insert_utf8(&[text]),
            Some((input::LogicalKey::Backspace, _)) => self.backspace(),
            Some((input::LogicalKey::Delete, _)) => self.delete(),
            Some((input::LogicalKey::Left, _)) if event.control() => self.word_left(),
            Some((input::LogicalKey::Right, _)) if event.control() => self.word_right(),
            Some((input::LogicalKey::Left, _)) => self.move_left(),
            Some((input::LogicalKey::Right, _)) => self.move_right(),
            Some((input::LogicalKey::Home, _)) => {
                self.home();
                true
            }
            Some((input::LogicalKey::End, _)) => {
                self.end();
                true
            }
            Some((input::LogicalKey::Up, _)) => self.history_previous(),
            Some((input::LogicalKey::Down, _)) => self.history_next(),
            _ => false,
        }
    }

    pub fn insert_utf8(&mut self, bytes: &[u8]) -> bool {
        if core::str::from_utf8(bytes).is_err() || self.length + bytes.len() > self.cells.len() {
            return false;
        }
        self.cells.copy_within(self.cursor..self.length, self.cursor + bytes.len());
        self.cells[self.cursor..self.cursor + bytes.len()].copy_from_slice(bytes);
        self.cursor += bytes.len();
        self.length += bytes.len();
        self.selection = None;
        true
    }

    pub fn move_left(&mut self) -> bool {
        let Some(cursor) = self.previous_boundary(self.cursor) else {
            return false;
        };
        self.cursor = cursor;
        self.selection = None;
        true
    }

    pub fn move_right(&mut self) -> bool {
        if self.cursor == self.length {
            return false;
        }
        self.cursor += 1;
        while self.cursor < self.length && self.cells[self.cursor] & 0xc0 == 0x80 {
            self.cursor += 1;
        }
        self.selection = None;
        true
    }

    pub fn home(&mut self) {
        self.cursor = 0;
        self.selection = None;
    }

    pub fn end(&mut self) {
        self.cursor = self.length;
        self.selection = None;
    }

    pub fn backspace(&mut self) -> bool {
        let Some(start) = self.previous_boundary(self.cursor) else {
            return false;
        };
        self.cells.copy_within(self.cursor..self.length, start);
        self.length -= self.cursor - start;
        self.cursor = start;
        self.selection = None;
        true
    }

    fn delete(&mut self) -> bool {
        if self.cursor == self.length {
            return false;
        }
        let mut end = self.cursor + 1;
        while end < self.length && self.cells[end] & 0xc0 == 0x80 {
            end += 1;
        }
        self.cells.copy_within(end..self.length, self.cursor);
        self.length -= end - self.cursor;
        self.selection = None;
        true
    }

    fn word_left(&mut self) -> bool {
        let start = self.cursor;
        while self.cursor > 0 && self.cells[self.cursor - 1] == b' ' {
            let _ = self.move_left();
        }
        while self.cursor > 0 && self.cells[self.cursor - 1] != b' ' {
            let _ = self.move_left();
        }
        self.cursor != start
    }

    fn word_right(&mut self) -> bool {
        let start = self.cursor;
        while self.cursor < self.length && self.cells[self.cursor] != b' ' {
            let _ = self.move_right();
        }
        while self.cursor < self.length && self.cells[self.cursor] == b' ' {
            let _ = self.move_right();
        }
        self.cursor != start
    }

    fn previous_boundary(&self, cursor: usize) -> Option<usize> {
        if cursor == 0 {
            return None;
        }
        let mut boundary = cursor - 1;
        while boundary > 0 && self.cells[boundary] & 0xc0 == 0x80 {
            boundary -= 1;
        }
        Some(boundary)
    }

    fn is_boundary(&self, index: usize) -> bool {
        index == self.length
            || index < self.length && (index == 0 || self.cells[index] & 0xc0 != 0x80)
    }

    pub fn render(&self, display: &mut display::Service, text: &text::Service) -> bool {
        if !display.clear(BACKGROUND) {
            return false;
        }
        let columns = self.columns(display);
        let mut column = 0;
        let mut row = 0;
        for offset in 0..self.output.length {
            if !self.render_bytes(
                display,
                text,
                self.output.line(offset).as_bytes(),
                columns,
                &mut row,
                &mut column,
            ) {
                return false;
            }
            row += 1;
            column = 0;
        }
        if !self.render_bytes(
            display,
            text,
            &self.cells[..self.length],
            columns,
            &mut row,
            &mut column,
        ) {
            return false;
        }
        self.render_caret(display, text)
    }

    pub fn render_caret(&self, display: &mut display::Service, text: &text::Service) -> bool {
        let columns = self.columns(display);
        let (caret_row, caret_column) = Self::position(self.columns_before_cursor(), columns);
        let output_rows = self.output_rows(columns);
        let caret = if self.caret_visible { ACCENT } else { BACKGROUND };
        let x = ORIGIN.0 + caret_column * text::Service::ADVANCE;
        let y = ORIGIN.1
            + (output_rows + caret_row) * text.metrics().height
            + text.metrics().height.saturating_sub(2);
        (0..text::Service::ADVANCE).all(|dx| display.present(x + dx, y, caret))
    }

    fn render_bytes(
        &self,
        display: &mut display::Service,
        text: &text::Service,
        bytes: &[u8],
        columns: usize,
        row: &mut usize,
        column: &mut usize,
    ) -> bool {
        for &glyph in bytes {
            if glyph & 0xc0 != 0x80
                && !text.render(
                    display,
                    glyph,
                    ORIGIN.0 + *column * text::Service::ADVANCE,
                    ORIGIN.1 + *row * text.metrics().height,
                    ACCENT,
                )
            {
                return false;
            }
            *column += usize::from(glyph & 0xc0 != 0x80);
            if *column == columns {
                *column = 0;
                *row += 1;
            }
        }
        true
    }

    pub fn blink(&mut self) {
        self.caret_visible = !self.caret_visible;
    }

    pub fn submit(&mut self) -> Submission {
        let submission = Submission::new(self.cells, self.length);
        self.push_scrollback(submission);
        self.length = 0;
        self.cursor = 0;
        self.selection = None;
        submission
    }

    pub fn write_output(&mut self, bytes: &[u8]) -> bool {
        let Some(line) = Submission::from_bytes(bytes) else {
            return false;
        };
        self.output.push(line)
    }

    pub fn output_line(&self, index: usize) -> Option<Submission> {
        (index < self.output.length).then(|| self.output.line(index))
    }

    pub fn input_line(&self) -> &[u8] {
        &self.cells[..self.length]
    }

    pub fn clear_output(&mut self) {
        self.output.clear();
    }

    pub fn select(&mut self, start: usize, end: usize) -> bool {
        if start > end || end > self.length || !self.is_boundary(start) || !self.is_boundary(end) {
            return false;
        }
        self.selection = Some(Selection { start, end });
        true
    }

    pub fn clear_selection(&mut self) {
        self.selection = None;
    }

    pub fn selected_bytes(&self) -> Option<&[u8]> {
        self.selection.map(|selection| &self.cells[selection.start..selection.end])
    }

    pub fn scrollback_len(&self) -> usize {
        self.scrollback_len
    }

    pub fn history_entry(&self, offset: usize) -> Option<Submission> {
        (offset < self.scrollback_len)
            .then(|| self.scrollback[(self.scrollback_head + SCROLLBACK - 1 - offset) % SCROLLBACK])
    }

    pub fn restore_history(&mut self, entries: &[Submission]) -> bool {
        if entries.len() > SCROLLBACK {
            return false;
        }
        self.scrollback = [Submission::EMPTY; SCROLLBACK];
        self.scrollback_head = 0;
        self.scrollback_len = 0;
        for entry in entries.iter().rev() {
            self.push_scrollback(*entry);
        }
        true
    }

    pub fn search(&self, query: &[u8]) -> Option<SearchMatch> {
        if query.is_empty() || core::str::from_utf8(query).is_err() {
            return None;
        }
        if let Some((start, end)) = Self::find(&self.cells[..self.length], query) {
            return Some(SearchMatch { scrollback_offset: None, start, end });
        }
        for offset in 0..self.output.length {
            let line = self.output.line(self.output.length - 1 - offset);
            if let Some((start, end)) = Self::find(line.as_bytes(), query) {
                return Some(SearchMatch { scrollback_offset: Some(offset), start, end });
            }
        }
        for offset in 0..self.scrollback_len {
            let submission =
                self.scrollback[(self.scrollback_head + SCROLLBACK - 1 - offset) % SCROLLBACK];
            if let Some((start, end)) = Self::find(submission.as_bytes(), query) {
                return Some(SearchMatch { scrollback_offset: Some(offset), start, end });
            }
        }
        None
    }

    fn push_scrollback(&mut self, submission: Submission) {
        self.scrollback[self.scrollback_head] = submission;
        self.scrollback_head = (self.scrollback_head + 1) % SCROLLBACK;
        self.scrollback_len = (self.scrollback_len + 1).min(SCROLLBACK);
        self.history_offset = None;
    }

    fn latest_scrollback(&self) -> Submission {
        self.scrollback[(self.scrollback_head + SCROLLBACK - 1) % SCROLLBACK]
    }

    fn history_previous(&mut self) -> bool {
        if self.scrollback_len == 0 {
            return false;
        }
        let offset =
            self.history_offset.map_or(0, |offset| (offset + 1).min(self.scrollback_len - 1));
        self.history_offset = Some(offset);
        self.load_history(offset);
        true
    }

    fn history_next(&mut self) -> bool {
        let Some(offset) = self.history_offset else {
            return false;
        };
        if offset == 0 {
            self.history_offset = None;
            self.length = 0;
            self.cursor = 0;
        } else {
            self.history_offset = Some(offset - 1);
            self.load_history(offset - 1);
        }
        true
    }

    fn load_history(&mut self, offset: usize) {
        let index = (self.scrollback_head + SCROLLBACK - 1 - offset) % SCROLLBACK;
        let submission = self.scrollback[index];
        self.cells = submission.cells;
        self.length = submission.length;
        self.cursor = self.length;
        self.selection = None;
    }

    fn find(haystack: &[u8], needle: &[u8]) -> Option<(usize, usize)> {
        let haystack = core::str::from_utf8(haystack).ok()?;
        let needle = core::str::from_utf8(needle).ok()?;
        haystack.find(needle).map(|start| (start, start + needle.len()))
    }

    pub fn self_check() -> bool {
        let mut model = Self::new();
        let text = input::Event::Key {
            physical: input::PhysicalKey(0x22),
            logical: input::LogicalKey::Text(b'g'),
            state: input::State::Press,
            modifiers: input::Modifiers::none(),
        };
        let edited = model.apply(text)
            && model.insert_utf8(b"\xc3\xa9")
            && model.move_left()
            && model.insert_utf8(b"b")
            && model.backspace()
            && model.move_right()
            && model.backspace();
        model.home();
        let home = model.cursor == 0;
        model.end();
        let visible = model.caret_visible;
        model.blink();
        let mut navigation = Self::new();
        let inserted = navigation.insert_utf8(b"one two");
        navigation.home();
        let navigation_ok = inserted
            && navigation.word_right()
            && navigation.delete()
            && navigation.word_left()
            && navigation.delete();
        let mut scrollback = Self::new();
        for _ in 0..SCROLLBACK + 1 {
            let _ = scrollback.insert_utf8(b"x");
            let _ = scrollback.submit();
        }
        let history = scrollback.history_previous()
            && scrollback.latest_scrollback().as_bytes() == b"x"
            && scrollback.history_next();
        let Some(x) = Submission::from_bytes(b"x") else {
            return false;
        };
        let Some(first) = Submission::from_bytes(b"first") else {
            return false;
        };
        let Some(second) = Submission::from_bytes(b"second") else {
            return false;
        };
        let persisted_history = scrollback.history_entry(0) == Some(x);
        let mut restored = Self::new();
        let restored_history = restored.restore_history(&[first, second])
            && restored.history_entry(0).is_some_and(|entry| entry.as_bytes() == b"first");
        let mut selection = Self::new();
        let selected = selection.insert_utf8(b"a\xc3\xa9")
            && selection.select(1, 3)
            && selection.selected_bytes() == Some(b"\xc3\xa9" as &[u8]);
        selection.clear_selection();
        let mut invalidated_selection = Self::new();
        let selection_invalidated = invalidated_selection.insert_utf8(b"text")
            && invalidated_selection.select(0, 4)
            && invalidated_selection.insert_utf8(b"!")
            && invalidated_selection.selected_bytes().is_none();
        let mut moved_selection = Self::new();
        let selection_collapsed = moved_selection.insert_utf8(b"text")
            && moved_selection.select(0, 4)
            && moved_selection.move_left()
            && moved_selection.selected_bytes().is_none();
        let mut search = Self::new();
        let output = search.write_output(b"old output") && search.write_output(b"new output");
        search.clear_output();
        let output_cleared = search.output.length == 0;
        let output =
            output && search.write_output(b"old output") && search.write_output(b"new output");
        let _ = search.insert_utf8(b"visible output");
        let visible_match = search.search(b"output")
            == Some(SearchMatch { scrollback_offset: None, start: 8, end: 14 });
        let scrollback_match = search.search(b"old")
            == Some(SearchMatch { scrollback_offset: Some(1), start: 0, end: 3 });
        let Some(first_display) =
            display::Service::new(core::ptr::dangling_mut(), 64 * 80 * 4, 64, 80, 64)
        else {
            return false;
        };
        let Some(replacement_display) =
            display::Service::new(core::ptr::dangling_mut(), 64 * 80 * 4, 64, 80, 64)
        else {
            return false;
        };
        let mut redraw = Self::new();
        let display_restart = redraw.write_output(b"x")
            && redraw.insert_utf8(b"y")
            && redraw.columns(&first_display) == redraw.columns(&replacement_display);
        let mut backpressure = Self::new();
        for _ in 0..SCROLLBACK {
            if !backpressure.write_output(b"x") {
                return false;
            }
        }
        edited
            && home
            && model.cursor == model.length
            && visible != model.caret_visible
            && model.submit().as_bytes() == b"g"
            && model.submit().as_bytes().is_empty()
            && navigation_ok
            && Self::position(6, 4) == (1, 2)
            && scrollback.scrollback_len() == SCROLLBACK
            && scrollback.latest_scrollback().as_bytes() == b"x"
            && history
            && persisted_history
            && restored_history
            && selected
            && selection.selected_bytes().is_none()
            && selection_invalidated
            && selection_collapsed
            && visible_match
            && output
            && output_cleared
            && scrollback_match
            && search.search(b"missing").is_none()
            && display_restart
            && backpressure.write_output(b"x")
    }

    fn columns_before_cursor(&self) -> usize {
        self.cells[..self.cursor].iter().filter(|byte| **byte & 0xc0 != 0x80).count()
    }

    fn output_rows(&self, columns: usize) -> usize {
        (0..self.output.length)
            .map(|offset| {
                let columns_used = self
                    .output
                    .line(offset)
                    .as_bytes()
                    .iter()
                    .filter(|byte| **byte & 0xc0 != 0x80)
                    .count();
                columns_used / columns + 1
            })
            .sum()
    }

    fn columns(&self, display: &display::Service) -> usize {
        let width = display.dimensions().0.saturating_sub(ORIGIN.0 * 2);
        (width / text::Service::ADVANCE).max(1)
    }

    const fn position(column: usize, columns: usize) -> (usize, usize) {
        (column / columns, column % columns)
    }
}
