use unicode_width::UnicodeWidthChar;

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

impl Theme {
    fn style(self) -> Style {
        Style {
            fg: self.foreground,
            bg: self.background,
            ..Style::default()
        }
    }

    fn indexed_color(self, index: u8) -> u32 {
        if index < 16 {
            self.palette[index as usize]
        } else {
            indexed_color(index)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Style {
    pub fg: u32,
    pub bg: u32,
    pub bold: bool,
    pub underline: bool,
    pub inverse: bool,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            fg: FOREGROUND,
            bg: BACKGROUND,
            bold: false,
            underline: false,
            inverse: false,
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

    fn push_combining(&mut self, ch: char) {
        let suffix = self.combining.as_deref().unwrap_or("");
        if self.base.len_utf8() + suffix.len() < 64 {
            let mut suffix = suffix.to_owned();
            suffix.push(ch);
            self.combining = Some(suffix.into_boxed_str());
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
}

impl Cell {
    fn blank(style: Style) -> Self {
        Self {
            content: Content::Text {
                text: CellText::new(' '),
                wide: false,
            },
            style,
        }
    }
}

#[derive(Default)]
enum Parser {
    #[default]
    Ground,
    Escape,
    Csi,
    String {
        escape: bool,
    },
    Charset,
}

pub struct Terminal {
    theme: Theme,
    pub rows: Vec<Vec<Cell>>,
    pub cursor: (usize, usize),
    pub cursor_visible: bool,
    pub application_cursor: bool,
    style: Style,
    saved: ((usize, usize), Style),
    parser: Parser,
    csi_bytes: [u8; 128],
    csi_len: usize,
    utf8: Vec<u8>,
    pending_wrap: bool,
    auto_wrap: bool,
    scroll_top: usize,
    scroll_bottom: usize,
}

impl Terminal {
    pub fn new(cols: usize, rows: usize) -> Self {
        Self::with_theme(cols, rows, Theme::default())
    }

    pub fn with_theme(cols: usize, rows: usize, theme: Theme) -> Self {
        let (cols, rows) = (cols.max(1), rows.max(1));
        Self {
            theme,
            rows: vec![vec![Cell::blank(theme.style()); cols]; rows],
            cursor: (0, 0),
            cursor_visible: true,
            application_cursor: false,
            style: theme.style(),
            saved: ((0, 0), theme.style()),
            parser: Parser::Ground,
            csi_bytes: [0; 128],
            csi_len: 0,
            utf8: Vec::with_capacity(4),
            pending_wrap: false,
            auto_wrap: true,
            scroll_top: 0,
            scroll_bottom: rows - 1,
        }
    }

    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    pub fn cols(&self) -> usize {
        self.rows[0].len()
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        let (cols, rows) = (cols.max(1), rows.max(1));
        if cols == self.cols() && rows == self.rows.len() {
            return;
        }
        // Keep the cursor's row visible when the window shrinks.
        if self.cursor.1 >= rows {
            let remove = self.cursor.1 + 1 - rows;
            self.rows.drain(..remove);
            self.cursor.1 -= remove;
        }
        let default_style = self.theme.style();
        self.rows
            .resize_with(rows, || vec![Cell::blank(default_style); cols]);
        for row in &mut self.rows {
            row.resize(cols, Cell::blank(default_style));
            Self::repair_row(row);
        }
        self.cursor.0 = self.cursor.0.min(cols - 1);
        self.cursor.1 = self.cursor.1.min(rows - 1);
        self.scroll_top = 0;
        self.scroll_bottom = rows - 1;
        self.pending_wrap = false;
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Vec<u8> {
        let mut replies = Vec::new();
        for &byte in bytes {
            let state = std::mem::take(&mut self.parser);
            match state {
                Parser::Ground => self.ground(byte),
                Parser::Escape => match byte {
                    b'[' => {
                        self.csi_len = 0;
                        self.parser = Parser::Csi;
                    }
                    b']' | b'P' | b'_' | b'^' | b'X' => {
                        self.parser = Parser::String { escape: false };
                    }
                    b'(' | b')' => self.parser = Parser::Charset,
                    b'7' => self.saved = (self.cursor, self.style),
                    b'8' => self.restore_cursor(),
                    b'D' => self.linefeed(),
                    b'E' => {
                        self.cursor.0 = 0;
                        self.linefeed();
                    }
                    b'M' => self.reverse_index(),
                    b'c' => {
                        *self = Self::with_theme(
                            self.cols(),
                            self.rows.len(),
                            self.theme,
                        );
                    }
                    0x1b => self.parser = Parser::Escape,
                    _ => (),
                },
                Parser::Csi => {
                    if (0x40..=0x7e).contains(&byte) {
                        let raw = self.csi_bytes;
                        self.csi(&raw[..self.csi_len], byte, &mut replies);
                    } else if byte == 0x1b {
                        self.parser = Parser::Escape;
                    } else if byte == 0x18 || byte == 0x1a {
                        self.parser = Parser::Ground;
                    } else if byte < 0x20 {
                        self.control(byte);
                        self.parser = Parser::Csi;
                    } else {
                        // Retain the state until the final byte, even on overflow.
                        if self.csi_len < self.csi_bytes.len() {
                            self.csi_bytes[self.csi_len] = byte;
                            self.csi_len += 1;
                        }
                        self.parser = Parser::Csi;
                    }
                }
                Parser::String { escape } => {
                    if byte != 7 && !(escape && byte == b'\\') {
                        self.parser = Parser::String {
                            escape: byte == 0x1b,
                        };
                    }
                }
                Parser::Charset => (),
            }
        }
        replies
    }

    fn ground(&mut self, byte: u8) {
        if byte < 0x20 || byte == 0x7f {
            if !self.utf8.is_empty() {
                self.utf8.clear();
                self.put('\u{fffd}');
            }
            self.control(byte);
            return;
        }
        if self.utf8.is_empty() && byte < 0x80 {
            self.put(byte as char);
            return;
        }
        if !self.utf8.is_empty() && byte & 0xc0 != 0x80 {
            self.utf8.clear();
            self.put('\u{fffd}');
            self.ground(byte);
            return;
        }
        self.utf8.push(byte);
        match std::str::from_utf8(&self.utf8) {
            Ok(text) => {
                let ch = text.chars().next().unwrap();
                self.utf8.clear();
                self.put(ch);
            }
            Err(error) if error.error_len().is_some() => {
                self.utf8.clear();
                self.put('\u{fffd}');
            }
            Err(_) => (),
        }
    }

    fn control(&mut self, byte: u8) {
        match byte {
            0x1b => self.parser = Parser::Escape,
            b'\r' => {
                self.cursor.0 = 0;
                self.pending_wrap = false;
            }
            b'\n' | 0x0b | 0x0c => self.linefeed(),
            8 => {
                self.cursor.0 = self.cursor.0.saturating_sub(1);
                self.pending_wrap = false;
            }
            b'\t' => {
                self.cursor.0 =
                    ((self.cursor.0 / 8 + 1) * 8).min(self.cols() - 1);
                self.pending_wrap = false;
            }
            _ => (),
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
                self.cursor.0 = 0;
                self.linefeed();
            }
            self.pending_wrap = false;
        }
        if width > self.cols() || self.cursor.0 + width > self.cols() {
            return;
        }
        let (x, y) = self.cursor;
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
        };
        if width == 2 {
            self.rows[y][x + 1] = Cell {
                content: Content::Continuation,
                style: self.style,
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
        let region = &mut self.rows[top..=self.scroll_bottom];
        let count = count.min(region.len());
        region.rotate_left(count);
        let start = region.len() - count;
        for row in &mut region[start..] {
            row.fill(Cell::blank(self.style));
        }
    }

    fn scroll_down(&mut self, top: usize, count: usize) {
        let region = &mut self.rows[top..=self.scroll_bottom];
        let count = count.min(region.len());
        region.rotate_right(count);
        for row in &mut region[..count] {
            row.fill(Cell::blank(self.style));
        }
    }

    fn restore_cursor(&mut self) {
        self.cursor = (
            self.saved.0.0.min(self.cols() - 1),
            self.saved.0.1.min(self.rows.len() - 1),
        );
        self.style = self.saved.1;
        self.pending_wrap = false;
    }

    fn csi(&mut self, raw: &[u8], end: u8, replies: &mut Vec<u8>) {
        let private = raw.first() == Some(&b'?');
        let raw = if private { &raw[1..] } else { raw };
        if raw.iter().any(|b| !b.is_ascii_digit() && *b != b';') {
            return;
        }
        // A 128-byte sequence has at most 129 semicolon-separated parameters.
        let mut values = [0usize; 129];
        let mut count = 0;
        for part in raw.split(|b| *b == b';') {
            values[count] = part.iter().fold(0usize, |n, b| {
                n.saturating_mul(10).saturating_add((b - b'0') as usize)
            });
            count += 1;
        }
        let params = &values[..count];
        let first = params[0];
        let n = first.clamp(1, 65535);
        if private {
            for &param in params {
                match (param, end) {
                    (1, b'h' | b'l') => self.application_cursor = end == b'h',
                    (7, b'h' | b'l') => {
                        self.auto_wrap = end == b'h';
                        self.pending_wrap = false;
                    }
                    (25, b'h' | b'l') => self.cursor_visible = end == b'h',
                    _ => (),
                }
            }
            return;
        }
        let (x, y) = self.cursor;
        match end {
            b'A' => self.cursor.1 = y.saturating_sub(n),
            b'B' | b'e' => self.cursor.1 = (y + n).min(self.rows.len() - 1),
            b'C' | b'a' => self.cursor.0 = (x + n).min(self.cols() - 1),
            b'D' => self.cursor.0 = x.saturating_sub(n),
            b'E' => self.cursor = (0, (y + n).min(self.rows.len() - 1)),
            b'F' => self.cursor = (0, y.saturating_sub(n)),
            b'G' | b'`' => self.cursor.0 = (n - 1).min(self.cols() - 1),
            b'd' => self.cursor.1 = (n - 1).min(self.rows.len() - 1),
            b'H' | b'f' => {
                let col = params.get(1).copied().unwrap_or(1).max(1);
                self.cursor = (
                    (col - 1).min(self.cols() - 1),
                    (n - 1).min(self.rows.len() - 1),
                );
            }
            b'J' => self.erase_display(first),
            b'K' => {
                let range = match first {
                    0 => x..self.cols(),
                    1 => 0..x + 1,
                    2 => 0..self.cols(),
                    _ => 0..0,
                };
                for col in range {
                    self.clear_cell(col, y);
                }
            }
            b'X' => {
                for col in x..(x + n).min(self.cols()) {
                    self.clear_cell(col, y);
                }
            }
            b'P' | b'@' => {
                let cols = self.cols();
                let count = n.min(cols - x);
                let row = &mut self.rows[y];
                if end == b'P' {
                    row.drain(x..x + count);
                    row.resize(cols, Cell::blank(self.style));
                } else {
                    row.splice(x..x, vec![Cell::blank(self.style); count]);
                    row.truncate(cols);
                }
                Self::repair_row(row);
            }
            b'L' | b'M'
                if (self.scroll_top..=self.scroll_bottom).contains(&y) =>
            {
                if end == b'L' {
                    self.scroll_down(y, n);
                } else {
                    self.scroll_up(y, n);
                }
            }
            b'S' => self.scroll_up(self.scroll_top, n),
            b'T' => self.scroll_down(self.scroll_top, n),
            b'm' => self.sgr(params),
            b'r' => {
                let bottom = params.get(1).copied().unwrap_or(0);
                let bottom = if bottom == 0 { self.rows.len() } else { bottom };
                if n < bottom && bottom <= self.rows.len() {
                    self.scroll_top = n - 1;
                    self.scroll_bottom = bottom - 1;
                    self.cursor = (0, 0);
                }
            }
            b's' => self.saved = (self.cursor, self.style),
            b'u' => self.restore_cursor(),
            b'c' if first == 0 => replies.extend_from_slice(b"\x1b[?1;0c"),
            b'n' if first == 5 => replies.extend_from_slice(b"\x1b[0n"),
            b'n' if first == 6 => {
                replies.extend_from_slice(
                    format!("\x1b[{};{}R", y + 1, x + 1).as_bytes(),
                );
            }
            _ => (),
        }
        if !matches!(end, b'm' | b'n' | b's' | b'c') {
            self.pending_wrap = false;
        }
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

    fn sgr(&mut self, params: &[usize]) {
        let mut i = 0;
        while i < params.len() {
            match params[i] {
                0 => self.style = self.theme.style(),
                1 => self.style.bold = true,
                4 => self.style.underline = true,
                7 => self.style.inverse = true,
                22 => self.style.bold = false,
                24 => self.style.underline = false,
                27 => self.style.inverse = false,
                30..=37 => self.style.fg = self.theme.palette[params[i] - 30],
                40..=47 => self.style.bg = self.theme.palette[params[i] - 40],
                90..=97 => {
                    self.style.fg = self.theme.palette[params[i] - 90 + 8]
                }
                100..=107 => {
                    self.style.bg = self.theme.palette[params[i] - 100 + 8]
                }
                39 => self.style.fg = self.theme.foreground,
                49 => self.style.bg = self.theme.background,
                38 | 48 => {
                    let fg = params[i] == 38;
                    let color = match params.get(i + 1) {
                        Some(5) if i + 2 < params.len() => {
                            i += 2;
                            Some(
                                self.theme
                                    .indexed_color(params[i].min(255) as u8),
                            )
                        }
                        Some(2) if i + 4 < params.len() => {
                            let rgb = &params[i + 2..i + 5];
                            i += 4;
                            Some(
                                ((rgb[0].min(255) as u32) << 16)
                                    | ((rgb[1].min(255) as u32) << 8)
                                    | rgb[2].min(255) as u32,
                            )
                        }
                        _ => return,
                    };
                    if let Some(color) = color {
                        if fg {
                            self.style.fg = color;
                        } else {
                            self.style.bg = color;
                        }
                    }
                }
                _ => (),
            }
            i += 1;
        }
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
