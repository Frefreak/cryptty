use std::collections::HashMap;

use crate::config::FontMapping;
use crate::terminal::FontStyle;
use anyhow::{Context, Result};
use fontdb::{Database, Family, ID, Query, Weight};
use swash::scale::image::Image;
use swash::scale::{Render, ScaleContext, Source, StrikeWith};
use swash::{CacheKey, FontRef};

struct Font {
    bytes: Vec<u8>,
    offset: u32,
    key: CacheKey,
}

impl Font {
    fn load(db: &Database, id: ID) -> Option<Self> {
        db.with_face_data(id, |bytes, index| {
            let font = FontRef::from_index(bytes, index as usize)?;
            Some(Self {
                bytes: bytes.to_vec(),
                offset: font.offset,
                key: font.key,
            })
        })?
    }

    fn as_ref(&self) -> FontRef<'_> {
        FontRef {
            data: &self.bytes,
            offset: self.offset,
            key: self.key,
        }
    }
}

pub struct Rasterizer {
    db: Database,
    fonts: HashMap<ID, Font>,
    faces: [ID; 4],
    mappings: Vec<(FontMapping, [ID; 4])>,
    fallbacks: Vec<[ID; 4]>,
    scale: ScaleContext,
    size: f32,
    hinting: bool,
    embolden: crate::config::FontEmbolden,
    pub cell_width: usize,
    pub cell_height: usize,
    pub baseline: i32,
}

impl Rasterizer {
    pub fn rasterize_label(
        &mut self,
        ch: char,
        embolden: crate::config::FontEmbolden,
    ) -> Option<Image> {
        self.embolden = embolden;
        self.rasterize(ch, FontStyle::default())
    }

    pub fn new(
        size: f32,
        hinting: bool,
        family: &str,
        mappings: &[FontMapping],
        fallbacks: &[String],
        embolden: crate::config::FontEmbolden,
    ) -> Result<Self> {
        let mut db = Database::new();
        db.load_system_fonts();
        let families = [
            Family::Name(family),
            Family::Name("FantasqueSansM Nerd Font Mono"),
            Family::Name("Menlo"),
            Family::Name("DejaVu Sans Mono"),
            Family::Name("Liberation Mono"),
            Family::Monospace,
        ];
        let regular = db
            .query(&Query {
                families: &families,
                ..Query::default()
            })
            .or_else(|| db.faces().find(|f| f.monospaced).map(|f| f.id))
            .context(
                "No monospace font found; install a system monospace font",
            )?;
        let faces = styled_faces(&db, regular);
        let mappings = mappings
            .iter()
            .map(|mapping| {
                let id = db.query(&Query {
                    families: &[Family::Name(mapping.family())],
                    ..Query::default()
                });
                if id.is_none() {
                    eprintln!(
                        "Warning: mapped font {:?} unavailable; using {:?}",
                        mapping.family(),
                        db.face(regular).unwrap().families[0].0,
                    );
                }
                (mapping.clone(), styled_faces(&db, id.unwrap_or(regular)))
            })
            .collect();
        let fallbacks = fallbacks
            .iter()
            .filter_map(|name| {
                let id = db.query(&Query {
                    families: &[Family::Name(name)],
                    ..Query::default()
                });
                if id.is_none() {
                    eprintln!(
                        "Warning: fallback font {name:?} unavailable; skipping"
                    );
                }
                id.map(|id| styled_faces(&db, id))
            })
            .collect();
        let font =
            Font::load(&db, regular).context("Cannot load monospace font")?;
        let face = font.as_ref();
        let metrics = face.metrics(&[]).scale(size);
        let advance = face
            .glyph_metrics(&[])
            .scale(size)
            .advance_width(face.charmap().map('M'));
        let cell_width = advance.floor().max(1.0) as usize;
        // Keep bottom coverage for descenders beyond rounded metrics.
        let cell_height = (metrics.ascent + metrics.descent + metrics.leading)
            .ceil()
            .max(size)
            .max(1.0) as usize
            + 1;
        let baseline = metrics.ascent.ceil() as i32;
        let mut fonts = HashMap::new();
        fonts.insert(regular, font);
        for id in faces.into_iter().skip(1) {
            if let Some(font) = Font::load(&db, id) {
                fonts.insert(id, font);
            }
        }
        Ok(Self {
            db,
            fonts,
            faces,
            mappings,
            fallbacks,
            scale: ScaleContext::new(),
            size,
            hinting,
            embolden,
            cell_width,
            cell_height,
            baseline,
        })
    }

    pub fn rasterize(&mut self, ch: char, style: FontStyle) -> Option<Image> {
        let id = self.face_for(ch, style);
        let size = self.size;
        if let std::collections::hash_map::Entry::Vacant(entry) =
            self.fonts.entry(id)
        {
            entry.insert(Font::load(&self.db, id)?);
        }
        let image = self.render_at_size(id, ch, size)?;
        if self.faces.contains(&id) {
            return Some(image);
        }
        super::glyph_fit::fit(
            ch,
            image,
            self.cell_width,
            self.cell_height,
            self.baseline,
            |scale| self.render_at_size(id, ch, size * scale),
        )
    }

    fn render_at_size(&mut self, id: ID, ch: char, size: f32) -> Option<Image> {
        let font = self.fonts.get(&id)?.as_ref();
        let glyph = font.charmap().map(ch);
        let mut scaler = self
            .scale
            .builder(font)
            .size(size)
            .hint(self.hinting)
            .build();
        Render::new(&[
            Source::ColorOutline(0),
            Source::ColorBitmap(StrikeWith::BestFit),
            Source::Outline,
        ])
        .embolden(self.embolden.pixels())
        .format(swash::zeno::Format::Alpha)
        .render(&mut scaler, glyph)
    }

    fn face_for(&self, ch: char, style: FontStyle) -> ID {
        let preferred = self.faces[style.index()];
        let mapped = self.mappings.iter().rev().find_map(|(mapping, faces)| {
            mapping.contains(ch).then_some(faces[style.index()])
        });
        mapped
            .into_iter()
            .chain(std::iter::once(preferred))
            .chain(self.fallbacks.iter().map(|faces| faces[style.index()]))
            .chain(self.db.faces().map(|face| face.id))
            .find(|&id| self.supports(id, ch))
            .unwrap_or(preferred)
    }

    fn supports(&self, id: ID, ch: char) -> bool {
        if let Some(font) = self.fonts.get(&id) {
            return font.as_ref().charmap().map(ch) != 0;
        }
        self.db
            .with_face_data(id, |bytes, index| {
                FontRef::from_index(bytes, index as usize)
                    .is_some_and(|font| font.charmap().map(ch) != 0)
            })
            .unwrap_or(false)
    }
}

fn styled_faces(db: &Database, regular: ID) -> [ID; 4] {
    let family = &db.face(regular).unwrap().families[0].0;
    let find = |weight, style, fallback| {
        db.query(&Query {
            families: &[Family::Name(family)],
            weight,
            style,
            ..Query::default()
        })
        .unwrap_or(fallback)
    };
    let bold = find(Weight::BOLD, fontdb::Style::Normal, regular);
    let italic = find(Weight::NORMAL, fontdb::Style::Italic, regular);
    let bold_italic = find(Weight::BOLD, fontdb::Style::Italic, bold);
    [regular, bold, italic, bold_italic]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embolden_increases_outline_coverage_without_changing_cell_metrics() {
        let mut normal = Rasterizer::new(
            38.0,
            false,
            "monospace",
            &[],
            &[],
            Default::default(),
        )
        .unwrap();
        let mut heavier = Rasterizer::new(
            38.0,
            false,
            "monospace",
            &[],
            &[],
            0.15.try_into().unwrap(),
        )
        .unwrap();
        assert_eq!(
            (normal.cell_width, normal.cell_height, normal.baseline),
            (heavier.cell_width, heavier.cell_height, heavier.baseline),
        );
        for bold in [false, true] {
            let style = FontStyle {
                bold,
                italic: false,
            };
            for ch in ['M', 'i', 'g', '@'] {
                let before = normal.rasterize(ch, style).unwrap();
                let after = heavier.rasterize(ch, style).unwrap();
                assert_eq!(before.content, swash::scale::image::Content::Mask);
                assert_eq!(after.content, before.content);
                let coverage = |image: &Image| {
                    image
                        .data
                        .iter()
                        .map(|&alpha| u64::from(alpha))
                        .sum::<u64>()
                };
                assert!(
                    coverage(&after) > coverage(&before),
                    "{ch}, bold={bold}"
                );
            }
        }
    }

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
            #[allow(unused_mut)]
            let mut r = Rasterizer::new(
                38.0,
                false,
                "Menlo",
                &config.font.codepoint_map,
                &config.font.fallback,
                Default::default(),
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
                    let index = r.face_for(ch, style);
                    assert_eq!(
                        r.db.face(index).unwrap().families[0].0.clone(),
                        expected
                    );
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

    #[test]
    fn fallback_keeps_configured_size_when_its_bitmap_fits() {
        let mut db = Database::new();
        db.load_system_fonts();
        for name in ["FantasqueSansM Nerd Font Mono", "DejaVu Sans Mono"] {
            if db
                .query(&Query {
                    families: &[Family::Name(name)],
                    ..Query::default()
                })
                .is_none()
            {
                eprintln!("Skipping font fixture: {name} is unavailable");
                return;
            }
        }
        let mut rasterizer = Rasterizer::new(
            17.0 * 96.0 / 72.0 * 1.25,
            false,
            "FantasqueSansM Nerd Font Mono",
            &[],
            &["DejaVu Sans Mono".to_owned()],
            Default::default(),
        )
        .unwrap();
        let style = FontStyle::default();
        let id = rasterizer.face_for('⇡', style);
        assert_eq!(
            rasterizer.db.face(id).unwrap().families[0].0,
            "DejaVu Sans Mono"
        );
        let actual = rasterizer.rasterize('⇡', style).unwrap();
        let expected =
            rasterizer.render_at_size(id, '⇡', rasterizer.size).unwrap();
        assert_eq!(actual.placement.left, expected.placement.left);
        assert_eq!(actual.placement.top, expected.placement.top);
        assert_eq!(actual.placement.width, expected.placement.width);
        assert_eq!(actual.placement.height, expected.placement.height);
        assert_eq!(actual.data, expected.data);
    }

    #[test]
    fn primary_glyphs_stay_raw_and_fallbacks_fit_across_sizes_and_styles() {
        for size in [16.0, 28.333334, 38.0] {
            let mut rasterizer = Rasterizer::new(
                size,
                false,
                "FantasqueSansM Nerd Font Mono",
                &[],
                &["DejaVu Sans Mono".to_owned()],
                Default::default(),
            )
            .unwrap();
            for bold in [false, true] {
                for italic in [false, true] {
                    for ch in "%AWmMHxgj⇡①中😀\u{e0b0}".chars() {
                        let style = FontStyle { bold, italic };
                        let id = rasterizer.face_for(ch, style);
                        let image = rasterizer.rasterize(ch, style).unwrap();
                        let p = image.placement;
                        if rasterizer.faces.contains(&id) {
                            let raw = rasterizer
                                .render_at_size(id, ch, size)
                                .unwrap();
                            assert_eq!(p.left, raw.placement.left);
                            assert_eq!(p.top, raw.placement.top);
                            assert_eq!(p.width, raw.placement.width);
                            assert_eq!(p.height, raw.placement.height);
                            assert_eq!(image.content, raw.content);
                            assert_eq!(image.data, raw.data);
                            continue;
                        }
                        let columns =
                            unicode_width::UnicodeWidthChar::width(ch).unwrap();
                        assert!(p.left >= 0, "{ch} {size} {p:?}");
                        assert!(
                            p.left as usize + p.width as usize
                                <= rasterizer.cell_width * columns,
                            "{ch} {size} {p:?}"
                        );
                        assert!(p.top <= rasterizer.baseline);
                        assert!(
                            rasterizer.baseline - p.top + p.height as i32
                                <= rasterizer.cell_height as i32
                        );
                        assert!(image.data.iter().any(|&v| v != 0));
                    }
                }
            }
        }
    }

    #[test]
    fn configured_family_is_used_for_regular_and_bold() {
        let default = Rasterizer::new(
            19.0,
            false,
            &crate::config::Font::default().family,
            &[],
            &[],
            Default::default(),
        )
        .unwrap();
        let default_family =
            &default.db.face(default.faces[0]).unwrap().families[0].0;
        let family = default
            .db
            .faces()
            .find(|face| {
                face.monospaced
                    && face.style == fontdb::Style::Normal
                    && face.weight == Weight::NORMAL
            })
            .expect("A system monospace family is required")
            .families[0]
            .0
            .clone();
        let selected =
            Rasterizer::new(19.0, false, &family, &[], &[], Default::default())
                .unwrap();
        for id in selected.faces {
            assert!(
                selected
                    .db
                    .face(id)
                    .unwrap()
                    .families
                    .iter()
                    .any(|(name, _)| name == &family)
            );
        }
        eprintln!(
            "Default family: {default_family}; requested family: {family}"
        );
    }

    #[test]
    fn disjoint_mappings_and_later_overrides_preserve_font_styles() {
        let base = Rasterizer::new(
            28.0,
            false,
            "monospace",
            &[],
            &[],
            Default::default(),
        )
        .unwrap();
        let primary =
            base.db.face(base.faces[0]).unwrap().families[0].0.clone();
        let alternate = base
            .db
            .faces()
            .find(|face| {
                face.monospaced
                    && face.style == fontdb::Style::Normal
                    && face.weight == Weight::NORMAL
                    && face.families[0].0 != primary
            })
            .expect("Two system monospace families are required")
            .families[0]
            .0
            .clone();
        let config = crate::config::Config::parse(&format!(
            "font:\n  codepoint_map:\n    - ranges: ['U+0041-U+0043', 'U+0058-U+005A']\n      family: {alternate:?}\n    - ranges: ['U+0042']\n      family: {primary:?}\n"
        )).unwrap();
        let r = Rasterizer::new(
            28.0,
            false,
            &primary,
            &config.font.codepoint_map,
            &config.font.fallback,
            Default::default(),
        )
        .unwrap();
        for style in [
            FontStyle::default(),
            FontStyle {
                bold: true,
                italic: false,
            },
            FontStyle {
                bold: false,
                italic: true,
            },
            FontStyle {
                bold: true,
                italic: true,
            },
        ] {
            for ch in ['A', 'C', 'Y'] {
                let face = r.db.face(r.face_for(ch, style)).unwrap();
                assert!(
                    face.families.iter().any(|(name, _)| name == &alternate)
                );
                assert_eq!(face.weight == Weight::BOLD, style.bold);
                assert_eq!(face.style == fontdb::Style::Italic, style.italic);
            }
            assert_eq!(r.face_for('B', style), r.faces[style.index()]);
            assert_eq!(r.face_for('d', style), r.faces[style.index()]);
        }
    }
}
