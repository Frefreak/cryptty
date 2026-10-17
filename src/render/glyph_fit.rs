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
    // Combining marks have no cell of their own; retain their attachment.
    let Some(columns) = ch.width().filter(|&width| width > 0) else {
        return Some(image);
    };
    let width = (cell_width * columns) as i32;
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
        let scale = (width as f32 / p.width.max(1) as f32)
            .min(height as f32 / p.height.max(1) as f32);
        render(scale)?
    } else {
        image
    };
    constrain_bitmap(&mut fitted, width as u32, height as u32)?;
    let f = &mut fitted.placement;
    f.left = f.left.clamp(0, width - f.width as i32);
    f.top = f.top.clamp(baseline - height + f.height as i32, baseline);
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
    fn glyphs_fit_even_when_a_bitmap_strike_ignores_size() {
        for ch in ['M', 'x', '中', '①', '→', '\u{e000}', '😀'] {
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
    fn combining_marks_remain_unchanged() {
        for ch in ['\u{301}', '\u{3099}'] {
            let image = fit(ch, glyph(60, 80), 19, 40, 30, |_| {
                panic!("combining marks must keep their native raster")
            })
            .unwrap();
            assert_eq!(image.placement.width, 60);
            assert_eq!(image.placement.left, -2);
        }
    }

    #[test]
    fn fitting_glyph_keeps_its_pixels_and_placement() {
        let mut original = glyph(8, 13);
        original.placement.left = 3;
        original.placement.top = 13;
        let fitted = fit('⇡', original.clone(), 14, 31, 24, |_| {
            panic!("a fitting glyph must keep its original size")
        })
        .unwrap();
        assert_eq!(fitted.placement.left, original.placement.left);
        assert_eq!(fitted.placement.top, original.placement.top);
        assert_eq!(fitted.placement.width, original.placement.width);
        assert_eq!(fitted.placement.height, original.placement.height);
        assert_eq!(fitted.data, original.data);
    }

    #[test]
    fn overhanging_bearings_shift_without_shrinking() {
        for (left, top, expected_left, expected_top) in [
            (-2, 12, 0, 12),
            (8, 12, 6, 12),
            (2, 30, 2, 24),
            (2, 2, 2, 6),
        ] {
            let mut original = glyph(8, 13);
            original.placement.left = left;
            original.placement.top = top;
            let fitted = fit('M', original.clone(), 14, 31, 24, |_| {
                panic!("a bearing overhang does not need a smaller raster")
            })
            .unwrap();
            assert_eq!(fitted.placement.left, expected_left);
            assert_eq!(fitted.placement.top, expected_top);
            assert_eq!(fitted.data, original.data);
        }
    }

    #[test]
    fn oversized_raster_is_rendered_at_the_measured_scale() {
        let fitted = fit('中', glyph(80, 60), 20, 40, 30, |scale| {
            assert_eq!(scale, 0.5);
            let mut smaller = glyph(40, 30);
            smaller.placement.left = -1;
            smaller.placement.top = 30;
            Some(smaller)
        })
        .unwrap();
        assert_eq!(fitted.placement.width, 40);
        assert_eq!(fitted.placement.height, 30);
        assert_eq!(fitted.placement.left, 0);
        assert_eq!(fitted.placement.top, 30);
    }
}
