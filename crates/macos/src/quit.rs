//! Intercepts AppKit's application-level termination (Dock Quit / Apple-event Quit) before
//! winit's will-terminate observer can close all windows without the editor's save/build gate.

use std::sync::{Arc, OnceLock};

use objc2::declare::ClassBuilder;
use objc2::runtime::{AnyObject, Sel};
use objc2::sel;
use objc2_app_kit::{NSApplication, NSApplicationTerminateReply};
use objc2_foundation::MainThreadMarker;

static REQUEST: OnceLock<Arc<dyn Fn() + Send + Sync>> = OnceLock::new();

extern "C" fn should_terminate(
    _delegate: &AnyObject,
    _selector: Sel,
    _application: &NSApplication,
) -> NSApplicationTerminateReply {
    forward_termination(REQUEST.get().map(|request| request.as_ref()))
}

fn forward_termination(request: Option<&(dyn Fn() + Send + Sync)>) -> NSApplicationTerminateReply {
    if let Some(request) = request {
        request();
    }
    // The editor requests its normal window close after the asynchronous build succeeds.
    // Cancel this AppKit termination so it never reaches winit's unconditional window.close().
    NSApplicationTerminateReply::NSTerminateCancel
}

pub(super) fn install(request: Arc<dyn Fn() + Send + Sync>) -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let app = NSApplication::sharedApplication(mtm);
    // SAFETY: NSApplication and its live delegate are accessed on the main thread.
    let Some(protocol_delegate) = (unsafe { app.delegate() }) else {
        return false;
    };
    // SAFETY: A second retained reference to the same main-thread delegate, for type erasure.
    let Some(delegate) = (unsafe { app.delegate() }) else {
        return false;
    };
    // SAFETY: Type erasure of the retained, live Objective-C delegate; AnyObject preserves its
    // identity and ownership, and no assumptions are made about winit's private Rust ivar type.
    let delegate = unsafe { objc2::rc::Retained::cast::<AnyObject>(delegate) };
    if REQUEST.get().is_some() {
        return delegate.class().name() == "TexCraftApplicationQuitDelegate";
    }
    // Keep winit's delegate, ivars, protocol implementations and superclass identity intact.
    // Its own get() checks is_kind_of(WinitApplicationDelegate), which this subclass satisfies.
    let Some(mut builder) = ClassBuilder::new("TexCraftApplicationQuitDelegate", delegate.class())
    else {
        return false;
    };
    // SAFETY: This adds only AppKit's optional applicationShouldTerminate: callback with its
    // exact object/selector/application argument and NSApplicationTerminateReply return ABI.
    unsafe {
        builder.add_method(
            sel!(applicationShouldTerminate:),
            should_terminate as extern "C" fn(_, _, _) -> _,
        );
    }
    let class = builder.register();
    if class.instance_size() != delegate.class().instance_size() {
        return false;
    }
    if REQUEST.set(request).is_err() {
        return false;
    }
    // SAFETY: Main-thread-only mutation of the one app delegate, to its subclass with no new
    // ivars and the same instance size. All inherited winit behavior and object layout remain.
    unsafe {
        AnyObject::set_class(&delegate, class);
    }
    // Refresh AppKit's cached optional-delegate selectors after adding should-terminate.
    // Both retains above keep the exact same delegate alive across these main-thread setters.
    app.setDelegate(None);
    app.setDelegate(Some(&protocol_delegate));
    true
}

pub(super) fn request() -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    // SAFETY: This is the same standard action as Dock Quit, sent on the main thread. The
    // installed delegate cancels it and routes it through the normal asynchronous close gate.
    unsafe {
        NSApplication::sharedApplication(mtm).terminate(None);
    }
    true
}

#[cfg(test)]
mod tests {
    #[test]
    fn native_termination_is_cancelled_and_forwarded_to_the_normal_exit_gate() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let calls = AtomicUsize::new(0);
        let forward = || {
            calls.fetch_add(1, Ordering::SeqCst);
        };
        assert_eq!(
            super::forward_termination(Some(&forward)),
            super::NSApplicationTerminateReply::NSTerminateCancel
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
