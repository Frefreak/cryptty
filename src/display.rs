use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use cryptty::{
    options::RendererChoice,
    render::{Renderer, gpu::Gpu},
    terminal::Terminal,
};
use softbuffer::{Context, Surface};
use winit::window::Window;

enum Backend {
    Software(Surface<Arc<Window>, Arc<Window>>),
    Gpu(Box<Gpu>),
}

pub struct Display {
    backend: Option<Backend>,
    automatic: bool,
}

impl Display {
    pub fn new(window: Arc<Window>, choice: RendererChoice) -> Result<Self> {
        let backend = match choice {
            RendererChoice::Software => software(window)?,
            RendererChoice::Auto | RendererChoice::Gpu { .. } => {
                let selected = match choice {
                    RendererChoice::Gpu { adapter } => adapter,
                    _ => None,
                };
                match Gpu::new(window.clone(), selected) {
                    Ok(gpu) => Backend::Gpu(Box::new(gpu)),
                    Err(error) if choice == RendererChoice::Auto => {
                        eprintln!("Cryptty: GPU unavailable: {error:#}");
                        software(window)?
                    }
                    Err(error) => return Err(error),
                }
            }
        };
        Ok(Self {
            backend: Some(backend),
            automatic: choice == RendererChoice::Auto,
        })
    }

    pub fn frame_interval(&self, window: &Window) -> Duration {
        // Leave the event loop available while Metal recycles its drawables.
        #[cfg(target_os = "macos")]
        if matches!(self.backend, Some(Backend::Gpu(_))) {
            let millihertz = window
                .current_monitor()
                .and_then(|monitor| monitor.refresh_rate_millihertz())
                .filter(|rate| *rate > 0)
                .unwrap_or(60_000);
            return Duration::from_secs_f64(1000.0 / f64::from(millihertz));
        }
        let _ = window;
        Duration::ZERO
    }

    pub fn clear_glyphs(&mut self) {
        if let Some(Backend::Gpu(gpu)) = &mut self.backend {
            gpu.clear_glyphs();
        }
    }

    pub fn draw(
        &mut self,
        window: &Arc<Window>,
        renderer: &mut Renderer,
        terminal: &Terminal,
        preedit: &str,
    ) -> Result<()> {
        if let Some(Backend::Gpu(gpu)) = &mut self.backend {
            match gpu.draw(window, renderer, terminal, preedit) {
                Ok(()) => return Ok(()),
                Err(error) if self.automatic => {
                    eprintln!(
                        "Cryptty: GPU failed: {error:#}; switching to software"
                    );
                    self.backend.take();
                    self.backend = Some(software(window.clone())?);
                    renderer.invalidate();
                }
                Err(error) => return Err(error),
            }
        }
        let Some(Backend::Software(surface)) = &mut self.backend else {
            anyhow::bail!("No display backend");
        };
        let size = window.inner_size();
        let (Some(width), Some(height)) =
            (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
        else {
            return Ok(());
        };
        surface
            .resize(width, height)
            .map_err(|e| anyhow::anyhow!("resize software surface: {e}"))?;
        renderer.draw(terminal, size.width, size.height, preedit);
        let mut buffer = surface
            .buffer_mut()
            .map_err(|e| anyhow::anyhow!("acquire software buffer: {e}"))?;
        buffer.copy_from_slice(&renderer.pixels);
        window.pre_present_notify();
        buffer
            .present()
            .map_err(|e| anyhow::anyhow!("present frame: {e}"))?;
        Ok(())
    }
}

fn software(window: Arc<Window>) -> Result<Backend> {
    let context = Context::new(window.clone())
        .map_err(|e| anyhow::anyhow!("software display: {e}"))?;
    let surface = Surface::new(&context, window)
        .map_err(|e| anyhow::anyhow!("software surface: {e}"))?;
    eprintln!("Cryptty: software renderer");
    Ok(Backend::Software(surface))
}
