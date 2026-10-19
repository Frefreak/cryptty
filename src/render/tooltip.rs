use unicode_width::UnicodeWidthChar;

use super::Renderer;

#[derive(PartialEq, Eq)]
pub(super) struct Tooltip {
    uri: String,
    pointer: (usize, usize),
}

pub(super) struct Layout {
    pub col: usize,
    pub row: usize,
    pub cols: usize,
    pub rows: usize,
    pub glyphs: Vec<(char, usize, usize, usize)>,
}

impl Renderer {
    pub fn set_notice(&mut self, message: Option<&str>) {
        self.notice = message
            .map(|text| text.split_whitespace().collect::<Vec<_>>().join(" "));
        self.invalidate();
    }

    pub fn set_link_tooltip(
        &mut self,
        uri: Option<&str>,
        pointer: (usize, usize),
    ) -> bool {
        let next = uri.map(|uri| Tooltip {
            uri: uri.into(),
            pointer,
        });
        if self.tooltip == next {
            return false;
        }
        self.tooltip = next;
        self.invalidate();
        true
    }

    pub(super) fn overlay_layout(
        &self,
        size: (usize, usize),
    ) -> Option<Layout> {
        let cols = size.0.saturating_sub(2 * self.padding.0) / self.cell_width;
        let rows = size.1.saturating_sub(2 * self.padding.1) / self.cell_height;
        if let Some(message) = &self.notice {
            return layout(message, (0, rows), (cols, rows), 8);
        }
        let tooltip = self.tooltip.as_ref()?;
        layout(&tooltip.uri, tooltip.pointer, (cols, rows), 4)
    }
}

fn layout(
    uri: &str,
    pointer: (usize, usize),
    size: (usize, usize),
    max_rows: usize,
) -> Option<Layout> {
    if size.0 < 4 || size.1 < 2 {
        return None;
    }
    let width = size.0.min(80) - 2;
    let max_rows = size.1.saturating_sub(1).min(max_rows);
    let mut glyphs = Vec::new();
    let (mut x, mut y) = (0, 0);
    let mut used = 0;
    for ch in uri.chars() {
        let ch = if ch.is_control() { '�' } else { ch };
        let columns = ch.width().unwrap_or(1).max(1);
        if x + columns > width {
            y += 1;
            x = 0;
        }
        if y == max_rows {
            let (_, col, row, _) = glyphs.pop()?;
            glyphs.push(('…', col, row, 1));
            break;
        }
        glyphs.push((ch, x, y, columns));
        x += columns;
        used = used.max(x);
    }
    let rows = glyphs.last()?.2 + 1;
    let cols = used + 2;
    let col = pointer.0.saturating_add(1).min(size.0 - cols);
    let row = if pointer.1.saturating_add(1 + rows) <= size.1 {
        pointer.1 + 1
    } else {
        pointer.1.saturating_sub(rows).min(size.1 - rows)
    };
    for (_, x, y, _) in &mut glyphs {
        *x += col + 1;
        *y += row;
    }
    Some(Layout {
        col,
        row,
        cols,
        rows,
        glyphs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tooltip_wraps_unicode_and_stays_inside_the_viewport() {
        for pointer in [(0, 0), (19, 5), (100, 100)] {
            let result = layout(
                &format!("file:///中文/{}", "abc".repeat(80)),
                pointer,
                (20, 6),
                4,
            )
            .unwrap();
            assert!(result.col + result.cols <= 20);
            assert!(result.row + result.rows <= 6);
            assert_eq!(result.glyphs.last().unwrap().0, '…');
            for (_, x, y, columns) in result.glyphs {
                assert!(x + columns < result.col + result.cols);
                assert!(y < result.row + result.rows);
            }
        }
        assert!(layout("file:///tmp", (0, 0), (3, 1), 4).is_none());
    }

    #[test]
    fn notice_overrides_hover_and_clears_without_changing_terminal() {
        let mut renderer = Renderer::new(16.0).unwrap();
        let terminal = crate::terminal::Terminal::new(40, 10);
        renderer.set_link_tooltip(Some("file:///tmp"), (2, 2));
        renderer.draw(&terminal, 640, 320, "");
        let before = renderer.pixels.clone();
        renderer
            .set_notice(Some("Config reload failed:\nwindow: invalid value"));
        let layout = renderer.overlay_layout((640, 320)).unwrap();
        let text: String = layout.glyphs.iter().map(|g| g.0).collect();
        assert_eq!(text, "Config reload failed: window: invalid value");
        renderer.draw(&terminal, 640, 320, "");
        assert_ne!(renderer.pixels, before);
        renderer.set_notice(None);
        renderer.draw(&terminal, 640, 320, "");
        assert_eq!(renderer.pixels, before);
    }

    #[test]
    fn dismissing_tooltip_restores_software_pixels() {
        let mut renderer = Renderer::new(16.0).unwrap();
        let terminal = crate::terminal::Terminal::new(40, 10);
        renderer.draw(&terminal, 640, 320, "");
        let before = renderer.pixels.clone();
        renderer.set_link_tooltip(Some("file:///tmp/test"), (5, 2));
        renderer.draw(&terminal, 640, 320, "");
        assert_ne!(renderer.pixels, before);
        renderer.set_link_tooltip(None, (5, 2));
        renderer.draw(&terminal, 640, 320, "");
        assert_eq!(renderer.pixels, before);
    }
}
