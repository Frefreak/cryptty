use anyhow::{Context, Result};

use super::{Renderer, atlas::Atlas, scene};
use crate::terminal::Terminal;

pub(super) struct Painter {
    atlas: Atlas,
    viewport: wgpu::Buffer,
    bindings: wgpu::BindGroup,
    pipelines: Vec<wgpu::RenderPipeline>,
    vertices: wgpu::Buffer,
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
        let pipelines = pipelines(device, &layout, format);
        let vertices = vertex_buffer(device, 64);
        Self {
            atlas,
            viewport,
            bindings,
            pipelines,
            vertices,
        }
    }

    pub fn clear_glyphs(&mut self) {
        self.atlas.clear();
    }

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
        let quads = scene::build(
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
        {
            let mut pass =
                encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("terminal frame"),
                    color_attachments: &[Some(
                        wgpu::RenderPassColorAttachment {
                            view,
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color {
                                    r: bg[0] as f64,
                                    g: bg[1] as f64,
                                    b: bg[2] as f64,
                                    a: 1.0,
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
                pass.draw(0..6, 0..quads.len() as u32);
            }
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
    [
        ("red", wgpu::ColorWrites::RED),
        ("green", wgpu::ColorWrites::GREEN),
        ("blue", wgpu::ColorWrites::BLUE),
    ]
    .into_iter()
    .map(|(entry, mask)| {
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
