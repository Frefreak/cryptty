use vte::{Params, Perform};

use super::{Cell, Style, Terminal};

impl Perform for Terminal {
    fn print(&mut self, ch: char) {
        let ch = if self.charsets[self.active_charset] {
            graphics(ch)
        } else {
            ch
        };
        self.preceding = Some(ch);
        self.put(ch);
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            7 => self.bell(),
            b'\r' => {
                self.cursor.0 = 0;
                self.pending_wrap = false;
            }
            b'\n' | 0x0b | 0x0c | 0x84 => {
                self.rows[self.cursor.1].wrapped = None;
                self.linefeed();
                if self.newline_mode {
                    self.cursor.0 = 0;
                }
            }
            8 => {
                self.cursor.0 = self.cursor.0.saturating_sub(1);
                self.pending_wrap = false;
            }
            b'\t' => self.tab(1, false),
            0x0e => self.active_charset = 1,
            0x0f => self.active_charset = 0,
            0x85 => {
                self.rows[self.cursor.1].wrapped = None;
                self.cursor.0 = 0;
                self.linefeed();
            }
            0x88 => self.tabs[self.cursor.0] = true,
            0x8d => self.reverse_index(),
            _ => (),
        }
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], ignore: bool, byte: u8) {
        if ignore {
            return;
        }
        match (intermediates, byte) {
            ([], b'7') => self.save_cursor(),
            ([], b'8') => self.restore_cursor(),
            ([], b'D') => {
                self.rows[self.cursor.1].wrapped = None;
                self.linefeed();
            }
            ([], b'E') => {
                self.rows[self.cursor.1].wrapped = None;
                self.cursor.0 = 0;
                self.linefeed();
            }
            ([], b'H') => self.tabs[self.cursor.0] = true,
            ([], b'M') => self.reverse_index(),
            ([], b'=') => self.application_keypad = true,
            ([], b'>') => self.application_keypad = false,
            ([], b'c') => self.reset(),
            ([], b'Z') => self.replies.extend_from_slice(b"\x1b[?1;0c"),
            ([b'('], b'0' | b'B') => self.charsets[0] = byte == b'0',
            ([b')'], b'0' | b'B') => self.charsets[1] = byte == b'0',
            ([b'#'], b'8') => {
                let mut cell = Cell::blank(Style::default());
                cell.content = super::Content::Text {
                    text: super::CellText::new('E'),
                    wide: false,
                };
                for row in &mut self.rows {
                    row.fill(cell.clone());
                }
                self.cursor = (0, 0);
                self.pending_wrap = false;
            }
            _ => (),
        }
    }

    fn csi_dispatch(
        &mut self,
        params: &Params,
        intermediates: &[u8],
        ignore: bool,
        action: char,
    ) {
        if !ignore {
            self.csi(params, intermediates, action);
        }
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], bell_terminated: bool) {
        if self.cancelling
            || params.iter().map(|p| p.len()).sum::<usize>() >= super::OSC_LIMIT
            || params.len() >= 16
        {
            return;
        }
        self.osc(params, bell_terminated);
    }

    fn hook(
        &mut self,
        _: &Params,
        intermediates: &[u8],
        ignore: bool,
        action: char,
    ) {
        self.dcs = match (ignore, intermediates, action) {
            (false, [b'$'], 'q') => Some(vec![b'$']),
            (false, [b'+'], 'q') => Some(vec![b'+']),
            _ => None,
        };
    }

    fn put(&mut self, byte: u8) {
        if let Some(data) = &mut self.dcs {
            if data.len() < 256 {
                data.push(byte);
            } else {
                self.dcs = None;
            }
        }
    }

    fn unhook(&mut self) {
        if let Some(data) = self.dcs.take()
            && !self.cancelling
        {
            self.dcs_query(&data);
        }
    }
}

impl Terminal {
    pub(super) fn tab(&mut self, count: usize, backwards: bool) {
        for _ in 0..count.min(self.cols()) {
            self.cursor.0 = if backwards {
                (0..self.cursor.0)
                    .rev()
                    .find(|&x| self.tabs[x])
                    .unwrap_or(0)
            } else {
                (self.cursor.0 + 1..self.cols())
                    .find(|&x| self.tabs[x])
                    .unwrap_or(self.cols() - 1)
            };
        }
        self.pending_wrap = false;
    }
}

fn graphics(ch: char) -> char {
    match ch {
        '`' => '◆',
        'a' => '▒',
        'b' => '␉',
        'c' => '␌',
        'd' => '␍',
        'e' => '␊',
        'f' => '°',
        'g' => '±',
        'h' => '␤',
        'i' => '␋',
        'j' => '┘',
        'k' => '┐',
        'l' => '┌',
        'm' => '└',
        'n' => '┼',
        'o' => '⎺',
        'p' => '⎻',
        'q' => '─',
        'r' => '⎼',
        's' => '⎽',
        't' => '├',
        'u' => '┤',
        'v' => '┴',
        'w' => '┬',
        'x' => '│',
        'y' => '≤',
        'z' => '≥',
        '{' => 'π',
        '|' => '≠',
        '}' => '£',
        '~' => '·',
        _ => ch,
    }
}
