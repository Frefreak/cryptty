use std::collections::HashMap;

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
    regular: ID,
    bold: ID,
    fallback_regular: Option<ID>,
    fallback_bold: Option<ID>,
    scale: ScaleContext,
    size: f32,
    hinting: bool,
    pub cell_width: usize,
    pub cell_height: usize,
    pub baseline: i32,
}

impl Rasterizer {
    pub fn new(size: f32, hinting: bool, family: &str) -> Result<Self> {
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
        let family = db.face(regular).unwrap().families[0].0.clone();
        let bold = db
            .query(&Query {
                families: &[Family::Name(&family)],
                weight: Weight::BOLD,
                ..Query::default()
            })
            .unwrap_or(regular);
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
        let cell_width = face
            .glyph_metrics(&[])
            .scale(size)
            .advance_width(face.charmap().map('M'))
            .floor()
            .max(1.0) as usize;
        let cell_height = (metrics.ascent + metrics.descent + metrics.leading)
            .ceil()
            .max(size)
            .max(1.0) as usize
            + 2;
        let baseline = metrics.ascent.ceil() as i32 + 1;
        let mut fonts = HashMap::new();
        fonts.insert(regular, font);
        if let Some(font) = Font::load(&db, bold) {
            fonts.insert(bold, font);
        }
        Ok(Self {
            db,
            fonts,
            regular,
            bold,
            fallback_regular,
            fallback_bold,
            scale: ScaleContext::new(),
            size,
            hinting,
            cell_width,
            cell_height,
            baseline,
        })
    }

    pub fn rasterize(&mut self, ch: char, bold: bool) -> Option<Image> {
        let preferred = if bold { self.bold } else { self.regular };
        let mut id = preferred;
        let supported = self
            .fonts
            .get(&id)
            .is_some_and(|font| font.as_ref().charmap().map(ch) != 0);
        if !supported {
            let preferred_fallback = if bold {
                self.fallback_bold
            } else {
                self.fallback_regular
            };
            let fallback = preferred_fallback
                .into_iter()
                .chain(self.db.faces().map(|face| face.id))
                .find_map(|id| {
                    self.db
                        .with_face_data(id, |bytes, index| {
                            let font =
                                FontRef::from_index(bytes, index as usize)?;
                            (font.charmap().map(ch) != 0).then_some(id)
                        })
                        .flatten()
                });
            if let Some(found) = fallback {
                id = found;
            }
        }
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
            .size(self.size)
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_family_is_used_for_regular_and_bold() {
        let default = Rasterizer::new(
            19.0,
            false,
            &crate::config::Font::default().family,
        )
        .unwrap();
        let default_family =
            &default.db.face(default.regular).unwrap().families[0].0;
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
        let selected = Rasterizer::new(19.0, false, &family).unwrap();
        for id in [selected.regular, selected.bold] {
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
}
