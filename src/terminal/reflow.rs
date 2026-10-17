use std::collections::VecDeque;

use super::{Cell, Content, Row, SavedCursor, Style};

pub(super) const HISTORY_LIMIT: usize = 10_000;

type Point = (usize, usize);

pub(super) fn resize_primary(
    screen: &mut VecDeque<Row>,
    history: &mut VecDeque<Row>,
    cursor: &mut Point,
    pending_wrap: &mut bool,
    saved: &mut SavedCursor,
    graphics: &mut crate::graphics::Graphics,
    cols: usize,
    rows: usize,
) {
    let old_cols = screen[0].len();
    if rows < screen.len() {
        let removed = (cursor.1 + 1).saturating_sub(rows);
        history.extend(screen.drain(..removed));
        cursor.1 -= removed;
        for p in &mut graphics.positioned {
            p.row -= removed as isize;
        }
        saved.position.1 = saved.position.1.saturating_sub(removed);
        if screen.len() > rows {
            screen[rows - 1].wrapped = None;
        }
        screen.truncate(rows);
    } else {
        let pulled = (rows - screen.len()).min(history.len());
        let mut prefix: VecDeque<_> =
            history.drain(history.len() - pulled..).collect();
        prefix.append(screen);
        *screen = prefix;
        cursor.1 += pulled;
        for p in &mut graphics.positioned {
            p.row += pulled as isize;
        }
        saved.position.1 += pulled;
        screen.resize_with(rows, || Row::new(old_cols, Style::default()));
    }
    saved.position.1 = saved.position.1.min(rows - 1);
    if old_cols != cols {
        let history_len = history.len();
        let mut anchors = vec![
            (
                cursor.0 + usize::from(*pending_wrap),
                cursor.1 + history_len,
            ),
            (
                saved.position.0 + usize::from(saved.pending_wrap),
                saved.position.1 + history_len,
            ),
        ];
        let image_indices: Vec<_> = graphics
            .positioned
            .iter()
            .enumerate()
            .filter_map(|(i, p)| {
                let row = p.row + history_len as isize;
                (row >= 0 && row < (history_len + screen.len()) as isize)
                    .then_some(i)
            })
            .collect();
        anchors.extend(image_indices.iter().map(|&i| {
            let p = &graphics.positioned[i];
            (p.col.min(old_cols), (p.row + history_len as isize) as usize)
        }));
        let source = history.drain(..).chain(screen.drain(..));
        let (mut output, mut mapped) = reflow(source, &anchors, cols);
        for point in &mut mapped {
            if point.0 == cols && output[point.1].wrapped.is_some() {
                *point = (0, point.1 + 1);
            }
        }
        let start = output.len().saturating_sub(rows);
        // Keep the active cursor visible when content below it expands.
        let start = start.min(mapped[0].1);
        if output.len() > start + rows {
            output[start + rows - 1].wrapped = None;
            output.truncate(start + rows);
        }
        let visible = output.split_off(start);
        history.extend(output);
        *screen = visible.into();
        screen.resize_with(rows, || Row::new(cols, Style::default()));
        (*cursor, *pending_wrap) =
            cursor_position(mapped[0], start, cols, rows);
        (saved.position, saved.pending_wrap) =
            cursor_position(mapped[1], start, cols, rows);
        for (&i, point) in image_indices.iter().zip(&mapped[2..]) {
            let p = &mut graphics.positioned[i];
            p.col = point.0;
            p.row = point.1 as isize - start as isize;
        }
    }
    let excess = history.len().saturating_sub(HISTORY_LIMIT);
    history.drain(..excess);
    if !graphics.positioned.is_empty() {
        graphics.positioned.retain(|p| {
            p.row < rows as isize && p.row >= -(HISTORY_LIMIT as isize)
        });
        graphics.revision = graphics.revision.wrapping_add(1);
    }
}

fn cursor_position(
    point: Point,
    start: usize,
    cols: usize,
    rows: usize,
) -> (Point, bool) {
    (
        (
            point.0.min(cols - 1),
            point.1.saturating_sub(start).min(rows - 1),
        ),
        point.0 == cols,
    )
}

fn reflow(
    source: impl Iterator<Item = Row>,
    anchors: &[Point],
    cols: usize,
) -> (Vec<Row>, Vec<Point>) {
    let blank = Cell::blank(Style::default());
    let mut output = Vec::new();
    let mut row = Row::new(cols, Style::default());
    let mut x = 0;
    let mut mapped = vec![(0, 0); anchors.len()];
    let mut continuing = false;
    for (y, source) in source.enumerate() {
        let mut end = source.wrapped.unwrap_or_else(|| {
            source
                .iter()
                .rposition(|cell| cell != &blank)
                .map_or(0, |i| i + 1)
        });
        for &(col, line) in anchors {
            if line == y {
                end = end.max(col.min(source.len()));
            }
        }
        for i in 0..=end {
            let cell = source.get(i).filter(|_| i < end);
            let width = match cell.map(|cell| &cell.content) {
                Some(Content::Text { text }) => text.columns(),
                _ => 0,
            };
            if width > 0 && x + width > cols && x > 0 {
                row.wrapped = Some(x);
                output.push(row);
                row = Row::new(cols, Style::default());
                x = 0;
            }
            for (index, &(col, line)) in anchors.iter().enumerate() {
                if line == y && col == i {
                    let continuation = matches!(
                        cell.map(|cell| &cell.content),
                        Some(Content::Continuation)
                    );
                    mapped[index] = (
                        x.saturating_sub(usize::from(continuation)),
                        output.len(),
                    );
                }
            }
            let Some(cell) = cell else { continue };
            if width == 0 {
                continue;
            }
            row[x] = cell.clone();
            if width == 2 {
                row[x + 1] = source[i + 1].clone();
            }
            x += width;
        }
        continuing = source.wrapped.is_some();
        if !continuing {
            output.push(row);
            row = Row::new(cols, Style::default());
            x = 0;
        }
    }
    if continuing {
        output.push(row);
    }
    (output, mapped)
}
