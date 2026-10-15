use std::collections::HashMap;

use anyhow::Result;
use swash::scale::image::{Content as ImageContent, Image};
use unicode_width::UnicodeWidthChar;

use crate::terminal::{Cell, Content, Terminal};

#[cfg(target_os = "macos")]
#[path = "render/native.rs"]
mod font;
#[cfg(not(target_os = "macos"))]
#[path = "render/swash.rs"]
mod font;

const CACHE_LIMIT: usize = 4096;

pub struct Renderer {
    rasterizer: font::Rasterizer,
    glyphs: HashMap<(char, bool), Option<Image>>,
    padding: (usize, usize),
    pub cell_width: usize,
    pub cell_height: usize,
    baseline: i32,
    width: usize,
    height: usize,
    pub pixels: Vec<u32>,
    previous: Vec<Vec<Cell>>,
    old_cursor: Option<(usize, usize)>,
    old_preedit_row: Option<usize>,
}

impl Renderer {
    pub fn new(size: f32) -> Result<Self> {
        Self::with_options(
            size,
            (0, 0),
            false,
            &crate::config::Font::default().family,
        )
    }

    pub fn with_options(
        size: f32,
        padding: (u32, u32),
        hinting: bool,
        family: &str,
    ) -> Result<Self> {
        let rasterizer = font::Rasterizer::new(size, hinting, family)?;
        Ok(Self {
            cell_width: rasterizer.cell_width,
            cell_height: rasterizer.cell_height,
            baseline: rasterizer.baseline,
            rasterizer,
            glyphs: HashMap::new(),
            padding: (padding.0 as usize, padding.1 as usize),
            width: 0,
            height: 0,
            pixels: Vec::new(),
            previous: Vec::new(),
            old_cursor: None,
            old_preedit_row: None,
        })
    }

    pub fn grid_size(&self, width: u32, height: u32) -> (usize, usize) {
        (
            ((width as usize).saturating_sub(2 * self.padding.0)
                / self.cell_width)
                .max(1),
            ((height as usize).saturating_sub(2 * self.padding.1)
                / self.cell_height)
                .max(1),
        )
    }

    pub fn cell_origin(&self, x: usize, y: usize) -> (usize, usize) {
        (
            self.padding.0 + x * self.cell_width,
            self.padding.1 + y * self.cell_height,
        )
    }

    fn content_end(&self) -> (usize, usize) {
        (
            self.width.saturating_sub(self.padding.0),
            self.height.saturating_sub(self.padding.1),
        )
    }

    pub fn invalidate(&mut self) {
        self.previous.clear();
    }

    pub fn draw(
        &mut self,
        terminal: &Terminal,
        width: u32,
        height: u32,
        preedit: &str,
    ) {
        let (width, height) = (width as usize, height as usize);
        if (width, height) != (self.width, self.height) {
            self.width = width;
            self.height = height;
            self.pixels = vec![terminal.theme().background; width * height];
            self.invalidate();
        }
        if self.previous.is_empty() {
            self.pixels.fill(terminal.theme().background);
        }
        let cursor = terminal.cursor_visible.then_some(terminal.cursor);
        let preedit_row = (!preedit.is_empty()).then_some(terminal.cursor.1);
        for (y, row) in terminal.rows.iter().enumerate() {
            let cursor_row = cursor.map(|c| c.1) == Some(y)
                || self.old_cursor.map(|c| c.1) == Some(y)
                || preedit_row == Some(y)
                || self.old_preedit_row == Some(y);
            if self.previous.get(y) == Some(row) && !cursor_row {
                continue;
            }
            self.draw_row(y, row);
        }
        if !preedit.is_empty() {
            let (mut x, y) = terminal.cursor;
            for ch in preedit.chars() {
                if x >= terminal.cols() {
                    break;
                }
                let columns = ch.width().unwrap_or(1).max(1);
                self.paint_char(
                    ch,
                    false,
                    x,
                    y,
                    columns,
                    terminal.theme().palette[6],
                );
                x += columns;
            }
        }
        if let Some((x, y)) = cursor {
            let (left, top) = self.cell_origin(x, y);
            let (right, bottom) = self.content_end();
            for py in top..(top + self.cell_height).min(bottom) {
                for px in left..(left + 2).min(right) {
                    self.pixels[py * width + px] = terminal.theme().cursor;
                }
            }
        }
        self.previous.clone_from(&terminal.rows);
        self.old_cursor = cursor;
        self.old_preedit_row = preedit_row;
    }

    fn draw_row(&mut self, y: usize, row: &[Cell]) {
        let (_, top) = self.cell_origin(0, y);
        let (right, bottom) = self.content_end();
        for (x, cell) in row.iter().enumerate() {
            let bg = if cell.style.inverse {
                cell.style.fg
            } else {
                cell.style.bg
            };
            let (left, _) = self.cell_origin(x, y);
            for py in top..(top + self.cell_height).min(bottom) {
                let start = py * self.width + left.min(right);
                let end = py * self.width + (left + self.cell_width).min(right);
                self.pixels[start..end].fill(bg);
            }
        }
        for (x, cell) in row.iter().enumerate() {
            if let Content::Text { text, wide } = &cell.content {
                let fg = if cell.style.inverse {
                    cell.style.bg
                } else {
                    cell.style.fg
                };
                let columns = if *wide { 2 } else { 1 };
                for ch in text.chars().filter(|c| *c != ' ') {
                    self.paint_char(ch, cell.style.bold, x, y, columns, fg);
                }
                if cell.style.underline {
                    let py = top + self.cell_height - 2;
                    if py < bottom {
                        let (left, _) = self.cell_origin(x, y);
                        let end = (left + columns * self.cell_width).min(right);
                        let start = left.min(right);
                        self.pixels
                            [py * self.width + start..py * self.width + end]
                            .fill(fg);
                    }
                }
            }
        }
    }

    fn paint_char(
        &mut self,
        ch: char,
        bold: bool,
        x: usize,
        y: usize,
        columns: usize,
        color: u32,
    ) {
        let key = (ch, bold);
        if !self.glyphs.contains_key(&key) {
            if self.glyphs.len() >= CACHE_LIMIT {
                self.glyphs.clear();
            }
            let image = self.rasterizer.rasterize(ch, bold);
            self.glyphs.insert(key, image);
        }
        let Some(Some(image)) = self.glyphs.get(&key) else {
            return;
        };
        let (left, top) = self.cell_origin(x, y);
        let (right, bottom) = self.content_end();
        let origin_x = left as i32 + image.placement.left;
        let origin_y = top as i32 + self.baseline - image.placement.top;
        for gy in 0..image.placement.height as usize {
            let py = origin_y + gy as i32;
            if py < top as i32
                || py >= (top + self.cell_height) as i32
                || py >= bottom as i32
            {
                continue;
            }
            for gx in 0..image.placement.width as usize {
                let px = origin_x + gx as i32;
                if px < left as i32
                    || px >= (left + columns * self.cell_width) as i32
                    || px >= right as i32
                {
                    continue;
                }
                let i = gy * image.placement.width as usize + gx;
                let dst =
                    &mut self.pixels[py as usize * self.width + px as usize];
                let (fg, alpha) = match image.content {
                    ImageContent::Mask => (color, image.data[i]),
                    ImageContent::Color => {
                        let p = &image.data[i * 4..i * 4 + 4];
                        (
                            ((p[0] as u32) << 16)
                                | ((p[1] as u32) << 8)
                                | p[2] as u32,
                            p[3],
                        )
                    }
                    ImageContent::SubpixelMask => {
                        *dst = blend_rgb(
                            *dst,
                            color,
                            &image.data[i * 4..i * 4 + 3],
                        );
                        continue;
                    }
                };
                *dst = blend(*dst, fg, alpha);
            }
        }
    }
}

fn blend(bg: u32, fg: u32, alpha: u8) -> u32 {
    blend_rgb(bg, fg, &[alpha; 3])
}

fn blend_rgb(bg: u32, fg: u32, alpha: &[u8]) -> u32 {
    let channel = |shift: u32, a: u8| {
        let a = a as u32;
        ((((fg >> shift) & 255) * a + ((bg >> shift) & 255) * (255 - a) + 127)
            / 255)
            << shift
    };
    channel(16, alpha[0]) | channel(8, alpha[1]) | channel(0, alpha[2])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::BACKGROUND;

    #[test]
    fn rgb_masks_blend_each_channel_independently() {
        assert_eq!(blend_rgb(0, 0xffffff, &[64, 128, 192]), 0x4080c0);
        assert_eq!(blend_rgb(0x123456, 0xabcdef, &[0; 3]), 0x123456);
        assert_eq!(blend_rgb(0x123456, 0xabcdef, &[255; 3]), 0xabcdef);
        assert_eq!(blend(0, 0xffffff, 128), 0x808080);
    }

    #[test]
    fn baseline_leaves_room_for_underscores_and_descenders() {
        for size in [16.0, 20.0, 32.0] {
            let mut renderer = Renderer::new(size).unwrap();
            let mut terminal = Terminal::new(6, 2);
            terminal.cursor_visible = false;
            terminal.feed(b"_gjpqy");
            let width = renderer.cell_width * 6;
            let height = renderer.cell_height * 2;
            renderer.draw(&terminal, width as u32, height as u32, "");
            for x in 0..renderer.cell_width {
                if (renderer.baseline as usize..renderer.cell_height)
                    .any(|y| renderer.pixels[y * width + x] != BACKGROUND)
                {
                    break;
                }
                assert!(
                    x + 1 < renderer.cell_width,
                    "underscore clipped at font size {size}"
                );
            }
            for ch in "_gjpqy".chars() {
                let image = renderer.glyphs[&(ch, false)].as_ref().unwrap();
                let bottom = renderer.baseline - image.placement.top
                    + image.placement.height as i32;
                assert!(
                    bottom <= renderer.cell_height as i32,
                    "descender clipped for {ch} at font size {size}"
                );
            }
        }
    }

    #[test]
    fn preedit_redraw_is_stable_with_hidden_cursor() {
        let mut renderer = Renderer::new(16.0).unwrap();
        let mut terminal = Terminal::new(10, 3);
        terminal.cursor_visible = false;
        renderer.draw(&terminal, 200, 100, "abc");
        let first = renderer.pixels.clone();
        let cache_size = renderer.glyphs.len();
        assert!(first.iter().any(|p| *p != BACKGROUND));
        renderer.draw(&terminal, 200, 100, "abc");
        assert_eq!(first, renderer.pixels);
        assert_eq!(cache_size, renderer.glyphs.len());
        terminal.feed(b"\x1b[2;1H");
        renderer.draw(&terminal, 200, 100, "abc");
        let incremental = renderer.pixels.clone();
        renderer.invalidate();
        renderer.draw(&terminal, 200, 100, "abc");
        assert_eq!(incremental, renderer.pixels);
        renderer.draw(&terminal, 200, 100, "");
        assert!(renderer.pixels.iter().all(|p| *p == BACKGROUND));
    }
    #[test]
    fn padding_preserves_grid_pixels_and_custom_background() {
        use crate::config::Config;
        let config: Config = Config::parse(
            r##"
colors:
  primary:
    background: "#123456"
  cursor:
    cursor: "#abcdef"
  normal:
    blue: "#6272a4"
"##,
        )
        .unwrap();
        let theme = config.colors.theme();
        let mut plain = Renderer::new(17.0 * 96.0 / 72.0).unwrap();
        let mut padded = Renderer::with_options(
            17.0 * 96.0 / 72.0,
            (3, 5),
            false,
            &crate::config::Font::default().family,
        )
        .unwrap();
        let width = plain.cell_width * 8;
        let height = plain.cell_height * 2;
        let padded_width = width + 6;
        let padded_height = height + 10;
        assert_eq!(
            padded.grid_size(padded_width as u32, padded_height as u32,),
            (8, 2)
        );
        assert_eq!(
            padded.cell_origin(1, 1),
            (3 + plain.cell_width, 5 + plain.cell_height,)
        );
        let mut terminal = Terminal::with_theme(8, 2, theme);
        terminal.feed("\x1b[44;4mABC中\x1b[0m".as_bytes());
        for preedit in ["a", "中", ""] {
            plain.draw(&terminal, width as u32, height as u32, preedit);
            padded.draw(
                &terminal,
                padded_width as u32,
                padded_height as u32,
                preedit,
            );
            for y in 0..padded_height {
                for x in 0..padded_width {
                    let expected = if (3..width + 3).contains(&x)
                        && (5..height + 5).contains(&y)
                    {
                        plain.pixels[(y - 5) * width + x - 3]
                    } else {
                        theme.background
                    };
                    assert_eq!(padded.pixels[y * padded_width + x], expected);
                }
            }
        }
        assert!(padded.pixels.contains(&theme.cursor));
        assert!(padded.pixels.contains(&0x6272a4));
        assert_eq!(padded.grid_size(1, 1), (1, 1));
        padded.draw(&terminal, 1, 1, "中");
        assert_eq!(padded.pixels, vec![theme.background]);
    }
}
