use anyhow::{Context, Result};
use core_foundation::base::{CFRange, TCFType};
use core_foundation::string::{CFString, CFStringRef};
use core_text::font::{CTFont, CTFontRef, new_from_descriptor};
use core_text::font_collection::create_for_family;
use core_text::font_descriptor::{
    SymbolicTraitAccessors, kCTFontDefaultOrientation,
};
use crossfont::{FontDesc, FontKey, Rasterize, Size, Style};

pub(super) struct Family {
    pub name: String,
    pub size: Size,
    pub faces: [Face; 4],
}

pub(super) struct Face {
    pub key: Option<FontKey>,
    font: CTFont,
}

impl Family {
    pub fn load(
        inner: &mut crossfont::Rasterizer,
        name: &str,
        size: Size,
        cell_width: Option<usize>,
    ) -> Result<Self> {
        let collection = create_for_family(name)
            .with_context(|| format!("Font family unavailable: {name}"))?;
        let descriptors = collection
            .get_descriptors()
            .context("Font family has no faces")?;
        let fonts: Vec<_> = descriptors
            .iter()
            .map(|descriptor| {
                new_from_descriptor(&descriptor, f64::from(size.as_pt()))
            })
            .collect();
        let regular = fonts
            .iter()
            .find(|font| {
                let traits = font.symbolic_traits();
                !traits.is_bold() && !traits.is_italic()
            })
            .or_else(|| fonts.first())
            .context("Font family has no faces")?;
        let size = if let Some(width) = cell_width {
            Size::new(super::normalization::font_size(
                size.as_pt(),
                width,
                |ch| advance(regular, ch),
            ))
        } else {
            size
        };
        let name = regular.family_name();
        let load = |inner: &mut crossfont::Rasterizer, bold, italic| {
            let font = fonts
                .iter()
                .find(|font| {
                    let traits = font.symbolic_traits();
                    traits.is_bold() == bold && traits.is_italic() == italic
                })
                .or_else(|| {
                    fonts.iter().find(|font| {
                        let traits = font.symbolic_traits();
                        traits.is_bold() == bold && !traits.is_italic()
                    })
                })
                .unwrap_or(regular);
            let key = inner.load_font(
                &FontDesc::new(&name, Style::Specific(font.style_name())),
                size,
            )?;
            Ok::<_, crossfont::Error>(Face {
                key: Some(key),
                font: font.clone_with_font_size(f64::from(size.as_pt())),
            })
        };
        let faces = [
            load(inner, false, false)?,
            load(inner, true, false)?,
            load(inner, false, true)?,
            load(inner, true, true)?,
        ];
        Ok(Self { name, size, faces })
    }

    pub fn from_fallback(font: CTFont, size: Size, width: usize) -> Self {
        use core_text::font_descriptor::{
            kCTFontBoldTrait, kCTFontItalicTrait,
        };
        let mask = kCTFontBoldTrait | kCTFontItalicTrait;
        let regular = with_traits(&font, 0, mask).unwrap_or(font);
        let regular = regular.clone_with_font_size(f64::from(size.as_pt()));
        let size = Size::new(super::normalization::font_size(
            size.as_pt(),
            width,
            |ch| advance(&regular, ch),
        ));
        let regular = regular.clone_with_font_size(f64::from(size.as_pt()));
        let faces =
            [0, kCTFontBoldTrait, kCTFontItalicTrait, mask].map(|traits| {
                let font = with_traits(&regular, traits, mask)
                    .or_else(|| {
                        with_traits(&regular, traits & kCTFontBoldTrait, mask)
                    })
                    .unwrap_or_else(|| regular.clone());
                Face { key: None, font }
            });
        Self {
            name: regular.family_name(),
            size,
            faces,
        }
    }
}

fn with_traits(font: &CTFont, traits: u32, mask: u32) -> Option<CTFont> {
    if font.symbolic_traits() & mask == traits {
        return Some(font.clone());
    }
    font.clone_with_symbolic_traits(traits, mask)
        .filter(|variant| variant.family_name() == font.family_name())
}

impl Face {
    pub fn supports(&self, ch: char) -> bool {
        glyph(&self.font, ch).is_some()
    }

    pub fn fallback_font(&self, ch: char) -> Option<CTFont> {
        let text = CFString::new(&ch.to_string());
        // CoreText returns an owned font; the range counts UTF-16 units.
        let font = unsafe {
            let raw = CTFontCreateForString(
                self.font.as_concrete_TypeRef(),
                text.as_concrete_TypeRef(),
                CFRange {
                    location: 0,
                    length: ch.len_utf16() as isize,
                },
            );
            if raw.is_null() {
                return None;
            }
            CTFont::wrap_under_create_rule(raw)
        };
        glyph(&font, ch).map(|_| font)
    }

    pub fn rasterize_scaled(
        &self,
        ch: char,
        scale: f32,
    ) -> Option<swash::scale::image::Image> {
        let font = self
            .font
            .clone_with_font_size(self.font.pt_size() * f64::from(scale));
        super::native_fallback::rasterize(&font, glyph(&font, ch)?, true)
    }

    pub fn rasterize(&self, ch: char) -> Option<swash::scale::image::Image> {
        super::native_fallback::rasterize(
            &self.font,
            glyph(&self.font, ch)?,
            false,
        )
    }
}

fn glyph(font: &CTFont, ch: char) -> Option<u16> {
    let mut units = [0; 2];
    let units = ch.encode_utf16(&mut units);
    let mut glyphs = [0; 2];
    unsafe {
        font.get_glyphs_for_characters(
            units.as_ptr(),
            glyphs.as_mut_ptr(),
            units.len() as isize,
        );
    }
    (glyphs[0] != 0).then_some(glyphs[0])
}

fn advance(font: &CTFont, ch: char) -> Option<f32> {
    let glyph = glyph(font, ch)?;
    Some(unsafe {
        font.get_advances_for_glyphs(
            kCTFontDefaultOrientation,
            &glyph,
            std::ptr::null_mut(),
            1,
        ) as f32
    })
}

#[link(name = "CoreText", kind = "framework")]
unsafe extern "C" {
    fn CTFontCreateForString(
        font: CTFontRef,
        string: CFStringRef,
        range: CFRange,
    ) -> CTFontRef;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emoji_keeps_two_columns_and_bold_survives_missing_italic() {
        let mut rasterizer = crossfont::Rasterizer::new().unwrap();
        let emoji = Family::load(
            &mut rasterizer,
            "Apple Color Emoji",
            Size::new(38.0),
            Some(19),
        )
        .unwrap();
        assert!(
            (advance(&emoji.faces[0].font, '😀').unwrap() - 38.0).abs() < 0.01
        );
        let cjk = Family::load(
            &mut rasterizer,
            "PingFang SC",
            Size::new(38.0),
            Some(19),
        )
        .unwrap();
        assert!(cjk.faces[3].font.symbolic_traits().is_bold());
        assert_eq!(
            cjk.faces[1].font.postscript_name(),
            cjk.faces[3].font.postscript_name()
        );
    }

    #[test]
    fn family_styles_share_one_size_and_match_the_target_advance() {
        let mut rasterizer = crossfont::Rasterizer::new().unwrap();
        for target in [10, 19, 27] {
            let family = Family::load(
                &mut rasterizer,
                "Menlo",
                Size::new(38.0),
                Some(target),
            )
            .unwrap();
            for face in &family.faces {
                assert!(
                    (face.font.pt_size() as f32 - family.size.as_pt()).abs()
                        < 0.001
                );
                assert!(
                    (advance(&face.font, 'M').unwrap() - target as f32).abs()
                        < 0.01
                );
            }
            for ch in ['▲', '▼', 'm', 'g'] {
                assert!(family.faces[0].supports(ch));
            }
        }
    }
}
