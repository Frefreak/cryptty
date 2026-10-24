use std::collections::BTreeMap;
use std::sync::Arc;

use crate::graphics::{Placeholder, Raster};
use crate::terminal::{Row, Terminal};

pub(super) struct Tile<'a> {
    pub id: u64,
    pub z: i32,
    pub image: &'a Arc<Raster>,
    pub rect: [f32; 4],
    pub uv: [f32; 4],
}

pub(super) fn tiles<'a>(
    terminal: &'a Terminal,
    row: &Row,
    row_index: usize,
    cell: (usize, usize),
    origin: (usize, usize),
    end: (usize, usize),
) -> Vec<Tile<'a>> {
    if terminal.graphics.images.is_empty() {
        return Vec::new();
    }
    let mut previous = None;
    let mut tiles = Vec::new();
    for (x, c) in row.iter().enumerate() {
        let placeholder = Placeholder::read(c, previous);
        previous = placeholder;
        let Some(p) = placeholder else { continue };
        let id = u64::from(p.image);
        let Some(image) = terminal.graphics.images.get(&id) else {
            continue;
        };
        let placement = terminal
            .graphics
            .placements
            .get(&(id, p.placement))
            .or_else(|| {
                (p.placement == 0)
                    .then(|| {
                        terminal
                            .graphics
                            .placements
                            .range((id, 0)..=(id, u32::MAX))
                            .next()
                            .map(|(_, placement)| placement)
                    })
                    .flatten()
            });
        let Some(placement) = placement else { continue };
        let geometry =
            placement.geometry(image, (cell.0 as u32, cell.1 as u32));
        let (cw, ch) = (cell.0 as f32, cell.1 as f32);
        let sx = p.col as f32 * cw - placement.offset.0 as f32;
        let sy = p.row as f32 * ch - placement.offset.1 as f32;
        let left = (origin.0 + x * cell.0) as f32;
        let top = origin.1 as f32;
        if let Some(tile) = clipped_tile(
            id,
            placement.z,
            image,
            [left - sx, top - sy, geometry.size.0, geometry.size.1],
            geometry.source,
            [
                left,
                top,
                (left + cw).min(end.0 as f32),
                (top + ch).min(end.1 as f32),
            ],
        ) {
            tiles.push(tile);
        }
    }
    let cell_pixels = (cell.0 as u32, cell.1 as u32);
    for p in &terminal.graphics.positioned {
        let Some(image) = terminal.graphics.images.get(&p.image) else {
            continue;
        };
        let geometry = p.layout.geometry(image, cell_pixels);
        let top = origin.1 as f32
            + (p.row as f32 - row_index as f32) * cell.1 as f32
            + p.layout.offset.1 as f32;
        let left =
            (origin.0 + p.col * cell.0) as f32 + p.layout.offset.0 as f32;
        let clip_top = (top + geometry.size.1 * p.clip.0).max(origin.1 as f32);
        let clip_bottom = (top + geometry.size.1 * p.clip.1)
            .min((origin.1 + cell.1).min(end.1) as f32);
        if let Some(tile) = clipped_tile(
            p.image,
            p.layout.z,
            image,
            [left, top, geometry.size.0, geometry.size.1],
            geometry.source,
            [origin.0 as f32, clip_top, end.0 as f32, clip_bottom],
        ) {
            tiles.push(tile);
        }
    }
    tiles.sort_by_key(|tile| (tile.z, tile.id));
    tiles
}

fn clipped_tile(
    id: u64,
    z: i32,
    image: &Arc<Raster>,
    rect: [f32; 4],
    source: [f32; 4],
    clip: [f32; 4],
) -> Option<Tile<'_>> {
    let [x, y, w, h] = rect;
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let left = x.max(clip[0]);
    let top = y.max(clip[1]);
    let right = (x + w).min(clip[2]);
    let bottom = (y + h).min(clip[3]);
    if right <= left || bottom <= top {
        return None;
    }
    Some(Tile {
        id,
        z,
        image,
        rect: [left, top, right - left, bottom - top],
        uv: [
            source[0] + (left - x) * source[2] / w,
            source[1] + (top - y) * source[3] / h,
            (right - left) * source[2] / w,
            (bottom - top) * source[3] / h,
        ],
    })
}

pub(super) struct GpuImage {
    source: Arc<Raster>,
    pub bindings: wgpu::BindGroup,
}

pub(super) fn prepare(
    cache: &mut BTreeMap<u64, GpuImage>,
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
