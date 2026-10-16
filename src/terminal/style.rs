#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Color {
    Foreground,
    Background,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Underline {
    #[default]
    None,
    Single,
    Double,
    Curly,
    Dotted,
    Dashed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Blink {
    #[default]
    None,
    Slow,
    Rapid,
}

impl Blink {
    pub fn visible(self, phase: u8) -> bool {
        match self {
            Self::None => true,
            Self::Slow => phase % 4 < 2,
            Self::Rapid => phase.is_multiple_of(2),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct FontStyle {
    pub bold: bool,
    pub italic: bool,
}

impl FontStyle {
    pub(crate) fn index(self) -> usize {
        usize::from(self.bold) | (usize::from(self.italic) << 1)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Style {
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub italic: bool,
    pub dim: bool,
    pub blink: Blink,
    pub hidden: bool,
    pub underline: Underline,
    pub underline_color: Option<Color>,
    pub strike: bool,
    pub overline: bool,
    pub inverse: bool,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            fg: Color::Foreground,
            bg: Color::Background,
            bold: false,
            italic: false,
            dim: false,
            blink: Blink::None,
            hidden: false,
            underline: Underline::None,
            underline_color: None,
            strike: false,
            overline: false,
            inverse: false,
        }
    }
}

impl Style {
    pub fn font(self) -> FontStyle {
        FontStyle {
            bold: self.bold,
            italic: self.italic,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CursorShape {
    Block,
    Underline,
    #[default]
    Beam,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CursorStyle {
    pub shape: CursorShape,
    pub blinking: bool,
}

impl CursorStyle {
    pub(super) fn from_parameter(value: u16) -> Option<Self> {
        let shape = match value {
            0..=2 => CursorShape::Block,
            3..=4 => CursorShape::Underline,
            5..=6 => CursorShape::Beam,
            _ => return None,
        };
        Some(Self {
            shape,
            blinking: value == 0 || value % 2 == 1,
        })
    }

    pub(super) fn parameter(self) -> u8 {
        let steady = match self.shape {
            CursorShape::Block => 2,
            CursorShape::Underline => 4,
            CursorShape::Beam => 6,
        };
        steady - u8::from(self.blinking)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MouseMode {
    #[default]
    Off,
    Click,
    Drag,
    Motion,
}

pub fn dim(rgb: u32) -> u32 {
    let channel = |shift: u32| (((rgb >> shift) & 255) * 2 / 3) << shift;
    channel(16) | channel(8) | channel(0)
}

const BOLD: u8 = 1;
const ITALIC: u8 = 1 << 1;
const DIM: u8 = 1 << 2;
const HIDDEN: u8 = 1 << 3;
const STRIKE: u8 = 1 << 4;
const OVERLINE: u8 = 1 << 5;
const INVERSE: u8 = 1 << 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CellStyle {
    fg: Color,
    bg: Color,
    underline_color: Option<Color>,
    blink: Blink,
    underline: Underline,
    flags: u8,
}

impl CellStyle {
    pub(super) fn erased(self) -> Self {
        Style {
            fg: self.fg,
            bg: self.bg,
            ..Style::default()
        }
        .into()
    }
}

impl From<Style> for CellStyle {
    fn from(style: Style) -> Self {
        Self {
            fg: style.fg,
            bg: style.bg,
            underline_color: style.underline_color,
            blink: style.blink,
            underline: style.underline,
            flags: (u8::from(style.bold) * BOLD)
                | (u8::from(style.italic) * ITALIC)
                | (u8::from(style.dim) * DIM)
                | (u8::from(style.hidden) * HIDDEN)
                | (u8::from(style.strike) * STRIKE)
                | (u8::from(style.overline) * OVERLINE)
                | (u8::from(style.inverse) * INVERSE),
        }
    }
}

impl From<CellStyle> for Style {
    fn from(style: CellStyle) -> Self {
        Self {
            fg: style.fg,
            bg: style.bg,
            underline_color: style.underline_color,
            blink: style.blink,
            underline: style.underline,
            bold: style.flags & BOLD != 0,
            italic: style.flags & ITALIC != 0,
            dim: style.flags & DIM != 0,
            hidden: style.flags & HIDDEN != 0,
            strike: style.flags & STRIKE != 0,
            overline: style.flags & OVERLINE != 0,
            inverse: style.flags & INVERSE != 0,
        }
    }
}

#[cfg(test)]
mod packing_tests {
    use super::*;

    #[test]
    fn packed_styles_preserve_every_attribute_combination() {
        for flags in 0..128 {
            for blink in [Blink::None, Blink::Slow, Blink::Rapid] {
                for underline in [
                    Underline::None,
                    Underline::Single,
                    Underline::Double,
                    Underline::Curly,
                    Underline::Dotted,
                    Underline::Dashed,
                ] {
                    let style = Style {
                        fg: Color::Indexed(255),
                        bg: Color::Rgb(0, 127, 255),
                        underline_color: Some(Color::Foreground),
                        blink,
                        underline,
                        bold: flags & 1 != 0,
                        italic: flags & 2 != 0,
                        dim: flags & 4 != 0,
                        hidden: flags & 8 != 0,
                        strike: flags & 16 != 0,
                        overline: flags & 32 != 0,
                        inverse: flags & 64 != 0,
                    };
                    let packed = CellStyle::from(style);
                    assert_eq!(Style::from(packed), style);
                    assert_eq!(
                        Style::from(packed.erased()),
                        Style {
                            fg: style.fg,
                            bg: style.bg,
                            ..Style::default()
                        }
                    );
                }
            }
        }
    }
}
