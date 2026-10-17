use std::collections::BTreeMap;
use std::sync::Arc;

use crate::graphics::{Placeholder, Raster};
use crate::terminal::{Row, Terminal};

pub(super) struct Tile<'a> {
    pub id: u32,
    pub image: &'a Arc<Raster>,
    pub rect: [f32; 4],
    pub uv: [f32; 4],
}

pub(super) fn tiles<'a>(
    terminal: &'a Terminal,
    row: &Row,
    cell: (usize, usize),
    origin: (usize, usize),
    end: (usize, usize),
) -> Vec<Tile<'a>> {
    let mut previous = None;
    let mut tiles = Vec::new();
    for (x, c) in row.iter().enumerate() {
        let placeholder = Placeholder::read(c, previous);
        previous = placeholder;
        let Some(p) = placeholder else { continue };
        let Some(image) = terminal.graphics.images.get(&p.image) else {
            continue;
        };
        let placement = terminal
            .graphics
            .placements
            .get(&(p.image, p.placement))
            .or_else(|| {
                (p.placement == 0)
                    .then(|| {
                        terminal
                            .graphics
                            .placements
                            .range((p.image, 0)..=(p.image, u32::MAX))
                            .next()
                            .map(|(_, placement)| placement)
                    })
                    .flatten()
            });
        let Some(placement) = placement else { continue };
        let (cw, ch) = (cell.0 as f32, cell.1 as f32);
        let (iw, ih) = (image.width as f32, image.height as f32);
        let scale = match (placement.cols, placement.rows) {
            (0, 0) => 1.0,
            (c, 0) => c as f32 * cw / iw,
            (0, r) => r as f32 * ch / ih,
            (c, r) => (c as f32 * cw / iw).min(r as f32 * ch / ih),
        };
        let (sx, sy) = (p.col as f32 * cw, p.row as f32 * ch);
        let (left, top) = (origin.0 + x * cell.0, origin.1);
        let width = cw
            .min(iw * scale - sx)
            .min(end.0.saturating_sub(left) as f32);
        let height = ch
            .min(ih * scale - sy)
            .min(end.1.saturating_sub(top) as f32);
        if width <= 0.0 || height <= 0.0 {
            continue;
        }
        tiles.push(Tile {
            id: p.image,
            image,
            rect: [left as f32, top as f32, width, height],
            uv: [sx / scale, sy / scale, width / scale, height / scale],
        });
    }
    tiles
}

pub(super) struct GpuImage {
    source: Arc<Raster>,
    pub bindings: wgpu::BindGroup,
}

pub(super) fn prepare(
    cache: &mut BTreeMap<u32, GpuImage>,
    terminal: &Terminal,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    viewport: &wgpu::Buffer,
    tiles: &[Tile<'_>],
) -> anyhow::Result<()> {
    cache.retain(|id, cached| {
        terminal
            .graphics
            .images
            .get(id)
            .is_some_and(|image| Arc::ptr_eq(image, &cached.source))
    });
    for tile in tiles {
        if cache.contains_key(&tile.id) {
            continue;
        }
        let image = tile.image;
        anyhow::ensure!(
            image.width <= device.limits().max_texture_dimension_2d
                && image.height <= device.limits().max_texture_dimension_2d,
            "Image exceeds GPU texture dimensions"
        );
        let size = wgpu::Extent3d {
            width: image.width,
            height: image.height,
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("terminal image"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &image.rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(image.width * 4),
                rows_per_image: None,
            },
            size,
        );
        let bindings = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("terminal image bindings"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: viewport.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(
                        &texture.create_view(&Default::default()),
                    ),
                },
            ],
        });
        cache.insert(
            tile.id,
            GpuImage {
                source: image.clone(),
                bindings,
            },
        );
    }
    Ok(())
}
