use std::collections::{BTreeMap, VecDeque};
use std::io::{Cursor, Read};
use std::sync::Arc;

use base64::{Engine, engine::general_purpose::STANDARD};

mod lifecycle;
mod placeholders;
mod placement;
pub use placement::{Geometry, Placement, Positioned, Screen};
pub mod sender;
pub mod stream;
pub use placeholders::{DIACRITICS, Placeholder};

pub const MARKER: char = '\u{10eeee}';
pub const MAX_BYTES: usize = 64 * 1024 * 1024;
const STORE_BYTES: usize = 128 * 1024 * 1024;

pub struct Raster {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Default)]
pub struct Graphics {
    pub images: BTreeMap<u64, Arc<Raster>>,
    pub placements: BTreeMap<(u64, u32), Placement>,
    pub positioned: Vec<Positioned>,
    next_anonymous: u32,
    pending: Option<Transfer>,
    order: VecDeque<u64>,
    pub revision: u64,
}

struct Transfer {
    control: Control,
    encoded: Vec<u8>,
    failed: bool,
}

#[derive(Default)]
struct Control(BTreeMap<u8, u32>);

impl Control {
    fn parse(text: &[u8]) -> Result<Self, &'static str> {
        let mut values = BTreeMap::new();
        for field in text.split(|&b| b == b',').filter(|s| !s.is_empty()) {
            if field.len() < 3 || field[1] != b'=' {
                return Err("EINVAL:invalid control");
            }
            let key = field[0];
            let value = if matches!(key, b'a' | b't' | b'd' | b'o') {
                if field.len() != 3 {
                    return Err("EINVAL:invalid control value");
                }
                u32::from(field[2])
            } else if key == b'z' {
                std::str::from_utf8(&field[2..])
                    .ok()
                    .and_then(|s| s.parse::<i32>().ok())
                    .ok_or("EINVAL:invalid z-index")? as u32
            } else {
                std::str::from_utf8(&field[2..])
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .ok_or("EINVAL:invalid integer")?
            };
            if values.insert(key, value).is_some() {
                return Err("EINVAL:duplicate key");
            }
        }
        Ok(Self(values))
    }

    fn get(&self, key: u8, default: u32) -> u32 {
        self.0.get(&key).copied().unwrap_or(default)
    }

    fn response(&self, result: Result<(), &'static str>) -> Vec<u8> {
        let quiet = self.get(b'q', 0);
        let id = self.get(b'i', 0);
        if quiet >= 2 || (quiet == 1 && result.is_ok()) || id == 0 {
            return Vec::new();
        }
        let placement = self.get(b'p', 0);
        let extra = if placement == 0 {
            String::new()
        } else {
            format!(",p={placement}")
        };
        let message = result.err().unwrap_or("OK");
        format!("\x1b_Gi={id}{extra};{message}\x1b\\").into_bytes()
    }
}

impl Graphics {
    pub(crate) fn snapshot(&self) -> Self {
        Self {
            images: self.images.clone(),
            placements: self.placements.clone(),
            positioned: self.positioned.clone(),
            revision: self.revision,
            ..Self::default()
        }
    }

    pub fn command(
        &mut self,
        bytes: &[u8],
        screen: Screen,
    ) -> (Vec<u8>, Option<(usize, usize)>) {
        let (header, data) = match bytes.iter().position(|&b| b == b';') {
            Some(i) => (&bytes[..i], &bytes[i + 1..]),
            None => (bytes, &[][..]),
        };
        let control = match Control::parse(header) {
            Ok(control) => control,
            Err(_) => {
                self.pending = None;
                return (Vec::new(), None);
            }
        };
        let more = control.get(b'm', 0) == 1;
        let continuation = control.0.keys().all(|k| matches!(k, b'm' | b'q'));
        let transfer = if continuation && self.pending.is_some() {
            self.pending.take().unwrap()
        } else {
            self.pending = None;
            Transfer {
                control,
                encoded: Vec::new(),
                failed: false,
            }
        };
        let mut transfer = transfer;
        if transfer.encoded.len() + data.len() > MAX_BYTES * 4 / 3 + 4 {
            transfer.failed = true;
        } else if !transfer.failed {
            transfer.encoded.extend_from_slice(data);
        }
        if more {
            self.pending = Some(transfer);
            return (Vec::new(), None);
        }
        let result = if transfer.failed {
            Err("EFBIG:transfer limit exceeded")
        } else {
            self.execute(&transfer.control, &transfer.encoded, screen)
        };
        let movement = result.as_ref().ok().copied().flatten();
        (transfer.control.response(result.map(|_| ())), movement)
    }

    fn execute(
        &mut self,
        c: &Control,
        data: &[u8],
        screen: Screen,
    ) -> Result<Option<(usize, usize)>, &'static str> {
        match c.get(b'a', b't' as u32) as u8 {
            b'd' => {
                self.delete(c, screen)?;
                Ok(None)
            }
            b'p' => self.place(c, u64::from(c.get(b'i', 0)), screen),
            b't' | b'T' | b'q' => {
                if c.get(b't', b'd' as u32) != b'd' as u32 {
                    return Err("ENOTSUP:use direct byte transmission");
                }
                let raster = decode(c, data)?;
                if c.get(b'a', 0) == b'q' as u32 {
                    return Ok(None);
                }
                let id = if c.get(b'i', 0) == 0 {
                    self.next_anonymous = self.next_anonymous.wrapping_add(1);
                    (1u64 << 32) | u64::from(self.next_anonymous)
                } else {
                    u64::from(c.get(b'i', 0))
                };
                if c.get(b'a', 0) == b'T' as u32 {
                    self.check_placement(c, id, screen, true)?;
                }
                self.remove_image(id);
                let mut used: usize =
                    self.images.values().map(|image| image.rgba.len()).sum();
                while used + raster.rgba.len() > STORE_BYTES
                    || self.images.len() >= 128
                {
                    let Some(old) = self.order.front().copied() else {
                        break;
                    };
                    used -= self.images[&old].rgba.len();
                    self.remove_image(old);
                }
                self.order.push_back(id);
                self.images.insert(id, Arc::new(raster));
                self.revision = self.revision.wrapping_add(1);
                if c.get(b'a', 0) == b'T' as u32 {
                    self.place(c, id, screen)
                } else {
                    Ok(None)
                }
            }
            _ => Err("ENOTSUP:unsupported graphics action"),
        }
    }

    fn remove_image(&mut self, id: u64) {
        self.images.remove(&id);
        self.placements.retain(|&(i, _), _| i != id);
        self.positioned.retain(|p| p.image != id);
        self.order.retain(|&i| i != id);
    }

    fn check_placement(
        &self,
        c: &Control,
        image: u64,
        screen: Screen,
        replacing: bool,
    ) -> Result<Placement, &'static str> {
        let layout = Placement::parse(c, screen.cell)?;
        let id = c.get(b'p', 0);
        let retained = self
            .positioned
            .iter()
            .filter(|p| {
                p.image != image || (!replacing && (id == 0 || p.id != id))
            })
            .count()
            + self
                .placements
                .keys()
                .filter(|&&(i, p)| i != image || (!replacing && p != id))
                .count();
        if retained >= 256 {
            return Err("ENOSPC:placement limit exceeded");
        }
        Ok(layout)
    }

    fn place(
        &mut self,
        c: &Control,
        image: u64,
        screen: Screen,
    ) -> Result<Option<(usize, usize)>, &'static str> {
        let layout = self.check_placement(c, image, screen, false)?;
        let raster = self.images.get(&image).ok_or("ENOENT:image not found")?;
        let cells = layout.geometry(raster, screen.cell).cells;
        let id = c.get(b'p', 0);
        if id != 0 {
            self.positioned.retain(|p| p.image != image || p.id != id);
            self.placements.remove(&(image, id));
        }
        let movement = if c.get(b'U', 0) == 1 {
            self.placements.insert((image, id), layout);
            None
        } else {
            self.positioned.push(Positioned {
                image,
                id,
                layout,
                col: screen.cursor.0,
                row: screen.cursor.1 as isize,
                clip: (0.0, 1.0),
            });
            (c.get(b'C', 0) == 0).then_some(cells)
        };
        self.revision = self.revision.wrapping_add(1);
        Ok(movement)
    }

    fn delete(
        &mut self,
        c: &Control,
        screen: Screen,
    ) -> Result<(), &'static str> {
        let mode = c.get(b'd', b'a' as u32) as u8;
        let lower = mode.to_ascii_lowercase();
        if !matches!(
            lower,
            b'a' | b'i' | b'c' | b'p' | b'q' | b'r' | b'x' | b'y' | b'z'
        ) {
            return Err("ENOTSUP:unsupported deletion mode");
        }
        let image = u64::from(c.get(b'i', 0));
        let placement = c.get(b'p', 0);
        let z = c.get(b'z', 0) as i32;
        let (x, y) = if lower == b'c' {
            screen.cursor
        } else {
            (
                c.get(b'x', 0).saturating_sub(1) as usize,
                c.get(b'y', 0).saturating_sub(1) as usize,
            )
        };
        let cw = screen.cell.0.max(1) as f32;
        let ch = screen.cell.1.max(1) as f32;
        let mut affected = Vec::new();
        self.positioned.retain(|p| {
            let [left, top, right, bottom] =
                p.bounds(&self.images[&p.image], screen.cell);
            let at_x = left < (x + 1) as f32 * cw && right > x as f32 * cw;
            let at_y = top < (y + 1) as f32 * ch && bottom > y as f32 * ch;
            let remove = match lower {
                b'a' => bottom > 0.0 && top < screen.size.1 as f32 * ch,
                b'i' => {
                    p.image == image && (placement == 0 || p.id == placement)
                }
                b'c' | b'p' => at_x && at_y,
                b'q' => at_x && at_y && p.layout.z == z,
                b'x' => at_x,
                b'y' => at_y,
                b'z' => p.layout.z == z,
                b'r' => {
                    p.image >= u64::from(c.get(b'x', 0))
                        && p.image <= u64::from(c.get(b'y', 0))
                }
                _ => false,
            };
            if remove {
                affected.push(p.image);
            }
            !remove
        });
        if matches!(lower, b'i' | b'r') {
            self.placements.retain(|&(i, p), _| {
                let remove = if lower == b'i' {
                    i == image && (placement == 0 || p == placement)
                } else {
                    i >= u64::from(c.get(b'x', 0))
                        && i <= u64::from(c.get(b'y', 0))
                };
                if remove {
                    affected.push(i);
                }
                !remove
            });
            if lower == b'i' {
                affected.push(image);
            }
            if lower == b'r' {
                affected.extend(self.images.keys().copied().filter(|&i| {
                    i >= u64::from(c.get(b'x', 0))
                        && i <= u64::from(c.get(b'y', 0))
                }));
            }
        }
        if mode.is_ascii_uppercase() {
            for id in affected {
                if !self.positioned.iter().any(|p| p.image == id)
                    && !self.placements.keys().any(|&(i, _)| i == id)
                {
                    self.remove_image(id);
                }
            }
        }
        self.revision = self.revision.wrapping_add(1);
        Ok(())
    }
}

fn decode(c: &Control, encoded: &[u8]) -> Result<Raster, &'static str> {
    let mut bytes = STANDARD.decode(encoded).map_err(|_| "EINVAL:base64")?;
    match c.get(b'o', 0) {
        0 => (),
        v if v == b'z' as u32 => {
            let mut decoded = Vec::new();
            flate2::read::ZlibDecoder::new(bytes.as_slice())
                .take(MAX_BYTES as u64 + 1)
                .read_to_end(&mut decoded)
                .map_err(|_| "EINVAL:zlib")?;
            bytes = decoded;
        }
        _ => return Err("ENOTSUP:compression"),
    }
    if bytes.len() > MAX_BYTES {
        return Err("EFBIG:decoded image too large");
    }
    let format = c.get(b'f', 32);
    if format == 100 {
        return decode_file(&bytes, image::ImageFormat::Png)
            .map_err(|_| "EINVAL:invalid or oversized PNG");
    }
    if !matches!(format, 24 | 32) {
        return Err("ENOTSUP:pixel format");
    }
    let (width, height) = (c.get(b's', 0), c.get(b'v', 0));
    let pixels = u64::from(width) * u64::from(height);
    if width == 0
        || height == 0
        || width > 8192
        || height > 8192
        || pixels * 4 > MAX_BYTES as u64
        || pixels * u64::from(format / 8) != bytes.len() as u64
    {
        return Err("EINVAL:pixel dimensions");
    }
    let rgba = if format == 24 {
        bytes
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect()
    } else {
        bytes
    };
    Ok(Raster {
        width,
        height,
        rgba,
    })
}

pub fn decode_file(
    bytes: &[u8],
    format: image::ImageFormat,
) -> anyhow::Result<Raster> {
    anyhow::ensure!(bytes.len() <= MAX_BYTES, "Image file exceeds 64 MiB");
    let mut reader =
        image::ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(MAX_BYTES as u64);
    reader.limits(limits);
    let decoded = reader.decode()?;
    anyhow::ensure!(
        u64::from(decoded.width()) * u64::from(decoded.height()) * 4
            <= MAX_BYTES as u64,
        "Decoded image exceeds 64 MiB"
    );
    let image = decoded.to_rgba8();
    Ok(Raster {
        width: image.width(),
        height: image.height(),
        rgba: image.into_raw(),
    })
}
