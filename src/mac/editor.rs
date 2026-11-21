//! The annotation editor: a window with a unified toolbar and a canvas view. The
//! canvas forwards input to `app::editor_event` (which updates the
//! `annotate::Doc`) and draws whatever item list it was last given.

use std::cell::{Cell, OnceCell, RefCell};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, ProtocolObject, Sel};
use objc2::{AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAffineTransformNSAppKitAdditions, NSAutoresizingMaskOptions, NSBackingStoreType, NSBezierPath, NSButton, NSColor,
    NSCompositingOperation, NSControlTextEditingDelegate, NSEvent, NSGraphicsContext, NSImage, NSScreen,
    NSSegmentSwitchTracking, NSSegmentedControl, NSShadow, NSTextField, NSTextFieldDelegate, NSTextView, NSToolbar,
    NSToolbarDelegate, NSToolbarDisplayMode, NSToolbarFlexibleSpaceItemIdentifier, NSToolbarItem, NSView, NSWindow,
    NSWindowCollectionBehavior, NSWindowDelegate, NSWindowStyleMask, NSWindowTitleVisibility, NSWindowToolbarStyle,
};
use objc2_core_foundation::{CFRetained, CGRect};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGBitmapContextCreateImage, CGColorSpace, CGContext, CGImage, CGImageAlphaInfo,
    kCGColorSpaceSRGB,
};
use objc2_foundation::{
    NSAffineTransform, NSArray, NSNotification, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString, ns_string,
};

use super::capture::{self, Image};
use super::{activate, ns, render};
use crate::app::{self, EditorEvent};
use crate::core::annotate::{Effect, Item, NUM_COLORS, NUM_SIZES, Point, Tool};

/// Height of the unified title bar + toolbar.
const TOOLBAR_H: f64 = 52.0;
const MIN_W: f64 = 560.0;
const SIZE_NAMES: [&str; NUM_SIZES] = ["Small", "Medium", "Large"];
const TOOL_SYMBOLS: [(&str, &str); 7] = [
    ("cursorarrow", "Select (V)"),
    ("arrow.up.right", "Arrow (A)"),
    ("rectangle", "Rectangle (R)"),
    ("circle", "Ellipse (O)"),
    ("line.diagonal", "Line (L)"),
    ("textformat", "Text (T)"),
    ("1.circle", "Numbered step (N)"),
];

pub struct CanvasIvars {
    image: Retained<NSImage>,
    img_w: f64,
    img_h: f64,
    scale: f64,
    items: RefCell<Vec<Item>>,
    field: RefCell<Option<(Retained<NSTextField>, u32, Point)>>,
    cancel_text: Cell<bool>,
    tools: OnceCell<Retained<NSSegmentedControl>>,
    colors: OnceCell<Retained<NSSegmentedControl>>,
    sizes: OnceCell<Retained<NSSegmentedControl>>,
    toolbar_items: OnceCell<Vec<Retained<NSToolbarItem>>>,
}

define_class!(
    // SAFETY: NSView has no subclassing requirements; no Drop impl.
    #[unsafe(super(NSView, objc2_app_kit::NSResponder, objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = CanvasIvars]
    pub struct Canvas;

    unsafe impl NSObjectProtocol for Canvas {}

    impl Canvas {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _e: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            self.draw();
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, e: &NSEvent) {
            app::editor_event(EditorEvent::Press(self.image_point(e), e.clickCount() as usize));
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, e: &NSEvent) {
            app::editor_event(EditorEvent::Drag(self.image_point(e)));
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, e: &NSEvent) {
            app::editor_event(EditorEvent::Release(self.image_point(e)));
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, e: &NSEvent) {
            let ev = match e.keyCode() {
                51 | 117 => Some(EditorEvent::Delete),
                53 => Some(EditorEvent::Deselect),
                _ => e.charactersIgnoringModifiers().and_then(|c| match c.to_string().as_str() {
                    "v" => Some(EditorEvent::Tool(Tool::Select)),
                    "a" => Some(EditorEvent::Tool(Tool::Arrow)),
                    "r" => Some(EditorEvent::Tool(Tool::Rect)),
                    "o" => Some(EditorEvent::Tool(Tool::Ellipse)),
                    "l" => Some(EditorEvent::Tool(Tool::Line)),
                    "t" => Some(EditorEvent::Tool(Tool::Text)),
                    "n" => Some(EditorEvent::Tool(Tool::Step)),
                    "[" => self.size_step(-1),
                    "]" => self.size_step(1),
                    k => k.parse::<usize>().ok().filter(|n| (1..=NUM_COLORS).contains(n)).map(|n| EditorEvent::Color(n - 1)),
                }),
            };
            match ev {
                Some(ev) => app::editor_event(ev),
                // SAFETY: forwarding to NSView's implementation (beeps).
                None => unsafe { msg_send![super(self), keyDown: e] },
            }
        }

        // Responder-chain actions from the (hidden) main menu.
        #[unsafe(method(undo:))]
        fn undo(&self, _s: Option<&AnyObject>) {
            app::editor_event(EditorEvent::Undo);
        }

        #[unsafe(method(redo:))]
        fn redo(&self, _s: Option<&AnyObject>) {
            app::editor_event(EditorEvent::Redo);
        }

        #[unsafe(method(copy:))]
        fn copy(&self, _s: Option<&AnyObject>) {
            self.end_text_editing();
            app::editor_event(EditorEvent::Copy);
        }

        #[unsafe(method(saveDocument:))]
        fn save_document(&self, _s: Option<&AnyObject>) {
            self.end_text_editing();
            app::editor_event(EditorEvent::Save);
        }

        #[unsafe(method(delete:))]
        fn delete(&self, _s: Option<&AnyObject>) {
            app::editor_event(EditorEvent::Delete);
        }

        // Toolbar actions.
        #[unsafe(method(toolChanged:))]
        fn tool_changed(&self, s: &NSSegmentedControl) {
            if let Some(&t) = Tool::ALL.get(s.selectedSegment() as usize) {
                app::editor_event(EditorEvent::Tool(t));
            }
        }

        #[unsafe(method(colorChanged:))]
        fn color_changed(&self, s: &NSSegmentedControl) {
            app::editor_event(EditorEvent::Color(s.selectedSegment().max(0) as usize));
        }

        #[unsafe(method(sizeChanged:))]
        fn size_changed(&self, s: &NSSegmentedControl) {
            app::editor_event(EditorEvent::Size(s.selectedSegment().max(0) as usize));
        }

        #[unsafe(method(copyImage:))]
        fn copy_image(&self, _s: Option<&AnyObject>) {
            self.end_text_editing();
            app::editor_event(EditorEvent::Copy);
        }

        #[unsafe(method(saveImage:))]
        fn save_image(&self, _s: Option<&AnyObject>) {
            self.end_text_editing();
            app::editor_event(EditorEvent::Save);
        }

        #[unsafe(method(done:))]
        fn done(&self, _s: Option<&AnyObject>) {
            self.end_text_editing();
            app::editor_event(EditorEvent::Done);
        }
    }

    unsafe impl NSWindowDelegate for Canvas {
        #[unsafe(method(windowShouldClose:))]
        fn window_should_close(&self, _w: &NSWindow) -> bool {
            self.end_text_editing();
            app::editor_should_close()
        }

        #[unsafe(method(windowWillClose:))]
        fn window_will_close(&self, n: &NSNotification) {
            if let Some(w) = n.object().and_then(|o| o.downcast::<NSWindow>().ok()) {
                w.setDelegate(None);
            }
            // Release the window after AppKit has finished closing it.
            super::on_main(|_| app::editor_closed());
        }
    }

    unsafe impl NSToolbarDelegate for Canvas {
        #[unsafe(method_id(toolbar:itemForItemIdentifier:willBeInsertedIntoToolbar:))]
        fn toolbar_item(&self, _t: &NSToolbar, id: &NSString, _insert: bool) -> Option<Retained<NSToolbarItem>> {
            self.ivars().toolbar_items.get().and_then(|v| v.iter().find(|i| &*i.itemIdentifier() == id).cloned())
        }

        #[unsafe(method_id(toolbarDefaultItemIdentifiers:))]
        fn toolbar_default(&self, _t: &NSToolbar) -> Retained<NSArray<NSString>> {
            self.toolbar_ids()
        }

        #[unsafe(method_id(toolbarAllowedItemIdentifiers:))]
        fn toolbar_allowed(&self, _t: &NSToolbar) -> Retained<NSArray<NSString>> {
            self.toolbar_ids()
        }
    }

    unsafe impl NSControlTextEditingDelegate for Canvas {
        #[unsafe(method(controlTextDidChange:))]
        fn control_text_did_change(&self, _n: &NSNotification) {
            if let Some((f, ..)) = &*self.ivars().field.borrow() {
                fit_field(f);
            }
        }

        #[unsafe(method(controlTextDidEndEditing:))]
        fn control_text_did_end_editing(&self, _n: &NSNotification) {
            let Some((field, id, at)) = self.ivars().field.take() else { return };
            let text = field.stringValue().to_string();
            field.removeFromSuperview();
            let ev = if self.ivars().cancel_text.replace(false) {
                EditorEvent::TextCancel
            } else {
                EditorEvent::TextCommit(id, at, text)
            };
            app::editor_event(ev);
        }

        #[unsafe(method(control:textView:doCommandBySelector:))]
        fn do_command(&self, _c: &objc2_app_kit::NSControl, _tv: &NSTextView, cmd: Sel) -> Bool {
            if cmd == sel!(cancelOperation:) {
                self.ivars().cancel_text.set(true);
            } else if cmd != sel!(insertNewline:) {
                return Bool::NO;
            }
            self.end_text_editing();
            Bool::YES
        }
    }

    unsafe impl NSTextFieldDelegate for Canvas {}
);

fn fit_field(f: &NSTextField) {
    f.sizeToFit();
    let mut fr = f.frame();
    fr.size.width = fr.size.width.max(60.0) + 8.0;
    f.setFrame(fr);
}

impl Canvas {
    /// Our items with a flexible space before the export buttons.
    fn toolbar_ids(&self) -> Retained<NSArray<NSString>> {
        let mut ids: Vec<Retained<NSString>> =
            self.ivars().toolbar_items.get().map(|v| v.iter().map(|i| i.itemIdentifier()).collect()).unwrap_or_default();
        // SAFETY: an immutable AppKit constant.
        ids.insert(ids.len().min(3), unsafe { NSToolbarFlexibleSpaceItemIdentifier }.retain());
        NSArray::from_retained_slice(&ids)
    }

    fn size_step(&self, delta: isize) -> Option<EditorEvent> {
        let cur = self.ivars().sizes.get()?.selectedSegment();
        Some(EditorEvent::Size((cur + delta).clamp(0, NUM_SIZES as isize - 1) as usize))
    }

    /// Image-to-view transform: (scale, x offset, y offset). The image is
    /// fitted to the view, never enlarged past one image point per screen point.
    fn fit(&self) -> (f64, f64, f64) {
        let iv = self.ivars();
        let b = self.bounds().size;
        let pad = 16.0;
        let s = ((b.width - 2.0 * pad) / iv.img_w)
            .min((b.height - 2.0 * pad) / iv.img_h)
            .min(1.0 / iv.scale)
            .max(0.01);
        (s, (b.width - iv.img_w * s) / 2.0, (b.height - iv.img_h * s) / 2.0)
    }

    fn image_point(&self, e: &NSEvent) -> Point {
        let p = self.convertPoint_fromView(e.locationInWindow(), None);
        let (s, ox, oy) = self.fit();
        Point { x: (p.x - ox) / s, y: (p.y - oy) / s }
    }

    fn draw(&self) {
        let iv = self.ivars();
        NSColor::underPageBackgroundColor().setFill();
        NSBezierPath::fillRect(self.bounds());

        let (s, ox, oy) = self.fit();
        let rect = NSRect::new(NSPoint::new(ox, oy), NSSize::new(iv.img_w * s, iv.img_h * s));
        let Some(ctx) = NSGraphicsContext::currentContext() else { return };
        ctx.saveGraphicsState();
        let shadow = NSShadow::new();
        shadow.setShadowBlurRadius(14.0);
        shadow.setShadowOffset(NSSize::new(0.0, -3.0));
        shadow.setShadowColor(Some(&NSColor::colorWithWhite_alpha(0.0, 0.3)));
        shadow.set();
        NSColor::whiteColor().setFill();
        NSBezierPath::fillRect(rect);
        ctx.restoreGraphicsState();
        // SAFETY: no hints dictionary.
        unsafe {
            iv.image.drawInRect_fromRect_operation_fraction_respectFlipped_hints(
                rect,
                NSRect::ZERO,
                NSCompositingOperation::SourceOver,
                1.0,
                true,
                None,
            )
        };

        ctx.saveGraphicsState();
        NSBezierPath::clipRect(rect);
        let t = NSAffineTransform::transform();
        t.translateXBy_yBy(ox, oy);
        t.scaleBy(s);
        t.concat();
        render::draw(&iv.items.borrow(), 1.0 / s);
        ctx.restoreGraphicsState();
    }

    fn end_text_editing(&self) {
        if self.ivars().field.borrow().is_some()
            && let Some(w) = self.window()
        {
            w.makeFirstResponder(Some(self));
        }
    }
}

/// A handle to an open editor window.
#[derive(Clone)]
pub struct Editor {
    window: Retained<NSWindow>,
    canvas: Retained<Canvas>,
}

fn symbol(name: &str, desc: &str) -> Retained<NSImage> {
    NSImage::imageWithSystemSymbolName_accessibilityDescription(&ns(name), Some(&ns(desc)))
        .unwrap_or_default()
}

fn swatch(i: usize) -> Retained<NSImage> {
    let c = render::color(i);
    let block = RcBlock::new(move |r: NSRect| -> Bool {
        let inset = NSRect::new(NSPoint::new(r.origin.x + 1.5, r.origin.y + 1.5), NSSize::new(r.size.width - 3.0, r.size.height - 3.0));
        let p = NSBezierPath::bezierPathWithOvalInRect(inset);
        c.setFill();
        p.fill();
        NSColor::colorWithWhite_alpha(0.5, 0.6).setStroke();
        p.setLineWidth(1.0);
        p.stroke();
        Bool::YES
    });
    let img = NSImage::imageWithSize_flipped_drawingHandler(NSSize::new(16.0, 16.0), false, &block);
    img.setAccessibilityDescription(Some(&ns(render::COLOR_NAMES[i])));
    img
}

/// A template dot whose diameter grows with the size index.
fn size_dot(i: usize) -> Retained<NSImage> {
    let d = [5.0, 8.5, 12.0][i];
    let block = RcBlock::new(move |r: NSRect| -> Bool {
        let (cx, cy) = (r.origin.x + r.size.width / 2.0, r.origin.y + r.size.height / 2.0);
        NSColor::blackColor().setFill();
        NSBezierPath::bezierPathWithOvalInRect(NSRect::new(NSPoint::new(cx - d / 2.0, cy - d / 2.0), NSSize::new(d, d))).fill();
        Bool::YES
    });
    let img = NSImage::imageWithSize_flipped_drawingHandler(NSSize::new(16.0, 16.0), false, &block);
    img.setTemplate(true);
    img.setAccessibilityDescription(Some(&ns(SIZE_NAMES[i])));
    img
}

fn segmented(images: &[Retained<NSImage>], target: &Canvas, action: Sel, mtm: MainThreadMarker) -> Retained<NSSegmentedControl> {
    let refs: Vec<&NSImage> = images.iter().map(|i| &**i).collect();
    // SAFETY: target outlives the control (both live in the editor window).
    unsafe {
        NSSegmentedControl::segmentedControlWithImages_trackingMode_target_action(
            &NSArray::from_slice(&refs),
            NSSegmentSwitchTracking::SelectOne,
            Some(target),
            Some(action),
            mtm,
        )
    }
}

fn view_item(id: &str, label: &str, tip: Option<&str>, view: &NSView, mtm: MainThreadMarker) -> Retained<NSToolbarItem> {
    let item = NSToolbarItem::initWithItemIdentifier(NSToolbarItem::alloc(mtm), &ns(id));
    item.setLabel(&ns(label));
    item.setToolTip(tip.map(ns).as_deref());
    item.setView(Some(view));
    item
}

fn button_item(id: &str, label: &str, tip: &str, image: &str, target: &Canvas, action: Sel, mtm: MainThreadMarker) -> Retained<NSToolbarItem> {
    let item = NSToolbarItem::initWithItemIdentifier(NSToolbarItem::alloc(mtm), &ns(id));
    item.setLabel(&ns(label));
    item.setToolTip(Some(&ns(tip)));
    item.setImage(Some(&symbol(image, label)));
    item.setBordered(true);
    // SAFETY: target outlives the toolbar (both live in the editor window).
    unsafe {
        item.setTarget(Some(target));
        item.setAction(Some(action));
    }
    item
}

/// Opens an editor for `img` (captured at `scale` pixels per point).
pub fn open(mtm: MainThreadMarker, img: &Image, scale: f64) -> Editor {
    let px = capture::size(img);
    let (w_pt, h_pt) = (px.width / scale, px.height / scale);
    let image = NSImage::initWithCGImage_size(NSImage::alloc(), img, NSSize::new(w_pt, h_pt));
    let ivars = CanvasIvars {
        image,
        img_w: px.width,
        img_h: px.height,
        scale,
        items: RefCell::new(Vec::new()),
        field: RefCell::new(None),
        cancel_text: Cell::new(false),
        tools: OnceCell::new(),
        colors: OnceCell::new(),
        sizes: OnceCell::new(),
        toolbar_items: OnceCell::new(),
    };
    let zero = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(0.0, 0.0));
    // SAFETY: NSView's designated initializer.
    let canvas: Retained<Canvas> = unsafe { msg_send![super(Canvas::alloc(mtm).set_ivars(ivars)), initWithFrame: zero] };
    canvas.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable);

    // Toolbar controls. Every tooltip names its keyboard shortcut.
    let tool_imgs: Vec<_> = TOOL_SYMBOLS.iter().map(|(s, d)| symbol(s, d)).collect();
    let tools = segmented(&tool_imgs, &canvas, sel!(toolChanged:), mtm);
    for (i, (_, tip)) in TOOL_SYMBOLS.iter().enumerate() {
        tools.setToolTip_forSegment(Some(&ns(tip)), i as isize);
    }
    let colors = segmented(&(0..NUM_COLORS).map(swatch).collect::<Vec<_>>(), &canvas, sel!(colorChanged:), mtm);
    for (i, name) in render::COLOR_NAMES.iter().enumerate() {
        colors.setToolTip_forSegment(Some(&ns(&format!("{name} ({})", i + 1))), i as isize);
    }
    let sizes = segmented(&(0..NUM_SIZES).map(size_dot).collect::<Vec<_>>(), &canvas, sel!(sizeChanged:), mtm);
    for (i, name) in SIZE_NAMES.iter().enumerate() {
        sizes.setToolTip_forSegment(Some(&ns(&format!("{name} ([ smaller, ] larger)"))), i as isize);
    }
    // SAFETY: canvas outlives the button.
    let done = unsafe { NSButton::buttonWithTitle_target_action(ns_string!("Done"), Some(&*canvas), Some(sel!(done:)), mtm) };
    // ⌘↩, not ↩: plain Return must reach a text field being edited.
    done.setKeyEquivalent(ns_string!("\r"));
    done.setKeyEquivalentModifierMask(objc2_app_kit::NSEventModifierFlags::Command);
    done.setBezelColor(Some(&NSColor::controlAccentColor()));

    let items = vec![
        view_item("tools", "Tools", None, &tools, mtm),
        view_item("colors", "Color", None, &colors, mtm),
        view_item("sizes", "Size", None, &sizes, mtm),
        button_item("copy", "Copy", "Copy to clipboard (⌘C)", "doc.on.doc", &canvas, sel!(copyImage:), mtm),
        button_item("save", "Save", "Save to folder (⌘S)", "square.and.arrow.down", &canvas, sel!(saveImage:), mtm),
        view_item("done", "Done", Some("Copy and close (⌘↩)"), &done, mtm),
    ];
    let n_items = items.len();

    let visible = NSScreen::mainScreen(mtm).map_or(NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1440.0, 900.0)), |s| s.visibleFrame());
    let cw = (w_pt + 48.0).clamp(MIN_W, visible.size.width * 0.9);
    let ch = (h_pt + 48.0).clamp(280.0, visible.size.height * 0.85 - TOOLBAR_H);
    let content = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(cw, ch));

    // SAFETY: we keep the window alive through Retained and disable release-on-close.
    let window = unsafe {
        let w = NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            content,
            NSWindowStyleMask::Titled
                | NSWindowStyleMask::Closable
                | NSWindowStyleMask::Miniaturizable
                | NSWindowStyleMask::Resizable
                | NSWindowStyleMask::UnifiedTitleAndToolbar,
            NSBackingStoreType::Buffered,
            false,
        );
        w.setReleasedWhenClosed(false);
        w
    };
    window.setTitle(&ns(&format!("snapit — {} × {}", px.width, px.height)));
    window.setTitleVisibility(NSWindowTitleVisibility::Hidden);
    window.setCollectionBehavior(NSWindowCollectionBehavior::MoveToActiveSpace);

    let _ = canvas.ivars().toolbar_items.set(items);
    let toolbar = NSToolbar::initWithIdentifier(NSToolbar::alloc(mtm), ns_string!("snapit.editor"));
    toolbar.setDelegate(Some(ProtocolObject::from_ref(&*canvas)));
    toolbar.setDisplayMode(NSToolbarDisplayMode::IconOnly);
    window.setToolbar(Some(&toolbar));
    window.setToolbarStyle(NSWindowToolbarStyle::Unified);

    canvas.setFrame(content);
    window.setContentView(Some(&canvas));
    window.setDelegate(Some(ProtocolObject::from_ref(&*canvas)));

    // Grow until no toolbar item spills into the overflow menu. Toolbar item
    // padding varies by macOS version, so measure rather than estimate.
    let mut w = cw;
    window.layoutIfNeeded();
    while toolbar.visibleItems().map_or(0, |v| v.count()) < n_items + 1 && w < visible.size.width {
        w += 24.0;
        window.setContentSize(NSSize::new(w, ch));
        window.layoutIfNeeded();
    }
    window.setContentMinSize(NSSize::new(w, 240.0));

    let _ = canvas.ivars().tools.set(tools);
    let _ = canvas.ivars().colors.set(colors);
    let _ = canvas.ivars().sizes.set(sizes);

    window.center();
    activate(mtm);
    window.makeKeyAndOrderFront(None);
    window.makeFirstResponder(Some(&canvas));
    Editor { window, canvas }
}

impl Editor {
    /// Replaces the drawn items and syncs the toolbar.
    pub fn update(&self, items: Vec<Item>, tool: Tool, color: usize, size: usize) {
        let iv = self.canvas.ivars();
        iv.items.replace(items);
        if let Some(t) = iv.tools.get() {
            t.setSelectedSegment(Tool::ALL.iter().position(|&x| x == tool).unwrap_or(0) as isize);
        }
        if let Some(c) = iv.colors.get() {
            c.setSelectedSegment(color as isize);
        }
        if let Some(s) = iv.sizes.get() {
            s.setSelectedSegment(size as isize);
        }
        self.canvas.setNeedsDisplay(true);
    }

    /// Opens a text field for a new or existing text annotation.
    pub fn begin_text(&self, eff: &Effect, font_px: f64, color: usize) {
        let mtm = MainThreadMarker::from(&*self.canvas);
        self.canvas.end_text_editing();
        let (s, ox, oy) = self.canvas.fit();
        let origin = NSPoint::new(ox + eff.text_at.x * s - 2.0, oy + eff.text_at.y * s - 1.0);
        let field = NSTextField::initWithFrame(NSTextField::alloc(mtm), NSRect::new(origin, NSSize::new(60.0, font_px * s * 1.3)));
        field.setStringValue(&ns(&eff.text));
        field.setFont(Some(&render::font(font_px * s)));
        field.setTextColor(Some(&render::color(color)));
        field.setBezeled(false);
        field.setBordered(false);
        field.setDrawsBackground(true);
        field.setBackgroundColor(Some(&NSColor::colorWithWhite_alpha(0.5, 0.35)));
        field.setEditable(true);
        field.setPlaceholderString(Some(ns_string!("Text")));
        // SAFETY: the canvas outlives the field (the field is its subview).
        unsafe { field.setDelegate(Some(ProtocolObject::from_ref(&*self.canvas))) };
        fit_field(&field);
        self.canvas.addSubview(&field);
        self.canvas.ivars().field.replace(Some((field.clone(), eff.text_id, eff.text_at)));
        self.window.makeFirstResponder(Some(&field));
    }

    pub fn close(&self) {
        self.window.close();
    }
}

/// Renders `items` over `img` at full resolution for export.
pub fn flatten(img: &CGImage, items: &[Item]) -> Option<Image> {
    let px = capture::size(img);
    let (w, h) = (px.width as usize, px.height as usize);
    let space = CGImage::color_space(Some(img))
        .filter(|cs| CGColorSpace::number_of_components(Some(cs)) == 3)
        .or_else(|| CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB })))?;
    // SAFETY: null data lets CG allocate; parameters describe 8-bit RGBA.
    let ctx: CFRetained<CGContext> =
        unsafe { CGBitmapContextCreate(std::ptr::null_mut(), w, h, 8, 0, Some(&space), CGImageAlphaInfo::PremultipliedLast.0) }?;
    CGContext::draw_image(Some(&ctx), CGRect::new(NSPoint::new(0.0, 0.0), NSSize::new(px.width, px.height)), Some(img));
    CGContext::translate_ctm(Some(&ctx), 0.0, px.height);
    CGContext::scale_ctm(Some(&ctx), 1.0, -1.0);

    let gc = NSGraphicsContext::graphicsContextWithCGContext_flipped(&ctx, true);
    let previous = NSGraphicsContext::currentContext();
    NSGraphicsContext::setCurrentContext(Some(&gc));
    render::draw(items, 1.0);
    NSGraphicsContext::setCurrentContext(previous.as_deref());
    CGBitmapContextCreateImage(Some(&ctx))
}

/// Text measurement for the document model (image pixels).
pub fn measure(text: &str, font_px: f64) -> (f64, f64) {
    render::measure(text, font_px)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::annotate::{Doc, pt};
    use objc2_core_graphics::{CGColor, CGDataProvider, CGImage as CGI};

    /// A flat grey 400×300 test image.
    fn grey_image() -> Image {
        let space = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB })).unwrap();
        let ctx = unsafe {
            CGBitmapContextCreate(std::ptr::null_mut(), 400, 300, 8, 0, Some(&space), CGImageAlphaInfo::PremultipliedLast.0)
        }
        .unwrap();
        let grey = CGColor::new_srgb(0.5, 0.5, 0.5, 1.0);
        CGContext::set_fill_color_with_color(Some(&ctx), Some(&grey));
        CGContext::fill_rect(Some(&ctx), CGRect::new(NSPoint::new(0.0, 0.0), NSSize::new(400.0, 300.0)));
        CGBitmapContextCreateImage(Some(&ctx)).unwrap()
    }

    /// RGBA of the pixel at (x, y), top-left origin.
    fn pixel(img: &CGI, x: usize, y: usize) -> [u8; 4] {
        let provider = CGI::data_provider(Some(img)).unwrap();
        let data = CGDataProvider::data(Some(&provider)).unwrap();
        let bpr = CGI::bytes_per_row(Some(img));
        let bytes = unsafe { std::slice::from_raw_parts(data.byte_ptr(), data.length() as usize) };
        let i = y * bpr + x * 4;
        [bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]
    }

    #[test]
    fn flatten_draws_annotations_upright() {
        let img = grey_image();
        let mut d = Doc::new(400.0, 300.0, 1.0, Some(Box::new(measure)));
        d.set_tool(Tool::Rect);
        d.press(pt(20.0, 20.0), 1);
        d.release(pt(180.0, 120.0)); // red (default colour 0) rectangle, top-left area
        d.deselect();
        d.set_color(3);
        d.set_tool(Tool::Step);
        d.press(pt(300.0, 220.0), 1); // blue step marker, bottom-right area
        d.release(pt(300.0, 220.0));
        d.set_tool(Tool::Arrow);
        d.press(pt(200.0, 280.0), 1);
        d.release(pt(380.0, 160.0));
        d.set_tool(Tool::Text);
        let eff = d.press(pt(30.0, 200.0), 1);
        d.commit_text(0, eff.text_at, "snapit");
        let mut items = d.render();
        items.iter_mut().for_each(|i| i.selected = false);

        let out = flatten(&img, &items).expect("flatten");
        assert_eq!(capture::size(&out).width, 400.0);

        // Rectangle's top edge at y=20 (top-left origin) must be red: catches a flipped context.
        let top = pixel(&out, 100, 20);
        assert!(top[0] > 200 && top[1] < 120 && top[2] < 120, "rect top edge = {top:?}");
        // Untouched area stays grey.
        let bg = pixel(&out, 100, 70);
        assert!((120..=136).contains(&bg[0]) && bg[0] == bg[1], "background = {bg:?}");
        // Step marker disc near its centre (left of the number) is blue.
        let step = pixel(&out, 290, 220);
        assert!(step[2] > 200 && step[0] < 60, "step = {step:?}");

        if let Some(png) = super::super::capture::encode_png_for_test(&out) {
            let _ = std::fs::write(concat!(env!("CARGO_MANIFEST_DIR"), "/target/flatten-test.png"), png);
        }
    }
}
