use super::{Control, Raster};

#[derive(Clone, Copy, Debug, Default)]
pub struct Placement {
    pub cols: u32,
    pub rows: u32,
    pub source: [u32; 4],
    pub offset: (u32, u32),
    pub z: i32,
}

pub struct Geometry {
    pub source: [f32; 4],
    pub size: (f32, f32),
    pub cells: (usize, usize),
}

#[derive(Clone, Debug)]
pub struct Positioned {
    pub image: u64,
    pub id: u32,
    pub layout: Placement,
    pub col: usize,
    pub row: isize,
    pub clip: (f32, f32),
}

#[derive(Clone, Copy)]
pub struct Screen {
    pub cursor: (usize, usize),
    pub cell: (u32, u32),
    pub size: (usize, usize),
}

impl Placement {
    pub(super) fn parse(
        c: &Control,
        cell: (u32, u32),
    ) -> Result<Self, &'static str> {
        if [b'P', b'Q', b'H', b'V'].iter().any(|k| c.get(*k, 0) != 0) {
            return Err("ENOTSUP:relative placements unsupported");
        }
        if c.get(b'U', 0) > 1 || c.get(b'C', 0) > 1 {
            return Err("EINVAL:placement flags");
        }
        let cols = c.get(b'c', 0);
        let rows = c.get(b'r', 0);
        if cols > 65535 || rows > 65535 {
            return Err("EINVAL:placement too large");
        }
        let offset = (c.get(b'X', 0), c.get(b'Y', 0));
        if offset.0 >= cell.0.max(1) || offset.1 >= cell.1.max(1) {
            return Err("EINVAL:offset exceeds cell size");
        }
        Ok(Self {
            cols,
            rows,
            source: [
                c.get(b'x', 0),
                c.get(b'y', 0),
                c.get(b'w', 0),
                c.get(b'h', 0),
            ],
            offset,
            z: c.get(b'z', 0) as i32,
        })
    }

    pub fn geometry(&self, image: &Raster, cell: (u32, u32)) -> Geometry {
        let [x, y, w, h] = self.source;
        let width = image.width.saturating_sub(x);
        let height = image.height.saturating_sub(y);
        let width = if w == 0 { width } else { w.min(width) } as f32;
        let height = if h == 0 { height } else { h.min(height) } as f32;
        let (cw, ch) = (cell.0.max(1) as f32, cell.1.max(1) as f32);
        let fit_x = (self.cols as f32 * cw - self.offset.0 as f32).max(0.0);
        let fit_y = (self.rows as f32 * ch - self.offset.1 as f32).max(0.0);
        let scale = if width == 0.0 || height == 0.0 {
            0.0
        } else {
            match (self.cols, self.rows) {
                (0, 0) => 1.0,
                (_, 0) => fit_x / width,
                (0, _) => fit_y / height,
                _ => (fit_x / width).min(fit_y / height),
            }
        };
        let size = (width * scale, height * scale);
        let cols = if self.cols > 0 {
            self.cols as usize
        } else {
            ((self.offset.0 as f32 + size.0) / cw).ceil() as usize
        };
        let rows = if self.rows > 0 {
            self.rows as usize
        } else {
            ((self.offset.1 as f32 + size.1) / ch).ceil() as usize
        };
        Geometry {
            source: [x as f32, y as f32, width, height],
            size,
            cells: (cols, rows),
        }
    }
}

impl Positioned {
    pub fn bounds(&self, image: &Raster, cell: (u32, u32)) -> [f32; 4] {
        let size = self.layout.geometry(image, cell).size;
        let x = self.col as f32 * cell.0.max(1) as f32
            + self.layout.offset.0 as f32;
        let y = self.row as f32 * cell.1.max(1) as f32
            + self.layout.offset.1 as f32;
        [
            x,
            y + size.1 * self.clip.0,
            x + size.0,
            y + size.1 * self.clip.1,
        ]
    }
}
