//! The selection overlay: one borderless, non-activating panel per screen.
//! Region mode dims the screen and lets the user drag a rectangle; window mode
//! highlights the window under the pointer. Non-activating panels can take key
//! focus (for Esc) without activating snapit, so the user's app keeps focus.
//!
//! Shareable content is fetched as soon as the overlay opens, so it is usually
//! ready by the time the mouse is released.

use std::cell::{Cell, RefCell};

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSBackingStoreType, NSBezierPath, NSColor, NSCursor, NSEvent, NSFont, NSFontAttributeName,
    NSForegroundColorAttributeName, NSPanel, NSScreen, NSScreenSaverWindowLevel, NSStringDrawing,
    NSTrackingArea, NSTrackingAreaOptions, NSView, NSWindowCollectionBehavior, NSWindowStyleMask, NSWindingRule,
};
use objc2_core_foundation::CFArray;
use objc2_core_graphics::{CGWindowListCopyWindowInfo, CGWindowListOption, kCGNullWindowID};
use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSPoint, NSRect, NSSize, NSString};
use objc2_screen_capture_kit::SCShareableContent;

use super::capture::{self, Image};
use super::ns;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Region,
    Window,
}

pub enum Outcome {
    Cancelled,
    Captured(Result<(Image, f64), String>),
}

type Done = Box<dyn FnOnce(Outcome)>;
type CaptureDone = Box<dyn FnOnce(Result<(Image, f64), String>)>;
type Job = Box<dyn FnOnce(&SCShareableContent, CaptureDone)>;
type Content = Result<Retained<SCShareableContent>, String>;

struct Win {
    id: u32,
    frame: NSRect, // Cocoa global coordinates
}

struct Session {
    generation: u64,
    mode: Mode,
    panels: Vec<(Retained<OverlayPanel>, Retained<OverlayView>)>,
    windows: Vec<Win>, // front to back
    hover: Option<usize>,
    content: Option<Content>,
    pending: Option<Job>,
    done: Option<Done>,
}

thread_local! {
    static SESSION: RefCell<Option<Session>> = const { RefCell::new(None) };
    static GEN: Cell<u64> = const { Cell::new(0) };
}

define_class!(
    // SAFETY: NSPanel has no subclassing requirements; no Drop impl.
    #[unsafe(super(NSPanel, objc2_app_kit::NSWindow, objc2_app_kit::NSResponder, objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    pub struct OverlayPanel;

    impl OverlayPanel {
        // Borderless panels refuse key status by default; we need it for Esc.
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key(&self) -> bool {
            true
        }
    }
);

pub struct ViewIvars {
    origin: NSPoint, // the screen's origin, Cocoa global coordinates
    height: f64,
    display_id: u32,
    scale: f64,
    mode: Mode,
    start: Cell<Option<NSPoint>>,
    current: Cell<NSPoint>,
}

define_class!(
    // SAFETY: NSView has no subclassing requirements; no Drop impl.
    #[unsafe(super(NSView, objc2_app_kit::NSResponder, objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = ViewIvars]
    pub struct OverlayView;

    impl OverlayView {
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            self.draw();
        }

        #[unsafe(method(cursorUpdate:))]
        fn cursor_update(&self, _event: &NSEvent) {
            self.set_cursor();
        }

        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, event: &NSEvent) {
            self.set_cursor();
            if self.ivars().mode == Mode::Window {
                hover_at(self.global(event));
            }
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            let p = self.local(event);
            match self.ivars().mode {
                Mode::Region => {
                    self.ivars().start.set(Some(p));
                    self.ivars().current.set(p);
                    self.setNeedsDisplay(true);
                }
                Mode::Window => {
                    hover_at(self.global(event));
                    finish_window();
                }
            }
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            if self.ivars().start.get().is_some() {
                self.ivars().current.set(self.local(event));
                self.setNeedsDisplay(true);
            }
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            if self.ivars().mode != Mode::Region {
                return;
            }
            self.ivars().current.set(self.local(event));
            let Some(sel) = self.selection() else { return };
            self.ivars().start.set(None);
            // A click or a sliver is a cancel, not an empty image.
            if sel.size.width < 2.0 || sel.size.height < 2.0 {
                return cancel();
            }
            let iv = self.ivars();
            // SCK wants points relative to the display with a top-left origin.
            let rect = capture::rect(sel.origin.x, iv.height - sel.origin.y - sel.size.height, sel.size.width, sel.size.height);
            let display_id = iv.display_id;
            finish(Box::new(move |content, done| capture::capture_region(content, display_id, rect, done)));
        }

        #[unsafe(method(rightMouseDown:))]
        fn right_mouse_down(&self, _event: &NSEvent) {
            cancel();
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            if event.keyCode() == 53 {
                cancel();
            }
        }

        #[unsafe(method(cancelOperation:))]
        fn cancel_operation(&self, _sender: Option<&AnyObject>) {
            cancel();
        }
    }
);

impl OverlayView {
    fn new(mtm: MainThreadMarker, size: NSSize, ivars: ViewIvars) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ivars);
        let frame = NSRect::new(NSPoint::new(0.0, 0.0), size);
        // SAFETY: NSView's designated initializer.
        let view: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };
        // SAFETY: the view owns the tracking area, so the owner outlives it.
        let area = unsafe { NSTrackingArea::initWithRect_options_owner_userInfo(
            NSTrackingArea::alloc(),
            frame,
            NSTrackingAreaOptions::MouseMoved
                | NSTrackingAreaOptions::CursorUpdate
                | NSTrackingAreaOptions::ActiveAlways
                | NSTrackingAreaOptions::InVisibleRect,
            Some(&view),
            None,
        ) };
        view.addTrackingArea(&area);
        view
    }

    /// Pointer position in view coordinates, clamped to this screen so a drag
    /// past the edge selects up to the edge.
    fn local(&self, event: &NSEvent) -> NSPoint {
        let p = event.locationInWindow();
        let b = self.bounds().size;
        NSPoint::new(p.x.clamp(0.0, b.width), p.y.clamp(0.0, b.height))
    }

    fn global(&self, event: &NSEvent) -> NSPoint {
        let p = event.locationInWindow();
        NSPoint::new(p.x + self.ivars().origin.x, p.y + self.ivars().origin.y)
    }

    fn selection(&self) -> Option<NSRect> {
        let a = self.ivars().start.get()?;
        let b = self.ivars().current.get();
        Some(NSRect::new(
            NSPoint::new(a.x.min(b.x).floor(), a.y.min(b.y).floor()),
            NSSize::new((a.x - b.x).abs().round(), (a.y - b.y).abs().round()),
        ))
    }

    fn set_cursor(&self) {
        match self.ivars().mode {
            Mode::Region => NSCursor::crosshairCursor().set(),
            Mode::Window => NSCursor::pointingHandCursor().set(),
        }
    }

    fn draw(&self) {
        let bounds = self.bounds();
        match self.ivars().mode {
            Mode::Region => {
                let path = NSBezierPath::bezierPathWithRect(bounds);
                let sel = self.selection();
                if let Some(sel) = sel {
                    path.appendBezierPathWithRect(sel);
                    path.setWindingRule(NSWindingRule::EvenOdd);
                }
                NSColor::colorWithWhite_alpha(0.0, 0.3).setFill();
                path.fill();
                if let Some(sel) = sel {
                    NSColor::whiteColor().setStroke();
                    let border = NSBezierPath::bezierPathWithRect(sel.inset(-0.5));
                    border.setLineWidth(1.0);
                    border.stroke();
                    let s = self.ivars().scale;
                    let label = format!("{} × {}", (sel.size.width * s).round(), (sel.size.height * s).round());
                    draw_label(&label, sel, bounds);
                }
            }
            Mode::Window => {
                let o = self.ivars().origin;
                let hover = SESSION.with_borrow(|s| {
                    let s = s.as_ref()?;
                    s.hover.map(|i| s.windows[i].frame)
                });
                if let Some(f) = hover {
                    let r = NSRect::new(NSPoint::new(f.origin.x - o.x, f.origin.y - o.y), f.size);
                    NSColor::systemBlueColor().colorWithAlphaComponent(0.25).setFill();
                    NSBezierPath::fillRect(r);
                    NSColor::systemBlueColor().setStroke();
                    let p = NSBezierPath::bezierPathWithRect(r.inset(1.0));
                    p.setLineWidth(2.0);
                    p.stroke();
                }
            }
        }
    }
}

trait Inset {
    fn inset(self, d: f64) -> NSRect;
}
impl Inset for NSRect {
    fn inset(self, d: f64) -> NSRect {
        NSRect::new(
            NSPoint::new(self.origin.x + d, self.origin.y + d),
            NSSize::new((self.size.width - 2.0 * d).max(0.0), (self.size.height - 2.0 * d).max(0.0)),
        )
    }
}

/// The live W × H pill, below the selection (or inside it near the screen edge).
fn draw_label(text: &str, sel: NSRect, bounds: NSRect) {
    let font = NSFont::monospacedDigitSystemFontOfSize_weight(12.0, 0.3);
    let color = NSColor::whiteColor();
    // SAFETY: attribute keys are valid statics; values have the expected types.
    let attrs = unsafe {
        NSDictionary::<NSString, AnyObject>::from_slices(
            &[NSFontAttributeName, NSForegroundColorAttributeName],
            &[&*font as &AnyObject, &*color as &AnyObject],
        )
    };
    let s = ns(text);
    // SAFETY: valid attributes dictionary.
    let size = unsafe { s.sizeWithAttributes(Some(&attrs)) };
    let (pw, ph) = (size.width + 12.0, size.height + 6.0);
    let mut x = sel.origin.x;
    let mut y = sel.origin.y - ph - 6.0;
    if y < bounds.origin.y + 4.0 {
        y = sel.origin.y + 6.0;
        x += 6.0;
    }
    x = x.min(bounds.size.width - pw - 4.0).max(4.0);
    let pill = NSRect::new(NSPoint::new(x, y), NSSize::new(pw, ph));
    NSColor::colorWithWhite_alpha(0.0, 0.75).setFill();
    NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(pill, 5.0, 5.0).fill();
    // SAFETY: valid attributes dictionary.
    unsafe { s.drawAtPoint_withAttributes(NSPoint::new(x + 6.0, y + 3.0), Some(&attrs)) };
}

fn display_id(screen: &NSScreen) -> u32 {
    screen
        .deviceDescription()
        .objectForKey(&ns("NSScreenNumber"))
        .and_then(|o| o.downcast::<NSNumber>().ok())
        .map_or(0, |n| n.unsignedIntValue())
}

/// On-screen, normal-layer windows of other apps, front to back, in Cocoa
/// global coordinates.
fn list_windows(primary_height: f64) -> Vec<Win> {
    let opts = CGWindowListOption::OptionOnScreenOnly | CGWindowListOption::ExcludeDesktopElements;
    let Some(info) = CGWindowListCopyWindowInfo(opts, kCGNullWindowID) else { return Vec::new() };
    // SAFETY: CFArray of CFDictionary is toll-free bridged to NSArray of NSDictionary.
    let arr: &NSArray<NSDictionary<NSString, AnyObject>> =
        unsafe { &*(&*info as *const CFArray as *const NSArray<NSDictionary<NSString, AnyObject>>) };
    let me = std::process::id() as i64;
    let num = |d: &NSDictionary<NSString, AnyObject>, k: &str| {
        d.objectForKey(&ns(k)).and_then(|o| o.downcast::<NSNumber>().ok()).map(|n| n.doubleValue())
    };
    arr.iter()
        .filter_map(|d| {
            if num(&d, "kCGWindowLayer")? != 0.0 || num(&d, "kCGWindowOwnerPID")? as i64 == me {
                return None;
            }
            if num(&d, "kCGWindowAlpha").unwrap_or(1.0) <= 0.0 {
                return None;
            }
            let b = d.objectForKey(&ns("kCGWindowBounds"))?.downcast::<NSDictionary>().ok()?;
            // SAFETY: the bounds dictionary has string keys and number values.
            let b: Retained<NSDictionary<NSString, AnyObject>> = unsafe { Retained::cast_unchecked(b) };
            let (x, y, w, h) = (num(&b, "X")?, num(&b, "Y")?, num(&b, "Width")?, num(&b, "Height")?);
            if w < 20.0 || h < 20.0 {
                return None;
            }
            let id = num(&d, "kCGWindowNumber")? as u32;
            Some(Win { id, frame: NSRect::new(NSPoint::new(x, primary_height - y - h), NSSize::new(w, h)) })
        })
        .collect()
}

/// Opens the overlay. `done` runs exactly once: on cancel, or with the capture.
pub fn begin(mtm: MainThreadMarker, mode: Mode, done: Done) {
    cancel();
    let generation = GEN.get() + 1;
    GEN.set(generation);

    let screens = NSScreen::screens(mtm);
    let primary_height = screens.firstObject().map_or(0.0, |s| s.frame().size.height);
    let mouse = NSEvent::mouseLocation();
    let mut panels = Vec::new();
    let mut key = None;
    for screen in screens.iter() {
        let frame = screen.frame();
        // SAFETY: NSWindow's designated initializer on our NSPanel subclass.
        let panel: Retained<OverlayPanel> = unsafe {
            msg_send![OverlayPanel::alloc(mtm),
                initWithContentRect: frame,
                styleMask: NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
                backing: NSBackingStoreType::Buffered,
                defer: false]
        };
        // SAFETY: we own the panel through a Retained and never call close.
        unsafe { panel.setReleasedWhenClosed(false) };
        panel.setLevel(NSScreenSaverWindowLevel);
        panel.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::FullScreenAuxiliary
                | NSWindowCollectionBehavior::Stationary
                | NSWindowCollectionBehavior::IgnoresCycle,
        );
        panel.setOpaque(false);
        panel.setBackgroundColor(Some(&NSColor::clearColor()));
        panel.setHasShadow(false);
        panel.setHidesOnDeactivate(false);
        panel.setAcceptsMouseMovedEvents(true);
        let view = OverlayView::new(
            mtm,
            frame.size,
            ViewIvars {
                origin: frame.origin,
                height: frame.size.height,
                display_id: display_id(&screen),
                scale: screen.backingScaleFactor(),
                mode,
                start: Cell::new(None),
                current: Cell::new(NSPoint::new(0.0, 0.0)),
            },
        );
        panel.setContentView(Some(&view));
        panel.makeFirstResponder(Some(&view));
        let contains = mouse.x >= frame.origin.x
            && mouse.x < frame.origin.x + frame.size.width
            && mouse.y >= frame.origin.y
            && mouse.y < frame.origin.y + frame.size.height;
        if contains {
            key = Some(panels.len());
        }
        panels.push((panel, view));
    }
    for (i, (panel, _)) in panels.iter().enumerate() {
        if Some(i) == key {
            panel.makeKeyAndOrderFront(None);
        }
        panel.orderFrontRegardless();
    }

    let windows = if mode == Mode::Window { list_windows(primary_height) } else { Vec::new() };
    SESSION.set(Some(Session { generation, mode, panels, windows, hover: None, content: None, pending: None, done: Some(done) }));
    if mode == Mode::Window {
        hover_at(mouse);
    }
    capture::fetch_content(move |res| on_content(generation, res));
}

fn close_panels(s: &mut Session) {
    let panels = std::mem::take(&mut s.panels);
    for (panel, _) in &panels {
        panel.orderOut(None);
    }
    // Usually called from one of these views' own mouse handlers: release
    // them after the event finishes.
    let panels = super::MainOnly(panels);
    super::on_main(move |_| drop(panels));
}

fn hover_at(p: NSPoint) {
    SESSION.with_borrow_mut(|s| {
        let Some(s) = s.as_mut().filter(|s| s.mode == Mode::Window) else { return };
        let hit = s.windows.iter().position(|w| {
            let f = w.frame;
            p.x >= f.origin.x && p.x < f.origin.x + f.size.width && p.y >= f.origin.y && p.y < f.origin.y + f.size.height
        });
        if hit != s.hover {
            s.hover = hit;
            for (_, view) in &s.panels {
                view.setNeedsDisplay(true);
            }
        }
    });
}

fn finish_window() {
    let Some(id) = SESSION.with_borrow(|s| s.as_ref().and_then(|s| s.hover.map(|i| s.windows[i].id))) else {
        return;
    };
    finish(Box::new(move |content, done| capture::capture_window(content, id, done)));
}

/// Closes the panels and runs `job` once shareable content is available.
fn finish(job: Job) {
    let run = SESSION.with_borrow_mut(|slot| {
        let s = slot.as_mut()?;
        close_panels(s);
        match s.content.take() {
            Some(content) => {
                let done = s.done.take();
                *slot = None;
                Some((content, job, done))
            }
            None => {
                s.pending = Some(job);
                None
            }
        }
    });
    if let Some((content, job, Some(done))) = run {
        run_job(content, job, done);
    }
}

fn on_content(generation: u64, res: Content) {
    let mut res = Some(res);
    let run = SESSION.with_borrow_mut(|slot| {
        let s = slot.as_mut().filter(|s| s.generation == generation)?;
        match s.pending.take() {
            Some(job) => {
                let done = s.done.take();
                *slot = None;
                Some((res.take().unwrap(), job, done))
            }
            None => {
                s.content = res.take();
                None
            }
        }
    });
    if let Some((content, job, Some(done))) = run {
        run_job(content, job, done);
    }
}

fn run_job(content: Content, job: Job, done: Done) {
    match content {
        Ok(c) => job(&c, Box::new(move |r| done(Outcome::Captured(r)))),
        Err(e) => done(Outcome::Captured(Err(e))),
    }
}

/// Closes the overlay without capturing. Safe to call when it isn't open.
pub fn cancel() {
    let done = SESSION.with_borrow_mut(|slot| {
        let mut s = slot.take()?;
        close_panels(&mut s);
        s.done.take()
    });
    if let Some(done) = done {
        done(Outcome::Cancelled);
    }
}
