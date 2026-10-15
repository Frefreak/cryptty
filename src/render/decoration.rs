use crate::terminal::{Content, CursorShape, Style, Terminal, Underline};
use unicode_width::UnicodeWidthChar;

pub(super) fn cursor_layout(
    terminal: &Terminal,
    preedit: &str,
) -> (usize, usize, usize, CursorShape) {
    let (mut x, y) = terminal.cursor;
    if !preedit.is_empty() {
        x += preedit
            .chars()
            .map(|c| c.width().unwrap_or(1).max(1))
            .sum::<usize>();
        return (x.min(terminal.cols() - 1), y, 1, CursorShape::Beam);
    }
    if matches!(terminal.rows[y][x].content, Content::Continuation) {
        x = x.saturating_sub(1);
    }
    let columns = if matches!(
        terminal.rows[y][x].content,
        Content::Text { wide: true, .. }
    ) {
        2
    } else {
        1
    };
    (x, y, columns, terminal.cursor_style.shape)
}

pub(super) fn cursor_rect(
    shape: CursorShape,
    w: usize,
    h: usize,
) -> [usize; 4] {
    match shape {
        CursorShape::Block => [0, 0, w, h],
        CursorShape::Underline => [0, h.saturating_sub(2), w, h.min(2)],
        CursorShape::Beam => [0, 0, w.min(2), h],
    }
}

pub(super) fn decorations(
    style: Style,
    w: usize,
    h: usize,
    mut paint: impl FnMut(usize, usize, usize, usize, bool),
) {
    let thickness = (h / 20).max(1);
    let y = h.saturating_sub(thickness * 2);
    match style.underline {
        Underline::None => (),
        Underline::Single => paint(0, y, w, thickness, true),
        Underline::Double => {
            paint(0, y, w, thickness, true);
            paint(0, y.saturating_sub(thickness * 2), w, thickness, true);
        }
        Underline::Curly => {
            for x in 0..w {
                let phase = (x / thickness) % 4;
                let offset = [0, 1, 2, 1][phase] * thickness;
                paint(x, y.saturating_sub(offset), 1, thickness, true);
            }
        }
        Underline::Dotted | Underline::Dashed => {
            let length = if style.underline == Underline::Dotted {
                thickness
            } else {
                thickness * 3
            };
            for x in (0..w).step_by(length + thickness) {
                paint(x, y, length.min(w - x), thickness, true);
            }
        }
    }
    if style.strike {
        paint(0, h / 2, w, thickness, false);
    }
    if style.overline {
        paint(0, 0, w, thickness, false);
    }
}
