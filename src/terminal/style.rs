#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Color {
    Foreground,
    Background,
    Indexed(u8),
    Rgb(u32),
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
            Self::Rapid => phase % 2 == 0,
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

    pub(super) fn erased(self) -> Self {
        Self {
            fg: self.fg,
            bg: self.bg,
            ..Self::default()
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
