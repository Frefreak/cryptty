use vte::Params;

use super::{Blink, Color, Style, Terminal, Underline};

impl Terminal {
    pub(super) fn sgr(&mut self, params: &Params) {
        let mut style: Style = self.style.into();
        let mut groups = params.iter();
        while let Some(group) = groups.next() {
            match group[0] {
                0 => style = Style::default(),
                1 => style.bold = true,
                2 => style.dim = true,
                3 => style.italic = true,
                5 => style.blink = Blink::Slow,
                6 => style.blink = Blink::Rapid,
                25 => style.blink = Blink::None,
                4 => {
                    style.underline = match group.get(1).unwrap_or(&1) {
                        0 => Underline::None,
                        1 => Underline::Single,
                        2 => Underline::Double,
                        3 => Underline::Curly,
                        4 => Underline::Dotted,
                        5 => Underline::Dashed,
                        _ => continue,
                    };
                }
                7 => style.inverse = true,
                8 => style.hidden = true,
                9 => style.strike = true,
                21 => style.underline = Underline::Double,
                22 => {
                    style.bold = false;
                    style.dim = false;
                }
                23 => style.italic = false,
                24 => style.underline = Underline::None,
                27 => style.inverse = false,
                28 => style.hidden = false,
                29 => style.strike = false,
                30..=37 => style.fg = Color::Indexed((group[0] - 30) as u8),
                40..=47 => style.bg = Color::Indexed((group[0] - 40) as u8),
                90..=97 => style.fg = Color::Indexed((group[0] - 82) as u8),
                100..=107 => style.bg = Color::Indexed((group[0] - 92) as u8),
                39 => style.fg = Color::Foreground,
                49 => style.bg = Color::Background,
                38 | 48 | 58 => {
                    if let Some(color) = extended_color(group, &mut groups) {
                        match group[0] {
                            38 => style.fg = color,
                            48 => style.bg = color,
                            _ => style.underline_color = Some(color),
                        }
                    }
                }
                53 => style.overline = true,
                55 => style.overline = false,
                59 => style.underline_color = None,
                _ => (),
            }
        }
        self.style = style.into();
    }

    pub(super) fn sgr_status(&self) -> String {
        let style: Style = self.style.into();
        let mut params = vec!["0".to_owned()];
        for (enabled, value) in [
            (style.bold, "1"),
            (style.dim, "2"),
            (style.blink == Blink::Slow, "5"),
            (style.blink == Blink::Rapid, "6"),
            (style.italic, "3"),
            (style.inverse, "7"),
            (style.hidden, "8"),
            (style.strike, "9"),
            (style.overline, "53"),
        ] {
            if enabled {
                params.push(value.into());
            }
        }
        let underline = match style.underline {
            Underline::None => None,
            Underline::Single => Some("4"),
            Underline::Double => Some("4:2"),
            Underline::Curly => Some("4:3"),
            Underline::Dotted => Some("4:4"),
            Underline::Dashed => Some("4:5"),
        };
        if let Some(value) = underline {
            params.push(value.into());
        }
        for (which, color) in [
            (38, Some(style.fg)),
            (48, Some(style.bg)),
            (58, style.underline_color),
        ] {
            match color {
                Some(Color::Indexed(index)) => {
                    params.push(format!("{which};5;{index}"));
                }
                Some(Color::Rgb(r, g, b)) => {
                    params.push(format!("{which};2;{r};{g};{b}"));
                }
                _ => (),
            }
        }
        format!("{}m", params.join(";"))
    }
}

fn extended_color<'a>(
    group: &[u16],
    groups: &mut impl Iterator<Item = &'a [u16]>,
) -> Option<Color> {
    if group.len() > 1 {
        return match group {
            [_, 5, index] => Some(Color::Indexed((*index).try_into().ok()?)),
            [_, 2, r, g, b] | [_, 2, 0, r, g, b] => rgb(*r, *g, *b),
            _ => None,
        };
    }
    let mode = single(groups.next()?)?;
    match mode {
        5 => Some(Color::Indexed(single(groups.next()?)?.try_into().ok()?)),
        2 => {
            let r = groups.next()?;
            let g = groups.next()?;
            let b = groups.next()?;
            rgb(single(r)?, single(g)?, single(b)?)
        }
        _ => None,
    }
}

fn single(group: &[u16]) -> Option<u16> {
    match group {
        [value] => Some(*value),
        _ => None,
    }
}

fn rgb(r: u16, g: u16, b: u16) -> Option<Color> {
    Some(Color::Rgb(
        r.try_into().ok()?,
        g.try_into().ok()?,
        b.try_into().ok()?,
    ))
}
