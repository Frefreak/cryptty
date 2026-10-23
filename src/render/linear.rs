use wgpu::{Device, TextureFormat, TextureView};

pub(super) fn format(
    backend: wgpu::Backend,
    color_format: crate::config::OpenGlColorFormat,
) -> TextureFormat {
    use crate::config::OpenGlColorFormat;
    match (backend, color_format) {
        // sRGB storage preserves linear blending with less bandwidth on GL.
        (wgpu::Backend::Gl, OpenGlColorFormat::Rgba8) => {
            TextureFormat::Rgba8UnormSrgb
        }
        _ => TextureFormat::Rgba16Float,
    }
}

pub(super) struct LinearFrame {
    pipeline: wgpu::RenderPipeline,
    format: TextureFormat,
    target: Option<Target>,
}

struct Target {
    size: (u32, u32),
    view: TextureView,
    bindings: wgpu::BindGroup,
}

impl LinearFrame {
    pub fn new(
        device: &Device,
        format: TextureFormat,
        intermediate: TextureFormat,
    ) -> Self {
        let shader =
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("linear frame encoding"),
                source: wgpu::ShaderSource::Wgsl(
                    include_str!("linear.wgsl").into(),
                ),
            });
        let pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("linear frame encoding"),
                layout: None,
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vertex"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("encode"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            });
        Self {
            pipeline,
            format: intermediate,
            target: None,
        }
    }

    pub fn prepare(
        &mut self,
        device: &Device,
        size: (u32, u32),
    ) -> &TextureView {
        if self
            .target
            .as_ref()
            .is_none_or(|target| target.size != size)
        {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("linear terminal frame"),
                size: wgpu::Extent3d {
                    width: size.0,
                    height: size.1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            let bindings =
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("linear frame bindings"),
                    layout: &self.pipeline.get_bind_group_layout(0),
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    }],
                });
            self.target = Some(Target {
                size,
                view,
                bindings,
            });
        }
        &self.target.as_ref().unwrap().view
    }

    pub fn encode(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        view: &TextureView,
    ) {
        let target = self.target.as_ref().unwrap();
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("encode transparent frame"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &target.bindings, &[]);
        pass.draw(0..3, 0..1);
    }
}

pub(super) fn decode(value: f64) -> f64 {
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::OpenGlColorFormat::{Rgba8, Rgba16f};

    #[test]
    fn compact_storage_is_opt_in_and_gl_only() {
        for backend in [wgpu::Backend::Metal, wgpu::Backend::Vulkan] {
            for choice in [Rgba8, Rgba16f] {
                assert_eq!(format(backend, choice), TextureFormat::Rgba16Float);
            }
        }
        assert_eq!(
            format(wgpu::Backend::Gl, Rgba16f),
            TextureFormat::Rgba16Float,
        );
        assert_eq!(
            format(wgpu::Backend::Gl, Rgba8),
            TextureFormat::Rgba8UnormSrgb,
        );
    }
}
