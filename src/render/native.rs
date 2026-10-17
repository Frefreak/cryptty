use crate::config::FontMapping;
use crate::terminal::FontStyle;
use anyhow::Result;
use crossfont::{BitmapBuffer, GlyphKey, Rasterize, Size};
use swash::scale::image::{Content, Image};
use swash::zeno::Placement;

use super::native_family::Family;

pub struct Rasterizer {
    inner: crossfont::Rasterizer,
    families: Vec<Family>,
    mappings: Vec<(FontMapping, usize)>,
    fallbacks: Vec<usize>,
    size: Size,
    pub cell_width: usize,
    pub cell_height: usize,
    pub baseline: i32,
}

impl Rasterizer {
    pub fn new(
        size: f32,
        _hinting: bool,
        family: &str,
        mappings: &[FontMapping],
        fallbacks: &[String],
    ) -> Result<Self> {
        // CoreText takes the scaled macOS point size directly.
        let size = Size::new(size);
        let mut inner = crossfont::Rasterizer::new()?;
        let primary =
            Family::load(&mut inner, family, size, None).or_else(|error| {
                eprintln!("Warning: {error}; using Menlo");
                Family::load(&mut inner, "Menlo", size, None)
            })?;
        let metrics = inner.metrics(primary.faces[0].key.unwrap(), size)?;
        let cell_width = metrics.average_advance.floor().max(1.0) as usize;
        let mut families = vec![primary];
        let mappings = mappings
            .iter()
            .map(|mapping| {
                let index = families
                    .iter()
                    .position(|family| family.name == mapping.family())
                    .or_else(|| {
                        match Family::load(
                            &mut inner,
                            mapping.family(),
                            size,
                            Some(cell_width),
                        ) {
                            Ok(family) => {
                                families.push(family);
                                Some(families.len() - 1)
                            }
                            Err(error) => {
                                eprintln!(
                                    "Warning: {error}; using primary font"
                                );
                                None
                            }
                        }
                    })
                    .unwrap_or(0);
                (mapping.clone(), index)
            })
            .collect();
        let fallbacks = fallbacks
            .iter()
            .filter_map(|name| {
                if let Some(index) =
                    families.iter().position(|family| family.name == *name)
                {
                    return Some(index);
                }
                match Family::load(&mut inner, name, size, Some(cell_width)) {
                    Ok(family) => {
                        families.push(family);
                        Some(families.len() - 1)
                    }
                    Err(error) => {
                        eprintln!("Warning: {error}; skipping fallback font");
                        None
                    }
                }
            })
            .collect();
        Ok(Self {
            inner,
            families,
            mappings,
            fallbacks,
            size,
            cell_width,
            // Keep bottom coverage for descenders beyond rounded metrics.
            cell_height: metrics.line_height.ceil().max(size.as_pt() as f64)
                as usize
                + 1,
            baseline: (metrics.line_height + metrics.descent as f64).ceil()
                as i32,
        })
    }

    pub fn rasterize(&mut self, ch: char, style: FontStyle) -> Option<Image> {
        let image = self.rasterize_raw(ch, style)?;
        let family = self.family_for(ch, style);
        super::symbol_fit::fit(
            ch,
            image,
            self.cell_width,
            self.cell_height,
            self.baseline,
            |scale| {
                self.families[family].faces[style.index()]
                    .rasterize_scaled(ch, scale)
            },
        )
    }

    fn rasterize_raw(&mut self, ch: char, style: FontStyle) -> Option<Image> {
        let family = self.family_for(ch, style);
        let family = &self.families[family];
        let face = &family.faces[style.index()];
        let Some(font_key) = face.key else {
            return face.rasterize(ch);
        };
        let key = GlyphKey {
            character: ch,
            font_key,
            size: family.size,
        };
        let glyph = match self.inner.get_glyph(key) {
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

    fn family_for(&mut self, ch: char, style: FontStyle) -> usize {
        let mut index = self
            .mappings
            .iter()
            .rev()
            .find_map(|(mapping, index)| mapping.contains(ch).then_some(*index))
            .unwrap_or(0);
        if self.families[index].faces[style.index()].supports(ch) {
            return index;
        }
        if self.families[0].faces[style.index()].supports(ch) {
            return 0;
        }
        if let Some(&index) = self.fallbacks.iter().find(|&&index| {
            self.families[index].faces[style.index()].supports(ch)
        }) {
            return index;
        }
        index = 0;
        let Some(font) =
            self.families[index].faces[style.index()].fallback_font(ch)
        else {
            return index;
        };
        let name = font.family_name();
        if let Some(index) =
            self.families.iter().position(|family| family.name == name)
        {
            return index;
        }
        let family = Family::load(
            &mut self.inner,
            &name,
            self.size,
            Some(self.cell_width),
        )
        .unwrap_or_else(|_| {
            Family::from_fallback(font, self.size, self.cell_width)
        });
        self.families.push(family);
        self.families.len() - 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn fallback_order_primary_and_explicit_mapping() {
        for names in [
            ["PingFang SC", "Hiragino Sans GB"],
            ["Hiragino Sans GB", "PingFang SC"],
        ] {
            let config = crate::config::Config::parse(&format!(
                "font:\n  fallback: [{:?}, {:?}]\n  codepoint_map:\n    - family: {:?}\n      ranges: ['U+6587']\n",
                names[0], names[1], names[1],
            )).unwrap();
            let mut r = Rasterizer::new(
                38.0,
                false,
                "Menlo",
                &config.font.codepoint_map,
                &config.font.fallback,
            )
            .unwrap();
            for style in [
                FontStyle::default(),
                FontStyle {
                    bold: true,
                    italic: true,
                },
            ] {
                for (ch, expected) in
                    [('中', names[0]), ('文', names[1]), ('M', "Menlo")]
                {
                    let index = r.family_for(ch, style);
                    assert_eq!(r.families[index].name.clone(), expected);
                }
                let image = r.rasterize('①', style).unwrap();
                let p = image.placement;
                assert!(p.left >= 0);
                assert!(p.left + p.width as i32 <= r.cell_width as i32);
                assert!(p.top <= r.baseline);
                assert!(
                    r.baseline - p.top + p.height as i32
                        <= r.cell_height as i32
                );
            }
        }
    }

    use crossfont::{FontDesc, Slant, Style, Weight};

    #[test]
    fn hidden_fallback_faces_remain_available() {
        let mut native =
            Rasterizer::new(38.0, false, "Menlo", &[], &[]).unwrap();
        for ch in ['\u{0526}', '\u{08ab}', '\u{0afe}', '\u{324e}'] {
            let index = native.family_for(ch, FontStyle::default());
            assert_ne!(index, 0, "Missing fallback for {ch}");
            assert!(native.families[index].faces[0].supports(ch));
            let image = native.rasterize(ch, FontStyle::default()).unwrap();
            assert!(image.placement.width > 0 && image.placement.height > 0);
            assert!(image.data.iter().any(|&byte| byte != 0));
        }
    }

    #[test]
    fn missing_family_falls_back_to_a_system_font() {
        let mut native = Rasterizer::new(
            19.0,
            false,
            "Cryptty deliberately nonexistent font family",
            &[],
            &[],
        )
        .unwrap();
        let image = native.rasterize('M', FontStyle::default()).unwrap();
        assert!(image.placement.width > 0 && image.placement.height > 0);
    }

    #[test]
    fn requested_family_and_weight_match_coretext() {
        let mut native =
            Rasterizer::new(38.0, false, "Menlo", &[], &[]).unwrap();
        let mut reference = crossfont::Rasterizer::new().unwrap();
        let size = Size::new(38.0);
        let metrics = native
            .inner
            .metrics(native.families[0].faces[0].key.unwrap(), size)
            .unwrap();
        assert_eq!(
            native.cell_height,
            metrics.line_height.ceil().max(size.as_pt() as f64) as usize + 1
        );
        assert_eq!(
            native.baseline,
            (metrics.line_height + metrics.descent as f64).ceil() as i32
        );
        for (bold, italic, weight, slant) in [
            (false, false, Weight::Normal, Slant::Normal),
            (true, false, Weight::Bold, Slant::Normal),
            (false, true, Weight::Normal, Slant::Italic),
            (true, true, Weight::Bold, Slant::Italic),
        ] {
            let key = reference
                .load_font(
                    &FontDesc::new(
                        "Menlo",
                        Style::Description { slant, weight },
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
            let actual =
                native.rasterize('M', FontStyle { bold, italic }).unwrap();
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
        let mut native =
            Rasterizer::new(38.0, false, "Menlo", &[], &[]).unwrap();
        for ch in ['中', '😀'] {
            let family = native.family_for(ch, FontStyle::default());
            assert_ne!(family, 0, "Expected an automatic fallback for {ch}");
            let family = &native.families[family];
            let original = native
                .inner
                .get_glyph(GlyphKey {
                    character: ch,
                    font_key: family.faces[0].key.unwrap(),
                    size: family.size,
                })
                .expect("CoreText must find a fallback glyph");
            let image = native.rasterize_raw(ch, FontStyle::default()).unwrap();
            assert_eq!(image.placement.left, original.left);
            assert_eq!(image.placement.top, original.top);
            assert_eq!(image.placement.width, original.width as u32);
            assert_eq!(image.placement.height, original.height as u32);
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
