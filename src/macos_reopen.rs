use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{Context, Result, ensure};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Sel};
use objc2::{class, msg_send, msg_send_id, sel};
use objc2_app_kit::NSApplication;
use objc2_foundation::{MainThreadMarker, NSString};
use winit::event_loop::EventLoopProxy;

static PROXY: OnceLock<EventLoopProxy<()>> = OnceLock::new();
static PENDING: AtomicUsize = AtomicUsize::new(0);

pub fn install(proxy: EventLoopProxy<()>) -> Result<()> {
    let mtm =
        MainThreadMarker::new().context("Reopen handler needs main thread")?;
    let app = NSApplication::sharedApplication(mtm);
    let delegate = unsafe { app.delegate() }
        .context("Missing native application delegate")?;
    let object = unsafe { &*Retained::as_ptr(&delegate).cast::<AnyObject>() };
    static CLASS: OnceLock<&AnyClass> = OnceLock::new();
    let class = CLASS.get_or_init(|| {
        let mut builder =
            ClassBuilder::new("CrypttyApplicationDelegate", object.class())
                .expect("Reopen handler class name must be unique");
        // The signature matches NSApplicationDelegate's reopen callback.
        unsafe {
            builder.add_method(
                sel!(applicationShouldHandleReopen:hasVisibleWindows:),
                reopen
                    as unsafe extern "C" fn(
                        *mut AnyObject,
                        Sel,
                        *mut AnyObject,
                        Bool,
                    ) -> Bool,
            );
        }
        builder.register()
    });
    ensure!(
        class.superclass() == Some(object.class())
            && class.instance_size() == object.class().instance_size(),
        "Unexpected native application delegate class"
    );
    PROXY
        .set(proxy)
        .map_err(|_| anyhow::anyhow!("Reopen handler already set"))?;
    // Preserve winit's delegate, its ivars, and all lifecycle methods.
    unsafe { AnyObject::set_class(object, class) };
    Ok(())
}

unsafe extern "C" fn reopen(
    _: *mut AnyObject,
    _: Sel,
    _: *mut AnyObject,
    _: Bool,
) -> Bool {
    if reopen_sender().as_deref() == Some("com.apple.dock") {
        return Bool::YES;
    }
    PENDING.fetch_add(1, Ordering::Relaxed);
    if let Some(proxy) = PROXY.get() {
        let _ = proxy.send_event(());
    }
    Bool::NO
}

fn reopen_sender() -> Option<String> {
    // Read the current event synchronously; do not retain or mutate it.
    unsafe {
        let manager: *mut AnyObject =
            msg_send![class!(NSAppleEventManager), sharedAppleEventManager];
        let event: *mut AnyObject = msg_send![manager, currentAppleEvent];
        if event.is_null() {
            return None;
        }
        let sender: *mut AnyObject = msg_send![event,
            attributeDescriptorForKeyword: u32::from_be_bytes(*b"spid")];
        if sender.is_null() {
            return None;
        }
        let pid: i32 = msg_send![sender, int32Value];
        if pid <= 0 {
            return None;
        }
        let app: Option<Retained<AnyObject>> = msg_send_id![
            class!(NSRunningApplication),
            runningApplicationWithProcessIdentifier: pid
        ];
        let name: Option<Retained<NSString>> =
            msg_send_id![&*app?, bundleIdentifier];
        name.map(|name| name.to_string())
    }
}

pub fn take_requests() -> usize {
    PENDING.swap(0, Ordering::Relaxed)
}
