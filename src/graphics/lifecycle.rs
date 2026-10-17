use super::Graphics;

impl Graphics {
    pub fn scroll(
        &mut self,
        top: usize,
        bottom: usize,
        delta: isize,
        cell: (u32, u32),
        history: bool,
    ) {
        if self.positioned.is_empty() {
            return;
        }
        let ch = cell.1.max(1) as f32;
        self.positioned.retain_mut(|p| {
            let image = &self.images[&p.image];
            let bounds = p.bounds(image, cell);
            let moves = if history {
                bounds[1] < (bottom + 1) as f32 * ch
            } else {
                bounds[3] > top as f32 * ch
                    && bounds[1] < (bottom + 1) as f32 * ch
                    && bounds[3] > 0.0
            };
            if !moves {
                return true;
            }
            p.row += delta;
            let height = p.layout.geometry(image, cell).size.1;
            if height <= 0.0 {
                return false;
            }
            let origin = p.row as f32 * ch + p.layout.offset.1 as f32;
            let min = if history {
                -10_000.0 * ch
            } else {
                top as f32 * ch
            };
            p.clip.0 = p.clip.0.max((min - origin) / height);
            p.clip.1 =
                p.clip.1.min(((bottom + 1) as f32 * ch - origin) / height);
            p.clip.0 < p.clip.1
        });
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn clear_visible(&mut self, rows: usize, cell: (u32, u32)) {
        self.positioned.retain(|p| {
            let bounds = p.bounds(&self.images[&p.image], cell);
            bounds[3] <= 0.0 || bounds[1] >= rows as f32 * cell.1.max(1) as f32
        });
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn clear_history(&mut self, cell: (u32, u32)) {
        self.positioned.retain_mut(|p| {
            if p.row >= 0 {
                return true;
            }
            let height = p.layout.geometry(&self.images[&p.image], cell).size.1;
            let origin =
                p.row as f32 * cell.1.max(1) as f32 + p.layout.offset.1 as f32;
            if height <= 0.0 {
                return false;
            }
            p.clip.0 = p.clip.0.max(-origin / height);
            p.clip.0 < p.clip.1
        });
        self.revision = self.revision.wrapping_add(1);
    }
}
