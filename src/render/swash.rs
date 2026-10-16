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
    fallback_regular: Option<ID>,
    fallback_bold: Option<ID>,
    scale: ScaleContext,
    size: f32,
    family_sizes: HashMap<String, f32>,
    hinting: bool,
    pub cell_width: usize,
    pub cell_height: usize,
    pub baseline: i32,
}

impl Rasterizer {
    pub fn new(
        size: f32,
        hinting: bool,
        family: &str,
        mappings: &[FontMapping],
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
        let fallback_regular = db.query(&Query {
            families: &[Family::Name("LXGW WenKai Mono Screen")],
            ..Query::default()
        });
        let fallback_bold = db
            .query(&Query {
                families: &[Family::Name("LXGW WenKai Mono Screen")],
                weight: Weight::BOLD,
                ..Query::default()
            })
            .or(fallback_regular);
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
        let family_sizes = HashMap::from([(
            db.face(regular).unwrap().families[0].0.clone(),
            size,
        )]);
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
            fallback_regular,
            fallback_bold,
            scale: ScaleContext::new(),
            size,
            family_sizes,
            hinting,
            cell_width,
            cell_height,
            baseline,
        })
    }

    pub fn rasterize(&mut self, ch: char, style: FontStyle) -> Option<Image> {
        let id = self.face_for(ch, style);
        let size = self.font_size(id)?;
        if let std::collections::hash_map::Entry::Vacant(entry) =
            self.fonts.entry(id)
        {
            entry.insert(Font::load(&self.db, id)?);
        }
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
        .format(swash::zeno::Format::Alpha)
        .render(&mut scaler, glyph)
    }

    fn font_size(&mut self, id: ID) -> Option<f32> {
        let family = self.db.face(id)?.families[0].0.clone();
        if let Some(&size) = self.family_sizes.get(&family) {
            return Some(size);
        }
        let regular = self
            .db
            .query(&Query {
                families: &[Family::Name(&family)],
                ..Query::default()
            })
            .unwrap_or(id);
        if let std::collections::hash_map::Entry::Vacant(entry) =
            self.fonts.entry(regular)
        {
            entry.insert(Font::load(&self.db, regular)?);
        }
        let font = self.fonts.get(&regular)?.as_ref();
        let metrics = font.glyph_metrics(&[]).scale(self.size);
        let size =
            super::normalization::font_size(self.size, self.cell_width, |ch| {
                let glyph = font.charmap().map(ch);
                (glyph != 0).then(|| metrics.advance_width(glyph))
            });
        self.family_sizes.insert(family, size);
        Some(size)
    }

    fn face_for(&self, ch: char, style: FontStyle) -> ID {
        let preferred = self.faces[style.index()];
        let mapped = self.mappings.iter().rev().find_map(|(mapping, faces)| {
            mapping.contains(ch).then_some(faces[style.index()])
        });
        let fallback = if style.bold {
            self.fallback_bold
        } else {
            self.fallback_regular
        };
        mapped
            .into_iter()
            .chain(std::iter::once(preferred))
            .chain(fallback)
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
    fn family_size_is_shared_across_styles_and_request_order() {
        let mut first = Rasterizer::new(38.0, false, "monospace", &[]).unwrap();
        let primary = first.faces[0];
        assert_eq!(first.font_size(primary), Some(38.0));
        let alternate = first
            .db
            .faces()
            .find(|face| {
                face.families[0].0
                    != first.db.face(primary).unwrap().families[0].0
                    && face.style == fontdb::Style::Normal
                    && face.weight == Weight::NORMAL
                    && first.supports(face.id, 'M')
                    && first.supports(face.id, 'g')
            })
            .unwrap()
            .id;
        let faces = styled_faces(&first.db, alternate);
        let family = first.db.face(alternate).unwrap().families[0].0.clone();
        let config = crate::config::Config::parse(&format!(
            "font:\n  codepoint_map:\n    - ranges: ['U+0041-U+007A']\n      family: {family:?}\n"
        )).unwrap();
        let mut second = Rasterizer::new(
            38.0,
            false,
            "monospace",
            &config.font.codepoint_map,
        )
        .unwrap();
        let bold_size = first.font_size(faces[1]).unwrap();
        let second_regular = second
            .db
            .query(&Query {
                families: &[Family::Name(&family)],
                ..Query::default()
            })
            .unwrap();
        let regular_size = second.font_size(second_regular).unwrap();
        assert_eq!(bold_size, regular_size);
        for face in faces {
            assert_eq!(first.font_size(face), Some(regular_size));
        }
        for (ch, style) in [
            ('g', FontStyle::default()),
            (
                'M',
                FontStyle {
                    bold: true,
                    italic: true,
                },
            ),
        ] {
            let image = second.rasterize(ch, style).unwrap();
            assert!(image.data.iter().any(|&pixel| pixel != 0));
        }
        assert!(second.cell_width > 0);
        assert!(second.baseline > 0);
        assert!(second.cell_height >= second.baseline as usize);
    }

    #[test]
    fn configured_family_is_used_for_regular_and_bold() {
        let default = Rasterizer::new(
            19.0,
            false,
            &crate::config::Font::default().family,
            &[],
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
        let selected = Rasterizer::new(19.0, false, &family, &[]).unwrap();
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
        let base = Rasterizer::new(28.0, false, "monospace", &[]).unwrap();
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
        let r =
            Rasterizer::new(28.0, false, &primary, &config.font.codepoint_map)
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
