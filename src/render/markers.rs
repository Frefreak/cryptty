//! Pixel-based marker badges shared by the software and GPU renderers.
use std::collections::HashMap;

use swash::scale::image::{Content, Image};

use super::{Renderer, font};
use crate::marker::Target;

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
    layout_targets: Vec<Target>,
    options: crate::marker::Options,
    font: Option<font::Rasterizer>,
    glyphs: HashMap<char, Image>,
    lanes: HashMap<String, usize>,
}

impl Markers {
    pub(super) fn is_empty(&self) -> bool {
        self.targets.is_empty()
    }
}

impl Renderer {
    pub fn set_marker_options(&mut self, options: &crate::marker::Options) {
        if self.markers.options.label_embolden.pixels()
            != options.label_embolden.pixels()
        {
            self.markers.options.label_embolden = options.label_embolden;
            self.markers.glyphs.clear();
            self.invalidate();
        }
        if self.markers.options.label_opacity != options.label_opacity
            || self.markers.options.indicator_opacity
                != options.indicator_opacity
            || self.markers.options.label_background != options.label_background
            || self.markers.options.accent_colors != options.accent_colors
        {
            self.markers.options.label_background = options.label_background;
            self.markers.options.accent_colors = options.accent_colors.clone();
            self.markers.options.label_opacity = options.label_opacity;
            self.markers.options.indicator_opacity = options.indicator_opacity;
            self.invalidate();
        }
    }

    pub fn set_markers(&mut self, targets: Vec<Target>) {
        if self.markers.targets != targets {
            if targets.is_empty() {
                self.markers.lanes.clear();
                self.markers.layout_targets.clear();
            }
            for (index, target) in targets.iter().enumerate() {
                if self.markers.lanes.contains_key(&target.label) {
                    continue;
                }
                let lane = (0..)
                    .find(|lane| {
                        targets[..index].iter().all(|other| {
                            self.markers.lanes.get(&other.label) != Some(lane)
                                || !targets_overlap(target, other)
                        })
                    })
                    .unwrap();
                self.markers.lanes.insert(target.label.clone(), lane);
                self.markers.layout_targets.push(target.clone());
            }
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
        let mut positions = HashMap::new();
        let mut occupied = Vec::new();
        for target in &self.markers.layout_targets {
            let Some(segment) = target.segments.first() else {
                continue;
            };
            let anchor = (
                self.padding.0 + segment.start.col * self.cell_width,
                self.padding.1 + segment.start.row * self.cell_height,
            );
            let bounds = place(
                anchor,
                (font.cell_width * target.label.len() + padding * 2, height),
                self.markers.lanes[&target.label],
                self.cell_height,
                size,
            );
            let bounds = avoid_collisions(bounds, size, &occupied);
            occupied.push(bounds);
            positions.insert(&target.label, bounds);
        }
        let mut rects = Vec::new();
        let mut highlights = Vec::new();
        let mut badges = Vec::new();
        for target in &self.markers.targets {
            let lane = self.markers.lanes[&target.label];
            let color = lane_color(lane, &self.markers.options);
            for segment in &target.segments {
                let x = self.padding.0 + segment.start.col * self.cell_width;
                let y = self.padding.1 + segment.start.row * self.cell_height;
                let width = segment.columns * self.cell_width;
                if lane == 0 {
                    highlights.push(Rect {
                        bounds: [x, y, width, self.cell_height],
                        color,
                        alpha: indicator_alpha / 12,
                    });
                }
                let offset = (lane * 4).min(self.cell_height.saturating_sub(4));
                let bottom = y + self.cell_height - 1 - offset;
                let tick = self.cell_height.min(4);
                rects.push(Rect {
                    bounds: [x, bottom, width, 1],
                    color,
                    alpha: indicator_alpha,
                });
                rects.push(Rect {
                    bounds: [x, bottom + 1 - tick, 1, tick.saturating_sub(1)],
                    color,
                    alpha: indicator_alpha,
                });
                rects.push(Rect {
                    bounds: [
                        x + width - 1,
                        bottom + 1 - tick,
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
            let bounds = positions[&target.label];
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
            stepped(&mut badges, bounds, color, label_alpha);
            for (index, ch) in target.label.chars().enumerate() {
                if !self.markers.glyphs.contains_key(&ch)
                    && let Some(image) = font.rasterize_label(
                        ch,
                        self.markers.options.label_embolden,
                    )
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
                        label_ink(color),
                    );
                }
            }
        }
        rects.extend(badges);
        highlights.extend(rects);
        highlights
    }
}

fn lane_color(lane: usize, options: &crate::marker::Options) -> u32 {
    let base = options.label_background.0;
    std::iter::once(base)
        .chain(
            options
                .accent_colors
                .iter()
                .map(|color| color.0)
                .filter(|&color| color != base),
        )
        .cycle()
        .nth(lane)
        .unwrap()
}

fn label_ink(background: u32) -> u32 {
    let luminance = |color: u32| {
        [(16, 0.2126), (8, 0.7152), (0, 0.0722)]
            .into_iter()
            .map(|(shift, weight)| {
                let channel = ((color >> shift) & 255) as f32 / 255.0;
                let linear = if channel <= 0.04045 {
                    channel / 12.92
                } else {
                    ((channel + 0.055) / 1.055).powf(2.4)
                };
                linear * weight
            })
            .sum::<f32>()
    };
    let background = luminance(background) + 0.05;
    let ink = luminance(ER_INK) + 0.05;
    let dark_contrast = background.max(ink) / background.min(ink);
    if 1.05 / background > dark_contrast {
        0xffffff
    } else {
        ER_INK
    }
}

fn targets_overlap(a: &Target, b: &Target) -> bool {
    a.segments.iter().any(|a| {
        b.segments.iter().any(|b| {
            a.start.row == b.start.row
                && a.start.col < b.start.col + b.columns
                && b.start.col < a.start.col + a.columns
        })
    })
}

fn place(
    anchor: (usize, usize),
    badge: (usize, usize),
    lane: usize,
    cell_height: usize,
    viewport: (usize, usize),
) -> [usize; 4] {
    let distance = (lane / 2 + 1) * badge.1;
    let above = anchor.1.checked_sub(distance);
    let below = anchor.1 + cell_height + distance - badge.1;
    let below = (below + badge.1 <= viewport.1).then_some(below);
    let y = if lane % 2 == 0 {
        above.or(below)
    } else {
        below.or(above)
    }
    .unwrap_or_else(|| anchor.1.min(viewport.1.saturating_sub(badge.1)));
    [
        anchor.0.min(viewport.0.saturating_sub(badge.0)),
        y,
        badge.0,
        badge.1,
    ]
}

fn avoid_collisions(
    preferred: [usize; 4],
    viewport: (usize, usize),
    occupied: &[[usize; 4]],
) -> [usize; 4] {
    if occupied
        .iter()
        .all(|other| !rects_overlap(preferred, *other))
    {
        return preferred;
    }
    let [x, y, width, height] = preferred;
    let mut xs = vec![x];
    let mut ys = vec![y];
    for &[left, top, w, h] in occupied {
        xs.extend([left.saturating_sub(width), left + w]);
        ys.extend([top.saturating_sub(height), top + h]);
    }
    xs.retain(|&left| left + width <= viewport.0 && left.abs_diff(x) <= width);
    ys.retain(|&top| top + height <= viewport.1);
    xs.sort_unstable();
    xs.dedup();
    ys.sort_unstable();
    ys.dedup();
    ys.into_iter()
        .flat_map(|top| xs.iter().map(move |&left| [left, top, width, height]))
        .filter(|bounds| {
            occupied.iter().all(|other| !rects_overlap(*bounds, *other))
        })
        .min_by_key(|bounds| {
            (bounds[1].abs_diff(y), bounds[0].abs_diff(x), bounds[0])
        })
        .unwrap_or(preferred)
}

fn rects_overlap(a: [usize; 4], b: [usize; 4]) -> bool {
    a[0] < b[0] + b[2]
        && b[0] < a[0] + a[2]
        && a[1] < b[1] + b[3]
        && b[1] < a[1] + a[3]
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

fn glyph_alpha(image: &Image, index: usize) -> u8 {
    match image.content {
        Content::Mask => image.data[index],
        _ => *image.data[index * 4..index * 4 + 3].iter().max().unwrap(),
    }
}

fn glyph_rects(
    rects: &mut Vec<Rect>,
    image: &Image,
    origin: (usize, usize),
    baseline: i32,
    color: u32,
) {
    let p = image.placement;
    for y in 0..p.height as usize {
        for x in 0..p.width as usize {
            let index = y * p.width as usize + x;
            let alpha = glyph_alpha(image, index) as u32;
            if alpha == 0 {
                continue;
            }
            let left = origin.0 as i64 + p.left as i64 + x as i64;
            let top =
                origin.1 as i64 + baseline as i64 - p.top as i64 + y as i64;
            if left >= 0 && top >= 0 {
                rects.push(Rect {
                    bounds: [left as usize, top as usize, 1, 1],
                    color,
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
    fn marker_outline_thickness_increases_coverage_without_resizing_cells() {
        let family = if cfg!(target_os = "macos") {
            "Menlo"
        } else {
            "DejaVu Sans Mono"
        };
        let mut font = font::Rasterizer::new(
            10.0,
            false,
            family,
            &[],
            &[],
            Default::default(),
        )
        .unwrap();
        let metrics = (font.cell_width, font.cell_height, font.baseline);
        for ch in ['a', 'm', 'i', 'g'] {
            let original =
                font.rasterize_label(ch, Default::default()).unwrap();
            let mut previous = 0;
            for amount in [0.0, 0.2, 0.5, 1.0] {
                let image = font
                    .rasterize_label(ch, amount.try_into().unwrap())
                    .unwrap();
                assert_eq!(image.content, Content::Mask);
                let coverage: u64 =
                    image.data.iter().map(|&a| u64::from(a)).sum();
                assert!(coverage > previous, "{ch}: {amount}");
                previous = coverage;
                assert_eq!(
                    (font.cell_width, font.cell_height, font.baseline),
                    metrics,
                );
            }
            let restored =
                font.rasterize_label(ch, Default::default()).unwrap();
            assert_eq!(restored.data, original.data);
            assert_eq!(restored.placement.left, original.placement.left);
            assert_eq!(restored.placement.top, original.placement.top);
        }
    }

    #[test]
    fn badges_alternate_sides_and_expand_outward() {
        for (lane, y) in [76, 140, 52, 164].into_iter().enumerate() {
            assert_eq!(
                place((10, 100), (32, 24), lane, 40, (500, 300)),
                [10, y, 32, 24],
            );
        }
    }

    #[test]
    fn badges_use_available_side_and_stay_inside_viewport() {
        assert_eq!(place((0, 0), (32, 24), 0, 40, (500, 300)), [0, 40, 32, 24],);
        assert_eq!(
            place((490, 260), (32, 24), 1, 40, (500, 300)),
            [468, 236, 32, 24],
        );
    }

    #[test]
    fn badges_avoid_neighbors_and_edge_fallback_collisions() {
        for anchor in [(10, 0), (468, 260), (10, 100)] {
            let mut occupied = Vec::new();
            for lane in 0..4 {
                let preferred = place(anchor, (32, 24), lane, 40, (500, 300));
                let bounds = avoid_collisions(preferred, (500, 300), &occupied);
                assert!(occupied.iter().all(|b| !rects_overlap(bounds, *b)));
                assert!(bounds[0].abs_diff(preferred[0]) <= 32);
                occupied.push(bounds);
            }
        }
        let first = place((10, 0), (32, 24), 0, 40, (500, 300));
        let next = place((10, 80), (32, 24), 0, 40, (500, 300));
        let next = avoid_collisions(next, (500, 300), &[first]);
        assert!(!rects_overlap(first, next));
    }

    #[test]
    fn custom_background_does_not_duplicate_the_next_accent() {
        let mut options = crate::marker::Options::default();
        for base in options.accent_colors.clone() {
            options.label_background = base;
            assert_eq!(lane_color(0, &options), base.0);
            assert_ne!(lane_color(1, &options), base.0);
            assert_ne!(lane_color(2, &options), base.0);
            assert_ne!(lane_color(1, &options), lane_color(2, &options));
        }
        options.accent_colors.clear();
        assert_eq!(lane_color(12, &options), options.label_background.0);
        assert_eq!(label_ink(0x101020), 0xffffff);
        assert_eq!(label_ink(0xfef991), ER_INK);
        assert_eq!(label_ink(0x00b000), ER_INK);
    }

    #[test]
    fn nested_targets_have_matching_colors_and_stable_rendering() {
        let mut renderer = Renderer::new(19.0).unwrap();
        let mut terminal = Terminal::new(80, 4);
        terminal.feed(b"\r\nRan \x1b[32mcat\x1b[0m /tmp/cryptty --number");
        let targets = marker::Session::new(
            terminal.visible_snapshot(),
            Vec::new(),
            &Default::default(),
        )
        .visible_targets();
        assert_eq!(targets.len(), 2);
        assert_ne!(targets[0].segments[0].start, targets[1].segments[0].start);
        renderer.set_markers(targets.clone());
        let size = (renderer.cell_width * 80, renderer.cell_height * 4);
        let rects = renderer.marker_rects(size);
        let colors = [
            renderer.markers.options.label_background.0,
            renderer.markers.options.accent_colors[0].0,
        ];
        let command = &targets[0].segments[0];
        let highlight = &rects[0];
        assert_eq!(highlight.color, colors[0]);
        assert_eq!(
            highlight.bounds,
            [
                renderer.padding.0 + command.start.col * renderer.cell_width,
                renderer.padding.1 + command.start.row * renderer.cell_height,
                command.columns * renderer.cell_width,
                renderer.cell_height,
            ]
        );
        assert!(highlight.alpha > 0 && highlight.alpha < 64);
        for (index, target) in targets.iter().enumerate() {
            let segment = &target.segments[0];
            let y = renderer.padding.1
                + (segment.start.row + 1) * renderer.cell_height
                - 1
                - index * 4;
            let x =
                renderer.padding.0 + segment.start.col * renderer.cell_width;
            assert!(rects.iter().any(|rect| {
                rect.color == colors[index]
                    && rect.bounds
                        == [x, y, segment.columns * renderer.cell_width, 1]
            }));
            assert!(rects.iter().any(|rect| {
                rect.color == colors[index]
                    && if index == 0 {
                        rect.bounds[1] < y
                    } else {
                        rect.bounds[1] > y
                    }
                    && rect.alpha
                        == (renderer.markers.options.label_opacity.get()
                            * 255.0)
                            .round() as u8
                    && rect.bounds[2] > 1
            }));
        }
        let lanes = renderer.markers.lanes.clone();
        let blue_badges = |rects: &[Rect]| {
            rects
                .iter()
                .filter(|rect| rect.color == colors[1] && rect.alpha == 204)
                .map(|rect| rect.bounds)
                .collect::<Vec<_>>()
        };
        let before_filter = blue_badges(&rects);
        renderer.set_markers(vec![targets[1].clone()]);
        assert_eq!(renderer.markers.lanes, lanes);
        assert_eq!(blue_badges(&renderer.marker_rects(size)), before_filter,);
        renderer.set_markers(targets);
        let restored = renderer.marker_rects(size);
        assert_eq!(rects.len(), restored.len());
        for (a, b) in rects.iter().zip(restored) {
            assert_eq!(
                (a.bounds, a.color, a.alpha),
                (b.bounds, b.color, b.alpha)
            );
        }
        let mut options = renderer.markers.options.clone();
        options.accent_colors = vec![crate::config::Color(0x183050)];
        renderer.set_marker_options(&options);
        let recolored = renderer.marker_rects(size);
        assert!(recolored.iter().any(|rect| rect.color == 0x183050));
        assert!(!recolored.iter().any(|rect| rect.color == colors[1]));
        renderer.set_markers(Vec::new());
        assert!(renderer.markers.lanes.is_empty());
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
        let thicker_options =
            crate::config::Config::parse("markers: {label_embolden: 0.2}")
                .unwrap()
                .markers;
        renderer.set_marker_options(&thicker_options);
        renderer.draw(&terminal, width, height, "");
        assert_ne!(renderer.pixels, marked);
        renderer.set_marker_options(&marker::Options::default());
        renderer.draw(&terminal, width, height, "");
        assert_eq!(renderer.pixels, marked);
        let options = crate::marker::Options {
            label_opacity: 0.7.try_into().unwrap(),
            indicator_opacity: 0.0.try_into().unwrap(),
            ..Default::default()
        };
        renderer.set_marker_options(&options);
        renderer.draw(&terminal, width, height, "");
        let adjusted = renderer.pixels.clone();
        assert_ne!(adjusted, marked);
        renderer.draw(&terminal, width, height, "");
        assert_eq!(renderer.pixels, adjusted);
        let options = crate::config::Config::parse(
            "markers: {label_background: '#e8dca0', indicator_opacity: 0}",
        )
        .unwrap()
        .markers;
        renderer.set_marker_options(&options);
        renderer.draw(&terminal, width, height, "");
        let recolored = renderer.pixels.clone();
        assert_ne!(recolored, adjusted);
        renderer.draw(&terminal, width, height, "");
        assert_eq!(renderer.pixels, recolored);
        renderer.set_markers(Vec::new());
        renderer.draw(&terminal, width, height, "");
        assert_eq!(renderer.pixels, original);
    }
}
