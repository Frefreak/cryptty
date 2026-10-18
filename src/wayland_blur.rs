use std::sync::Arc;

use anyhow::Result;
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle,
    backend::{Backend, ObjectId},
    delegate_noop,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{wl_compositor, wl_region, wl_registry, wl_surface},
};
use wayland_protocols::ext::background_effect::v1::client::{
    ext_background_effect_manager_v1::ExtBackgroundEffectManagerV1,
    ext_background_effect_surface_v1::ExtBackgroundEffectSurfaceV1,
};
use winit::{
    raw_window_handle::{
        HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle,
    },
    window::Window,
};

pub struct Blur {
    effect: ExtBackgroundEffectSurfaceV1,
    region: wl_region::WlRegion,
    connection: Connection,
    // The foreign Wayland display must outlive its borrowed connection.
    _window: Arc<Window>,
}

impl Blur {
    pub fn new(window: Arc<Window>) -> Result<Option<Self>> {
        let RawDisplayHandle::Wayland(display) =
            window.display_handle()?.as_raw()
        else {
            return Ok(None);
        };
        let RawWindowHandle::Wayland(handle) = window.window_handle()?.as_raw()
        else {
            return Ok(None);
        };
        // Window owns both pointers and stays alive until after connection.
        let backend = unsafe {
            Backend::from_foreign_display(display.display.as_ptr().cast())
        };
        let connection = Connection::from_backend(backend);
        let (globals, queue) = registry_queue_init::<State>(&connection)?;
        let qh = queue.handle();
        let manager: ExtBackgroundEffectManagerV1 =
            match globals.bind(&qh, 1..=1, ()) {
                Ok(manager) => manager,
                Err(error) => {
                    globals.destroy();
                    return match error {
                        wayland_client::globals::BindError::NotPresent => {
                            Ok(None)
                        }
                        error => Err(error.into()),
                    };
                }
            };
        let compositor = globals.bind(&qh, 1..=1, ());
        globals.destroy();
        let compositor: wl_compositor::WlCompositor = compositor?;
        // Borrow the winit-owned surface without destroying it.
        let surface_id = unsafe {
            ObjectId::from_ptr(
                wl_surface::WlSurface::interface(),
                handle.surface.as_ptr().cast(),
            )?
        };
        let surface = wl_surface::WlSurface::from_id(&connection, surface_id)?;
        let effect = manager.get_background_effect(&surface, &qh, ());
        manager.destroy();
        let region = compositor.create_region(&qh, ());
        // The compositor clips this region to the current surface bounds.
        region.add(0, 0, i32::MAX, i32::MAX);
        Ok(Some(Self {
            effect,
            region,
            connection,
            _window: window,
        }))
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.effect.set_blur_region(enabled.then_some(&self.region));
        // The next frame commits this double-buffered surface state.
    }
}

impl Drop for Blur {
    fn drop(&mut self) {
        self.effect.destroy();
        self.region.destroy();
        let _ = self.connection.flush();
    }
}

struct State;

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

delegate_noop!(State: ignore ExtBackgroundEffectManagerV1);
delegate_noop!(State: ExtBackgroundEffectSurfaceV1);
delegate_noop!(State: wl_compositor::WlCompositor);
delegate_noop!(State: wl_region::WlRegion);
