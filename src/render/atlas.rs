use crate::terminal::FontStyle;
use std::collections::HashMap;

use swash::scale::image::{Content, Image};

const SIZE: u32 = 2048;

#[derive(Clone, Copy)]
pub(super) struct Glyph {
    pub origin: [u32; 2],
    pub color: bool,
}

pub(super) struct Atlas {
    pub texture: wgpu::Texture,
    glyphs: HashMap<(char, FontStyle), Glyph>,
    next: [u32; 2],
    row_height: u32,
}

impl Atlas {
    pub fn new(device: &wgpu::Device) -> Self {
        Self {
            texture: device.create_texture(&wgpu::TextureDescriptor {
                label: Some("glyph atlas"),
                size: wgpu::Extent3d {
                    width: SIZE,
                    height: SIZE,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            }),
            glyphs: HashMap::new(),
            next: [0, 0],
            row_height: 0,
        }
    }

    pub fn clear(&mut self) {
        self.glyphs.clear();
        self.next = [0, 0];
        self.row_height = 0;
    }

    pub fn get(
        &mut self,
        queue: &wgpu::Queue,
        key: (char, FontStyle),
        image: &Image,
    ) -> Option<Glyph> {
        if let Some(glyph) = self.glyphs.get(&key) {
            return Some(*glyph);
        }
        let width = image.placement.width;
        let height = image.placement.height;
        if width > SIZE || height > SIZE {
            return None;
        }
        if self.next[0] + width > SIZE {
            self.next[0] = 0;
            self.next[1] += self.row_height;
            self.row_height = 0;
        }
        if self.next[1] + height > SIZE {
            return None;
        }
        let glyph = Glyph {
            origin: self.next,
            color: image.content == Content::Color,
        };
        let rgba = match image.content {
            Content::Mask => image
                .data
                .iter()
                .flat_map(|&a| [a, a, a, 255])
                .collect::<Vec<_>>(),
            Content::Color | Content::SubpixelMask => image.data.clone(),
        };
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: self.next[0],
                    y: self.next[1],
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            &rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.next[0] += width;
        self.row_height = self.row_height.max(height);
        self.glyphs.insert(key, glyph);
        Some(glyph)
    }
}
