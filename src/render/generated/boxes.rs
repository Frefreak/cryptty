use super::Canvas;

#[derive(Clone, Copy, PartialEq)]
enum Stroke {
    Absent,
    Light,
    Heavy,
    Double,
}

pub(super) fn draw(canvas: &mut Canvas, ch: char) {
    let w = canvas.width as f32;
    let h = canvas.height as f32;
    let thickness = (w / 8.0).round().max(1.0).min(w.min(h));
    if ('╱'..='╳').contains(&ch) {
        let length = w.hypot(h);
        canvas.sample(|x, y| {
            (ch != '╲'
                && (h * x + w * y - w * h).abs() <= thickness * length / 2.0)
                || (ch != '╱'
                    && (h * x - w * y).abs() <= thickness * length / 2.0)
        });
    } else if ('╭'..='╰').contains(&ch) {
        rounded(canvas, ch, thickness);
    } else if let Some((horizontal, count, heavy)) = dashes(ch) {
        let t = if heavy { thickness * 2.0 } else { thickness };
        let (length, cross) = if horizontal { (w, h) } else { (h, w) };
        let (low, high) = band(cross, t);
        let count = if length < (count * 2 - 1) as f32 {
            1
        } else {
            count
        };
        let gap = (length / 8.0).round().max(1.0);
        let dash =
            ((length - gap * (count - 1) as f32) / count as f32).max(0.0);
        for i in 0..count {
            let start = i as f32 * (dash + gap);
            let end = if i + 1 == count { length } else { start + dash };
            if horizontal {
                canvas.rect(start, low, end, high);
            } else {
                canvas.rect(low, start, high, end);
            }
        }
    } else {
        let [left, right, up, down] = arms(ch);
        axis(canvas, [left, right], [up, down], thickness, false);
        axis(canvas, [up, down], [left, right], thickness, true);
    }
}

fn band(size: f32, thickness: f32) -> (f32, f32) {
    let thickness = thickness.min(size);
    let start = ((size - thickness) / 2.0).floor();
    (start, start + thickness)
}

fn rails(size: f32, thickness: f32, stroke: Stroke) -> Vec<(f32, f32)> {
    match stroke {
        Stroke::Absent => Vec::new(),
        Stroke::Light => vec![band(size, thickness)],
        Stroke::Heavy => vec![band(size, thickness * 2.0)],
        Stroke::Double => {
            let t = thickness.min((size / 3.0).floor().max(1.0));
            let start = ((size - t * 3.0) / 2.0).floor().max(0.0);
            vec![
                (start, (start + t).min(size)),
                ((start + t * 2.0).min(size - t), (start + t * 3.0).min(size)),
            ]
        }
    }
}

fn axis(
    canvas: &mut Canvas,
    along: [Stroke; 2],
    across: [Stroke; 2],
    thickness: f32,
    transpose: bool,
) {
    let (length, cross) = if transpose {
        (canvas.height as f32, canvas.width as f32)
    } else {
        (canvas.width as f32, canvas.height as f32)
    };
    let perpendicular: Vec<_> = across
        .iter()
        .flat_map(|&stroke| rails(length, thickness, stroke))
        .collect();
    let low = perpendicular
        .iter()
        .map(|r| r.0)
        .reduce(f32::min)
        .unwrap_or((length / 2.0).floor());
    let high = perpendicular
        .iter()
        .map(|r| r.1)
        .reduce(f32::max)
        .unwrap_or((length / 2.0).ceil());
    for (side, &stroke) in along.iter().enumerate() {
        let lines = rails(cross, thickness, stroke);
        for (index, &(a, b)) in lines.iter().enumerate() {
            // Double junctions meet at the near rail to preserve the channel.
            let both = across.iter().all(|s| *s != Stroke::Absent);
            let toward = across[index.min(1)] != Stroke::Absent;
            let near = across.contains(&Stroke::Double)
                && (both || (lines.len() == 2 && toward));
            let endpoint = if side == 0 {
                if near {
                    perpendicular
                        .iter()
                        .map(|r| r.1)
                        .reduce(f32::min)
                        .unwrap_or(high)
                } else {
                    high
                }
            } else if near {
                perpendicular
                    .iter()
                    .map(|r| r.0)
                    .reduce(f32::max)
                    .unwrap_or(low)
            } else {
                low
            };
            let (start, end) = if side == 0 {
                (0.0, endpoint)
            } else {
                (endpoint, length)
            };
            if transpose {
                canvas.rect(a, start, b, end);
            } else {
                canvas.rect(start, a, end, b);
            }
        }
    }
}

fn rounded(canvas: &mut Canvas, ch: char, thickness: f32) {
    let w = canvas.width as f32;
    let h = canvas.height as f32;
    let (x0, x1) = band(w, thickness);
    let (y0, y1) = band(h, thickness);
    let cx = (x0 + x1) / 2.0;
    let cy = (y0 + y1) / 2.0;
    let right = matches!(ch, '╭' | '╰');
    let down = matches!(ch, '╭' | '╮');
    let sx = if right { 1.0 } else { -1.0 };
    let sy = if down { 1.0 } else { -1.0 };
    let rx = if right { w - cx } else { cx };
    let ry = if down { h - cy } else { cy };
    let r = (rx.min(ry) - thickness / 2.0 - 1.0).max(0.0);
    canvas.sample(|x, y| {
        let x = (x - cx) * sx;
        let y = (y - cy) * sy;
        if x >= r {
            y.abs() <= thickness / 2.0
        } else if y >= r {
            x.abs() <= thickness / 2.0
        } else {
            ((x - r).hypot(y - r) - r).abs() <= thickness / 2.0
        }
    });
}

fn dashes(ch: char) -> Option<(bool, usize, bool)> {
    Some(match ch {
        '┄' => (true, 3, false),
        '┅' => (true, 3, true),
        '┆' => (false, 3, false),
        '┇' => (false, 3, true),
        '┈' => (true, 4, false),
        '┉' => (true, 4, true),
        '┊' => (false, 4, false),
        '┋' => (false, 4, true),
        '╌' => (true, 2, false),
        '╍' => (true, 2, true),
        '╎' => (false, 2, false),
        '╏' => (false, 2, true),
        _ => return None,
    })
}

fn arms(ch: char) -> [Stroke; 4] {
    use Stroke::{Absent as N, Double as D, Heavy as H, Light as L};
    match ch {
        '─' => [L, L, N, N],
        '━' => [H, H, N, N],
        '│' => [N, N, L, L],
        '┃' => [N, N, H, H],
        '┌' => [N, L, N, L],
        '┍' => [N, H, N, L],
        '┎' => [N, L, N, H],
        '┏' => [N, H, N, H],
        '┐' => [L, N, N, L],
        '┑' => [H, N, N, L],
        '┒' => [L, N, N, H],
        '┓' => [H, N, N, H],
        '└' => [N, L, L, N],
        '┕' => [N, H, L, N],
        '┖' => [N, L, H, N],
        '┗' => [N, H, H, N],
        '┘' => [L, N, L, N],
        '┙' => [H, N, L, N],
        '┚' => [L, N, H, N],
        '┛' => [H, N, H, N],
        '├' => [N, L, L, L],
        '┝' => [N, H, L, L],
        '┞' => [N, L, H, L],
        '┟' => [N, L, L, H],
        '┠' => [N, L, H, H],
        '┡' => [N, H, H, L],
        '┢' => [N, H, L, H],
        '┣' => [N, H, H, H],
        '┤' => [L, N, L, L],
        '┥' => [H, N, L, L],
        '┦' => [L, N, H, L],
        '┧' => [L, N, L, H],
        '┨' => [L, N, H, H],
        '┩' => [H, N, H, L],
        '┪' => [H, N, L, H],
        '┫' => [H, N, H, H],
        '┬' => [L, L, N, L],
        '┭' => [H, L, N, L],
        '┮' => [L, H, N, L],
        '┯' => [H, H, N, L],
        '┰' => [L, L, N, H],
        '┱' => [H, L, N, H],
        '┲' => [L, H, N, H],
        '┳' => [H, H, N, H],
        '┴' => [L, L, L, N],
        '┵' => [H, L, L, N],
        '┶' => [L, H, L, N],
        '┷' => [H, H, L, N],
        '┸' => [L, L, H, N],
        '┹' => [H, L, H, N],
        '┺' => [L, H, H, N],
        '┻' => [H, H, H, N],
        '┼' => [L, L, L, L],
        '┽' => [H, L, L, L],
        '┾' => [L, H, L, L],
        '┿' => [H, H, L, L],
        '╀' => [L, L, H, L],
        '╁' => [L, L, L, H],
        '╂' => [L, L, H, H],
        '╃' => [H, L, H, L],
        '╄' => [L, H, H, L],
        '╅' => [H, L, L, H],
        '╆' => [L, H, L, H],
        '╇' => [H, H, H, L],
        '╈' => [H, H, L, H],
        '╉' => [H, L, H, H],
        '╊' => [L, H, H, H],
        '╋' => [H, H, H, H],
        '═' => [D, D, N, N],
        '║' => [N, N, D, D],
        '╒' => [N, D, N, L],
        '╓' => [N, L, N, D],
        '╔' => [N, D, N, D],
        '╕' => [D, N, N, L],
        '╖' => [L, N, N, D],
        '╗' => [D, N, N, D],
        '╘' => [N, D, L, N],
        '╙' => [N, L, D, N],
        '╚' => [N, D, D, N],
        '╛' => [D, N, L, N],
        '╜' => [L, N, D, N],
        '╝' => [D, N, D, N],
        '╞' => [N, D, L, L],
        '╟' => [N, L, D, D],
        '╠' => [N, D, D, D],
        '╡' => [D, N, L, L],
        '╢' => [L, N, D, D],
        '╣' => [D, N, D, D],
        '╤' => [D, D, N, L],
        '╥' => [L, L, N, D],
        '╦' => [D, D, N, D],
        '╧' => [D, D, L, N],
        '╨' => [L, L, D, N],
        '╩' => [D, D, D, N],
        '╪' => [D, D, L, L],
        '╫' => [L, L, D, D],
        '╬' => [D, D, D, D],
        '╴' => [L, N, N, N],
        '╵' => [N, N, L, N],
        '╶' => [N, L, N, N],
        '╷' => [N, N, N, L],
        '╸' => [H, N, N, N],
        '╹' => [N, N, H, N],
        '╺' => [N, H, N, N],
        '╻' => [N, N, N, H],
        '╼' => [L, H, N, N],
        '╽' => [N, N, L, H],
        '╾' => [H, L, N, N],
        '╿' => [N, N, H, L],
        _ => unreachable!(),
    }
}
