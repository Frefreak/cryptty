use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use unicode_width::UnicodeWidthChar;

mod control;
mod osc;
mod parser;
mod reflow;
mod sgr;
mod style;
use style::CellStyle;
#[cfg(test)]
mod tests;
mod text;
pub use style::{
    Blink, Color, CursorShape, CursorStyle, FontStyle, MouseMode, Style,
    Underline,
};
pub use text::{Link, Position, TextRange, supported_uri};

const OSC_LIMIT: usize = 65536;

pub const BACKGROUND: u32 = 0x282a36;
pub const FOREGROUND: u32 = 0xf8f8f2;
pub const PALETTE: [u32; 16] = [
    0x21222c, 0xff5555, 0x50fa7b, 0xf1fa8c, 0xbd93f9, 0xff79c6, 0x8be9fd,
    0xf8f8f2, 0x6272a4, 0xff6e6e, 0x69ff94, 0xffffa5, 0xd6acff, 0xff92df,
    0xa4ffff, 0xffffff,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    pub foreground: u32,
    pub background: u32,
    pub cursor: u32,
    pub palette: [u32; 16],
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            foreground: FOREGROUND,
            background: BACKGROUND,
            cursor: FOREGROUND,
            palette: PALETTE,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CellText {
    base: Character,
    // Keep optional string metadata out of every cell.
    #[allow(clippy::box_collection)]
    combining: Option<Box<String>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Character {
    Narrow(char),
    Wide(char),
}

impl CellText {
    fn new(base: char, wide: bool) -> Self {
        Self {
            base: if wide {
                Character::Wide(base)
            } else {
                Character::Narrow(base)
            },
            combining: None,
        }
    }

    fn base(&self) -> char {
        match self.base {
            Character::Narrow(ch) | Character::Wide(ch) => ch,
        }
    }

    pub fn columns(&self) -> usize {
        match self.base {
            Character::Narrow(_) => 1,
            Character::Wide(_) => 2,
        }
    }

    pub fn chars(&self) -> impl Iterator<Item = char> + '_ {
        std::iter::once(self.base())
            .chain(self.combining.as_deref().map_or("", String::as_str).chars())
    }

    #[inline(never)]
    fn push_combining(&mut self, ch: char) {
        let len = self.combining.as_ref().map_or(0, |text| text.len());
        if self.base().len_utf8() + len < 64 {
            self.combining
                .get_or_insert_with(|| Box::new(String::new()))
                .push(ch);
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Content {
    Text { text: CellText },
    Continuation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cell {
    pub content: Content,
    style: CellStyle,
    pub hyperlink: Option<Arc<String>>,
}

impl Cell {
    pub fn style(&self) -> Style {
        self.style.into()
    }

    fn blank(style: impl Into<CellStyle>) -> Self {
        Self {
            content: Content::Text {
                text: CellText::new(' ', false),
            },
            style: style.into().erased(),
            hyperlink: None,
        }
    }
}

#[derive(Debug)]
pub struct Row {
    cells: Vec<Cell>,
    // Only this prefix is visible; the suffix reads as `blank`.
    occupied: usize,
    blank: Cell,
    pub wrapped: Option<usize>,
}

impl Row {
    fn new(cols: usize, style: impl Into<CellStyle>) -> Self {
        let blank = Cell::blank(style);
        Self {
            cells: vec![blank.clone(); cols],
            occupied: 0,
            blank,
            wrapped: None,
        }
    }

    fn clear(&mut self, style: impl Into<CellStyle>) {
        self.occupied = 0;
        self.blank = Cell::blank(style);
        self.wrapped = None;
    }

    fn erase(
        &mut self,
        mut range: std::ops::Range<usize>,
        style: impl Into<CellStyle>,
    ) {
        let style = style.into();
        if range.is_empty() {
            return;
        }
        if range == (0..self.len()) {
            self.clear(style);
            return;
        }
        let clears_wrap = range.end == self.len();
        if range.start > 0
            && matches!(self[range.start].content, Content::Continuation)
        {
            range.start -= 1;
        }
        if matches!(
            &self[range.end - 1].content,
            Content::Text { text } if text.columns() == 2
        ) {
            range.end = (range.end + 1).min(self.len());
        }
        if range.end == self.len() {
            self.prepare(range.start);
            self.occupied = range.start;
            self.blank = Cell::blank(style);
        } else {
            self.prepare(range.end);
            self.occupied = self.occupied.max(range.end);
            self.cells[range].fill(Cell::blank(style));
        }
        if clears_wrap {
            self.wrapped = None;
        }
    }

    #[inline(always)]
    fn prepare(&mut self, col: usize) {
        if col > self.occupied {
            self.cells[self.occupied..col].fill(self.blank.clone());
        }
    }

    #[inline(always)]
    fn set(&mut self, col: usize, cell: Cell) {
        self.prepare(col);
        self.cells[col] = cell;
        self.occupied = self.occupied.max(col + 1);
    }

    #[inline(always)]
    fn write_text(
        &mut self,
        col: usize,
        ch: char,
        wide: bool,
        style: &CellStyle,
        hyperlink: &Option<Arc<String>>,
    ) {
        self.prepare(col);
        let cell = &mut self.cells[col];
        cell.content = Content::Text {
            text: CellText::new(ch, wide),
        };
        cell.style = *style;
        cell.hyperlink.clone_from(hyperlink);
        self.occupied = self.occupied.max(col + 1);
    }

    #[inline(always)]
    fn write_continuation(
        &mut self,
        col: usize,
        style: &CellStyle,
        hyperlink: &Option<Arc<String>>,
    ) {
        self.prepare(col);
        let cell = &mut self.cells[col];
        cell.content = Content::Continuation;
        cell.style = *style;
        cell.hyperlink.clone_from(hyperlink);
        self.occupied = self.occupied.max(col + 1);
    }

    fn cell_mut(&mut self, col: usize) -> &mut Cell {
        if col >= self.occupied {
            self.prepare(col + 1);
            self.occupied = col + 1;
        }
        &mut self.cells[col]
    }

    fn materialized_mut(&mut self) -> &mut [Cell] {
        self.prepare(self.len());
        self.occupied = self.len();
        &mut self.cells
    }

    fn fill(&mut self, cell: Cell) {
        self.cells.fill(cell);
        self.occupied = self.len();
        self.wrapped = None;
    }

    pub fn len(&self) -> usize {
        self.cells.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    pub fn get(&self, col: usize) -> Option<&Cell> {
        (col < self.len()).then(|| &self[col])
    }

    /// Identifies the allocation, independently of its logical contents.
    pub fn storage_id(&self) -> usize {
        self.cells.as_ptr() as usize
    }

    pub fn iter(&self) -> RowIter<'_> {
        RowIter {
            row: self,
            indices: 0..self.len(),
        }
    }
}

impl Clone for Row {
    fn clone(&self) -> Self {
        let mut cells = self.cells[..self.occupied].to_vec();
        cells.resize(self.len(), self.blank.clone());
        Self {
            cells,
            occupied: self.occupied,
            blank: self.blank.clone(),
            wrapped: self.wrapped,
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.cells.truncate(source.occupied);
        for (target, source) in self.cells.iter_mut().zip(&source.cells) {
            target.clone_from(source);
        }
        self.cells.extend_from_slice(
            &source.cells[self.cells.len()..source.occupied],
        );
        self.cells.resize(source.len(), source.blank.clone());
        self.occupied = source.occupied;
        self.blank.clone_from(&source.blank);
        self.wrapped = source.wrapped;
    }
}

impl PartialEq for Row {
    fn eq(&self, other: &Self) -> bool {
        self.wrapped == other.wrapped && self.iter().eq(other.iter())
    }
}

impl Eq for Row {}

impl std::ops::Index<usize> for Row {
    type Output = Cell;

    fn index(&self, col: usize) -> &Cell {
        assert!(col < self.len());
        if col < self.occupied {
            &self.cells[col]
        } else {
            &self.blank
        }
    }
}

impl std::ops::IndexMut<usize> for Row {
    fn index_mut(&mut self, col: usize) -> &mut Cell {
        self.cell_mut(col)
    }
}

pub struct RowIter<'a> {
    row: &'a Row,
    indices: std::ops::Range<usize>,
}

impl<'a> Iterator for RowIter<'a> {
    type Item = &'a Cell;

    fn next(&mut self) -> Option<Self::Item> {
        self.indices.next().map(|col| &self.row[col])
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.indices.size_hint()
    }
}

impl DoubleEndedIterator for RowIter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.indices.next_back().map(|col| &self.row[col])
    }
}

impl ExactSizeIterator for RowIter<'_> {}

impl<'a> IntoIterator for &'a Row {
    type Item = &'a Cell;
    type IntoIter = RowIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

#[derive(Clone)]
struct SavedCursor {
    position: (usize, usize),
    style: Style,
    origin: bool,
    auto_wrap: bool,
    pending_wrap: bool,
    charsets: [bool; 2],
    active_charset: usize,
    hyperlink: Option<Arc<String>>,
}

impl Default for SavedCursor {
    fn default() -> Self {
        Self {
            position: (0, 0),
            style: Style::default(),
            origin: false,
            auto_wrap: true,
            pending_wrap: false,
            charsets: [false; 2],
            active_charset: 0,
            hyperlink: None,
        }
    }
}

struct InactiveScreen {
    graphics: crate::graphics::Graphics,
    rows: VecDeque<Row>,
    cursor: (usize, usize),
    saved: SavedCursor,
    pending_wrap: bool,
    keyboard: crate::keyboard::ModeStack,
}

pub enum Effect {
    Clipboard(String),
    Bell,
    Notification(String),
    UnknownOsc {
        command: Option<u32>,
        bytes: usize,
        count: u64,
    },
}

pub struct Terminal {
    pub graphics: crate::graphics::Graphics,
    image_stream: crate::graphics::stream::Stream,
    pub cell_pixels: (u32, u32),
    pub word_characters: String,
    theme: Theme,
    initial_theme: Theme,
    palette: [u32; 256],
    pub color_revision: u64,
    pub rows: VecDeque<Row>,
    history: VecDeque<Row>,
    pub selection: Option<TextRange>,
    pub hovered_link: Vec<TextRange>,
    pub text_blink_phase: u8,
    pub cursor: (usize, usize),
    pub cursor_visible: bool,
    pub vim_mode: bool,
    pub cursor_style: CursorStyle,
    pub cursor_blink_on: bool,
    pub application_cursor: bool,
    pub application_keypad: bool,
    pub keyboard: crate::keyboard::ModeStack,
    pub modify_other_keys_2: bool,
    pub bracketed_paste: bool,
    pub focus_reporting: bool,
    pub mouse_mode: MouseMode,
    pub mouse_sgr: bool,
    pub title: String,
    pub effects: Vec<Effect>,
    pub reverse_video: bool,
    style: CellStyle,
    saved: SavedCursor,
    parser: Option<Box<parser::Parser>>,
    cancelling: bool,
    replies: Vec<u8>,
    dcs: Option<Vec<u8>>,
    pending_wrap: bool,
    auto_wrap: bool,
    origin_mode: bool,
    insert_mode: bool,
    newline_mode: bool,
    charsets: [bool; 2],
    active_charset: usize,
    tabs: Vec<bool>,
    scroll_top: usize,
    scroll_bottom: usize,
    inactive: InactiveScreen,
    alternate: bool,
    hyperlink: Option<Arc<String>>,
    preceding: Option<char>,
    unknown_osc: BTreeMap<Option<u32>, u64>,
    sync_deadline: Option<Instant>,
}

impl Terminal {
    pub fn new(cols: usize, rows: usize) -> Self {
        Self::with_theme(cols, rows, Theme::default())
    }

    pub fn with_theme(cols: usize, rows: usize, theme: Theme) -> Self {
        let (cols, rows) = (cols.max(2), rows.max(1));
        let style = CellStyle::from(Style::default());
        let rows: VecDeque<_> =
            (0..rows).map(|_| Row::new(cols, style)).collect();
        let inactive = InactiveScreen {
            graphics: Default::default(),
            rows: rows.clone(),
            cursor: (0, 0),
            saved: SavedCursor::default(),
            pending_wrap: false,
            keyboard: crate::keyboard::ModeStack::default(),
        };
        let scroll_bottom = rows.len() - 1;
        Self {
            graphics: Default::default(),
            image_stream: Default::default(),
            cell_pixels: (8, 16),
            word_characters: crate::config::Selection::default()
                .word_characters,
            theme,
            initial_theme: theme,
            palette: std::array::from_fn(|i| {
                if i < 16 {
                    theme.palette[i]
                } else {
                    indexed_color(i as u8)
                }
            }),
            color_revision: 0,
            rows,
            history: VecDeque::new(),
            selection: None,
            hovered_link: Vec::new(),
            text_blink_phase: 0,
            cursor: (0, 0),
            cursor_visible: true,
            vim_mode: false,
            cursor_style: CursorStyle::default(),
            cursor_blink_on: true,
            application_cursor: false,
            application_keypad: false,
            keyboard: crate::keyboard::ModeStack::default(),
            modify_other_keys_2: false,
            bracketed_paste: false,
            focus_reporting: false,
            mouse_mode: MouseMode::Off,
            mouse_sgr: false,
            title: "Cryptty".into(),
            effects: Vec::new(),
            reverse_video: false,
            style,
            saved: SavedCursor::default(),
            parser: Some(Box::default()),
            cancelling: false,
            replies: Vec::new(),
            dcs: None,
            pending_wrap: false,
            auto_wrap: true,
            origin_mode: false,
            insert_mode: false,
            newline_mode: false,
            charsets: [false; 2],
            active_charset: 0,
            tabs: (0..cols).map(|i| i % 8 == 0).collect(),
            scroll_top: 0,
            scroll_bottom,
            inactive,
            alternate: false,
            hyperlink: None,
            preceding: None,
            unknown_osc: BTreeMap::new(),
            sync_deadline: None,
        }
    }

    /// A renderable copy of the visible screen, without parser or history state.
    pub fn visible_snapshot(&self) -> Self {
        let mut snapshot =
            Self::with_theme(self.cols(), self.rows.len(), self.theme);
        snapshot.rows = self.rows.clone();
        snapshot.palette = self.palette;
        snapshot.color_revision = self.color_revision;
        snapshot.graphics = self.graphics.snapshot();
        snapshot.cell_pixels = self.cell_pixels;
        snapshot.word_characters = self.word_characters.clone();
        snapshot.cursor = self.cursor;
        snapshot.cursor_style = self.cursor_style;
        snapshot.cursor_visible = self.cursor_visible;
        snapshot.reverse_video = self.reverse_video;
        snapshot.title = self.title.clone();
        snapshot
    }

    pub(crate) fn overlay(
        &mut self,
        position: Position,
        label: Option<char>,
        style: Style,
    ) {
        let row = &mut self.rows[position.row];
        let mut col = position.col;
        if matches!(row[col].content, Content::Continuation) {
            col = col.saturating_sub(1);
        }
        let width = match &row[col].content {
            Content::Text { text } => text.columns(),
            _ => 1,
        };
        for x in col..(col + width).min(row.len()) {
            row[x].style = style.into();
            if let Some(ch) = label {
                row[x].content = Content::Text {
                    text: CellText::new(if x == col { ch } else { ' ' }, false),
                };
            }
        }
    }

    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    pub fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
        self.initial_theme = theme;
        self.palette = std::array::from_fn(|i| {
            if i < 16 {
                theme.palette[i]
            } else {
                indexed_color(i as u8)
            }
        });
        self.color_revision = self.color_revision.wrapping_add(1);
    }

    pub fn cols(&self) -> usize {
        self.tabs.len()
    }

    pub fn color(&self, color: Color) -> u32 {
        match color {
            Color::Foreground => self.theme.foreground,
            Color::Background => self.theme.background,
            Color::Indexed(index) => self.palette[index as usize],
            Color::Rgb(r, g, b) => {
                (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)
            }
        }
    }

    pub fn colors(&self, style: Style) -> (u32, u32) {
        let (mut fg, mut bg) = (self.color(style.fg), self.color(style.bg));
        if style.inverse ^ self.reverse_video {
            std::mem::swap(&mut fg, &mut bg);
        }
        if style.dim {
            fg = style::dim(fg);
        }
        (fg, bg)
    }

    pub fn alternate_screen(&self) -> bool {
        self.alternate
    }

    pub fn synchronized_until(&self) -> Option<Instant> {
        self.sync_deadline
    }

    pub fn finish_synchronized_update(&mut self) {
        self.sync_deadline = None;
    }

    pub fn reset_cursor_blink(&mut self) -> bool {
        let was_drawn = self.cursor_drawn();
        self.cursor_blink_on = true;
        was_drawn != self.cursor_drawn()
    }

    pub fn cursor_drawn(&self) -> bool {
        self.cursor_visible
            && (!self.cursor_style.blinking || self.cursor_blink_on)
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        self.selection = None;
        self.hovered_link.clear();
        let (cols, rows) = (cols.max(2), rows.max(1));
        if cols == self.cols() && rows == self.rows.len() {
            return;
        }
        if self.alternate {
            Self::resize_screen(&mut self.rows, &mut self.cursor, cols, rows);
            self.pending_wrap = false;
            self.saved.position.0 = self.saved.position.0.min(cols - 1);
            self.saved.position.1 = self.saved.position.1.min(rows - 1);
            self.saved.pending_wrap = false;
            reflow::resize_primary(
                &mut self.inactive.rows,
                &mut self.history,
                &mut self.inactive.cursor,
                &mut self.inactive.pending_wrap,
                &mut self.inactive.saved,
                &mut self.inactive.graphics,
                cols,
                rows,
            );
        } else {
            reflow::resize_primary(
                &mut self.rows,
                &mut self.history,
                &mut self.cursor,
                &mut self.pending_wrap,
                &mut self.saved,
                &mut self.graphics,
                cols,
                rows,
            );
            Self::resize_screen(
                &mut self.inactive.rows,
                &mut self.inactive.cursor,
                cols,
                rows,
            );
            self.inactive.pending_wrap = false;
            self.inactive.saved.position.0 =
                self.inactive.saved.position.0.min(cols - 1);
            self.inactive.saved.position.1 =
                self.inactive.saved.position.1.min(rows - 1);
            self.inactive.saved.pending_wrap = false;
        }
        let old_cols = self.tabs.len();
        self.tabs.resize(cols, false);
        for col in old_cols..cols {
            self.tabs[col] = col % 8 == 0;
        }
        self.scroll_top = 0;
        self.scroll_bottom = rows - 1;
    }

    fn resize_screen(
        screen: &mut VecDeque<Row>,
        cursor: &mut (usize, usize),
        cols: usize,
        rows: usize,
    ) {
        if cursor.1 >= rows {
            let remove = cursor.1 + 1 - rows;
            screen.drain(..remove);
            cursor.1 -= remove;
        }
        let blank = Cell::blank(Style::default());
        if screen.len() > rows {
            screen[rows - 1].wrapped = None;
        }
        screen.resize_with(rows, || Row::new(cols, Style::default()));
        for row in screen {
            if row.len() != cols {
                row.wrapped = None;
            }
            row.materialized_mut();
            row.cells.resize(cols, blank.clone());
            row.occupied = cols;
            Self::repair_row(row.materialized_mut());
        }
        cursor.0 = cursor.0.min(cols - 1);
        cursor.1 = cursor.1.min(rows - 1);
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Vec<u8> {
        let selected = self.selection.map(|range| self.selected_text(range));
        if !bytes.is_empty() {
            self.hovered_link.clear();
        }
        // Detach the parser while it calls back into the terminal model.
        let mut parser = self.parser.take().expect("parser outside feed");
        let mut stream = std::mem::take(&mut self.image_stream);
        let mut performer = parser::Buffered::new(self);
        stream.feed(bytes, |event| match event {
            crate::graphics::stream::Event::Text(mut bytes) => {
                while let Some(index) = memchr::memchr2(0x18, 0x1a, bytes) {
                    parser.advance(&mut performer, &bytes[..index]);
                    performer.flush();
                    performer.terminal.cancelling = true;
                    parser.advance(&mut performer, &bytes[index..index + 1]);
                    performer.terminal.cancelling = false;
                    bytes = &bytes[index + 1..];
                }
                parser.advance(&mut performer, bytes);
            }
            crate::graphics::stream::Event::Graphics(command) => {
                performer.flush();
                performer.terminal.graphics_command(command);
            }
        });
        performer.flush();
        self.image_stream = stream;
        self.parser = Some(parser);
        if let (Some(text), Some(range)) = (selected, self.selection)
            && self.selected_text(range) != text
        {
            self.selection = None;
        }
        std::mem::take(&mut self.replies)
    }

    fn graphics_command(&mut self, command: &[u8]) {
        let screen = crate::graphics::Screen {
            cursor: self.cursor,
            cell: self.cell_pixels,
            size: (self.cols(), self.rows.len()),
        };
        let (reply, movement) = self.graphics.command(command, screen);
        self.replies.extend(reply);
        if let Some((cols, rows)) = movement {
            for _ in 1..rows.min(self.rows.len()) {
                self.linefeed();
            }
            let col = self.cursor.0.saturating_add(cols);
            if col >= self.cols() {
                self.cursor.0 = 0;
                self.linefeed();
            } else {
                self.cursor.0 = col;
            }
            self.pending_wrap = false;
        }
    }

    pub fn has_blinking_text(&self) -> bool {
        self.rows
            .iter()
            .flatten()
            .any(|cell| cell.style().blink != Blink::None)
    }

    pub fn text_visible(&self, style: Style) -> bool {
        !style.hidden && style.blink.visible(self.text_blink_phase)
    }

    fn bell(&mut self) {
        if self.effects.len() < 64
            && !self
                .effects
                .iter()
                .any(|effect| matches!(effect, Effect::Bell))
        {
            self.effects.push(Effect::Bell);
        }
    }

    fn unknown_osc(&mut self, command: Option<u32>, bytes: usize) {
        if self.unknown_osc.len() >= 32
            && !self.unknown_osc.contains_key(&command)
        {
            return;
        }
        let count = self.unknown_osc.entry(command).or_default();
        *count = count.saturating_add(1);
        if count.is_power_of_two() && self.effects.len() < 64 {
            self.effects.push(Effect::UnknownOsc {
                command,
                bytes,
                count: *count,
            });
        }
    }

    fn print_ascii(&mut self, mut bytes: &[u8]) {
        while let Some(&first) = bytes.first() {
            let (x, y) = self.cursor;
            if self.pending_wrap
                || self.insert_mode
                || self.charsets[self.active_charset]
                || x + 1 == self.cols()
                || !matches!(
                    &self.rows[y][x].content,
                    Content::Text { text } if text.columns() == 1
                )
            {
                vte::Perform::print(self, char::from(first));
                bytes = &bytes[1..];
                continue;
            }
            let end = (x + bytes.len()).min(self.cols() - 1);
            let row = &mut self.rows[y];
            let end = if x >= row.occupied {
                end
            } else {
                row.cells[x..end.min(row.occupied)]
                    .iter()
                    .position(|cell| {
                        !matches!(
                            &cell.content,
                            Content::Text { text } if text.columns() == 1
                        )
                    })
                    .map_or(end, |offset| x + offset)
            };
            row.prepare(x);
            let len = end - x;
            for (cell, &byte) in row.cells[x..end].iter_mut().zip(&bytes[..len])
            {
                cell.content = Content::Text {
                    text: CellText::new(char::from(byte), false),
                };
                cell.style = self.style;
                cell.hyperlink.clone_from(&self.hyperlink);
            }
            row.occupied = row.occupied.max(end);
            self.cursor.0 = end;
            self.preceding = Some(char::from(bytes[len - 1]));
            bytes = &bytes[len..];
        }
    }

    #[inline(always)]
    fn put(&mut self, ch: char) {
        let (x, y) = self.cursor;
        let width = ch.width().unwrap_or(0);
        if width > 0
            && !self.pending_wrap
            && !self.insert_mode
            && x + width < self.cols()
            && x >= self.rows[y].occupied
        {
            let row = &mut self.rows[y];
            row.write_text(x, ch, width == 2, &self.style, &self.hyperlink);
            if width == 2 {
                row.write_continuation(x + 1, &self.style, &self.hyperlink);
            }
            self.cursor.0 += width;
            return;
        }

        if (' '..='~').contains(&ch)
            && !self.pending_wrap
            && !self.insert_mode
            && x + 1 < self.cols()
            && matches!(
                &self.rows[y][x].content,
                Content::Text { text } if text.columns() == 1
            )
        {
            self.rows[y].write_text(x, ch, false, &self.style, &self.hyperlink);
            self.cursor.0 += 1;
            return;
        }
        self.put_slow(ch);
    }

    fn put_slow(&mut self, ch: char) {
        let width = ch.width().unwrap_or(0);
        if width == 0 {
            let (mut x, y) = self.cursor;
            if !self.pending_wrap {
                if x == 0 {
                    return;
                }
                x -= 1;
            }
            if matches!(self.rows[y][x].content, Content::Continuation) {
                x = x.saturating_sub(1);
            }
            if let Content::Text { text, .. } =
                &mut self.rows[y].cell_mut(x).content
            {
                text.push_combining(ch);
            }
            return;
        }
        if self.pending_wrap || (width == 2 && self.cursor.0 + 1 == self.cols())
        {
            if self.auto_wrap {
                let end = if self.pending_wrap {
                    self.cols()
                } else {
                    self.cursor.0
                };
                let row = self.cursor.1;
                let wrapped_row = if row == self.scroll_bottom {
                    (row > self.scroll_top).then(|| row - 1)
                } else {
                    Some(row)
                };
                self.cursor.0 = 0;
                self.linefeed();
                if let Some(row) = wrapped_row {
                    self.rows[row].wrapped = Some(end);
                } else if !self.alternate
                    && row == 0
                    && let Some(row) = self.history.back_mut()
                {
                    row.wrapped = Some(end);
                }
            }
            self.pending_wrap = false;
        }
        if width > self.cols() || self.cursor.0 + width > self.cols() {
            return;
        }
        let (x, y) = self.cursor;
        if self.insert_mode {
            self.insert_characters(width);
        }
        for col in x..x + width {
            if !matches!(
                &self.rows[y][col].content,
                Content::Text { text } if text.columns() == 1
            ) {
                self.clear_cell(col, y);
            }
        }
        let row = &mut self.rows[y];
        row.write_text(x, ch, width == 2, &self.style, &self.hyperlink);
        if width == 2 {
            row.write_continuation(x + 1, &self.style, &self.hyperlink);
        }
        if x + width >= self.cols() {
            self.cursor.0 = self.cols() - 1;
            self.pending_wrap = self.auto_wrap;
        } else {
            self.cursor.0 += width;
        }
    }

    fn clear_cell(&mut self, x: usize, y: usize) {
        if x + 1 == self.cols() {
            self.rows[y].wrapped = None;
        }
        match &self.rows[y][x].content {
            Content::Continuation if x > 0 => {
                self.rows[y].set(x - 1, Cell::blank(self.style));
            }
            Content::Text { text }
                if text.columns() == 2 && x + 1 < self.cols() =>
            {
                self.rows[y].set(x + 1, Cell::blank(self.style));
            }
            _ => (),
        }
        self.rows[y].set(x, Cell::blank(self.style));
    }

    fn repair_row(row: &mut [Cell]) {
        for x in 0..row.len() {
            let valid = match &row[x].content {
                Content::Continuation => {
                    x > 0
                        && matches!(
                            &row[x - 1].content,
                            Content::Text { text } if text.columns() == 2
                        )
                }
                Content::Text { text } if text.columns() == 2 => {
                    x + 1 < row.len()
                        && matches!(row[x + 1].content, Content::Continuation)
                }
                _ => true,
            };
            if !valid {
                row[x] = Cell::blank(row[x].style);
            }
        }
    }

    fn linefeed(&mut self) {
        self.pending_wrap = false;
        if self.cursor.1 == self.scroll_bottom {
            self.scroll_up(self.scroll_top, 1);
        } else {
            self.cursor.1 = (self.cursor.1 + 1).min(self.rows.len() - 1);
        }
    }

    fn reverse_index(&mut self) {
        self.pending_wrap = false;
        if self.cursor.1 == self.scroll_top {
            self.scroll_down(self.scroll_top, 1);
        } else {
            self.cursor.1 = self.cursor.1.saturating_sub(1);
        }
    }

    fn rotate_rows(&mut self, top: usize, count: usize, up: bool) {
        if top == 0 && self.scroll_bottom + 1 == self.rows.len() {
            if up {
                self.rows.rotate_left(count);
            } else {
                self.rows.rotate_right(count);
            }
        } else {
            let region =
                &mut self.rows.make_contiguous()[top..=self.scroll_bottom];
            if up {
                region.rotate_left(count);
            } else {
                region.rotate_right(count);
            }
        }
    }

    fn scroll_up(&mut self, top: usize, count: usize) {
        if top > 0 {
            self.rows[top - 1].wrapped = None;
        }
        let count = count.min(self.scroll_bottom + 1 - top);
        self.graphics.scroll(
            top,
            self.scroll_bottom,
            -(count as isize),
            self.cell_pixels,
            top == 0
                && self.scroll_bottom + 1 == self.rows.len()
                && !self.alternate,
        );
        self.scroll_selection(top, count, true);
        self.rotate_rows(top, count, true);
        let start = self.scroll_bottom + 1 - count;
        if start > top {
            self.rows[start - 1].wrapped = None;
        }
        for row in self.rows.range_mut(start..=self.scroll_bottom) {
            if top == 0 && !self.alternate {
                let blank = if self.history.len() == reflow::HISTORY_LIMIT {
                    let mut oldest = self.history.pop_front().unwrap();
                    oldest.clear(self.style);
                    oldest
                } else {
                    Row::new(row.len(), self.style)
                };
                self.history.push_back(std::mem::replace(row, blank));
            } else {
                row.fill(Cell::blank(self.style));
            }
        }
    }

    fn scroll_down(&mut self, top: usize, count: usize) {
        if top == 0
            && !self.alternate
            && let Some(row) = self.history.back_mut()
        {
            row.wrapped = None;
        }
        if top > 0 {
            self.rows[top - 1].wrapped = None;
        }
        let count = count.min(self.scroll_bottom + 1 - top);
        self.graphics.scroll(
            top,
            self.scroll_bottom,
            count as isize,
            self.cell_pixels,
            false,
        );
        self.scroll_selection(top, count, false);
        self.rotate_rows(top, count, false);
        self.rows[self.scroll_bottom].wrapped = None;
        for row in self.rows.range_mut(top..top + count) {
            row.fill(Cell::blank(self.style));
        }
    }

    fn save_cursor(&mut self) {
        self.saved = SavedCursor {
            position: self.cursor,
            style: self.style.into(),
            origin: self.origin_mode,
            auto_wrap: self.auto_wrap,
            pending_wrap: self.pending_wrap,
            charsets: self.charsets,
            active_charset: self.active_charset,
            hyperlink: self.hyperlink.clone(),
        };
    }

    fn restore_cursor(&mut self) {
        self.cursor = (
            self.saved.position.0.min(self.cols() - 1),
            self.saved.position.1.min(self.rows.len() - 1),
        );
        self.style = self.saved.style.into();
        self.origin_mode = self.saved.origin;
        self.auto_wrap = self.saved.auto_wrap;
        self.pending_wrap = self.saved.pending_wrap;
        self.charsets = self.saved.charsets;
        self.active_charset = self.saved.active_charset;
        self.hyperlink = self.saved.hyperlink.clone();
    }

    fn switch_screen(&mut self, alternate: bool, clear: bool) {
        if self.alternate == alternate {
            return;
        }
        self.selection = None;
        std::mem::swap(&mut self.rows, &mut self.inactive.rows);
        std::mem::swap(&mut self.graphics, &mut self.inactive.graphics);
        self.graphics.revision = self
            .graphics
            .revision
            .max(self.inactive.graphics.revision)
            .wrapping_add(1);
        std::mem::swap(&mut self.keyboard, &mut self.inactive.keyboard);
        std::mem::swap(&mut self.cursor, &mut self.inactive.cursor);
        std::mem::swap(&mut self.saved, &mut self.inactive.saved);
        std::mem::swap(&mut self.pending_wrap, &mut self.inactive.pending_wrap);
        self.alternate = alternate;
        if clear {
            self.graphics
                .clear_visible(self.rows.len(), self.cell_pixels);
            self.pending_wrap = false;
            for row in &mut self.rows {
                row.fill(Cell::blank(Style::default()));
            }
        }
    }

    fn insert_characters(&mut self, count: usize) {
        let (x, y) = self.cursor;
        let count = count.min(self.cols() - x);
        let row = &mut self.rows[y];
        row.wrapped = None;
        row.materialized_mut()[x..].rotate_right(count);
        row.materialized_mut()[x..x + count].fill(Cell::blank(self.style));
        Self::repair_row(row.materialized_mut());
    }

    fn erase_display(&mut self, mode: usize) {
        if mode == 3 {
            self.history.clear();
            self.graphics.clear_history(self.cell_pixels);
            return;
        }
        if mode == 2 {
            self.graphics
                .clear_visible(self.rows.len(), self.cell_pixels);
        }
        if mode == 2
            && !self.alternate
            && let Some(row) = self.history.back_mut()
        {
            row.wrapped = None;
        }
        let (x, y) = self.cursor;
        for row in 0..self.rows.len() {
            let range = match mode {
                0 if row == y => x..self.cols(),
                0 if row > y => 0..self.cols(),
                1 if row == y => 0..x + 1,
                1 if row < y => 0..self.cols(),
                2 => 0..self.cols(),
                _ => 0..0,
            };
            self.rows[row].erase(range, self.style);
        }
    }

    fn reset(&mut self) {
        let word_characters = std::mem::take(&mut self.word_characters);
        let cell_pixels = self.cell_pixels;
        let parser = self.parser.take();
        let replies = std::mem::take(&mut self.replies);
        let effects = std::mem::take(&mut self.effects);
        let revision = self.color_revision.wrapping_add(1);
        *self =
            Self::with_theme(self.cols(), self.rows.len(), self.initial_theme);
        self.parser = parser;
        self.word_characters = word_characters;
        self.cell_pixels = cell_pixels;
        self.replies = replies;
        self.effects = effects;
        self.color_revision = revision;
    }
}

pub fn indexed_color(index: u8) -> u32 {
    match index {
        0..=15 => PALETTE[index as usize],
        16..=231 => {
            let n = (index - 16) as u32;
            let level = |v| if v == 0 { 0 } else { 55 + v * 40 };
            (level(n / 36) << 16) | (level(n / 6 % 6) << 8) | level(n % 6)
        }
        _ => {
            let gray = 8 + (index as u32 - 232) * 10;
            gray * 0x010101
        }
    }
}
