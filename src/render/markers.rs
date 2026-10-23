//! Pixel-based marker badges shared by the software and GPU renderers.
use std::collections::HashMap;

use swash::scale::image::{Content, Image};

use super::{Renderer, font};
use crate::marker::Target;
use crate::terminal::FontStyle;

const ER_YELLOW: u32 = 0xfef991;
const ER_INK: u32 = 0x232323;
const ER_GRAY_60: u32 = 0x7b7b7b;
const CLIP_CORNER: usize = 1;

#[derive(Clone, Copy)]
pub(super) struct Rect {
    pub bounds: [usize; 4],
    pub color: u32,
    pub alpha: u8,
}

#[derive(Default)]
pub(super) struct Markers {
    targets: Vec<Target>,
    options: crate::marker::Options,
    font: Option<font::Rasterizer>,
    glyphs: HashMap<char, Image>,
}

impl Markers {
    pub(super) fn is_empty(&self) -> bool {
        self.targets.is_empty()
    }
}

impl Renderer {
    pub fn set_marker_opacity(&mut self, options: &crate::marker::Options) {
        if self.markers.options.label_opacity != options.label_opacity
            || self.markers.options.indicator_opacity
                != options.indicator_opacity
        {
            self.markers.options.label_opacity = options.label_opacity;
            self.markers.options.indicator_opacity = options.indicator_opacity;
            self.invalidate();
        }
    }

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
                Default::default(),
            )
            .ok();
        }
        let Some(font) = &mut self.markers.font else {
            return Vec::new();
        };
        let label_alpha =
            (self.markers.options.label_opacity.get() * 255.0).round() as u8;
        let indicator_alpha = (self.markers.options.indicator_opacity.get()
            * 255.0)
            .round() as u8;
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
                let color = ER_YELLOW;
                let tick = self.cell_height.min(4);
                rects.push(Rect {
                    bounds: [
                        x,
                        y + self.cell_height.saturating_sub(1),
                        width,
                        1,
                    ],
                    color,
                    alpha: indicator_alpha,
                });
                rects.push(Rect {
                    bounds: [
                        x,
                        y + self.cell_height - tick,
                        1,
                        tick.saturating_sub(1),
                    ],
                    color,
                    alpha: indicator_alpha,
                });
                rects.push(Rect {
                    bounds: [
                        x + width - 1,
                        y + self.cell_height - tick,
                        1,
                        tick.saturating_sub(1),
                    ],
                    color,
                    alpha: indicator_alpha,
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
                color: ER_GRAY_60,
                alpha: indicator_alpha,
            });
            rects.push(Rect {
                bounds: [
                    center.0,
                    anchor.1.min(center.1) + usize::from(center.1 > anchor.1),
                    1,
                    anchor.1.abs_diff(center.1),
                ],
                color: ER_GRAY_60,
                alpha: indicator_alpha,
            });
            stepped(&mut badges, bounds, ER_YELLOW, label_alpha);
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

fn stepped(rects: &mut Vec<Rect>, rect: [usize; 4], color: u32, alpha: u8) {
    let [x, y, w, h] = rect;
    for row in 0..h {
        let inset = if row == 0 || row + 1 == h {
            CLIP_CORNER.min(w / 2)
        } else {
            0
        };
        rects.push(Rect {
            bounds: [x + inset, y + row, w - inset * 2, 1],
            color,
            alpha,
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
            let left = origin.0 as i64 + p.left as i64 + x as i64;
            let top =
                origin.1 as i64 + baseline as i64 - p.top as i64 + y as i64;
            if left >= 0 && top >= 0 {
                rects.push(Rect {
                    bounds: [left as usize, top as usize, 1, 1],
                    color: ER_INK,
                    alpha: alpha as u8,
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
        let options = crate::marker::Options {
            label_opacity: 0.7.try_into().unwrap(),
            indicator_opacity: 0.0.try_into().unwrap(),
            ..Default::default()
        };
        renderer.set_marker_opacity(&options);
        renderer.draw(&terminal, width, height, "");
        let adjusted = renderer.pixels.clone();
        assert_ne!(adjusted, marked);
        renderer.draw(&terminal, width, height, "");
        assert_eq!(renderer.pixels, adjusted);
        renderer.set_markers(Vec::new());
        renderer.draw(&terminal, width, height, "");
        assert_eq!(renderer.pixels, original);
    }
}
