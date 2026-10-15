use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::terminal::Theme;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub font: Font,
    pub window: Window,
    pub colors: Colors,
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = user_path(
            std::env::var_os("XDG_CONFIG_HOME"),
            std::env::var_os("HOME"),
        )?;
        Self::read(&path)
    }

    pub fn parse(text: &str) -> serde_yaml_ng::Result<Self> {
        serde_yaml_ng::from_str::<Option<Self>>(text)
            .map(Option::unwrap_or_default)
    }

    pub fn read(path: &Path) -> Result<Self> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("Cannot read config {}", path.display())
                });
            }
        };
        Self::parse(&text)
            .with_context(|| format!("Invalid config {}", path.display()))
    }
}

pub fn user_path(
    xdg: Option<OsString>,
    home: Option<OsString>,
) -> Result<PathBuf> {
    let root = if let Some(xdg) = xdg.filter(|value| !value.is_empty()) {
        let path = PathBuf::from(xdg);
        if !path.is_absolute() {
            bail!("XDG_CONFIG_HOME must be an absolute path");
        }
        path
    } else {
        let home = home
            .filter(|value| !value.is_empty())
            .context("Set HOME or XDG_CONFIG_HOME to locate your config")?;
        let path = PathBuf::from(home);
        if !path.is_absolute() {
            bail!("HOME must be an absolute path");
        }
        path.join(".config")
    };
    Ok(root.join("cryptty/cryptty.yaml"))
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Font {
    pub hinting: bool,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Window {
    pub padding: Padding,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Padding {
    pub x: u16,
    pub y: u16,
}

impl Padding {
    pub fn physical(self, scale: f64) -> (u32, u32) {
        (
            (f64::from(self.x) * scale).floor() as u32,
            (f64::from(self.y) * scale).floor() as u32,
        )
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Colors {
    primary: Primary,
    cursor: Cursor,
    normal: Palette,
    bright: Palette,
}

impl Colors {
    pub fn theme(&self) -> Theme {
        let mut theme = Theme::default();
        if let Some(color) = self.primary.foreground {
            theme.foreground = color.0;
        }
        if let Some(color) = self.primary.background {
            theme.background = color.0;
        }
        theme.cursor =
            self.cursor.cursor.map_or(theme.foreground, |color| color.0);
        for (index, color) in self
            .normal
            .entries()
            .into_iter()
            .chain(self.bright.entries())
            .enumerate()
        {
            if let Some(color) = color {
                theme.palette[index] = color.0;
            }
        }
        theme
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Primary {
    foreground: Option<Color>,
    background: Option<Color>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Cursor {
    cursor: Option<Color>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Palette {
    black: Option<Color>,
    red: Option<Color>,
    green: Option<Color>,
    yellow: Option<Color>,
    blue: Option<Color>,
    magenta: Option<Color>,
    cyan: Option<Color>,
    white: Option<Color>,
}

impl Palette {
    fn entries(&self) -> [Option<Color>; 8] {
        [
            self.black,
            self.red,
            self.green,
            self.yellow,
            self.blue,
            self.magenta,
            self.cyan,
            self.white,
        ]
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(try_from = "String")]
struct Color(u32);

impl TryFrom<String> for Color {
    type Error = &'static str;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let hex = value
            .strip_prefix('#')
            .or_else(|| value.strip_prefix("0x"))
            .ok_or("Expected a color such as '#282a36' or '0x282a36'")?;
        if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("Expected exactly six hexadecimal color digits");
        }
        u32::from_str_radix(hex, 16)
            .map(Self)
            .map_err(|_| "Invalid hexadecimal color")
    }
}
