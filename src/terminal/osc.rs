use base64::Engine;
use base64::engine::general_purpose::STANDARD;

use super::{Arc, Effect, Terminal, indexed_color};

impl Terminal {
    pub(super) fn osc(&mut self, params: &[&[u8]], bell: bool) {
        let Some(command) = params.first().and_then(|p| number(p)) else {
            self.unknown_osc(None, params.iter().map(|p| p.len()).sum());
            return;
        };
        let terminator = if bell { "\x07" } else { "\x1b\\" };
        match command {
            9 if params.len() >= 2 => {
                if params.len() >= 3 && params[1] == b"4" {
                    self.unknown_osc(
                        Some(9),
                        params.iter().map(|p| p.len()).sum(),
                    );
                    return;
                }
                let message: String =
                    String::from_utf8_lossy(&params[1..].join(&b';'))
                        .chars()
                        .filter(|c| !c.is_control())
                        .take(1024)
                        .collect();
                if !message.trim().is_empty() && self.effects.len() < 64 {
                    self.effects.push(Effect::Notification(message));
                }
            }
            0 | 2 if params.len() >= 2 => {
                self.title = String::from_utf8_lossy(&params[1..].join(&b';'))
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(1024)
                    .collect();
            }
            4 => {
                for pair in params[1..].chunks_exact(2) {
                    let Some(index) = number(pair[0]).filter(|i| *i < 256)
                    else {
                        continue;
                    };
                    if pair[1] == b"?" {
                        let rgb = rgb_reply(self.palette[index]);
                        self.reply(format!("\x1b]4;{index};{rgb}{terminator}"));
                    } else if let Some(color) = parse_color(pair[1]) {
                        self.set_palette(index, color);
                    }
                }
            }
            8 if params.len() >= 3 => {
                let uri = params[2..].join(&b';');
                self.hyperlink = String::from_utf8(uri)
                    .ok()
                    .filter(|s| {
                        !s.is_empty() && !s.chars().any(char::is_control)
                    })
                    .map(Arc::new);
            }
            10..=12 => {
                for (offset, value) in params[1..].iter().enumerate() {
                    let which = command + offset;
                    if which > 12 {
                        break;
                    }
                    if *value == b"?" {
                        let color = match which {
                            10 => self.theme.foreground,
                            11 => self.theme.background,
                            _ => self.theme.cursor,
                        };
                        self.reply(format!(
                            "\x1b]{which};{}{terminator}",
                            rgb_reply(color),
                        ));
                    } else if let Some(color) = parse_color(value) {
                        match which {
                            10 => self.theme.foreground = color,
                            11 => self.theme.background = color,
                            _ => self.theme.cursor = color,
                        }
                        self.color_revision =
                            self.color_revision.wrapping_add(1);
                    }
                }
            }
            52 if params.len() == 3 => {
                let selection = params[1];
                if (selection.is_empty() || selection.contains(&b'c'))
                    && let Ok(bytes) = STANDARD.decode(params[2])
                    && let Ok(text) = String::from_utf8(bytes)
                {
                    self.effects.push(Effect::Clipboard(text));
                }
            }
            104 => {
                if params.len() == 1 || params[1..] == [b""] {
                    for index in 0..256 {
                        self.reset_palette(index);
                    }
                } else {
                    for value in &params[1..] {
                        if let Some(index) = number(value).filter(|i| *i < 256)
                        {
                            self.reset_palette(index);
                        }
                    }
                }
            }
            110 => {
                self.theme.foreground = self.initial_theme.foreground;
                self.color_revision = self.color_revision.wrapping_add(1);
            }
            111 => {
                self.theme.background = self.initial_theme.background;
                self.color_revision = self.color_revision.wrapping_add(1);
            }
            112 => {
                self.theme.cursor = self.initial_theme.cursor;
                self.color_revision = self.color_revision.wrapping_add(1);
            }
            _ => self.unknown_osc(
                u32::try_from(command).ok(),
                params.iter().map(|p| p.len()).sum(),
            ),
        }
    }

    fn set_palette(&mut self, index: usize, color: u32) {
        self.palette[index] = color;
        if index < 16 {
            self.theme.palette[index] = color;
        }
        self.color_revision = self.color_revision.wrapping_add(1);
    }

    fn reset_palette(&mut self, index: usize) {
        let color = if index < 16 {
            self.initial_theme.palette[index]
        } else {
            indexed_color(index as u8)
        };
        self.set_palette(index, color);
    }

    pub(super) fn dcs_query(&mut self, data: &[u8]) {
        if data[0] == b'$' {
            let response = match &data[1..] {
                b"m" => Some(self.sgr_status()),
                b"r" => Some(format!(
                    "{};{}r",
                    self.scroll_top + 1,
                    self.scroll_bottom + 1
                )),
                b" q" => Some(format!("{} q", self.cursor_style.parameter())),
                _ => None,
            };
            match response {
                Some(value) => self.reply(format!("\x1bP1$r{value}\x1b\\")),
                None => self.reply("\x1bP0$r\x1b\\"),
            }
        } else {
            for name in data[1..].split(|b| *b == b';') {
                let value = match name {
                    b"544e" | b"544E" => Some("787465726d2d323536636f6c6f72"),
                    b"436f" | b"436F" => Some("323536"),
                    b"524742" | b"5463" => Some(""),
                    _ => None,
                };
                let name = String::from_utf8_lossy(name);
                match value {
                    Some("") => self.reply(format!("\x1bP1+r{name}\x1b\\")),
                    Some(value) => {
                        self.reply(format!("\x1bP1+r{name}={value}\x1b\\"))
                    }
                    None => self.reply(format!("\x1bP0+r{name}\x1b\\")),
                }
            }
        }
    }
}

fn number(bytes: &[u8]) -> Option<usize> {
    std::str::from_utf8(bytes).ok()?.parse().ok()
}

fn rgb_reply(rgb: u32) -> String {
    format!(
        "rgb:{:04x}/{:04x}/{:04x}",
        ((rgb >> 16) & 255) * 257,
        ((rgb >> 8) & 255) * 257,
        (rgb & 255) * 257
    )
}

fn parse_color(bytes: &[u8]) -> Option<u32> {
    let value = std::str::from_utf8(bytes).ok()?;
    let channel = |part: &str, scale: bool| {
        if part.is_empty() || part.len() > 4 {
            return None;
        }
        let value = u32::from_str_radix(part, 16).ok()?;
        Some(if scale {
            value * 255 / ((1 << (part.len() * 4)) - 1)
        } else if part.len() == 1 {
            value << 4
        } else {
            value >> ((part.len() - 2) * 4)
        })
    };
    let (r, g, b) = if let Some(rgb) = value.strip_prefix("rgb:") {
        let mut parts = rgb.split('/');
        let rgb = (
            channel(parts.next()?, true)?,
            channel(parts.next()?, true)?,
            channel(parts.next()?, true)?,
        );
        if parts.next().is_some() {
            return None;
        }
        rgb
    } else if let Some(hex) = value.strip_prefix('#') {
        if !hex.is_ascii() || ![3, 6, 9, 12].contains(&hex.len()) {
            return None;
        }
        let n = hex.len() / 3;
        (
            channel(&hex[..n], false)?,
            channel(&hex[n..2 * n], false)?,
            channel(&hex[2 * n..], false)?,
        )
    } else {
        match value {
            "black" => (0, 0, 0),
            "white" => (255, 255, 255),
            _ => return None,
        }
    };
    Some((r << 16) | (g << 8) | b)
}
