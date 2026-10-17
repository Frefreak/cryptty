use swash::scale::image::Image;
use unicode_width::UnicodeWidthChar;

pub(super) fn fit(
    ch: char,
    image: Image,
    cell_width: usize,
    cell_height: usize,
    baseline: i32,
    render: impl FnOnce(f32) -> Option<Image>,
) -> Option<Image> {
    // Keep letters, combining marks, and joining cell graphics untouched.
    if !matches!(ch as u32,
        0x2190..=0x24ff | 0x25a0..=0x27ff | 0x2900..=0x2bff
            | 0xe000..=0xf8ff | 0x1f000..=0x1faff
            | 0xf0000..=0xffffd | 0x100000..=0x10fffd)
    {
        return Some(image);
    }
    let width = (cell_width * ch.width().unwrap_or(1).max(1)) as i32;
    let height = cell_height as i32;
    let p = image.placement;
    if p.left >= 0
        && p.left + p.width as i32 <= width
        && p.top <= baseline
        && baseline - p.top + p.height as i32 <= height
    {
        return Some(image);
    }
    let mut fitted = if p.width > width as u32 || p.height > height as u32 {
        // Reserve one pixel for rasterizer rounding at the smaller size.
        let scale = ((width - 1).max(1) as f32 / p.width.max(1) as f32)
            .min((height - 1).max(1) as f32 / p.height.max(1) as f32);
        render(scale)?
    } else {
        image
    };
    constrain_bitmap(&mut fitted, width as u32, height as u32)?;
    let f = &mut fitted.placement;
    f.left = (width - f.width as i32) / 2;
    let top = p.top - (p.height as i32 - f.height as i32) / 2;
    f.top = top.clamp(baseline - height + f.height as i32, baseline);
    Some(fitted)
}

// Bitmap strikes and hinting can ignore the requested raster size.
fn constrain_bitmap(image: &mut Image, width: u32, height: u32) -> Option<()> {
    use ::image::{DynamicImage, GrayImage, RgbaImage, imageops::FilterType};
    use swash::scale::image::Content;

    let p = &mut image.placement;
    if p.width <= width && p.height <= height {
        return Some(());
    }
    let scale =
        (width as f64 / p.width as f64).min(height as f64 / p.height as f64);
    let w = (p.width as f64 * scale).floor().max(1.0) as u32;
    let h = (p.height as f64 * scale).floor().max(1.0) as u32;
    let data = std::mem::take(&mut image.data);
    let bitmap = match image.content {
        Content::Mask => DynamicImage::ImageLuma8(GrayImage::from_raw(
            p.width, p.height, data,
        )?),
        Content::Color | Content::SubpixelMask => DynamicImage::ImageRgba8(
            RgbaImage::from_raw(p.width, p.height, data)?,
        ),
    };
    image.data = bitmap.resize_exact(w, h, FilterType::Triangle).into_bytes();
    p.width = w;
    p.height = h;
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use swash::zeno::Placement;

    fn glyph(width: u32, height: u32) -> Image {
        Image {
            placement: Placement {
                left: -2,
                top: 40,
                width,
                height,
            },
            data: vec![255; (width * height) as usize],
            ..Image::default()
        }
    }

    #[test]
    fn symbols_fit_even_when_a_bitmap_strike_ignores_size() {
        for ch in ['①', '→', '\u{e000}', '😀'] {
            let image =
                fit(ch, glyph(60, 80), 19, 40, 30, |_| Some(glyph(60, 80)))
                    .unwrap();
            let p = image.placement;
            assert!(p.left >= 0);
            assert!(p.left + p.width as i32 <= 19 * ch.width().unwrap() as i32);
            assert!(p.top <= 30);
            assert!(30 - p.top + p.height as i32 <= 40);
            assert_eq!(image.data.len(), (p.width * p.height) as usize);
        }
    }

    #[test]
    fn letters_combining_marks_and_cell_graphics_remain_unchanged() {
        for ch in ['g', '中', '\u{301}', '─', '█', '⣿'] {
            let image = fit(ch, glyph(60, 80), 19, 40, 30, |_| {
                panic!("ordinary glyphs must keep their native raster")
            })
            .unwrap();
            assert_eq!(image.placement.width, 60);
            assert_eq!(image.placement.left, -2);
        }
    }
}
