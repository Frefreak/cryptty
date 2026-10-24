use bytemuck::{Pod, Zeroable};
use unicode_width::UnicodeWidthChar;

use super::{
    Renderer,
    atlas::Atlas,
    decoration::{cursor_layout, cursor_rects, decorations, vim_frame},
};
use crate::terminal::{Content, CursorShape, FontStyle, Terminal};

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Pod, Zeroable)]
pub(super) struct Quad {
    rect: [f32; 4],
    uv: [f32; 4],
    color: [f32; 4],
    kind: u32,
    padding: [u32; 3],
}

pub(super) struct Frame {
    pub quads: Vec<Quad>,
    pub overlay: usize,
    pub backgrounds: usize,
}

#[derive(Default)]
pub(super) struct Cache {
    key: Option<CacheKey>,
    rows: Vec<CachedRow>,
    pub spare: Vec<Quad>,
    #[cfg(test)]
    pub rebuilt_rows: usize,
}

#[derive(PartialEq)]
struct CacheKey {
    size: (u32, u32),
    cell: (usize, usize),
    padding: (usize, usize),
    baseline: i32,
    colors: u64,
    images: u64,
    blink: u8,
    selection: Option<crate::terminal::TextRange>,
    hovered: Vec<crate::terminal::TextRange>,
}

struct CachedRow {
    row: crate::terminal::Row,
    backgrounds: Vec<Quad>,
    text: Vec<Quad>,
}

impl Cache {
    pub fn clear(&mut self) {
        self.rows.clear();
    }
}

impl Quad {
    pub fn image(tile: &super::images::Tile<'_>) -> Self {
        Self {
            rect: tile.rect,
            uv: tile.uv,
            kind: 3,
            ..Self::zeroed()
        }
    }
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
        self.solid_alpha(x, y, w, h, rgb, 255);
    }

    fn solid_alpha(
        &mut self,
        x: usize,
        y: usize,
        w: usize,
        h: usize,
        rgb: u32,
        alpha: u8,
    ) {
        let right = (x + w).min(self.end.0);
        let bottom = (y + h).min(self.end.1);
        if right <= x || bottom <= y {
            return;
        }
        let mut rgba = color(rgb);
        rgba[3] = alpha as f32 / 255.0;
        self.quads.push(Quad {
            rect: [x as f32, y as f32, (right - x) as f32, (bottom - y) as f32],
            color: rgba,
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
        if ch == crate::graphics::MARKER {
            return Some(());
        }
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
    cache: &mut Cache,
) -> Option<Frame> {
    let key = CacheKey {
        size,
        cell: (renderer.cell_width, renderer.cell_height),
        padding: renderer.padding,
        baseline: renderer.baseline,
        colors: terminal.color_revision,
        images: terminal.graphics.revision,
        blink: terminal.text_blink_phase,
        selection: terminal.selection,
        hovered: terminal.hovered_link.clone(),
    };
    if cache.key.as_ref() != Some(&key) {
        cache.clear();
        cache.key = Some(key);
    }
    cache.rows.truncate(terminal.rows.len());
    #[cfg(test)]
    {
        cache.rebuilt_rows = 0;
    }
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
        if cache.rows.get(y).is_some_and(|cached| cached.row == *row) {
            continue;
        }
        #[cfg(test)]
        {
            cache.rebuilt_rows += 1;
        }
        scene.quads = cache
            .rows
            .get_mut(y)
            .map(|cached| std::mem::take(&mut cached.backgrounds))
            .unwrap_or_default();
        scene.quads.clear();
        for x in 0..row.len() {
            if let Some(bg) = super::cell_background(terminal, x, y) {
                let (left, top) = scene.renderer.cell_origin(x, y);
                scene.solid(left, top, w, h, bg);
            }
        }
        let backgrounds = std::mem::take(&mut scene.quads);
        scene.quads = cache
            .rows
            .get_mut(y)
            .map(|cached| std::mem::take(&mut cached.text))
            .unwrap_or_default();
        scene.quads.clear();
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
                if text.chars().next() == Some(crate::graphics::MARKER) {
                    break;
                }
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
        let text = std::mem::take(&mut scene.quads);
        if let Some(cached) = cache.rows.get_mut(y) {
            cached.row.clone_from(row);
            cached.backgrounds = backgrounds;
            cached.text = text;
        } else {
            cache.rows.push(CachedRow {
                row: row.clone(),
                backgrounds,
                text,
            });
        }
    }
    scene.quads = std::mem::take(&mut cache.spare);
    scene.quads.clear();
    for row in &cache.rows {
        scene.quads.extend_from_slice(&row.backgrounds);
    }
    let backgrounds = scene.quads.len();
    for row in &cache.rows {
        scene.quads.extend_from_slice(&row.text);
    }
    let overlay = scene.quads.len();
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
            terminal.theme().palette[2],
        )?;
        x += columns;
    }
    if terminal.cursor_drawn() {
        let (x, y, columns, shape) = cursor_layout(terminal, preedit);
        let (left, top) = scene.renderer.cell_origin(x, y);
        for [dx, dy, cw, ch] in cursor_rects(
            shape,
            w,
            h,
            columns,
            terminal.vim_mode && preedit.is_empty(),
        ) {
            scene.solid(left + dx, top + dy, cw, ch, terminal.theme().cursor);
        }
        let cell = &terminal.rows[y][x];
        if !terminal.vim_mode
            && shape == CursorShape::Block
            && terminal.text_visible(cell.style())
            && let Content::Text { text } = &cell.content
        {
            let (_, bg) = terminal.colors(cell.style());
            for ch in text.chars().filter(|c| *c != ' ') {
                if text.chars().next() == Some(crate::graphics::MARKER) {
                    break;
                }
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
    if terminal.vim_mode {
        for [x, y, w, h] in vim_frame(
            (size.0 as usize, size.1 as usize),
            scene.renderer.padding,
            h,
        ) {
            scene.solid(x, y, w, h, terminal.theme().palette[5]);
        }
    }
    for rect in scene
        .renderer
        .marker_rects((size.0 as usize, size.1 as usize))
    {
        let [x, y, w, h] = rect.bounds;
        scene.solid_alpha(x, y, w, h, rect.color, rect.alpha);
    }
    if let Some(layout) = scene
        .renderer
        .overlay_layout((size.0 as usize, size.1 as usize))
    {
        let (left, top) = scene.renderer.cell_origin(layout.col, layout.row);
        let width = layout.cols * w;
        let height = layout.rows * h;
        scene.solid(left, top, width, height, terminal.theme().foreground);
        scene.solid(
            left + 1,
            top + 1,
            width.saturating_sub(2),
            height.saturating_sub(2),
            terminal.theme().background,
        );
        for (ch, col, row, columns) in layout.glyphs {
            scene.glyph(
                ch,
                FontStyle::default(),
                (col, row),
                columns,
                terminal.theme().foreground,
            )?;
        }
    }
    Some(Frame {
        quads: scene.quads,
        overlay,
        backgrounds,
    })
}
