use core_graphics::base::kCGImageAlphaPremultipliedLast;
use core_graphics::color_space::CGColorSpace;
use core_graphics::context::CGContext;
use core_graphics::geometry::{CGPoint, CGRect, CGSize};
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
    let bounds =
        font.get_bounding_rects_for_glyphs(kCTFontDefaultOrientation, &[glyph]);
    let left = bounds.origin.x.floor();
    let bottom = bounds.origin.y.floor();
    let top = (bounds.origin.y + bounds.size.height).ceil();
    let width = (bounds.origin.x + bounds.size.width).ceil() - left;
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
    let colored = font.symbolic_traits() & kCTFontColorGlyphsTrait != 0;
    context.set_rgb_fill_color(0.0, 0.0, 0.0, 0.0);
    context.fill_rect(CGRect::new(
        &CGPoint::new(0.0, 0.0),
        &CGSize::new(width, height),
    ));
    context.set_should_smooth_fonts(smooth && !colored);
    context.set_should_antialias(true);
    context.set_rgb_fill_color(1.0, 1.0, 1.0, 1.0);
    font.draw_glyphs(
        &[glyph],
        &[CGPoint::new(-left, -bottom)],
        context.clone(),
    );
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
    Some(Image {
        content,
        data,
        placement: Placement {
            left: left as i32,
            top: top as i32,
            width: width as u32,
            height: height as u32,
        },
        ..Image::default()
    })
}
