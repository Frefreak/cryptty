use std::collections::{BTreeMap, VecDeque};
use std::io::{Cursor, Read};
use std::sync::Arc;

use base64::{Engine, engine::general_purpose::STANDARD};

mod placeholders;
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

#[derive(Clone, Copy, Default)]
pub struct Placement {
    pub cols: u32,
    pub rows: u32,
}

#[derive(Default)]
pub struct Graphics {
    pub images: BTreeMap<u32, Arc<Raster>>,
    pub placements: BTreeMap<(u32, u32), Placement>,
    pending: Option<Transfer>,
    order: VecDeque<u32>,
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
    pub fn command(&mut self, bytes: &[u8]) -> Vec<u8> {
        let (header, data) = match bytes.iter().position(|&b| b == b';') {
            Some(i) => (&bytes[..i], &bytes[i + 1..]),
            None => (bytes, &[][..]),
        };
        let control = match Control::parse(header) {
            Ok(control) => control,
            Err(_) => {
                self.pending = None;
                return Vec::new();
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
            return Vec::new();
        }
        let result = if transfer.failed {
            Err("EFBIG:transfer limit exceeded")
        } else {
            self.execute(&transfer.control, &transfer.encoded)
        };
        transfer.control.response(result)
    }

    fn execute(
        &mut self,
        c: &Control,
        data: &[u8],
    ) -> Result<(), &'static str> {
        match c.get(b'a', b't' as u32) as u8 {
            b'd' => self.delete(c),
            b'p' => self.place(c),
            b't' | b'T' | b'q' => {
                if c.get(b't', b'd' as u32) != b'd' as u32 {
                    return Err("ENOTSUP:use direct byte transmission");
                }
                if c.get(b'a', 0) == b'T' as u32 && c.get(b'U', 0) != 1 {
                    return Err("ENOTSUP:use Unicode placeholders");
                }
                let raster = decode(c, data)?;
                if c.get(b'a', 0) == b'q' as u32 {
                    return Ok(());
                }
                let id = c.get(b'i', 0);
                if id == 0 {
                    return Err("EINVAL:explicit image id required");
                }
                if c.get(b'a', 0) == b'T' as u32 {
                    self.check_placement(c, true)?;
                }
                self.images.remove(&id);
                self.placements.retain(|&(i, _), _| i != id);
                self.order.retain(|&old| old != id);
                let mut used: usize =
                    self.images.values().map(|image| image.rgba.len()).sum();
                while used + raster.rgba.len() > STORE_BYTES
                    || self.images.len() >= 128
                {
                    let Some(old) = self.order.pop_front() else {
                        break;
                    };
                    if let Some(image) = self.images.remove(&old) {
                        used -= image.rgba.len();
                    }
                    self.placements.retain(|&(i, _), _| i != old);
                }
                self.order.push_back(id);
                self.images.insert(id, Arc::new(raster));
                self.revision = self.revision.wrapping_add(1);
                if c.get(b'a', 0) == b'T' as u32 {
                    self.place(c)?;
                }
                Ok(())
            }
            _ => Err("ENOTSUP:unsupported graphics action"),
        }
    }

    fn check_placement(
        &self,
        c: &Control,
        replacing: bool,
    ) -> Result<(), &'static str> {
        if c.get(b'U', 0) != 1 {
            return Err("ENOTSUP:use Unicode placeholders");
        }
        if [b'x', b'y', b'w', b'h', b'X', b'Y', b'P', b'Q']
            .iter()
            .any(|&k| c.get(k, 0) != 0)
        {
            return Err("ENOTSUP:cropping and relative placements unsupported");
        }
        if c.get(b'c', 0) > 65535 || c.get(b'r', 0) > 65535 {
            return Err("EINVAL:placement too large");
        }
        let key = (c.get(b'i', 0), c.get(b'p', 0));
        if self
            .placements
            .keys()
            .filter(|&&(id, _)| !replacing || id != key.0)
            .count()
            >= 256
            && !self.placements.contains_key(&key)
        {
            return Err("ENOSPC:placement limit exceeded");
        }
        Ok(())
    }

    fn place(&mut self, c: &Control) -> Result<(), &'static str> {
        self.check_placement(c, false)?;
        let id = c.get(b'i', 0);
        if !self.images.contains_key(&id) {
            return Err("ENOENT:image not found");
        }
        self.placements.insert(
            (id, c.get(b'p', 0)),
            Placement {
                cols: c.get(b'c', 0),
                rows: c.get(b'r', 0),
            },
        );
        self.revision = self.revision.wrapping_add(1);
        Ok(())
    }

    fn delete(&mut self, c: &Control) -> Result<(), &'static str> {
        let mode = c.get(b'd', b'a' as u32) as u8;
        match mode {
            // Virtual placements have no screen position.
            b'a' | b'A' | b'c' | b'C' | b'p' | b'P' => return Ok(()),
            b'i' | b'I' => {
                let id = c.get(b'i', 0);
                let placement = c.get(b'p', 0);
                self.placements.retain(|&(i, p), _| {
                    i != id || (placement != 0 && p != placement)
                });
                if mode == b'I'
                    && !self.placements.keys().any(|&(i, _)| i == id)
                {
                    self.images.remove(&id);
                    self.order.retain(|&old| old != id);
                }
            }
            _ => return Err("ENOTSUP:unsupported deletion mode"),
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
