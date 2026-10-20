use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::terminal::Theme;

mod defaults;
mod font_mapping;
mod keyboard;

pub use font_mapping::FontMapping;
pub use keyboard::Keyboard;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub font: Font,
    pub window: Window,
    pub colors: Colors,
    pub keyboard: Keyboard,
    pub selection: Selection,
    pub markers: crate::marker::Options,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Selection {
    pub word_characters: String,
}

impl Default for Selection {
    fn default() -> Self {
        Self {
            word_characters: "/\\._-~:@%+=,#?".into(),
        }
    }
}

impl Config {
    pub fn load() -> Result<Self> {
        let xdg = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|value| !value.is_empty());
        let home = std::env::var_os("HOME").filter(|value| !value.is_empty());
        if xdg.is_none() && home.is_none() {
            eprintln!(
                "Warning: HOME and XDG_CONFIG_HOME are unset or empty; \
                 using built-in defaults without creating a config file"
            );
            return Ok(Self::default());
        }
        let path = user_path(xdg, home)?;
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
                if let Some(parent) = path
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                {
                    std::fs::create_dir_all(parent).with_context(|| {
                        format!(
                            "Cannot create config directory {}",
                            parent.display()
                        )
                    })?;
                }
                return Self::update_defaults(path).with_context(|| {
                    format!("Cannot initialize config {}", path.display())
                });
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("Cannot read config {}", path.display())
                });
            }
        };
        Self::read_existing(path, &text)
    }

    fn read_existing(path: &Path, text: &str) -> Result<Self> {
        let config = Self::parse(text)
            .with_context(|| format!("Invalid config {}", path.display()))?;
        if defaults::updated(text)? != text {
            let result = Self::update_defaults(path);
            match result {
                Ok(config) => return Ok(config),
                Err(error) => eprintln!(
                    "Warning: cannot update default comments to {}: {error}",
                    path.display()
                ),
            }
        }
        Ok(config)
    }

    fn update_defaults(path: &Path) -> Result<Self> {
        let path = match std::fs::canonicalize(path) {
            Ok(path) => path,
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    && !path.is_symlink() =>
            {
                path.to_path_buf()
            }
            Err(error) => return Err(error.into()),
        };
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path.with_extension("yaml.lock"))?;
        lock.lock()?;
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                String::new()
            }
            Err(error) => return Err(error.into()),
        };
        let config = Self::parse(&text)?;
        let updated = defaults::updated(&text)?;
        if updated != text {
            let parent = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new("."));
            let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
            temporary.write_all(updated.as_bytes())?;
            if let Ok(metadata) = std::fs::metadata(&path) {
                let permissions = metadata.permissions();
                if permissions.readonly() {
                    bail!("Config file is read-only");
                }
                temporary.as_file().set_permissions(permissions)?;
            }
            temporary.persist(&path)?;
        }
        Ok(config)
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

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Font {
    pub family: String,
    pub size: FontSize,
    pub hinting: bool,
    pub fallback: Vec<String>,
    pub codepoint_map: Vec<FontMapping>,
}

impl Default for Font {
    fn default() -> Self {
        Self {
            family: "FantasqueSansM Nerd Font Mono".to_string(),
            size: FontSize::default(),
            hinting: false,
            fallback: vec!["LXGW WenKai Mono Screen".to_string()],
            codepoint_map: FontMapping::defaults(),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(try_from = "f32")]
pub struct FontSize(f32);

impl Default for FontSize {
    fn default() -> Self {
        Self(19.0)
    }
}

impl TryFrom<f32> for FontSize {
    type Error = &'static str;

    fn try_from(points: f32) -> Result<Self, Self::Error> {
        if !points.is_finite() || points <= 0.0 {
            return Err("Font size must be a finite positive number of points");
        }
        Ok(Self(points))
    }
}

impl FontSize {
    pub fn pixels(self, scale: f64) -> f32 {
        let pixels = if cfg!(target_os = "macos") {
            self.0
        } else {
            self.0 * 96.0 / 72.0
        };
        pixels * scale as f32
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Window {
    pub padding: Padding,
    pub option_as_alt: OptionAsAlt,
    pub opacity: Opacity,
    pub blur: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(try_from = "f32")]
pub struct Opacity(f32);

impl Default for Opacity {
    fn default() -> Self {
        Self(1.0)
    }
}

impl TryFrom<f32> for Opacity {
    type Error = &'static str;

    fn try_from(value: f32) -> Result<Self, Self::Error> {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err("Window opacity must be a finite number from 0 to 1");
        }
        Ok(Self(value))
    }
}

impl Opacity {
    pub fn get(self) -> f32 {
        self.0
    }

    pub fn transparent(self) -> bool {
        self.0 < 1.0
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OptionAsAlt {
    None,
    #[default]
    OnlyLeft,
    OnlyRight,
    Both,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Padding {
    pub x: u16,
    pub y: u16,
}

impl Default for Padding {
    fn default() -> Self {
        Self { x: 2, y: 2 }
    }
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
