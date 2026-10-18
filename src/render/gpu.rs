use std::sync::{Arc, Mutex};

use anyhow::{Result, bail};
use winit::window::Window;

use super::{Renderer, painter::Painter};
use crate::terminal::Terminal;

fn instance() -> wgpu::Instance {
    wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::METAL | wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    })
}

async fn adapters(instance: &wgpu::Instance) -> Vec<wgpu::Adapter> {
    let mut adapters = instance
        .enumerate_adapters(wgpu::Backends::METAL | wgpu::Backends::VULKAN)
        .await;
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

pub fn list_adapters() {
    let instance = instance();
    let adapters = pollster::block_on(adapters(&instance));
    if adapters.is_empty() {
        println!("No Metal or Vulkan adapters found.");
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
    pub fn new(window: Arc<Window>, selected: Option<usize>) -> Result<Self> {
        pollster::block_on(Self::initialize(window, selected))
    }

    async fn initialize(
        window: Arc<Window>,
        selected: Option<usize>,
    ) -> Result<Self> {
        let instance = instance();
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
            candidates.sort_by_key(|(_, a)| match a.get_info().device_type {
                wgpu::DeviceType::IntegratedGpu => 0,
                wgpu::DeviceType::DiscreteGpu => 1,
                wgpu::DeviceType::VirtualGpu => 2,
                _ => 3,
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
            let painter = Painter::new(&device, config.format);
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
                transparent_alpha: capabilities
                    .alpha_modes
                    .iter()
                    .copied()
                    .find(|mode| {
                        *mode == wgpu::CompositeAlphaMode::PreMultiplied
                    }),
            });
        }
        bail!("No usable GPU found. {}", errors.join("; "))
    }

    pub fn set_opacity(&mut self, opacity: crate::config::Opacity) -> bool {
        let (opacity, alpha) = match (
            opacity.transparent(),
            self.transparent_alpha,
        ) {
            (true, Some(alpha)) => (opacity, alpha),
            (true, None) => {
                eprintln!(
                    "Cryptty: GPU surface lacks premultiplied transparency; \
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
    use super::surface_format;
    use wgpu::TextureFormat::*;

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
