//! The Preferences window: four shortcut recorders, the save folder, the
//! "also save quick captures" toggle, the capture sound and launch at login
//! (SMAppService).

use std::cell::{Cell, OnceCell};
use std::path::{Path, PathBuf};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSBackingStoreType, NSButton, NSColor, NSControlStateValueOff, NSControlStateValueOn, NSEvent,
    NSEventModifierFlags, NSFont, NSLineBreakMode, NSMenuItem, NSModalResponseOK, NSOpenPanel, NSPopUpButton,
    NSTextField, NSView, NSWindow, NSWindowDelegate, NSWindowStyleMask,
};
use objc2_foundation::{NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, ns_string};
use objc2_service_management::{SMAppService, SMAppServiceStatus};

use super::ns;
use crate::app::{self, PrefsEvent};
use crate::core::hotkey::{self, Binding};
use crate::core::settings::{Action, SOUNDS, Settings};

const UNSET: &str = "Record Shortcut";
const NO_SOUND: &str = "None";

pub struct RecorderIvars {
    action: Action,
    recording: Cell<bool>,
}

define_class!(
    // SAFETY: NSButton has no subclassing requirements; no Drop impl.
    #[unsafe(super(NSButton, objc2_app_kit::NSControl, NSView, objc2_app_kit::NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = RecorderIvars]
    pub struct Recorder;

    impl Recorder {
        #[unsafe(method(startRecording:))]
        fn start_recording(&self, _s: Option<&AnyObject>) {
            if self.ivars().recording.get() {
                return self.stop(None);
            }
            self.ivars().recording.set(true);
            self.setTitle(ns_string!("Type shortcut…"));
            if let Some(w) = self.window() {
                w.makeFirstResponder(Some(self));
            }
            app::prefs_event(PrefsEvent::Recording(true));
        }

        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }

        #[unsafe(method(performKeyEquivalent:))]
        fn perform_key_equivalent(&self, e: &NSEvent) -> bool {
            if !self.ivars().recording.get() {
                // SAFETY: forwarding to NSButton.
                return unsafe { msg_send![super(self), performKeyEquivalent: e] };
            }
            self.record(e);
            true
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, e: &NSEvent) {
            if self.ivars().recording.get() {
                self.record(e);
            } else {
                // SAFETY: forwarding to NSButton.
                unsafe { msg_send![super(self), keyDown: e] }
            }
        }

        #[unsafe(method(resignFirstResponder))]
        fn resign_first_responder(&self) -> bool {
            if self.ivars().recording.get() {
                self.stop(None);
            }
            true
        }
    }
);

fn carbon_mods(flags: NSEventModifierFlags) -> u32 {
    let mut m = 0;
    for (f, c) in [
        (NSEventModifierFlags::Command, hotkey::CMD),
        (NSEventModifierFlags::Shift, hotkey::SHIFT),
        (NSEventModifierFlags::Option, hotkey::OPT),
        (NSEventModifierFlags::Control, hotkey::CTRL),
    ] {
        if flags.contains(f) {
            m |= c;
        }
    }
    m
}

impl Recorder {
    fn record(&self, e: &NSEvent) {
        let code = e.keyCode() as u32;
        let mods = carbon_mods(e.modifierFlags());
        match (code, mods) {
            (53, 0) => self.stop(None),                          // Esc cancels
            (51 | 117, 0) => self.stop(Some(Binding::default())), // Delete clears
            _ => self.stop(Some(Binding { code, mods })),
        }
    }

    /// Ends recording; `Some` applies a new binding (unbound = clear).
    fn stop(&self, binding: Option<Binding>) {
        self.ivars().recording.set(false);
        let action = self.ivars().action;
        match binding {
            Some(b) => app::prefs_event(PrefsEvent::SetHotkey(action, b)),
            None => {
                app::prefs_event(PrefsEvent::Recording(false));
                app::prefs_event(PrefsEvent::SetHotkey(action, current_binding(action)));
            }
        }
    }
}

/// The binding currently saved for an action (re-applied after a cancel so the row redraws).
fn current_binding(action: Action) -> Binding {
    let (s, _) = crate::core::settings::load(&crate::core::settings::path());
    Binding::parse(s.hotkeys.get(action)).unwrap_or_default()
}

pub struct PrefsIvars {
    recorders: OnceCell<Vec<Retained<Recorder>>>,
    warnings: OnceCell<Vec<Retained<NSTextField>>>,
    folder: OnceCell<Retained<NSTextField>>,
    login: OnceCell<Retained<NSButton>>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements; no Drop impl.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = PrefsIvars]
    pub struct PrefsTarget;

    unsafe impl NSObjectProtocol for PrefsTarget {}

    impl PrefsTarget {
        #[unsafe(method(clearShortcut:))]
        fn clear_shortcut(&self, sender: &NSButton) {
            if let Some(&action) = Action::ALL.get(sender.tag() as usize) {
                app::prefs_event(PrefsEvent::SetHotkey(action, Binding::default()));
            }
        }

        #[unsafe(method(chooseFolder:))]
        fn choose_folder(&self, _s: Option<&AnyObject>) {
            let mtm = self.mtm();
            let panel = NSOpenPanel::openPanel(mtm);
            panel.setCanChooseDirectories(true);
            panel.setCanChooseFiles(false);
            panel.setCanCreateDirectories(true);
            panel.setPrompt(Some(ns_string!("Choose")));
            if panel.runModal() == NSModalResponseOK
                && let Some(path) = panel.URL().and_then(|u| u.path())
            {
                app::prefs_event(PrefsEvent::SetFolder(PathBuf::from(path.to_string())));
            }
        }

        #[unsafe(method(toggleSaveQuick:))]
        fn toggle_save_quick(&self, sender: &NSButton) {
            app::prefs_event(PrefsEvent::SaveQuick(sender.state() == NSControlStateValueOn));
        }

        #[unsafe(method(chooseSound:))]
        fn choose_sound(&self, sender: &NSPopUpButton) {
            let title = sender.titleOfSelectedItem().map(|t| t.to_string()).unwrap_or_default();
            let name = if title == NO_SOUND { String::new() } else { title };
            app::prefs_event(PrefsEvent::Sound(name));
        }

        #[unsafe(method(toggleLogin:))]
        fn toggle_login(&self, sender: &NSButton) {
            app::prefs_event(PrefsEvent::LaunchAtLogin(sender.state() == NSControlStateValueOn));
        }
    }

    unsafe impl NSWindowDelegate for PrefsTarget {
        #[unsafe(method(windowWillClose:))]
        fn window_will_close(&self, n: &NSNotification) {
            if let Some(w) = n.object().and_then(|o| o.downcast::<NSWindow>().ok()) {
                w.setDelegate(None);
            }
            super::on_main(|_| app::prefs_event(PrefsEvent::Closed));
        }
    }
);

/// A handle to the open Preferences window.
pub struct Prefs {
    window: Retained<NSWindow>,
    target: Retained<PrefsTarget>,
}

fn label(mtm: MainThreadMarker, text: &str, frame: NSRect) -> Retained<NSTextField> {
    let l = NSTextField::labelWithString(&ns(text), mtm);
    l.setFrame(frame);
    l
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}

pub fn open(mtm: MainThreadMarker, settings: &Settings, login_enabled: bool) -> Prefs {
    let (w, h) = (520.0, 440.0);
    // SAFETY: kept alive through Retained; release-on-close disabled.
    let window = unsafe {
        let win = NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            rect(0.0, 0.0, w, h),
            NSWindowStyleMask::Titled | NSWindowStyleMask::Closable,
            NSBackingStoreType::Buffered,
            false,
        );
        win.setReleasedWhenClosed(false);
        win
    };
    window.setTitle(ns_string!("snapit Preferences"));
    let root = window.contentView().expect("content view");

    let ivars = PrefsIvars { recorders: OnceCell::new(), warnings: OnceCell::new(), folder: OnceCell::new(), login: OnceCell::new() };
    // SAFETY: plain NSObject init.
    let target: Retained<PrefsTarget> = unsafe { msg_send![super(PrefsTarget::alloc(mtm).set_ivars(ivars)), init] };

    let mut y = h - 44.0;
    let heading = label(mtm, "Shortcuts", rect(20.0, y, 300.0, 20.0));
    heading.setFont(Some(&NSFont::boldSystemFontOfSize(13.0)));
    root.addSubview(&heading);

    let mut recorders = Vec::new();
    let mut warnings = Vec::new();
    for action in Action::ALL {
        y -= 34.0;
        root.addSubview(&label(mtm, action.label(), rect(20.0, y + 4.0, 200.0, 18.0)));
        let rec_ivars = RecorderIvars { action, recording: Cell::new(false) };
        // SAFETY: NSButton's designated initializer.
        let rec: Retained<Recorder> =
            unsafe { msg_send![super(Recorder::alloc(mtm).set_ivars(rec_ivars)), initWithFrame: rect(230.0, y, 170.0, 26.0)] };
        rec.setBezelStyle(objc2_app_kit::NSBezelStyle::Push);
        rec.setTitle(ns_string!(""));
        // SAFETY: the recorder is its own target.
        unsafe {
            rec.setTarget(Some(&rec));
            rec.setAction(Some(sel!(startRecording:)));
        }
        root.addSubview(&rec);

        // SAFETY: target outlives the button.
        let clear = unsafe {
            NSButton::buttonWithTitle_target_action(ns_string!("Clear"), Some(&target), Some(sel!(clearShortcut:)), mtm)
        };
        clear.setFrame(rect(404.0, y, 80.0, 26.0));
        clear.setTag(action.index() as isize);
        root.addSubview(&clear);

        let warn = label(mtm, "", rect(230.0, y - 14.0, 270.0, 14.0));
        warn.setFont(Some(&NSFont::systemFontOfSize(10.0)));
        warn.setTextColor(Some(&NSColor::secondaryLabelColor()));
        warn.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        root.addSubview(&warn);
        recorders.push(rec);
        warnings.push(warn);
        y -= 10.0;
    }

    y -= 44.0;
    let heading = label(mtm, "Saving", rect(20.0, y, 300.0, 20.0));
    heading.setFont(Some(&NSFont::boldSystemFontOfSize(13.0)));
    root.addSubview(&heading);
    y -= 32.0;
    root.addSubview(&label(mtm, "Save folder", rect(20.0, y + 4.0, 200.0, 18.0)));
    let folder = label(mtm, "", rect(230.0, y + 4.0, 170.0, 18.0));
    folder.setLineBreakMode(NSLineBreakMode::ByTruncatingMiddle);
    root.addSubview(&folder);
    // SAFETY: target outlives the button.
    let choose = unsafe {
        NSButton::buttonWithTitle_target_action(ns_string!("Choose…"), Some(&target), Some(sel!(chooseFolder:)), mtm)
    };
    choose.setFrame(rect(404.0, y, 80.0, 26.0));
    root.addSubview(&choose);

    y -= 32.0;
    // SAFETY: target outlives the checkbox.
    let save_quick = unsafe {
        NSButton::checkboxWithTitle_target_action(
            ns_string!("Also save quick captures to the folder"),
            Some(&target),
            Some(sel!(toggleSaveQuick:)),
            mtm,
        )
    };
    save_quick.setFrame(rect(228.0, y, 280.0, 20.0));
    save_quick.setState(if settings.save_quick_captures { NSControlStateValueOn } else { NSControlStateValueOff });
    root.addSubview(&save_quick);

    y -= 36.0;
    root.addSubview(&label(mtm, "Capture sound", rect(20.0, y + 4.0, 200.0, 18.0)));
    let sound = NSPopUpButton::initWithFrame_pullsDown(NSPopUpButton::alloc(mtm), rect(228.0, y, 174.0, 26.0), false);
    sound.addItemWithTitle(&ns(NO_SOUND));
    if let Some(menu) = sound.menu() {
        menu.addItem(&NSMenuItem::separatorItem(mtm));
    }
    for name in SOUNDS {
        sound.addItemWithTitle(&ns(name));
    }
    sound.selectItemWithTitle(&ns(if settings.sound.is_empty() { NO_SOUND } else { &settings.sound }));
    // SAFETY: target outlives the pop-up.
    unsafe {
        sound.setTarget(Some(&target));
        sound.setAction(Some(sel!(chooseSound:)));
    }
    root.addSubview(&sound);

    y -= 40.0;
    // SAFETY: target outlives the checkbox.
    let login = unsafe {
        NSButton::checkboxWithTitle_target_action(ns_string!("Launch snapit at login"), Some(&target), Some(sel!(toggleLogin:)), mtm)
    };
    login.setFrame(rect(18.0, y, 300.0, 20.0));
    root.addSubview(&login);

    let _ = target.ivars().recorders.set(recorders);
    let _ = target.ivars().warnings.set(warnings);
    let _ = target.ivars().folder.set(folder);
    let _ = target.ivars().login.set(login);
    window.setDelegate(Some(ProtocolObject::from_ref(&*target)));

    let p = Prefs { window, target };
    p.set_folder(&settings.save_dir);
    p.set_login(login_enabled);
    p.window.center();
    p.front();
    p
}

impl Prefs {
    pub fn front(&self) {
        self.window.makeKeyAndOrderFront(None);
    }

    pub fn set_row(&self, action: Action, display: &str, warning: &str) {
        let iv = self.target.ivars();
        if let Some(r) = iv.recorders.get().and_then(|v| v.get(action.index())) {
            r.setTitle(&ns(if display.is_empty() { UNSET } else { display }));
        }
        if let Some(l) = iv.warnings.get().and_then(|v| v.get(action.index())) {
            l.setStringValue(&ns(warning));
        }
    }

    pub fn set_folder(&self, dir: &Path) {
        if let Some(l) = self.target.ivars().folder.get() {
            let home = crate::core::files::home();
            let shown = match dir.strip_prefix(&home) {
                Ok(rest) => format!("~/{}", rest.display()),
                Err(_) => dir.display().to_string(),
            };
            l.setStringValue(&ns(&shown));
            l.setToolTip(Some(&ns(&dir.display().to_string())));
        }
    }

    pub fn set_login(&self, on: bool) {
        if let Some(b) = self.target.ivars().login.get() {
            b.setState(if on { NSControlStateValueOn } else { NSControlStateValueOff });
        }
    }
}

pub fn login_item_enabled() -> bool {
    // SAFETY: simple class property and status read.
    unsafe { SMAppService::mainAppService().status() == SMAppServiceStatus::Enabled }
}

pub fn set_login_item(on: bool) -> Result<(), String> {
    // SAFETY: SMAppService calls with a valid main app service.
    let r = unsafe {
        let svc = SMAppService::mainAppService();
        if on { svc.registerAndReturnError() } else { svc.unregisterAndReturnError() }
    };
    r.map_err(|e| e.localizedDescription().to_string())
}
