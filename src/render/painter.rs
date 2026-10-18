use anyhow::{Context, Result};

use super::{Renderer, atlas::Atlas, linear, scene};
use crate::terminal::Terminal;

pub(super) struct Painter {
    atlas: Atlas,
    linear: Option<linear::LinearFrame>,
    opacity: crate::config::Opacity,
    viewport: wgpu::Buffer,
    bindings: wgpu::BindGroup,
    pipelines: Vec<wgpu::RenderPipeline>,
    vertices: wgpu::Buffer,
    layout: wgpu::BindGroupLayout,
    images: std::collections::BTreeMap<u64, super::images::GpuImage>,
}

impl Painter {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let atlas = Atlas::new(device);
        let viewport = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("viewport"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("terminal bindings"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float {
                                filterable: false,
                            },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                ],
            });
        let bindings = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("terminal bindings"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: viewport.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(
                        &atlas.texture.create_view(&Default::default()),
                    ),
                },
            ],
        });
        let pipelines = pipelines(device, &layout, format, false);
        let vertices = vertex_buffer(device, 64);
        Self {
            atlas,
            linear: None,
            opacity: Default::default(),
            viewport,
            bindings,
            pipelines,
            vertices,
            layout,
            images: Default::default(),
        }
    }

    pub fn set_opacity(
        &mut self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        opacity: crate::config::Opacity,
    ) {
        if self.opacity.transparent() != opacity.transparent() {
            let target_format = if opacity.transparent() {
                self.linear = Some(linear::LinearFrame::new(device, format));
                linear::FORMAT
            } else {
                self.linear = None;
                format
            };
            self.pipelines = pipelines(
                device,
                &self.layout,
                target_format,
                opacity.transparent(),
            );
        }
        self.opacity = opacity;
    }

    pub fn clear_glyphs(&mut self) {
        self.atlas.clear();
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        view: &wgpu::TextureView,
        renderer: &mut Renderer,
        terminal: &Terminal,
        size: (u32, u32),
        preedit: &str,
    ) -> Result<()> {
        let frame = scene::build(
            renderer,
            &mut self.atlas,
            queue,
            terminal,
            size,
            preedit,
        )
        .or_else(|| {
            self.atlas.clear();
            scene::build(
                renderer,
                &mut self.atlas,
                queue,
                terminal,
                size,
                preedit,
            )
        })
        .context("Visible glyphs exceed GPU atlas capacity")?;
        let mut tiles: Vec<_> = terminal
            .rows
            .iter()
            .enumerate()
            .flat_map(|(y, row)| {
                super::images::tiles(
                    terminal,
                    row,
                    y,
                    (renderer.cell_width, renderer.cell_height),
                    renderer.cell_origin(0, y),
                    (
                        (size.0 as usize).saturating_sub(renderer.padding.0),
                        (size.1 as usize).saturating_sub(renderer.padding.1),
                    ),
                )
            })
            .collect();
        tiles.sort_by_key(|tile| (tile.z, tile.id));
        super::images::prepare(
            &mut self.images,
            terminal,
            device,
            queue,
            &self.layout,
            &self.viewport,
            &tiles,
        )?;
        let overlay = frame.overlay;
        let mut quads = frame.quads;
        let text_end = quads.len();
        quads.extend(tiles.iter().map(scene::Quad::image));
        let bytes = bytemuck::cast_slice(&quads);
        if bytes.len() as u64 > self.vertices.size() {
            self.vertices =
                vertex_buffer(device, bytes.len().next_power_of_two() as u64);
        }
        queue.write_buffer(&self.vertices, 0, bytes);
        queue.write_buffer(
            &self.viewport,
            0,
            bytemuck::cast_slice(&[size.0 as f32, size.1 as f32, 0.0, 0.0]),
        );
        let mut encoder = device.create_command_encoder(&Default::default());
        let bg = scene::color(terminal.theme().background);
        let bg = bg.map(|channel| {
            if self.linear.is_some() {
                linear::decode(f64::from(channel))
            } else {
                f64::from(channel)
            }
        });
        let target = match &mut self.linear {
            Some(frame) => frame.prepare(device, size),
            None => view,
        };
        let opacity = f64::from(self.opacity.get());
        {
            let mut pass =
                encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("terminal frame"),
                    color_attachments: &[Some(
                        wgpu::RenderPassColorAttachment {
                            view: target,
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color {
                                    r: bg[0] * opacity,
                                    g: bg[1] * opacity,
                                    b: bg[2] * opacity,
                                    a: opacity,
                                }),
                                store: wgpu::StoreOp::Store,
                            },
                        },
                    )],
                    ..Default::default()
                });
            pass.set_bind_group(0, &self.bindings, &[]);
            pass.set_vertex_buffer(0, self.vertices.slice(..));
            for pipeline in &self.pipelines {
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, &self.bindings, &[]);
                let behind_background =
                    tiles.partition_point(|t| t.z < -1_073_741_824);
                let behind_text = tiles.partition_point(|t| t.z < 0);
                for (images, text) in [
                    (0..behind_background, 0..frame.backgrounds),
                    (
                        behind_background..behind_text,
                        frame.backgrounds..overlay,
                    ),
                    (behind_text..tiles.len(), overlay..text_end),
                ] {
                    let mut start = images.start;
                    while start < images.end {
                        let id = tiles[start].id;
                        let end = start
                            + tiles[start..images.end]
                                .iter()
                                .take_while(|t| t.id == id)
                                .count();
                        pass.set_bind_group(0, &self.images[&id].bindings, &[]);
                        pass.draw(
                            0..6,
                            (text_end + start) as u32..(text_end + end) as u32,
                        );
                        start = end;
                    }
                    pass.set_bind_group(0, &self.bindings, &[]);
                    pass.draw(0..6, text.start as u32..text.end as u32);
                }
            }
        }
        if let Some(frame) = &self.linear {
            frame.encode(&mut encoder, view);
        }
        queue.submit([encoder.finish()]);
        Ok(())
    }
}

fn vertex_buffer(device: &wgpu::Device, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("terminal quads"),
        size,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn pipelines(
    device: &wgpu::Device,
    bindings: &wgpu::BindGroupLayout,
    format: wgpu::TextureFormat,
    transparent: bool,
) -> Vec<wgpu::RenderPipeline> {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("terminal shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("gpu.wgsl").into()),
    });
    let layout =
        device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("terminal pipeline"),
            bind_group_layouts: &[Some(bindings)],
            immediate_size: 0,
        });
    let entries: &[(&str, wgpu::ColorWrites)] = if transparent {
        &[("transparent", wgpu::ColorWrites::ALL)]
    } else {
        &[
            ("red", wgpu::ColorWrites::RED),
            ("green", wgpu::ColorWrites::GREEN),
            ("blue", wgpu::ColorWrites::BLUE),
        ]
    };
    entries
        .iter()
        .map(|&(entry, mask)| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vertex"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: size_of::<scene::Quad>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &wgpu::vertex_attr_array![
                            0 => Float32x4, 1 => Float32x4,
                            2 => Float32x4, 3 => Uint32,
                        ],
                    })],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: mask,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        })
        .collect()
}
