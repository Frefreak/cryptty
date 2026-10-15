use anyhow::{Context, Result};
use crossfont::{
    BitmapBuffer, FontDesc, FontKey, GlyphKey, Rasterize, Size, Slant, Style,
    Weight,
};
use swash::scale::image::{Content, Image};
use swash::zeno::Placement;

pub struct Rasterizer {
    inner: crossfont::Rasterizer,
    regular: FontKey,
    bold: FontKey,
    size: Size,
    pub cell_width: usize,
    pub cell_height: usize,
    pub baseline: i32,
}

impl Rasterizer {
    pub fn new(size: f32, _hinting: bool, family: &str) -> Result<Self> {
        // CoreText takes the scaled macOS point size directly.
        let size = Size::new(size);
        let mut inner = crossfont::Rasterizer::new()?;
        let desc = |weight| {
            FontDesc::new(
                family,
                Style::Description {
                    slant: Slant::Normal,
                    weight,
                },
            )
        };
        let regular = inner
            .load_font(&desc(Weight::Normal), size)
            .context("Cannot load native monospace font")?;
        let bold = inner
            .load_font(&desc(Weight::Bold), size)
            .unwrap_or(regular);
        let metrics = inner.metrics(regular, size)?;
        Ok(Self {
            inner,
            regular,
            bold,
            size,
            cell_width: metrics.average_advance.floor().max(1.0) as usize,
            cell_height: metrics.line_height.ceil().max(size.as_pt() as f64)
                as usize
                + 2,
            baseline: (metrics.line_height + metrics.descent as f64).ceil()
                as i32
                + 1,
        })
    }

    pub fn rasterize(&mut self, ch: char, bold: bool) -> Option<Image> {
        let glyph = match self.inner.get_glyph(GlyphKey {
            character: ch,
            font_key: if bold { self.bold } else { self.regular },
            size: self.size,
        }) {
            Ok(glyph) | Err(crossfont::Error::MissingGlyph(glyph)) => glyph,
            Err(_) => return None,
        };
        let (content, data) = match glyph.buffer {
            BitmapBuffer::Rgb(rgb) => (
                Content::SubpixelMask,
                rgb.chunks_exact(3)
                    .flat_map(|p| [p[0], p[1], p[2], 255])
                    .collect(),
            ),
            BitmapBuffer::Rgba(mut rgba) => {
                // The shared compositor expects straight-alpha color pixels.
                for pixel in rgba.chunks_exact_mut(4) {
                    let alpha = pixel[3] as u32;
                    if alpha > 0 {
                        for channel in &mut pixel[..3] {
                            *channel = ((*channel as u32 * 255 + alpha / 2)
                                / alpha)
                                .min(255)
                                as u8;
                        }
                    }
                }
                (Content::Color, rgba)
            }
        };
        Some(Image {
            content,
            data,
            placement: Placement {
                left: glyph.left,
                top: glyph.top,
                width: glyph.width as u32,
                height: glyph.height as u32,
            },
            ..Image::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_family_falls_back_to_a_system_font() {
        let mut native = Rasterizer::new(
            19.0,
            false,
            "Cryptty deliberately nonexistent font family",
        )
        .unwrap();
        let image = native.rasterize('M', false).unwrap();
        assert!(image.placement.width > 0 && image.placement.height > 0);
    }

    #[test]
    fn requested_family_and_weight_match_coretext() {
        let mut native = Rasterizer::new(38.0, false, "Menlo").unwrap();
        let mut reference = crossfont::Rasterizer::new().unwrap();
        let size = Size::new(38.0);
        for (bold, weight) in [(false, Weight::Normal), (true, Weight::Bold)] {
            let key = reference
                .load_font(
                    &FontDesc::new(
                        "Menlo",
                        Style::Description {
                            slant: Slant::Normal,
                            weight,
                        },
                    ),
                    size,
                )
                .unwrap();
            let expected = reference
                .get_glyph(GlyphKey {
                    character: 'M',
                    font_key: key,
                    size,
                })
                .unwrap();
            let actual = native.rasterize('M', bold).unwrap();
            assert_eq!(actual.placement.left, expected.left);
            assert_eq!(actual.placement.top, expected.top);
            assert_eq!(actual.placement.width, expected.width as u32);
            assert_eq!(actual.placement.height, expected.height as u32);
            assert_eq!(actual.content, Content::SubpixelMask);
            let BitmapBuffer::Rgb(rgb) = expected.buffer else {
                panic!("Expected a text mask");
            };
            let actual_rgb: Vec<_> = actual
                .data
                .chunks_exact(4)
                .flat_map(|p| p[..3].iter().copied())
                .collect();
            assert_eq!(actual_rgb, rgb);
        }
    }

    #[test]
    fn native_fallback_renders_cjk_and_color_emoji() {
        let mut native = Rasterizer::new(38.0, false, "Menlo").unwrap();
        for ch in ['中', '😀'] {
            let original = native
                .inner
                .get_glyph(GlyphKey {
                    character: ch,
                    font_key: native.regular,
                    size: native.size,
                })
                .expect("CoreText must find a fallback glyph");
            let image = native.rasterize(ch, false).unwrap();
            assert!(image.placement.width > 0 && image.placement.height > 0);
            assert_eq!(
                image.data.len(),
                image.placement.width as usize
                    * image.placement.height as usize
                    * 4
            );
            if let BitmapBuffer::Rgba(rgba) = original.buffer {
                assert_eq!(image.content, Content::Color);
                for (straight, premultiplied) in
                    image.data.chunks_exact(4).zip(rgba.chunks_exact(4))
                {
                    assert_eq!(straight[3], premultiplied[3]);
                    for i in 0..3 {
                        let restored =
                            (straight[i] as u32 * straight[3] as u32 + 127)
                                / 255;
                        assert!(
                            restored.abs_diff(premultiplied[i] as u32) <= 1
                        );
                    }
                }
            } else {
                assert_eq!(image.content, Content::SubpixelMask);
            }
            if ch == '😀' {
                assert_eq!(image.content, Content::Color);
            }
        }
    }
}
