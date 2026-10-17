use std::collections::HashMap;

use anyhow::Result;
use swash::scale::image::{Content as ImageContent, Image};
use unicode_width::UnicodeWidthChar;

use crate::terminal::{Content, CursorShape, FontStyle, Terminal};
use decoration::{cursor_layout, cursor_rect, decorations};

#[cfg(target_os = "macos")]
#[path = "render/native.rs"]
mod font;
#[cfg(not(target_os = "macos"))]
#[path = "render/swash.rs"]
mod font;

mod atlas;
mod decoration;
mod generated;
pub mod gpu;
#[cfg(test)]
mod gpu_tests;
mod images;
#[cfg(target_os = "macos")]
mod native_fallback;
#[cfg(target_os = "macos")]
mod native_family;
mod normalization;
mod painter;
mod scene;
#[cfg(all(test, target_os = "macos"))]
#[path = "render/swash.rs"]
mod swash_tests;

const CACHE_LIMIT: usize = 4096;
const PREEDIT_COLOR: u32 = 0x6366f1;

pub struct Renderer {
    rasterizer: font::Rasterizer,
    glyphs: HashMap<(char, FontStyle), Option<Image>>,
    padding: (usize, usize),
    pub cell_width: usize,
    pub cell_height: usize,
    baseline: i32,
    width: usize,
    height: usize,
    pub pixels: Vec<u32>,
    previous: std::collections::VecDeque<crate::terminal::Row>,
    color_revision: u64,
    image_revision: u64,
    text_blink_phase: u8,
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
            &[],
        )
    }

    pub fn with_options(
        size: f32,
        padding: (u32, u32),
        hinting: bool,
        family: &str,
        mappings: &[crate::config::FontMapping],
    ) -> Result<Self> {
        let rasterizer =
            font::Rasterizer::new(size, hinting, family, mappings)?;
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
            previous: std::collections::VecDeque::new(),
            color_revision: 0,
            image_revision: 0,
            text_blink_phase: 0,
            old_cursor: None,
            old_preedit_row: None,
        })
    }

    pub fn grid_size(&self, width: u32, height: u32) -> (usize, usize) {
        (
            ((width as usize).saturating_sub(2 * self.padding.0)
                / self.cell_width)
                .max(2),
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
        if self.image_revision != terminal.graphics.revision {
            self.image_revision = terminal.graphics.revision;
            self.invalidate();
        }
        if self.color_revision != terminal.color_revision
            || self.text_blink_phase != terminal.text_blink_phase
        {
            self.color_revision = terminal.color_revision;
            self.text_blink_phase = terminal.text_blink_phase;
            self.invalidate();
        }
        if self.previous.is_empty() {
            self.pixels.fill(terminal.theme().background);
        }
        let cursor = terminal.cursor_drawn().then_some(terminal.cursor);
        let preedit_row = (!preedit.is_empty()).then_some(terminal.cursor.1);
        for (y, row) in terminal.rows.iter().enumerate() {
            let cursor_row = cursor.map(|c| c.1) == Some(y)
                || self.old_cursor.map(|c| c.1) == Some(y)
                || preedit_row == Some(y)
                || self.old_preedit_row == Some(y);
            if self.previous.get(y) == Some(row) && !cursor_row {
                continue;
            }
            self.draw_row(y, terminal);
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
                    FontStyle::default(),
                    x,
                    y,
                    columns,
                    PREEDIT_COLOR,
                );
                x += columns;
            }
        }
        if cursor.is_some() {
            let (x, y, columns, shape) = cursor_layout(terminal, preedit);
            let (left, top) = self.cell_origin(x, y);
            let [dx, dy, w, h] =
                cursor_rect(shape, self.cell_width, self.cell_height, columns);
            self.solid(left + dx, top + dy, w, h, terminal.theme().cursor);
            let cell = &terminal.rows[y][x];
            if shape == CursorShape::Block
                && terminal.text_visible(cell.style())
                && let Content::Text { text } = &cell.content
            {
                let (_, bg) = terminal.colors(cell.style());
                for ch in text.chars().filter(|c| *c != ' ') {
                    if text.chars().next() == Some(crate::graphics::MARKER) {
                        break;
                    }
                    self.paint_char(
                        ch,
                        cell.style().font(),
                        x,
                        y,
                        text.columns(),
                        bg,
                    );
                }
            }
        }
        self.previous.clone_from(&terminal.rows);
        self.old_cursor = cursor;
        self.old_preedit_row = preedit_row;
    }

    fn solid(&mut self, x: usize, y: usize, w: usize, h: usize, rgb: u32) {
        let (right, bottom) = self.content_end();
        for py in y..(y + h).min(bottom) {
            let start = py * self.width + x.min(right);
            let end = py * self.width + (x + w).min(right);
            self.pixels[start..end].fill(rgb);
        }
    }

    fn draw_row(&mut self, y: usize, terminal: &Terminal) {
        let row = &terminal.rows[y];
        let (_, top) = self.cell_origin(0, y);
        for x in 0..row.len() {
            let (_, bg) = terminal.colors(terminal.cell_style(x, y));
            let (left, _) = self.cell_origin(x, y);
            self.solid(left, top, self.cell_width, self.cell_height, bg);
        }
        for (x, cell) in row.iter().enumerate() {
            let style = terminal.cell_style(x, y);
            if !terminal.text_visible(style) {
                continue;
            }
            if let Content::Text { text } = &cell.content {
                let (fg, _) = terminal.colors(style);
                let columns = text.columns();
                for ch in text.chars().filter(|c| *c != ' ') {
                    if text.chars().next() == Some(crate::graphics::MARKER) {
                        break;
                    }
                    self.paint_char(ch, cell.style().font(), x, y, columns, fg);
                }
                let (left, _) = self.cell_origin(x, y);
                let underline = style
                    .underline_color
                    .map_or(fg, |color| terminal.color(color));
                decorations(
                    style,
                    self.cell_width * columns,
                    self.cell_height,
                    |x, y, w, h, is_underline| {
                        self.solid(
                            left + x,
                            top + y,
                            w,
                            h,
                            if is_underline { underline } else { fg },
                        );
                    },
                );
            }
        }
        for tile in images::tiles(
            terminal,
            row,
            (self.cell_width, self.cell_height),
            self.cell_origin(0, y),
            self.content_end(),
        ) {
            let [x, y, w, h] = tile.rect;
            let [u, v, uw, vh] = tile.uv;
            for py in 0..(h - 0.5).ceil().max(0.0) as usize {
                for px in 0..(w - 0.5).ceil().max(0.0) as usize {
                    let sx = (u + (px as f32 + 0.5) * uw / w + 0.0001) as u32;
                    let sy = (v + (py as f32 + 0.5) * vh / h + 0.0001) as u32;
                    if sx >= tile.image.width || sy >= tile.image.height {
                        continue;
                    }
                    let at = (sy * tile.image.width + sx) as usize * 4;
                    let rgba = &tile.image.rgba[at..at + 4];
                    let index =
                        (y as usize + py) * self.width + x as usize + px;
                    let rgb = u32::from(rgba[0]) << 16
                        | u32::from(rgba[1]) << 8
                        | u32::from(rgba[2]);
                    self.pixels[index] =
                        blend(self.pixels[index], rgb, rgba[3]);
                }
            }
        }
    }

    fn paint_char(
        &mut self,
        ch: char,
        font: FontStyle,
        x: usize,
        y: usize,
        columns: usize,
        color: u32,
    ) {
        self.cache_glyph(ch, font);
        let key = (ch, font);
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

    fn cache_glyph(&mut self, ch: char, font: FontStyle) {
        let key = (ch, font);
        if !self.glyphs.contains_key(&key) {
            if self.glyphs.len() >= CACHE_LIMIT {
                self.glyphs.clear();
            }
            let image = generated::rasterize(
                ch,
                self.cell_width,
                self.cell_height,
                self.baseline,
            )
            .or_else(|| self.rasterizer.rasterize(ch, font));
            self.glyphs.insert(key, image);
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
    fn generated_glyphs_override_mappings_and_font_styles() {
        let config = crate::config::Config::parse(
            "font: {codepoint_map: [{ranges: ['U+2500-U+259F'], family: Menlo}]}",
        ).unwrap();
        let mut renderer = Renderer::with_options(
            23.75,
            (0, 0),
            false,
            &config.font.family,
            &config.font.codepoint_map,
        )
        .unwrap();
        for ch in ['█', '░', '╬', '╭', '▚'] {
            for style in [
                FontStyle::default(),
                FontStyle {
                    bold: true,
                    italic: true,
                },
            ] {
                renderer.cache_glyph(ch, style);
                let actual = renderer.glyphs[&(ch, style)].as_ref().unwrap();
                let expected = generated::rasterize(
                    ch,
                    renderer.cell_width,
                    renderer.cell_height,
                    renderer.baseline,
                )
                .unwrap();
                assert_eq!(actual.data, expected.data);
                assert_eq!(actual.placement.top, renderer.baseline);
            }
        }
    }

    #[test]
    fn rgb_masks_blend_each_channel_independently() {
        assert_eq!(blend_rgb(0, 0xffffff, &[64, 128, 192]), 0x4080c0);
        assert_eq!(blend_rgb(0x123456, 0xabcdef, &[0; 3]), 0x123456);
        assert_eq!(blend_rgb(0x123456, 0xabcdef, &[255; 3]), 0xabcdef);
        assert_eq!(blend(0, 0xffffff, 128), 0x808080);
    }

    #[test]
    fn block_cursor_covers_wide_glyph_and_preserves_preedit() {
        let theme = crate::terminal::Theme {
            cursor: 0x1234ab,
            ..Default::default()
        };
        let mut terminal = Terminal::with_theme(6, 2, theme);
        let mut renderer = Renderer::new(19.0).unwrap();
        let (w, h) = (renderer.cell_width, renderer.cell_height);
        terminal.feed("中\x1b[H\x1b[2 q".as_bytes());
        renderer.draw(&terminal, (w * 6) as u32, (h * 2) as u32, "");
        let first = renderer.pixels.clone();
        let mut right_half = 0;
        for y in 0..h {
            for x in w..w * 2 {
                right_half +=
                    usize::from(renderer.pixels[y * w * 6 + x] == theme.cursor);
            }
        }
        assert!(right_half > w * h / 2);
        terminal.feed(b"\x1b[1;2H");
        renderer.draw(&terminal, (w * 6) as u32, (h * 2) as u32, "");
        assert_eq!(renderer.pixels, first);
        terminal.feed(b"\x1b[H");
        renderer.draw(&terminal, (w * 6) as u32, (h * 2) as u32, "abc");
        let composed = renderer.pixels.clone();
        terminal.cursor_visible = false;
        renderer.draw(&terminal, (w * 6) as u32, (h * 2) as u32, "abc");
        for y in 0..h {
            let start = y * w * 6;
            assert_eq!(
                &renderer.pixels[start..start + w * 3],
                &composed[start..start + w * 3]
            );
        }
    }

    #[test]
    fn osc_palette_changes_invalidate_cached_rows() {
        let mut terminal = Terminal::new(4, 2);
        terminal.cursor_visible = false;
        terminal.feed(b"\x1b[41mX");
        let mut renderer = Renderer::new(19.0).unwrap();
        renderer.draw(&terminal, 120, 80, "");
        let first = renderer.pixels.clone();
        terminal.feed(b"\x1b]4;1;#010203\x07\x1b]11;#040506\x07");
        renderer.draw(&terminal, 120, 80, "");
        let incremental = renderer.pixels.clone();
        assert_ne!(first, incremental);
        renderer.invalidate();
        renderer.draw(&terminal, 120, 80, "");
        assert_eq!(renderer.pixels, incremental);
    }

    #[test]
    fn baseline_leaves_room_for_underscores_and_descenders() {
        let defaults = crate::config::Font::default();
        let fallback = if cfg!(target_os = "macos") {
            "Menlo"
        } else {
            "monospace"
        };
        for family in [defaults.family.as_str(), fallback] {
            for size in [16.0, 19.0, 20.0, 23.75, 32.0, 38.0] {
                let mut renderer =
                    Renderer::with_options(size, (0, 0), false, family, &[])
                        .unwrap();
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
                    let image = renderer.glyphs[&(ch, FontStyle::default())]
                        .as_ref()
                        .unwrap();
                    let bottom = renderer.baseline - image.placement.top
                        + image.placement.height as i32;
                    assert!(
                        bottom <= renderer.cell_height as i32,
                        "descender clipped for {ch} at font size {size}"
                    );
                }
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
            &[],
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
        assert_eq!(padded.grid_size(1, 1), (2, 1));
        padded.draw(&terminal, 1, 1, "中");
        assert_eq!(padded.pixels, vec![theme.background]);
    }
}
