//! AppKit / ScreenCaptureKit glue. Everything here runs on the main thread;
//! background work hops back with [`on_main`].

pub mod capture;
pub mod editor;
pub mod hotkey;
pub mod overlay;
pub mod prefs;
pub mod render;
pub mod status;

use dispatch2::DispatchQueue;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSAlert, NSAlertFirstButtonReturn, NSAlertStyle, NSApplication};
use objc2_foundation::NSString;

/// Moves a value to the main queue. Only sound when the value is touched
/// exclusively on the main thread, which [`on_main`] guarantees.
pub struct MainOnly<T>(pub T);
// SAFETY: see the type's docs; values are only unwrapped inside on_main.
unsafe impl<T> Send for MainOnly<T> {}

/// Runs `f` asynchronously on the main thread.
pub fn on_main(f: impl FnOnce(MainThreadMarker) + Send + 'static) {
    DispatchQueue::main().exec_async(move || {
        // SAFETY: the main queue runs on the main thread.
        f(unsafe { MainThreadMarker::new_unchecked() })
    });
}

pub fn ns(s: &str) -> Retained<NSString> {
    NSString::from_str(s)
}

pub fn activate(mtm: MainThreadMarker) {
    let app = NSApplication::sharedApplication(mtm);
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);
}

/// Shows a modal alert and returns the index of the button pressed. Never call
/// it while holding the app state borrow: the modal loop dispatches events.
pub fn alert(mtm: MainThreadMarker, title: &str, message: &str, buttons: &[&str], warning: bool) -> usize {
    activate(mtm);
    let a = NSAlert::new(mtm);
    a.setMessageText(&ns(title));
    a.setInformativeText(&ns(message));
    a.setAlertStyle(if warning { NSAlertStyle::Warning } else { NSAlertStyle::Informational });
    for b in buttons {
        a.addButtonWithTitle(&ns(b));
    }
    (a.runModal() - NSAlertFirstButtonReturn).max(0) as usize
}

/// An alert that waits for the current event to finish, so callers can show
/// one while holding state.
pub fn alert_later(title: &str, message: &str) {
    let (t, m) = (title.to_string(), message.to_string());
    on_main(move |mtm| {
        alert(mtm, &t, &m, &["OK"], true);
    });
}

pub fn beep() {
    unsafe extern "C" {
        fn NSBeep();
    }
    // SAFETY: no arguments, no preconditions.
    unsafe { NSBeep() }
}
