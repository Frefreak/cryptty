use bytemuck::{Pod, Zeroable};
use unicode_width::UnicodeWidthChar;

use super::{
    Renderer,
    atlas::Atlas,
    decoration::{cursor_layout, cursor_rect, decorations},
};
use crate::terminal::{Content, CursorShape, FontStyle, Terminal};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(super) struct Quad {
    rect: [f32; 4],
    uv: [f32; 4],
    color: [f32; 4],
    kind: u32,
    padding: [u32; 3],
}

pub(super) fn color(rgb: u32) -> [f32; 4] {
    [
        ((rgb >> 16) & 255) as f32 / 255.0,
        ((rgb >> 8) & 255) as f32 / 255.0,
        (rgb & 255) as f32 / 255.0,
        1.0,
    ]
}

struct Scene<'a> {
    renderer: &'a mut Renderer,
    atlas: &'a mut Atlas,
    queue: &'a wgpu::Queue,
    end: (usize, usize),
    quads: Vec<Quad>,
}

impl Scene<'_> {
    fn solid(&mut self, x: usize, y: usize, w: usize, h: usize, rgb: u32) {
        let right = (x + w).min(self.end.0);
        let bottom = (y + h).min(self.end.1);
        if right <= x || bottom <= y {
            return;
        }
        self.quads.push(Quad {
            rect: [x as f32, y as f32, (right - x) as f32, (bottom - y) as f32],
            color: color(rgb),
            ..Quad::zeroed()
        });
    }

    fn glyph(
        &mut self,
        ch: char,
        font: FontStyle,
        cell: (usize, usize),
        columns: usize,
        rgb: u32,
    ) -> Option<()> {
        self.renderer.cache_glyph(ch, font);
        let Some(image) = self.renderer.glyphs[&(ch, font)].as_ref() else {
            return Some(());
        };
        if image.placement.width == 0 || image.placement.height == 0 {
            return Some(());
        }
        let (left, top) = self.renderer.cell_origin(cell.0, cell.1);
        let origin_x = left as i64 + image.placement.left as i64;
        let origin_y = top as i64 + self.renderer.baseline as i64
            - image.placement.top as i64;
        let x = origin_x.max(left as i64);
        let y = origin_y.max(top as i64);
        let right = (origin_x + image.placement.width as i64)
            .min((left + columns * self.renderer.cell_width).min(self.end.0)
                as i64);
        let bottom = (origin_y + image.placement.height as i64)
            .min((top + self.renderer.cell_height).min(self.end.1) as i64);
        if right <= x || bottom <= y {
            return Some(());
        }
        let glyph = self.atlas.get(self.queue, (ch, font), image)?;
        self.quads.push(Quad {
            rect: [x as f32, y as f32, (right - x) as f32, (bottom - y) as f32],
            uv: [
                (glyph.origin[0] as i64 + x - origin_x) as f32,
                (glyph.origin[1] as i64 + y - origin_y) as f32,
                (right - x) as f32,
                (bottom - y) as f32,
            ],
            color: color(rgb),
            kind: if glyph.color { 2 } else { 1 },
            padding: [0; 3],
        });
        Some(())
    }
}

pub(super) fn build(
    renderer: &mut Renderer,
    atlas: &mut Atlas,
    queue: &wgpu::Queue,
    terminal: &Terminal,
    size: (u32, u32),
    preedit: &str,
) -> Option<Vec<Quad>> {
    let end = (
        (size.0 as usize).saturating_sub(renderer.padding.0),
        (size.1 as usize).saturating_sub(renderer.padding.1),
    );
    let (w, h) = (renderer.cell_width, renderer.cell_height);
    let mut scene = Scene {
        renderer,
        atlas,
        queue,
        end,
        quads: Vec::new(),
    };
    for (y, row) in terminal.rows.iter().enumerate() {
        for x in 0..row.len() {
            let (_, bg) = terminal.colors(terminal.cell_style(x, y));
            let (left, top) = scene.renderer.cell_origin(x, y);
            scene.solid(left, top, w, h, bg);
        }
        for (x, cell) in row.iter().enumerate() {
            let Content::Text { text } = &cell.content else {
                continue;
            };
            let style = terminal.cell_style(x, y);
            if !terminal.text_visible(style) {
                continue;
            }
            let (fg, _) = terminal.colors(style);
            let columns = text.columns();
            for ch in text.chars().filter(|c| *c != ' ') {
                scene.glyph(ch, cell.style().font(), (x, y), columns, fg)?;
            }
            let (left, top) = scene.renderer.cell_origin(x, y);
            let underline = style
                .underline_color
                .map_or(fg, |color| terminal.color(color));
            decorations(style, w * columns, h, |x, y, w, h, is_underline| {
                scene.solid(
                    left + x,
                    top + y,
                    w,
                    h,
                    if is_underline { underline } else { fg },
                );
            });
        }
    }
    let (mut x, y) = terminal.cursor;
    for ch in preedit.chars() {
        if x >= terminal.cols() {
            break;
        }
        let columns = ch.width().unwrap_or(1).max(1);
        scene.glyph(
            ch,
            FontStyle::default(),
            (x, y),
            columns,
            terminal.theme().palette[6],
        )?;
        x += columns;
    }
    if terminal.cursor_drawn() {
        let (x, y, columns, shape) = cursor_layout(terminal, preedit);
        let (left, top) = scene.renderer.cell_origin(x, y);
        let [dx, dy, cw, ch] = cursor_rect(shape, w * columns, h);
        scene.solid(left + dx, top + dy, cw, ch, terminal.theme().cursor);
        let cell = &terminal.rows[y][x];
        if shape == CursorShape::Block
            && terminal.text_visible(cell.style())
            && let Content::Text { text } = &cell.content
        {
            let (_, bg) = terminal.colors(cell.style());
            for ch in text.chars().filter(|c| *c != ' ') {
                scene.glyph(
                    ch,
                    cell.style().font(),
                    (x, y),
                    text.columns(),
                    bg,
                )?;
            }
        }
    }
    Some(scene.quads)
}
