use swash::scale::image::{Content, Image};
use swash::zeno::Placement;

mod boxes;

pub(super) fn rasterize(
    ch: char,
    width: usize,
    height: usize,
    baseline: i32,
) -> Option<Image> {
    if !('\u{2500}'..='\u{259f}').contains(&ch) {
        return None;
    }
    let mut canvas = Canvas::new(width, height);
    if ch < '\u{2580}' {
        boxes::draw(&mut canvas, ch);
    } else {
        blocks(&mut canvas, ch);
    }
    Some(Image {
        content: Content::Mask,
        placement: Placement {
            left: 0,
            top: baseline,
            width: width as u32,
            height: height as u32,
        },
        data: canvas.data,
        ..Default::default()
    })
}

struct Canvas {
    width: usize,
    height: usize,
    data: Vec<u8>,
}

impl Canvas {
    fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            data: vec![0; width * height],
        }
    }

    fn rect(&mut self, x0: f32, y0: f32, x1: f32, y1: f32) {
        for y in
            y0.max(0.0).floor() as usize..(y1.ceil() as usize).min(self.height)
        {
            for x in x0.max(0.0).floor() as usize
                ..(x1.ceil() as usize).min(self.width)
            {
                let area = (x1.min(x as f32 + 1.0) - x0.max(x as f32))
                    * (y1.min(y as f32 + 1.0) - y0.max(y as f32));
                let alpha = (area.max(0.0) * 255.0).round() as u8;
                let pixel = &mut self.data[y * self.width + x];
                *pixel = (*pixel).max(alpha);
            }
        }
    }

    fn sample(&mut self, inside: impl Fn(f32, f32) -> bool) {
        for y in 0..self.height {
            for x in 0..self.width {
                let mut count = 0;
                for sy in 0..4 {
                    for sx in 0..4 {
                        count += u32::from(inside(
                            x as f32 + (sx as f32 + 0.5) / 4.0,
                            y as f32 + (sy as f32 + 0.5) / 4.0,
                        ));
                    }
                }
                self.data[y * self.width + x] = ((count * 255 + 8) / 16) as u8;
            }
        }
    }
}

fn blocks(canvas: &mut Canvas, ch: char) {
    let w = canvas.width as f32;
    let h = canvas.height as f32;
    match ch {
        '▀' => canvas.rect(0.0, 0.0, w, h / 2.0),
        '▁'..='█' => {
            let fraction = (ch as u32 - '▁' as u32 + 1) as f32 / 8.0;
            canvas.rect(0.0, h * (1.0 - fraction), w, h);
        }
        '▉'..='▏' => {
            let fraction = ('▏' as u32 - ch as u32 + 1) as f32 / 8.0;
            canvas.rect(0.0, 0.0, w * fraction, h);
        }
        '▐' => canvas.rect(w / 2.0, 0.0, w, h),
        '░' => canvas.data.fill(64),
        '▒' => canvas.data.fill(128),
        '▓' => canvas.data.fill(192),
        '▔' => canvas.rect(0.0, 0.0, w, h / 8.0),
        '▕' => canvas.rect(w * 7.0 / 8.0, 0.0, w, h),
        '▖'..='▟' => {
            // Bits follow top-left, top-right, bottom-left, bottom-right.
            let quadrants = [4, 8, 1, 13, 9, 7, 11, 2, 6, 14];
            let mask = quadrants[ch as usize - '▖' as usize];
            let mut coverage = vec![0u16; canvas.data.len()];
            for bit in 0..4 {
                if mask & (1 << bit) == 0 {
                    continue;
                }
                canvas.data.fill(0);
                let x = (bit % 2) as f32 * w / 2.0;
                let y = (bit / 2) as f32 * h / 2.0;
                canvas.rect(x, y, x + w / 2.0, y + h / 2.0);
                for (sum, &alpha) in coverage.iter_mut().zip(&canvas.data) {
                    *sum += u16::from(alpha);
                }
            }
            for (pixel, sum) in canvas.data.iter_mut().zip(coverage) {
                *pixel = sum.min(255) as u8;
            }
        }
        _ => unreachable!(),
    }
}

#[cfg(test)]
mod tests;
