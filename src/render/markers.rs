//! Pixel-based marker badges shared by the software and GPU renderers.
use std::collections::HashMap;

use swash::scale::image::{Content, Image};

use super::{Renderer, font};
use crate::marker::Target;
use crate::terminal::FontStyle;

#[derive(Clone, Copy)]
pub(super) struct Rect {
    pub bounds: [usize; 4],
    pub color: u32,
}

#[derive(Default)]
pub(super) struct Markers {
    targets: Vec<Target>,
    font: Option<font::Rasterizer>,
    glyphs: HashMap<char, Image>,
}

impl Renderer {
    pub fn set_markers(&mut self, targets: Vec<Target>) {
        if self.markers.targets != targets {
            self.markers.targets = targets;
            self.invalidate();
        }
    }

    pub(super) fn marker_rects(&mut self, size: (usize, usize)) -> Vec<Rect> {
        if self.markers.targets.is_empty() {
            return Vec::new();
        }
        if self.markers.font.is_none() {
            let family = if cfg!(target_os = "macos") {
                "Menlo"
            } else {
                "DejaVu Sans Mono"
            };
            self.markers.font = font::Rasterizer::new(
                (self.cell_height as f32 * 0.48).max(8.0),
                false,
                family,
                &[],
                &[],
            )
            .ok();
        }
        let Some(font) = &mut self.markers.font else {
            return Vec::new();
        };
        let padding = (font.cell_height / 5).max(2);
        let height = font.cell_height + padding * 2;
        let size = (
            size.0.saturating_sub(self.padding.0),
            size.1.saturating_sub(self.padding.1),
        );
        let mut rects = Vec::new();
        let mut badges = Vec::new();
        let mut occupied = Vec::new();
        for target in &self.markers.targets {
            for segment in &target.segments {
                let x = self.padding.0 + segment.start.col * self.cell_width;
                let y = self.padding.1 + segment.start.row * self.cell_height;
                let width = segment.columns * self.cell_width;
                let color = 0x89cbd4;
                rects.push(Rect {
                    bounds: [
                        x,
                        y + self.cell_height.saturating_sub(2),
                        width,
                        2,
                    ],
                    color,
                });
                rects.push(Rect {
                    bounds: [x, y + 2, 1, self.cell_height.saturating_sub(4)],
                    color,
                });
                rects.push(Rect {
                    bounds: [
                        x + width - 1,
                        y + 2,
                        1,
                        self.cell_height.saturating_sub(4),
                    ],
                    color,
                });
            }
            let Some(segment) = target.segments.first() else {
                continue;
            };
            let width = font.cell_width * target.label.len() + padding * 2;
            let anchor = (
                self.padding.0 + segment.start.col * self.cell_width,
                self.padding.1 + segment.start.row * self.cell_height,
            );
            let bounds = place(anchor, (width, height), size, &occupied);
            occupied.push(bounds);
            // Connector keeps displaced badges attached to their text range.
            let center = (bounds[0] + width / 2, bounds[1] + height / 2);
            rects.push(Rect {
                bounds: [
                    anchor.0.min(center.0),
                    anchor.1,
                    anchor.0.abs_diff(center.0) + 1,
                    1,
                ],
                color: 0x89cbd4,
            });
            rects.push(Rect {
                bounds: [
                    center.0,
                    anchor.1.min(center.1),
                    1,
                    anchor.1.abs_diff(center.1) + 1,
                ],
                color: 0x89cbd4,
            });
            rounded(&mut badges, bounds, padding, 0xe8d9a8);
            for (index, ch) in target.label.chars().enumerate() {
                if !self.markers.glyphs.contains_key(&ch)
                    && let Some(image) =
                        font.rasterize(ch, FontStyle::default())
                {
                    self.markers.glyphs.insert(ch, image);
                }
                if let Some(image) = self.markers.glyphs.get(&ch) {
                    glyph_rects(
                        &mut badges,
                        image,
                        (
                            bounds[0] + padding + index * font.cell_width,
                            bounds[1] + padding,
                        ),
                        font.baseline,
                    );
                }
            }
        }
        rects.extend(badges);
        rects
    }
}

fn place(
    anchor: (usize, usize),
    badge: (usize, usize),
    size: (usize, usize),
    occupied: &[[usize; 4]],
) -> [usize; 4] {
    let x = anchor.0.min(size.0.saturating_sub(badge.0));
    let y = anchor
        .1
        .saturating_sub(badge.1 / 2)
        .min(size.1.saturating_sub(badge.1));
    let first = [x, y, badge.0, badge.1];
    // Search nearest horizontal lanes, then adjacent rows.
    let step_x = badge.0 + 3;
    let step_y = badge.1 + 3;
    for row in 0..=size.1 / step_y {
        for direction in [1isize, -1] {
            let top = y as isize + direction * (row * step_y) as isize;
            if top < 0 || top as usize + badge.1 > size.1 {
                continue;
            }
            for column in 0..=size.0 / step_x {
                for side in [1isize, -1] {
                    let left = x as isize + side * (column * step_x) as isize;
                    if left < 0 || left as usize + badge.0 > size.0 {
                        continue;
                    }
                    let rect = [left as usize, top as usize, badge.0, badge.1];
                    if occupied.iter().all(|other| !intersects(rect, *other)) {
                        return rect;
                    }
                }
            }
        }
    }
    first
}

fn intersects(a: [usize; 4], b: [usize; 4]) -> bool {
    a[0] < b[0] + b[2] + 2
        && b[0] < a[0] + a[2] + 2
        && a[1] < b[1] + b[3] + 2
        && b[1] < a[1] + a[3] + 2
}

fn rounded(rects: &mut Vec<Rect>, rect: [usize; 4], radius: usize, color: u32) {
    let [x, y, w, h] = rect;
    for row in 0..h {
        let edge = row.min(h - 1 - row);
        let inset = if edge < radius {
            let dy = (radius - edge) as f32 - 0.5;
            (radius as f32 - ((radius * radius) as f32 - dy * dy).sqrt()).ceil()
                as usize
        } else {
            0
        };
        rects.push(Rect {
            bounds: [x + inset, y + row, w - inset * 2, 1],
            color,
        });
    }
}

fn glyph_rects(
    rects: &mut Vec<Rect>,
    image: &Image,
    origin: (usize, usize),
    baseline: i32,
) {
    let p = image.placement;
    for y in 0..p.height as usize {
        for x in 0..p.width as usize {
            let index = y * p.width as usize + x;
            let alpha = match image.content {
                Content::Mask => image.data[index],
                _ => {
                    *image.data[index * 4..index * 4 + 3].iter().max().unwrap()
                }
            } as u32;
            if alpha == 0 {
                continue;
            }
            let mut color = 0;
            for shift in [0, 8, 16] {
                let bg = (0xe8d9a8 >> shift) & 255;
                let fg = (0x20252c >> shift) & 255;
                color |= ((fg * alpha + bg * (255 - alpha)) / 255) << shift;
            }
            let left = origin.0 as i64 + p.left as i64 + x as i64;
            let top =
                origin.1 as i64 + baseline as i64 - p.top as i64 + y as i64;
            if left >= 0 && top >= 0 {
                rects.push(Rect {
                    bounds: [left as usize, top as usize, 1, 1],
                    color,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{marker, terminal::Terminal};

    #[test]
    fn overlapping_anchors_receive_separate_visible_badges() {
        let mut occupied = Vec::new();
        for anchor in [(10, 10), (10, 10), (35, 10), (490, 290)] {
            let rect = place(anchor, (32, 24), (500, 300), &occupied);
            assert!(rect[0] + rect[2] <= 500);
            assert!(rect[1] + rect[3] <= 300);
            assert!(occupied.iter().all(|other| !intersects(rect, *other)));
            occupied.push(rect);
        }
    }

    #[test]
    fn badges_draw_stably_and_dismiss_without_changing_terminal() {
        let mut renderer = Renderer::new(19.0).unwrap();
        let mut terminal = Terminal::new(80, 8);
        terminal.feed(
            b"\x1b[32mOpen src/main.rs to continue\x1b[0m\r\n\
            https://example.com/a/b 192.168.1.1 deadbee",
        );
        let width = renderer.cell_width as u32 * 80;
        let height = renderer.cell_height as u32 * 8;
        renderer.draw(&terminal, width, height, "");
        let original = renderer.pixels.clone();
        let session = marker::Session::new(
            terminal.visible_snapshot(),
            Vec::new(),
            &marker::Options::default(),
        );
        renderer.set_markers(session.visible_targets());
        renderer.draw(&terminal, width, height, "");
        let marked = renderer.pixels.clone();
        assert_ne!(marked, original);
        renderer.draw(&terminal, width, height, "");
        assert_eq!(renderer.pixels, marked);
        renderer.set_markers(Vec::new());
        renderer.draw(&terminal, width, height, "");
        assert_eq!(renderer.pixels, original);
    }
}
