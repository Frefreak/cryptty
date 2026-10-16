use vte::Params;

use super::{Cell, CursorStyle, Duration, Instant, MouseMode, Style, Terminal};

impl Terminal {
    pub(super) fn csi(&mut self, params: &Params, prefix: &[u8], end: char) {
        if prefix.is_empty() && end == 'm' {
            self.sgr(params);
            return;
        }
        if params.iter().any(|p| p.len() != 1) {
            return;
        }
        let mut values = [0usize; 32];
        let mut count = 0;
        for p in params {
            values[count] = p[0] as usize;
            count += 1;
        }
        let params = &values[..count.max(1)];
        let first = params[0];
        let n = first.max(1);
        match (prefix, end) {
            ([b'>'], 'm') if first == 0 && params.len() == 1 => {
                self.modify_other_keys_2 = false;
                return;
            }
            ([b'>'], 'm') if first == 4 => {
                match params.get(1).copied().unwrap_or(0) {
                    0 | 1 => self.modify_other_keys_2 = false,
                    2 => self.modify_other_keys_2 = true,
                    _ => (),
                }
                return;
            }
            ([b'>'], 'n') if first == 4 => {
                self.modify_other_keys_2 = false;
                return;
            }
            ([b'?'], 'm') if first == 4 => {
                let mode = if self.modify_other_keys_2 { 2 } else { 0 };
                self.reply(format!("\x1b[>4;{mode}m"));
                return;
            }
            ([b'?'], 'u') => {
                self.reply(format!("\x1b[?{}u", self.keyboard.flags().bits()));
                return;
            }
            ([b'>'], 'u') => {
                self.keyboard.push(first);
                return;
            }
            ([b'<'], 'u') => {
                self.keyboard.pop(n);
                return;
            }
            ([b'='], 'u') => {
                self.keyboard
                    .set(first, params.get(1).copied().unwrap_or(1));
                return;
            }
            ([b'?'], 'h' | 'l') => {
                for &mode in params {
                    self.private_mode(mode, end == 'h');
                }
                return;
            }
            ([b'?'], 'n') if first == 6 => {
                self.report_cursor(true);
                return;
            }
            ([b'?', b'$'], 'p') => {
                for &mode in params {
                    let status = self.mode_status(mode);
                    self.reply(format!("\x1b[?{mode};{status}$y"));
                }
                return;
            }
            ([b'$'], 'p') => {
                for &mode in params {
                    let enabled = match mode {
                        4 => Some(self.insert_mode),
                        20 => Some(self.newline_mode),
                        _ => None,
                    };
                    let status = enabled.map_or(0, |on| if on { 1 } else { 2 });
                    self.reply(format!("\x1b[{mode};{status}$y"));
                }
                return;
            }
            ([b' '], 'q') => {
                if let Some(style) = CursorStyle::from_parameter(first as u16) {
                    self.cursor_style = style;
                }
                return;
            }
            ([b'>'], 'c') if first == 0 => {
                self.reply("\x1b[>0;100;0c");
                return;
            }
            ([b'>'], 'q') => {
                self.reply(concat!(
                    "\x1bP>|Cryptty(",
                    env!("CARGO_PKG_VERSION"),
                    ")\x1b\\"
                ));
                return;
            }
            ([b'!'], 'p') => {
                self.soft_reset();
                return;
            }
            ([], _) => (),
            _ => return,
        }
        let (x, y) = self.cursor;
        let top = if y >= self.scroll_top {
            self.scroll_top
        } else {
            0
        };
        let bottom = if y <= self.scroll_bottom {
            self.scroll_bottom
        } else {
            self.rows.len() - 1
        };
        match end {
            'A' => self.cursor.1 = y.saturating_sub(n).max(top),
            'B' | 'e' => self.cursor.1 = (y + n).min(bottom),
            'C' | 'a' => self.cursor.0 = (x + n).min(self.cols() - 1),
            'D' => self.cursor.0 = x.saturating_sub(n),
            'E' => self.cursor = (0, (y + n).min(bottom)),
            'F' => self.cursor = (0, y.saturating_sub(n).max(top)),
            'G' | '`' => self.cursor.0 = (n - 1).min(self.cols() - 1),
            'd' => self.goto(x, n - 1),
            'H' | 'f' => {
                let col = params.get(1).copied().unwrap_or(1).max(1);
                self.goto(col - 1, n - 1);
            }
            'I' => self.tab(n, false),
            'Z' => self.tab(n, true),
            'g' => match first {
                0 => self.tabs[x] = false,
                3 => self.tabs.fill(false),
                _ => (),
            },
            'J' => self.erase_display(first),
            'K' => {
                let range = match first {
                    0 => x..self.cols(),
                    1 => 0..x + 1,
                    2 => 0..self.cols(),
                    _ => 0..0,
                };
                self.rows[y].erase(range, self.style);
            }
            'X' => {
                let end = (x + n).min(self.cols());
                self.rows[y].erase(x..end, self.style);
            }
            'P' => {
                let count = n.min(self.cols() - x);
                let row = &mut self.rows[y];
                row.wrapped = None;
                row.materialized_mut()[x..].rotate_left(count);
                let start = row.len() - count;
                row.materialized_mut()[start..].fill(Cell::blank(self.style));
                Self::repair_row(row.materialized_mut());
            }
            '@' => self.insert_characters(n),
            'L' | 'M'
                if (self.scroll_top..=self.scroll_bottom).contains(&y) =>
            {
                if end == 'L' {
                    self.scroll_down(y, n);
                } else {
                    self.scroll_up(y, n);
                }
            }
            'S' => self.scroll_up(self.scroll_top, n),
            'T' => self.scroll_down(self.scroll_top, n),
            'r' => {
                let bottom = params.get(1).copied().unwrap_or(0);
                let bottom = if bottom == 0 { self.rows.len() } else { bottom };
                if n < bottom && bottom <= self.rows.len() {
                    self.scroll_top = n - 1;
                    self.scroll_bottom = bottom - 1;
                    self.goto(0, 0);
                }
            }
            's' => self.save_cursor(),
            'u' => self.restore_cursor(),
            'b' => {
                if let Some(ch) = self.preceding {
                    for _ in 0..n {
                        self.put(ch);
                    }
                }
            }
            'h' | 'l' => {
                for &mode in params {
                    match mode {
                        4 => self.insert_mode = end == 'h',
                        20 => self.newline_mode = end == 'h',
                        _ => (),
                    }
                }
                return;
            }
            'c' if first == 0 => self.reply("\x1b[?1;0c"),
            'n' if first == 5 => self.reply("\x1b[0n"),
            'n' if first == 6 => self.report_cursor(false),
            't' if first == 18 => {
                self.reply(format!(
                    "\x1b[8;{};{}t",
                    self.rows.len(),
                    self.cols()
                ));
            }
            _ => return,
        }
        if !matches!(end, 'n' | 's' | 'u' | 'c' | 'b' | 't') {
            self.pending_wrap = false;
        }
    }

    fn goto(&mut self, x: usize, y: usize) {
        let (top, bottom) = if self.origin_mode {
            (self.scroll_top, self.scroll_bottom)
        } else {
            (0, self.rows.len() - 1)
        };
        self.cursor = (x.min(self.cols() - 1), (top + y).min(bottom));
        self.pending_wrap = false;
    }

    fn report_cursor(&mut self, private: bool) {
        let y = if self.origin_mode {
            self.cursor.1.saturating_sub(self.scroll_top)
        } else {
            self.cursor.1
        };
        let prefix = if private { "?" } else { "" };
        self.reply(format!("\x1b[{prefix}{};{}R", y + 1, self.cursor.0 + 1));
    }

    fn private_mode(&mut self, mode: usize, enabled: bool) {
        match mode {
            1 => self.application_cursor = enabled,
            5 => {
                self.reverse_video = enabled;
                self.color_revision = self.color_revision.wrapping_add(1);
            }
            6 => {
                self.origin_mode = enabled;
                self.goto(0, 0);
            }
            7 => {
                self.auto_wrap = enabled;
                self.pending_wrap = false;
            }
            12 => self.cursor_style.blinking = enabled,
            25 => self.cursor_visible = enabled,
            47 => self.switch_screen(enabled, false),
            66 => self.application_keypad = enabled,
            1000 | 1002 | 1003 => {
                self.mouse_mode = if !enabled {
                    MouseMode::Off
                } else {
                    match mode {
                        1000 => MouseMode::Click,
                        1002 => MouseMode::Drag,
                        _ => MouseMode::Motion,
                    }
                };
            }
            1004 => self.focus_reporting = enabled,
            1006 => self.mouse_sgr = enabled,
            1047 => {
                if !enabled && self.alternate {
                    self.erase_display(2);
                }
                self.switch_screen(enabled, false);
            }
            1048 => {
                if enabled {
                    self.save_cursor();
                } else {
                    self.restore_cursor();
                }
            }
            1049 if enabled && !self.alternate => {
                self.save_cursor();
                let cursor = self.cursor;
                self.switch_screen(true, true);
                self.cursor = cursor;
            }
            1049 if !enabled && self.alternate => {
                self.switch_screen(false, false);
                self.restore_cursor();
            }
            2004 => self.bracketed_paste = enabled,
            2026 => {
                self.sync_deadline = enabled
                    .then(|| Instant::now() + Duration::from_millis(150));
            }
            _ => (),
        }
    }

    fn mode_status(&self, mode: usize) -> u8 {
        let enabled = match mode {
            1 => self.application_cursor,
            5 => self.reverse_video,
            6 => self.origin_mode,
            7 => self.auto_wrap,
            12 => self.cursor_style.blinking,
            25 => self.cursor_visible,
            47 | 1047 | 1049 => self.alternate,
            66 => self.application_keypad,
            1000 => self.mouse_mode == MouseMode::Click,
            1002 => self.mouse_mode == MouseMode::Drag,
            1003 => self.mouse_mode == MouseMode::Motion,
            1004 => self.focus_reporting,
            1006 => self.mouse_sgr,
            2004 => self.bracketed_paste,
            2026 => self.sync_deadline.is_some_and(|t| t > Instant::now()),
            _ => return 0,
        };
        if enabled { 1 } else { 2 }
    }

    fn soft_reset(&mut self) {
        self.style = Style::default().into();
        self.hyperlink = None;
        self.origin_mode = false;
        self.insert_mode = false;
        self.application_cursor = false;
        self.application_keypad = false;
        self.auto_wrap = true;
        self.pending_wrap = false;
        self.cursor_visible = true;
        self.scroll_top = 0;
        self.scroll_bottom = self.rows.len() - 1;
        self.charsets = [false; 2];
        self.active_charset = 0;
        self.saved = super::SavedCursor::default();
    }

    pub(super) fn reply(&mut self, text: impl AsRef<str>) {
        self.replies.extend_from_slice(text.as_ref().as_bytes());
    }
}
