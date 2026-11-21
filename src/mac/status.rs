//! Menu bar item, its menu, capture feedback, and the hidden main menu that
//! gives the editor its ⌘Z / ⌘C / ⌘S key equivalents.

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, Sel};
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSBezierPath, NSColor, NSCompositingOperation, NSEventModifierFlags, NSGraphicsContext, NSImage,
    NSLineCapStyle, NSMenu, NSMenuItem, NSSound, NSStatusBar, NSStatusItem, NSVariableStatusItemLength,
};
use objc2_foundation::{NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, ns_string};

use super::ns;
use crate::app::{self, MenuCommand};

define_class!(
    // SAFETY: NSObject has no subclassing requirements; no Drop impl.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    pub struct MenuTarget;

    unsafe impl NSObjectProtocol for MenuTarget {}

    impl MenuTarget {
        #[unsafe(method(showPreferences:))]
        fn show_preferences(&self, _sender: Option<&AnyObject>) {
            app::on_menu(MenuCommand::Preferences);
        }

        #[unsafe(method(openSaveFolder:))]
        fn open_save_folder(&self, _sender: Option<&AnyObject>) {
            app::on_menu(MenuCommand::OpenFolder);
        }

        #[unsafe(method(showAbout:))]
        fn show_about(&self, _sender: Option<&AnyObject>) {
            app::on_menu(MenuCommand::About);
        }

        #[unsafe(method(restoreIcon:))]
        fn restore_icon(&self, _sender: Option<&AnyObject>) {
            app::with(|a| a.status.set_icon(&a.status.icon));
        }
    }
);

const DONE_ICON: &str = "checkmark.circle.fill";

pub struct Status {
    item: Retained<NSStatusItem>,
    target: Retained<MenuTarget>,
    icon: Retained<NSImage>,
    done_icon: Retained<NSImage>,
}

/// The app's six-blade aperture as an 18 pt template image (same geometry as
/// scripts/make-icon.swift, with wider gaps so they survive at menu bar size).
fn aperture_icon() -> Retained<NSImage> {
    let block = RcBlock::new(|r: NSRect| -> Bool {
        let (cx, cy) = (r.origin.x + r.size.width / 2.0, r.origin.y + r.size.height / 2.0);
        let big_r = r.size.width / 2.0 - 1.0;
        let (hex_r, gap) = (big_r * 0.42, 1.25);
        let v: Vec<NSPoint> = (0..6)
            .map(|k| {
                let a = std::f64::consts::FRAC_PI_2 + k as f64 * std::f64::consts::FRAC_PI_3;
                NSPoint::new(cx + hex_r * a.cos(), cy + hex_r * a.sin())
            })
            .collect();
        NSColor::blackColor().setFill();
        NSBezierPath::bezierPathWithOvalInRect(NSRect::new(
            NSPoint::new(cx - big_r, cy - big_r),
            NSSize::new(2.0 * big_r, 2.0 * big_r),
        ))
        .fill();
        let Some(ctx) = NSGraphicsContext::currentContext() else { return Bool::YES };
        ctx.setCompositingOperation(NSCompositingOperation::Clear);
        // Each blade edge runs along one hexagon side and on out past the rim.
        for k in 0..6 {
            let (a, b) = (v[k], v[(k + 1) % 6]);
            let len = (b.x - a.x).hypot(b.y - a.y);
            let (dx, dy) = ((b.x - a.x) / len, (b.y - a.y) / len);
            let line = NSBezierPath::bezierPath();
            line.moveToPoint(a);
            line.lineToPoint(NSPoint::new(a.x + dx * 3.0 * big_r, a.y + dy * 3.0 * big_r));
            line.setLineWidth(gap);
            line.setLineCapStyle(NSLineCapStyle::Butt);
            line.stroke();
        }
        let hex = NSBezierPath::bezierPath();
        hex.moveToPoint(v[0]);
        for p in &v[1..] {
            hex.lineToPoint(*p);
        }
        hex.closePath();
        hex.fill();
        Bool::YES
    });
    let img = NSImage::imageWithSize_flipped_drawingHandler(NSSize::new(18.0, 18.0), false, &block);
    img.setTemplate(true);
    img.setAccessibilityDescription(Some(ns_string!("snapit")));
    img
}

fn menu_item(mtm: MainThreadMarker, title: &str, action: Option<Sel>, key: &str) -> Retained<NSMenuItem> {
    // SAFETY: the action selectors exist on the target or responder chain.
    unsafe { NSMenuItem::initWithTitle_action_keyEquivalent(NSMenuItem::alloc(mtm), &ns(title), action, &ns(key)) }
}

impl Status {
    pub fn new(mtm: MainThreadMarker) -> Status {
        let item = NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
        // SAFETY: plain NSObject subclass init.
        let target: Retained<MenuTarget> = unsafe { msg_send![MenuTarget::alloc(mtm), init] };

        let menu = NSMenu::new(mtm);
        for (title, action, key) in [
            ("Preferences…", Some(sel!(showPreferences:)), ","),
            ("Open Save Folder", Some(sel!(openSaveFolder:)), ""),
            ("About snapit", Some(sel!(showAbout:)), ""),
        ] {
            let mi = menu_item(mtm, title, action, key);
            // SAFETY: target outlives the menu (both owned by Status).
            unsafe { mi.setTarget(Some(&target)) };
            menu.addItem(&mi);
        }
        menu.addItem(&NSMenuItem::separatorItem(mtm));
        menu.addItem(&menu_item(mtm, "Quit snapit", Some(sel!(terminate:)), "q"));
        item.setMenu(Some(&menu));

        let done_icon = NSImage::imageWithSystemSymbolName_accessibilityDescription(ns_string!(DONE_ICON), Some(ns_string!("Captured")))
            .unwrap_or_default();
        done_icon.setTemplate(true);
        let s = Status { item, target, icon: aperture_icon(), done_icon };
        s.set_icon(&s.icon);
        install_main_menu(mtm);
        s
    }

    fn set_icon(&self, img: &NSImage) {
        let mtm = MainThreadMarker::from(&*self.target);
        if let Some(button) = self.item.button(mtm) {
            button.setImage(Some(img));
        }
    }

    /// Hover text on the menu bar icon listing the bound capture shortcuts.
    pub fn set_tooltip(&self, text: &str) {
        let mtm = MainThreadMarker::from(&*self.target);
        if let Some(button) = self.item.button(mtm) {
            button.setToolTip(Some(&ns(text)));
        }
    }

    /// Brief confirmation after a capture: a checkmark in the menu bar and a soft sound.
    pub fn flash(&self) {
        self.set_icon(&self.done_icon);
        if let Some(sound) = NSSound::soundNamed(ns_string!("Tink")) {
            sound.play();
        }
        // SAFETY: restoreIcon: exists on MenuTarget and takes an object argument.
        let _: () = unsafe {
            msg_send![&*self.target, performSelector: sel!(restoreIcon:), withObject: Option::<&AnyObject>::None, afterDelay: 0.7f64]
        };
    }
}

/// An accessory app shows no menu bar, but its main menu still supplies key
/// equivalents to the key window, which routes them down the responder chain.
fn install_main_menu(mtm: MainThreadMarker) {
    let main = NSMenu::new(mtm);
    let add_submenu = |title: &str, items: &[(&str, Sel, &str, bool)]| {
        let sub = NSMenu::initWithTitle(NSMenu::alloc(mtm), &ns(title));
        for &(t, action, key, shift) in items {
            let mi = menu_item(mtm, t, Some(action), key);
            if shift {
                mi.setKeyEquivalentModifierMask(NSEventModifierFlags::Command | NSEventModifierFlags::Shift);
            }
            sub.addItem(&mi);
        }
        let holder = menu_item(mtm, title, None, "");
        holder.setSubmenu(Some(&sub));
        main.addItem(&holder);
    };
    add_submenu("snapit", &[("Quit snapit", sel!(terminate:), "q", false)]);
    add_submenu(
        "File",
        &[("Save", sel!(saveDocument:), "s", false), ("Close", sel!(performClose:), "w", false)],
    );
    add_submenu(
        "Edit",
        &[
            ("Undo", sel!(undo:), "z", false),
            ("Redo", sel!(redo:), "z", true),
            ("Cut", sel!(cut:), "x", false),
            ("Copy", sel!(copy:), "c", false),
            ("Paste", sel!(paste:), "v", false),
            ("Select All", sel!(selectAll:), "a", false),
        ],
    );
    NSApplication::sharedApplication(mtm).setMainMenu(Some(&main));
}
