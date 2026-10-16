use swash::scale::image::{Content, Image};

pub(super) fn glyph(
    mut image: Image,
    advance: f32,
    cell: (usize, usize, i32),
) -> Image {
    let (width, height, baseline) = cell;
    let old = image.placement;
    if old.width == 0 || old.height == 0 {
        return image;
    }
    let scale = (width as f32 / advance.max(1.0))
        .min(width as f32 / old.width as f32)
        .min(height as f32 / old.height as f32)
        .min(1.0);
    let new_width = (old.width as f32 * scale).round().max(1.0) as u32;
    let new_height = (old.height as f32 * scale).round().max(1.0) as u32;
    if (new_width, new_height) != (old.width, old.height) {
        image.data = resize(&image, new_width, new_height);
    }
    let offset = (width as f32 - advance * scale).max(0.0) / 2.0;
    image.placement.left = ((old.left as f32 * scale + offset).round() as i32)
        .clamp(0, (width - new_width as usize) as i32);
    image.placement.top = (old.top as f32 * scale).round() as i32;
    image.placement.top = image
        .placement
        .top
        .clamp(baseline - (height - new_height as usize) as i32, baseline);
    image.placement.width = new_width;
    image.placement.height = new_height;
    image
}

pub(super) fn block(
    mut image: Image,
    advance: f32,
    reference: swash::zeno::Placement,
    cell: (usize, usize, i32),
) -> Image {
    if reference.height == 0
        || image.placement.width == 0
        || image.placement.height == 0
    {
        return image;
    }
    let (width, height, baseline) = cell;
    let sx = width as f32 / advance.max(1.0);
    let sy = height as f32 / reference.height as f32;
    let old = image.placement;
    let width = (old.width as f32 * sx).round().max(1.0) as u32;
    let height = (old.height as f32 * sy).round().max(1.0) as u32;
    if (width, height) != (old.width, old.height) {
        image.data = resize(&image, width, height);
    }
    image.placement.left = (old.left as f32 * sx).round() as i32;
    image.placement.top =
        baseline + ((old.top - reference.top) as f32 * sy).round() as i32;
    image.placement.width = width;
    image.placement.height = height;
    image
}

fn resize(image: &Image, width: u32, height: u32) -> Vec<u8> {
    let channels = if image.content == Content::Mask { 1 } else { 4 };
    let old = image.placement;
    let mut output = Vec::with_capacity((width * height) as usize * channels);
    let step_x = old.width as f32 / width as f32;
    let step_y = old.height as f32 / height as f32;
    for y in 0..height {
        let top = y as f32 * step_y;
        let bottom = (y + 1) as f32 * step_y;
        for x in 0..width {
            let left = x as f32 * step_x;
            let right = (x + 1) as f32 * step_x;
            let mut value = [0.0; 4];
            for sy in top as u32..(bottom.ceil() as u32).min(old.height) {
                let wy = bottom.min(sy as f32 + 1.0) - top.max(sy as f32);
                for sx in left as u32..(right.ceil() as u32).min(old.width) {
                    let wx = right.min(sx as f32 + 1.0) - left.max(sx as f32);
                    let weight = wx * wy / (step_x * step_y);
                    let index = (sy * old.width + sx) as usize * channels;
                    for (c, value) in value[..channels].iter_mut().enumerate() {
                        let mut sample = image.data[index + c] as f32;
                        if image.content == Content::Color && c < 3 {
                            sample *= image.data[index + 3] as f32 / 255.0;
                        }
                        *value += sample * weight;
                    }
                }
            }
            if image.content == Content::Color && value[3] > 0.0 {
                let unpremultiply = 255.0 / value[3];
                for c in &mut value[..3] {
                    *c *= unpremultiply;
                }
            }
            output.extend(value[..channels].iter().map(|c| c.round() as u8));
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use swash::zeno::Placement;

    fn image(
        content: Content,
        width: u32,
        height: u32,
        data: Vec<u8>,
    ) -> Image {
        Image {
            content,
            placement: Placement {
                left: -1,
                top: 10,
                width,
                height,
            },
            data,
            ..Image::default()
        }
    }

    #[test]
    fn oversized_masks_fit_cell_bounds_without_losing_thin_strokes() {
        let mask = image(Content::Mask, 8, 1, vec![255, 0, 0, 0, 0, 0, 0, 0]);
        let fitted = glyph(mask, 8.0, (1, 3, 2));
        assert_eq!((fitted.placement.width, fitted.placement.height), (1, 1));
        assert_eq!(fitted.placement.left, 0);
        assert!(fitted.placement.top <= 2);
        assert!(2 - fitted.placement.top < 3);
        assert_eq!(fitted.data, vec![32]);
    }

    #[test]
    fn block_transform_preserves_half_block_geometry() {
        let reference = Placement {
            left: 0,
            top: 18,
            width: 8,
            height: 20,
        };
        let cases = [
            (0, 18, 8, 20, (0, 25, 14, 32)),
            (0, 18, 8, 10, (0, 25, 14, 16)),
            (0, 8, 8, 10, (0, 9, 14, 16)),
            (0, 18, 4, 20, (0, 25, 7, 32)),
            (4, 18, 4, 20, (7, 25, 7, 32)),
        ];
        for (left, top, width, height, expected) in cases {
            let mut source = image(
                Content::Mask,
                width,
                height,
                vec![255; (width * height) as usize],
            );
            source.placement.left = left;
            source.placement.top = top;
            let fitted = block(source, 8.0, reference, (14, 32, 25));
            let p = fitted.placement;
            assert_eq!((p.left, p.top, p.width, p.height), expected);
            assert!(fitted.data.iter().all(|&alpha| alpha == 255));
        }
    }

    #[test]
    fn color_resampling_preserves_straight_alpha_without_color_fringes() {
        let mut pixels = Vec::new();
        for _ in 0..2 {
            pixels.extend([255, 0, 0, 255, 0, 0, 255, 0]);
        }
        let source = image(Content::Color, 2, 2, pixels);
        let fitted = glyph(source, 2.0, (1, 1, 1));
        assert_eq!(fitted.data, vec![255, 0, 0, 128]);
    }

    #[test]
    fn fitting_preserves_subpixel_channels_and_unscaled_baselines() {
        let source =
            image(Content::SubpixelMask, 2, 2, [0, 128, 255, 255].repeat(4));
        let fitted = glyph(source, 2.0, (1, 1, 1));
        assert_eq!(fitted.data, vec![0, 128, 255, 255]);
        let mut source = image(Content::Mask, 3, 4, vec![255; 12]);
        source.placement.top = 5;
        let fitted = glyph(source, 4.0, (6, 10, 8));
        assert_eq!(fitted.placement.top, 5);
        assert_eq!((fitted.placement.width, fitted.placement.height), (3, 4));
    }
}
