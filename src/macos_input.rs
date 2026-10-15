use std::sync::OnceLock;

use anyhow::{Context, Result, ensure};
use objc2::runtime::{AnyClass, AnyObject, ClassBuilder, Sel};
use objc2::{msg_send, sel};
use objc2_app_kit::{NSApplication, NSEventModifierFlags, NSEventType};
use objc2_foundation::{
    MainThreadMarker, NSAttributedString, NSObject, NSObjectProtocol, NSRange,
    NSString,
};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

pub fn install(window: &Window) -> Result<()> {
    MainThreadMarker::new().context("Input adapter requires main thread")?;
    let handle = window.window_handle()?;
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        anyhow::bail!("Expected an AppKit window");
    };
    // The window owns this view, and the handle keeps it valid for this call.
    let view = unsafe { handle.ns_view.cast::<AnyObject>().as_ref() };
    static CLASS: OnceLock<&AnyClass> = OnceLock::new();
    let class = CLASS.get_or_init(|| {
        let mut builder = ClassBuilder::new("CrypttyInputView", view.class())
            .expect("Input adapter class name must be unique");
        // The signature matches NSTextInputClient's insertText method.
        unsafe {
            builder.add_method(
                sel!(insertText:replacementRange:),
                insert_text
                    as unsafe extern "C" fn(
                        *mut AnyObject,
                        Sel,
                        *mut NSObject,
                        NSRange,
                    ),
            );
        }
        builder.register()
    });
    if view.class() == *class {
        return Ok(());
    }
    ensure!(
        class.superclass() == Some(view.class())
            && class.instance_size() == view.class().instance_size(),
        "Unexpected native view class for input adapter"
    );
    // Main-thread-only subclassing adds no ivars or ownership changes.
    let previous = unsafe { AnyObject::set_class(view, class) };
    assert_eq!(Some(previous), class.superclass());
    Ok(())
}

unsafe extern "C" fn insert_text(
    view: *mut AnyObject,
    _selector: Sel,
    value: *mut NSObject,
    replacement: NSRange,
) {
    // Both pointers are non-null AppKit method arguments for this call.
    let view = unsafe { &*view };
    let value = unsafe { &*value };
    let superclass = view.class().superclass().unwrap();
    // AppKit supplies either NSString or NSAttributedString on the main thread.
    let text = if value.is_kind_of::<NSAttributedString>() {
        unsafe { &*(value as *const NSObject).cast::<NSAttributedString>() }
            .string()
            .to_string()
    } else {
        unsafe { &*(value as *const NSObject).cast::<NSString>() }.to_string()
    };
    let marked: bool = unsafe { msg_send![view, hasMarkedText] };
    let mtm =
        MainThreadMarker::new().expect("AppKit input runs on main thread");
    let event = NSApplication::sharedApplication(mtm).currentEvent();
    if let Some(event) = event {
        let key_down = unsafe { event.r#type() } == NSEventType::KeyDown;
        let shortcut = unsafe { event.modifierFlags() }.intersects(
            NSEventModifierFlags::NSEventModifierFlagControl
                | NSEventModifierFlags::NSEventModifierFlagCommand,
        );
        let original = unsafe { event.characters() }.map(|s| s.to_string());
        if key_down
            && !marked
            && !shortcut
            && transformed_commit(&text, original.as_deref())
        {
            // Let winit commit converted text and suppress the original key.
            unsafe {
                let _: () = msg_send![super(view, superclass),
                    setMarkedText: value,
                    selectedRange: NSRange::new(0, 0),
                    replacementRange: replacement,
                ];
            }
        }
    }
    unsafe {
        let _: () = msg_send![super(view, superclass),
            insertText: value, replacementRange: replacement,
        ];
    }
}

fn transformed_commit(text: &str, original: Option<&str>) -> bool {
    !text.is_empty()
        && !text.chars().any(char::is_control)
        && original.is_some_and(|original| text != original)
}

#[cfg(test)]
mod tests {
    use super::transformed_commit;

    #[test]
    fn direct_punctuation_and_converted_text_get_marked() {
        for (text, key) in [("。", "."), ("，", ","), ("！", "!"), ("é", "e")]
        {
            assert!(transformed_commit(text, Some(key)));
        }
    }

    #[test]
    fn unchanged_layouts_and_control_text_keep_normal_handling() {
        for text in ["a", " ", "я", "", "\r", "\t", "\u{3}"] {
            assert!(!transformed_commit(text, Some(text)));
        }
        assert!(!transformed_commit("\r", Some("x")));
        assert!(!transformed_commit("。", None));
    }
}
