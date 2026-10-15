use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use unicode_width::UnicodeWidthChar;

mod control;
mod osc;
mod parser;
mod sgr;
mod style;
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
    base: char,
    combining: Option<Box<str>>,
}

impl CellText {
    fn new(base: char) -> Self {
        Self {
            base,
            combining: None,
        }
    }

    pub fn chars(&self) -> impl Iterator<Item = char> + '_ {
        std::iter::once(self.base)
            .chain(self.combining.as_deref().unwrap_or("").chars())
    }

    #[inline(never)]
    fn push_combining(&mut self, ch: char) {
        let suffix = self.combining.as_deref().unwrap_or("");
        if self.base.len_utf8() + suffix.len() < 64 {
            let mut combined =
                String::with_capacity(suffix.len() + ch.len_utf8());
            combined.push_str(suffix);
            combined.push(ch);
            self.combining = Some(combined.into_boxed_str());
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Content {
    Text { text: CellText, wide: bool },
    Continuation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cell {
    pub content: Content,
    pub style: Style,
    pub hyperlink: Option<Arc<str>>,
}

impl Cell {
    fn blank(style: Style) -> Self {
        Self {
            content: Content::Text {
                text: CellText::new(' '),
                wide: false,
            },
            style: style.erased(),
            hyperlink: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    cells: Vec<Cell>,
    pub wrapped: Option<usize>,
}

impl Row {
    fn new(cols: usize, style: Style) -> Self {
        Self {
            cells: vec![Cell::blank(style); cols],
            wrapped: None,
        }
    }

    fn fill(&mut self, cell: Cell) {
        self.cells.fill(cell);
        self.wrapped = None;
    }
}

impl std::ops::Deref for Row {
    type Target = [Cell];

    fn deref(&self) -> &Self::Target {
        &self.cells
    }
}

impl std::ops::DerefMut for Row {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.cells
    }
}

impl<'a> IntoIterator for &'a Row {
    type Item = &'a Cell;
    type IntoIter = std::slice::Iter<'a, Cell>;

    fn into_iter(self) -> Self::IntoIter {
        self.cells.iter()
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
    hyperlink: Option<Arc<str>>,
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
    rows: Vec<Row>,
    cursor: (usize, usize),
    saved: SavedCursor,
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
    theme: Theme,
    initial_theme: Theme,
    palette: [u32; 256],
    pub color_revision: u64,
    pub rows: Vec<Row>,
    pub selection: Option<TextRange>,
    pub hovered_link: Option<TextRange>,
    pub text_blink_phase: u8,
    pub cursor: (usize, usize),
    pub cursor_visible: bool,
    pub cursor_style: CursorStyle,
    pub cursor_blink_on: bool,
    pub application_cursor: bool,
    pub application_keypad: bool,
    pub bracketed_paste: bool,
    pub focus_reporting: bool,
    pub mouse_mode: MouseMode,
    pub mouse_sgr: bool,
    pub title: String,
    pub effects: Vec<Effect>,
    pub reverse_video: bool,
    style: Style,
    saved: SavedCursor,
    parser: Option<Box<vte::Parser<OSC_LIMIT>>>,
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
    hyperlink: Option<Arc<str>>,
    preceding: Option<char>,
    unknown_osc: BTreeMap<Option<u32>, u64>,
    sync_deadline: Option<Instant>,
}

impl Terminal {
    pub fn new(cols: usize, rows: usize) -> Self {
        Self::with_theme(cols, rows, Theme::default())
    }

    pub fn with_theme(cols: usize, rows: usize, theme: Theme) -> Self {
        let (cols, rows) = (cols.max(1), rows.max(1));
        let style = Style::default();
        let rows = vec![Row::new(cols, style); rows];
        let inactive = InactiveScreen {
            rows: rows.clone(),
            cursor: (0, 0),
            saved: SavedCursor::default(),
        };
        let scroll_bottom = rows.len() - 1;
        Self {
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
            selection: None,
            hovered_link: None,
            text_blink_phase: 0,
            cursor: (0, 0),
            cursor_visible: true,
            cursor_style: CursorStyle::default(),
            cursor_blink_on: true,
            application_cursor: false,
            application_keypad: false,
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

    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    pub fn cols(&self) -> usize {
        self.rows[0].len()
    }

    pub fn color(&self, color: Color) -> u32 {
        match color {
            Color::Foreground => self.theme.foreground,
            Color::Background => self.theme.background,
            Color::Indexed(index) => self.palette[index as usize],
            Color::Rgb(rgb) => rgb,
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

    pub fn cursor_drawn(&self) -> bool {
        self.cursor_visible
            && (!self.cursor_style.blinking || self.cursor_blink_on)
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        self.selection = None;
        self.hovered_link = None;
        let (cols, rows) = (cols.max(1), rows.max(1));
        if cols == self.cols() && rows == self.rows.len() {
            return;
        }
        Self::resize_screen(&mut self.rows, &mut self.cursor, cols, rows);
        Self::resize_screen(
            &mut self.inactive.rows,
            &mut self.inactive.cursor,
            cols,
            rows,
        );
        let old_cols = self.tabs.len();
        self.tabs.resize(cols, false);
        for col in old_cols..cols {
            self.tabs[col] = col % 8 == 0;
        }
        self.scroll_top = 0;
        self.scroll_bottom = rows - 1;
        self.pending_wrap = false;
    }

    fn resize_screen(
        screen: &mut Vec<Row>,
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
            row.cells.resize(cols, blank.clone());
            Self::repair_row(row);
        }
        cursor.0 = cursor.0.min(cols - 1);
        cursor.1 = cursor.1.min(rows - 1);
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Vec<u8> {
        if !bytes.is_empty() {
            self.selection = None;
            self.hovered_link = None;
        }
        // Detach the parser while it calls back into the terminal model.
        let mut parser = self.parser.take().expect("parser outside feed");
        let mut bytes = bytes;
        while let Some(index) = memchr::memchr2(0x18, 0x1a, bytes) {
            parser.advance(self, &bytes[..index]);
            self.cancelling = true;
            parser.advance(self, &bytes[index..index + 1]);
            self.cancelling = false;
            bytes = &bytes[index + 1..];
        }
        parser.advance(self, bytes);
        self.parser = Some(parser);
        std::mem::take(&mut self.replies)
    }

    pub fn has_blinking_text(&self) -> bool {
        self.rows
            .iter()
            .flatten()
            .any(|cell| cell.style.blink != Blink::None)
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

    fn put(&mut self, ch: char) {
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
            if let Content::Text { text, .. } = &mut self.rows[y][x].content {
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
                self.rows[y][col].content,
                Content::Text { wide: false, .. }
            ) {
                self.clear_cell(col, y);
            }
        }
        self.rows[y][x] = Cell {
            content: Content::Text {
                text: CellText::new(ch),
                wide: width == 2,
            },
            style: self.style,
            hyperlink: self.hyperlink.clone(),
        };
        if width == 2 {
            self.rows[y][x + 1] = Cell {
                content: Content::Continuation,
                style: self.style,
                hyperlink: self.hyperlink.clone(),
            };
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
        match self.rows[y][x].content {
            Content::Continuation if x > 0 => {
                self.rows[y][x - 1] = Cell::blank(self.style);
            }
            Content::Text { wide: true, .. } if x + 1 < self.cols() => {
                self.rows[y][x + 1] = Cell::blank(self.style);
            }
            _ => (),
        }
        self.rows[y][x] = Cell::blank(self.style);
    }

    fn repair_row(row: &mut [Cell]) {
        for x in 0..row.len() {
            let valid = match row[x].content {
                Content::Continuation => {
                    x > 0
                        && matches!(
                            row[x - 1].content,
                            Content::Text { wide: true, .. }
                        )
                }
                Content::Text { wide: true, .. } => {
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

    fn scroll_up(&mut self, top: usize, count: usize) {
        if top > 0 {
            self.rows[top - 1].wrapped = None;
        }
        let region = &mut self.rows[top..=self.scroll_bottom];
        let count = count.min(region.len());
        region.rotate_left(count);
        let start = region.len() - count;
        if start > 0 {
            region[start - 1].wrapped = None;
        }
        for row in &mut region[start..] {
            row.fill(Cell::blank(self.style));
        }
    }

    fn scroll_down(&mut self, top: usize, count: usize) {
        if top > 0 {
            self.rows[top - 1].wrapped = None;
        }
        let region = &mut self.rows[top..=self.scroll_bottom];
        let count = count.min(region.len());
        region.rotate_right(count);
        region.last_mut().unwrap().wrapped = None;
        for row in &mut region[..count] {
            row.fill(Cell::blank(self.style));
        }
    }

    fn save_cursor(&mut self) {
        self.saved = SavedCursor {
            position: self.cursor,
            style: self.style,
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
        self.style = self.saved.style;
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
        std::mem::swap(&mut self.rows, &mut self.inactive.rows);
        std::mem::swap(&mut self.cursor, &mut self.inactive.cursor);
        std::mem::swap(&mut self.saved, &mut self.inactive.saved);
        self.alternate = alternate;
        self.pending_wrap = false;
        if clear {
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
        row[x..].rotate_right(count);
        row[x..x + count].fill(Cell::blank(self.style));
        Self::repair_row(row);
    }

    fn erase_display(&mut self, mode: usize) {
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
            for col in range {
                self.clear_cell(col, row);
            }
        }
    }

    fn reset(&mut self) {
        let parser = self.parser.take();
        let replies = std::mem::take(&mut self.replies);
        let effects = std::mem::take(&mut self.effects);
        let revision = self.color_revision.wrapping_add(1);
        *self =
            Self::with_theme(self.cols(), self.rows.len(), self.initial_theme);
        self.parser = parser;
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
