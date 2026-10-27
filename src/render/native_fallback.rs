use core_graphics::base::kCGImageAlphaPremultipliedLast;
use core_graphics::color_space::CGColorSpace;
use core_graphics::context::{CGContext, CGLineJoin, CGPathDrawingMode};
use core_graphics::geometry::{CGAffineTransform, CGPoint, CGRect, CGSize};
use core_text::font::CTFont;
use core_text::font_descriptor::{
    kCTFontColorGlyphsTrait, kCTFontDefaultOrientation,
};
use swash::scale::image::{Content, Image};
use swash::zeno::Placement;

pub(super) fn rasterize(
    font: &CTFont,
    glyph: u16,
    smooth: bool,
) -> Option<Image> {
    rasterize_inner(font, glyph, smooth, None)
}

pub(super) fn rasterize_outline(
    font: &CTFont,
    glyph: u16,
    embolden: crate::config::FontEmbolden,
) -> Option<Image> {
    rasterize_inner(font, glyph, false, Some(f64::from(embolden.pixels())))
}

fn rasterize_inner(
    font: &CTFont,
    glyph: u16,
    smooth: bool,
    embolden: Option<f64>,
) -> Option<Image> {
    let bounds =
        font.get_bounding_rects_for_glyphs(kCTFontDefaultOrientation, &[glyph]);
    let colored = font.symbolic_traits() & kCTFontColorGlyphsTrait != 0;
    let padding = if smooth && !colored { 2.0 } else { 0.0 }
        + embolden.unwrap_or(0.0).ceil();
    let left = bounds.origin.x.floor() - padding;
    let bottom = bounds.origin.y.floor() - padding;
    let top = (bounds.origin.y + bounds.size.height).ceil() + padding;
    let width = (bounds.origin.x + bounds.size.width).ceil() + padding - left;
    let height = top - bottom;
    if width <= 0.0 || height <= 0.0 {
        return Some(Image::default());
    }
    let mut context = CGContext::create_bitmap_context(
        None,
        width as usize,
        height as usize,
        8,
        width as usize * 4,
        &CGColorSpace::create_device_rgb(),
        kCGImageAlphaPremultipliedLast,
    );
    context.set_rgb_fill_color(0.0, 0.0, 0.0, 0.0);
    context.fill_rect(CGRect::new(
        &CGPoint::new(0.0, 0.0),
        &CGSize::new(width, height),
    ));
    context.set_should_smooth_fonts(smooth && !colored);
    context.set_should_antialias(true);
    context.set_rgb_fill_color(1.0, 1.0, 1.0, 1.0);
    if let Some(amount) = embolden {
        let transform = CGAffineTransform {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            tx: -left,
            ty: -bottom,
        };
        let path = font.create_path_for_glyph(glyph, &transform).ok()?;
        context.add_path(&path);
        context.set_rgb_stroke_color(1.0, 1.0, 1.0, 1.0);
        // A centered stroke expands each side by half its width.
        context.set_line_width(amount * 2.0);
        context.set_line_join(CGLineJoin::CGLineJoinRound);
        context.draw_path(if amount > 0.0 {
            CGPathDrawingMode::CGPathFillStroke
        } else {
            CGPathDrawingMode::CGPathFill
        });
    } else {
        font.draw_glyphs(
            &[glyph],
            &[CGPoint::new(-left, -bottom)],
            context.clone(),
        );
    }
    let pixels = context.data();
    let (content, data) = if colored {
        let mut rgba = pixels.to_vec();
        for pixel in rgba.as_chunks_mut::<4>().0 {
            let alpha = u32::from(pixel[3]);
            for channel in &mut pixel[..3] {
                *channel = (u32::from(*channel) * 255 + alpha / 2)
                    .checked_div(alpha)
                    .unwrap_or(0)
                    .min(255) as u8;
            }
        }
        (Content::Color, rgba)
    } else {
        (
            Content::Mask,
            pixels.as_chunks::<4>().0.iter().map(|p| p[3]).collect(),
        )
    };
    Some(trim(Image {
        content,
        data,
        placement: Placement {
            left: left as i32,
            top: top as i32,
            width: width as u32,
            height: height as u32,
        },
        ..Image::default()
    }))
}

// Transparent raster margins must not influence fallback sizing.
pub(super) fn trim(mut image: Image) -> Image {
    let p = image.placement;
    let stride = if image.content == Content::Mask { 1 } else { 4 };
    let (mut left, mut top, mut right, mut bottom) = (p.width, p.height, 0, 0);
    for y in 0..p.height {
        for x in 0..p.width {
            let offset = (y * p.width + x) as usize * stride;
            let visible = match image.content {
                Content::Mask => image.data[offset] != 0,
                Content::Color => image.data[offset + 3] != 0,
                Content::SubpixelMask => image.data[offset..offset + 3]
                    .iter()
                    .any(|&channel| channel != 0),
            };
            if visible {
                left = left.min(x);
                top = top.min(y);
                right = right.max(x + 1);
                bottom = bottom.max(y + 1);
            }
        }
    }
    if right == 0 {
        return Image::default();
    }
    if (left, top, right, bottom) == (0, 0, p.width, p.height) {
        return image;
    }
    let mut data =
        Vec::with_capacity(((right - left) * (bottom - top)) as usize * stride);
    for y in top..bottom {
        let start = (y * p.width + left) as usize * stride;
        let end = (y * p.width + right) as usize * stride;
        data.extend_from_slice(&image.data[start..end]);
    }
    image.data = data;
    image.placement = Placement {
        left: p.left + left as i32,
        top: p.top - top as i32,
        width: right - left,
        height: bottom - top,
    };
    image
}
