use std::sync::{Arc, Mutex};

use anyhow::{Result, bail};
use winit::window::Window;

use super::{Renderer, painter::Painter};
use crate::terminal::Terminal;

fn backends() -> wgpu::Backends {
    let mut backends = wgpu::Backends::METAL | wgpu::Backends::VULKAN;
    if cfg!(target_os = "linux") {
        backends |= wgpu::Backends::GL;
    }
    backends
}

fn adapter_priority(
    device: wgpu::DeviceType,
    backend: wgpu::Backend,
) -> (u8, bool) {
    let device = match device {
        wgpu::DeviceType::IntegratedGpu => 0,
        wgpu::DeviceType::DiscreteGpu => 1,
        wgpu::DeviceType::VirtualGpu => 2,
        _ => 3,
    };
    (device, backend == wgpu::Backend::Gl)
}

pub(super) fn instance(descriptor: wgpu::InstanceDescriptor) -> wgpu::Instance {
    wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: backends(),
        ..descriptor
    })
}

async fn adapters(instance: &wgpu::Instance) -> Vec<wgpu::Adapter> {
    let mut adapters = instance.enumerate_adapters(backends()).await;
    adapters.sort_by_key(|adapter| {
        let info = adapter.get_info();
        (
            format!("{:?}", info.backend),
            info.vendor,
            info.device,
            info.name,
        )
    });
    adapters
}

pub fn list_adapters(descriptor: wgpu::InstanceDescriptor) {
    let instance = instance(descriptor);
    let adapters = pollster::block_on(adapters(&instance));
    if adapters.is_empty() {
        println!("No compatible GPU adapters found.");
    }
    for (index, adapter) in adapters.iter().enumerate() {
        let info = adapter.get_info();
        println!(
            "{index}: {} ({:?}, {:?})",
            info.name, info.backend, info.device_type
        );
    }
}

pub struct Gpu {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    painter: Painter,
    failure: Arc<Mutex<Option<String>>>,
    needs_configure: bool,
    opaque_alpha: wgpu::CompositeAlphaMode,
    transparent_alpha: Option<wgpu::CompositeAlphaMode>,
}

impl Gpu {
    pub fn new(
        window: Arc<Window>,
        selected: Option<usize>,
        color_format: crate::config::OpenGlColorFormat,
    ) -> Result<Self> {
        pollster::block_on(Self::initialize(window, selected, color_format))
    }

    async fn initialize(
        window: Arc<Window>,
        selected: Option<usize>,
        color_format: crate::config::OpenGlColorFormat,
    ) -> Result<Self> {
        let instance =
            instance(wgpu::InstanceDescriptor::new_with_display_handle(
                Box::new(window.clone()),
            ));
        let surface = instance.create_surface(window.clone())?;
        let mut candidates: Vec<_> =
            adapters(&instance).await.into_iter().enumerate().collect();
        if let Some(index) = selected {
            candidates.retain(|(i, _)| *i == index);
            if candidates.is_empty() {
                bail!("GPU index {index} unavailable; use --list-gpus");
            }
        } else {
            candidates.retain(|(_, a)| {
                a.get_info().device_type != wgpu::DeviceType::Cpu
            });
            candidates.sort_by_key(|(_, a)| {
                let info = a.get_info();
                adapter_priority(info.device_type, info.backend)
            });
        }
        let mut errors = Vec::new();
        for (index, adapter) in candidates {
            if !adapter.is_surface_supported(&surface) {
                errors.push(format!(
                    "adapter {index}: cannot present to this window"
                ));
                continue;
            }
            let info = adapter.get_info();
            let requested = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: Some("Cryptty GPU"),
                    ..Default::default()
                })
                .await;
            let (device, queue) = match requested {
                Ok(pair) => pair,
                Err(error) => {
                    errors.push(format!("adapter {index}: {error}"));
                    continue;
                }
            };
            let failure = Arc::new(Mutex::new(None));
            let lost = failure.clone();
            let wake = Arc::downgrade(&window);
            device.set_device_lost_callback(move |reason, message| {
                *lost.lock().unwrap() =
                    Some(format!("GPU lost: {reason:?}: {message}"));
                if let Some(window) = wake.upgrade() {
                    window.request_redraw();
                }
            });
            let uncaptured = failure.clone();
            let wake = Arc::downgrade(&window);
            device.on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
                *uncaptured.lock().unwrap() = Some(error.to_string());
                if let Some(window) = wake.upgrade() {
                    window.request_redraw();
                }
            }));
            let size = window.inner_size();
            let Some(mut config) = surface.get_default_config(
                &adapter,
                size.width.max(1),
                size.height.max(1),
            ) else {
                errors
                    .push(format!("adapter {index}: no surface configuration"));
                continue;
            };
            let capabilities = surface.get_capabilities(&adapter);
            let Some(format) = surface_format(&capabilities.formats) else {
                errors.push(format!(
                    "adapter {index}: no non-sRGB surface with 8-bit alpha"
                ));
                continue;
            };
            config.format = format;
            // Prefer low latency when the Linux surface supports it.
            config.present_mode = if cfg!(target_os = "linux")
                && capabilities
                    .present_modes
                    .contains(&wgpu::PresentMode::Immediate)
            {
                wgpu::PresentMode::Immediate
            } else if capabilities
                .present_modes
                .contains(&wgpu::PresentMode::Mailbox)
            {
                wgpu::PresentMode::Mailbox
            } else {
                wgpu::PresentMode::AutoVsync
            };
            config.desired_maximum_frame_latency = 2;
            let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
            surface.configure(&device, &config);
            let painter = Painter::new(
                &device,
                config.format,
                info.backend,
                color_format,
            );
            if let Some(error) = scope.pop().await {
                errors.push(format!("adapter {index}: {error}"));
                continue;
            }
            if let Some(error) = failure.lock().unwrap().take() {
                errors.push(format!("adapter {index}: {error}"));
                continue;
            }
            eprintln!(
                "Cryptty: GPU {index}: {} ({:?}, {:?})",
                info.name, info.backend, config.present_mode
            );
            let opaque_alpha = config.alpha_mode;
            return Ok(Self {
                surface,
                device,
                queue,
                config,
                painter,
                failure,
                needs_configure: false,
                opaque_alpha,
                transparent_alpha: transparent_alpha(
                    &capabilities.alpha_modes,
                    info.backend,
                ),
            });
        }
        bail!("No usable GPU found. {}", errors.join("; "))
    }

    pub fn set_opacity(&mut self, opacity: crate::config::Opacity) -> bool {
        let (opacity, alpha) =
            match (opacity.transparent(), self.transparent_alpha) {
                (true, Some(alpha)) => (opacity, alpha),
                (true, None) => {
                    eprintln!(
                        "Cryptty: GPU surface lacks transparency support; \
                     using an opaque background"
                    );
                    (crate::config::Opacity::default(), self.opaque_alpha)
                }
                (false, _) => (opacity, self.opaque_alpha),
            };
        self.painter
            .set_opacity(&self.device, self.config.format, opacity);
        if self.config.alpha_mode != alpha {
            self.config.alpha_mode = alpha;
            self.needs_configure = true;
        }
        opacity.transparent()
    }

    pub fn clear_glyphs(&mut self) {
        self.painter.clear_glyphs();
    }

    pub fn draw(
        &mut self,
        window: &Window,
        renderer: &mut Renderer,
        terminal: &Terminal,
        preedit: &str,
        request_frame_callback: bool,
    ) -> Result<()> {
        if let Some(error) = self.failure.lock().unwrap().take() {
            bail!(error);
        }
        let size = window.inner_size();
        if size.width == 0 || size.height == 0 {
            return Ok(());
        }
        if size.width > self.device.limits().max_texture_dimension_2d
            || size.height > self.device.limits().max_texture_dimension_2d
        {
            bail!("Window exceeds GPU texture limits");
        }
        if self.needs_configure
            || (self.config.width, self.config.height)
                != (size.width, size.height)
        {
            self.config.width = size.width;
            self.config.height = size.height;
            self.surface.configure(&self.device, &self.config);
            self.needs_configure = false;
        }
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => frame,
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                self.needs_configure = true;
                window.request_redraw();
                frame
            }
            wgpu::CurrentSurfaceTexture::Timeout => {
                window.request_redraw();
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Occluded => return Ok(()),
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.surface.configure(&self.device, &self.config);
                window.request_redraw();
                return Ok(());
            }
            other => bail!("GPU surface unavailable: {other:?}"),
        };
        let size = (size.width, size.height);
        let view = frame.texture.create_view(&Default::default());
        self.painter.draw(
            &self.device,
            &self.queue,
            &view,
            renderer,
            terminal,
            size,
            preedit,
        )?;
        if request_frame_callback {
            window.pre_present_notify();
        }
        self.queue.present(frame);
        Ok(())
    }
}

fn transparent_alpha(
    modes: &[wgpu::CompositeAlphaMode],
    backend: wgpu::Backend,
) -> Option<wgpu::CompositeAlphaMode> {
    use wgpu::CompositeAlphaMode::{Opaque, PostMultiplied, PreMultiplied};
    if modes.contains(&PreMultiplied) {
        Some(PreMultiplied)
    } else if backend == wgpu::Backend::Metal && modes.contains(&PostMultiplied)
    {
        // wgpu 30 labels Metal's premultiplied compositor as postmultiplied.
        Some(PostMultiplied)
    } else if cfg!(target_os = "linux")
        && backend == wgpu::Backend::Gl
        && modes.contains(&Opaque)
    {
        // wgpu 30 reports only Opaque for EGL; Wayland composites its alpha.
        Some(Opaque)
    } else {
        None
    }
}

fn surface_format(
    formats: &[wgpu::TextureFormat],
) -> Option<wgpu::TextureFormat> {
    // RGB10A2's four alpha levels cause fringes around transparent glyphs.
    formats.iter().copied().find(|format| {
        matches!(
            format,
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm
        )
    })
}

#[cfg(test)]
mod tests {
    use super::{adapter_priority, surface_format, transparent_alpha};
    use wgpu::TextureFormat::*;

    #[test]
    fn native_backend_precedes_gl_without_changing_device_preference() {
        use wgpu::Backend::{Gl, Vulkan};
        use wgpu::DeviceType::{DiscreteGpu, IntegratedGpu};
        let mut adapters = [
            (IntegratedGpu, Gl),
            (DiscreteGpu, Vulkan),
            (IntegratedGpu, Vulkan),
        ];
        adapters.sort_by_key(|&(device, backend)| {
            adapter_priority(device, backend)
        });
        assert_eq!(
            adapters,
            [
                (IntegratedGpu, Vulkan),
                (IntegratedGpu, Gl),
                (DiscreteGpu, Vulkan),
            ]
        );
    }

    #[test]
    fn transparent_surface_uses_premultiplied_compositor_pixels() {
        use wgpu::Backend::{Gl, Metal, Vulkan};
        use wgpu::CompositeAlphaMode::{Opaque, PostMultiplied, PreMultiplied};
        assert_eq!(transparent_alpha(&[Opaque], Metal), None);
        assert_eq!(
            transparent_alpha(&[Opaque, PostMultiplied], Metal),
            Some(PostMultiplied),
        );
        assert_eq!(transparent_alpha(&[PostMultiplied], Vulkan), None);
        assert_eq!(
            transparent_alpha(&[Opaque], Gl),
            cfg!(target_os = "linux").then_some(Opaque),
        );
        assert_eq!(transparent_alpha(&[], Gl), None);
        for backend in [Metal, Vulkan, Gl] {
            assert_eq!(
                transparent_alpha(&[PostMultiplied, PreMultiplied], backend),
                Some(PreMultiplied),
            );
        }
    }

    #[test]
    fn surface_preserves_glyph_alpha_precision_without_srgb_encoding() {
        assert_eq!(
            surface_format(&[Rgb10a2Unorm, Bgra8UnormSrgb, Bgra8Unorm]),
            Some(Bgra8Unorm),
        );
        assert_eq!(surface_format(&[Rgba8Unorm]), Some(Rgba8Unorm));
        assert_eq!(surface_format(&[Rgb10a2Unorm, Bgra8UnormSrgb]), None);
    }
}
