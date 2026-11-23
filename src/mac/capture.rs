//! Screen Recording permission, ScreenCaptureKit screenshots, and export
//! (PNG to the pasteboard and/or a file). Pixels stay in CGImage/NSData; the
//! PNG is encoded on a background queue.

use std::cell::Cell;
use std::path::PathBuf;
use std::ptr::NonNull;
use std::sync::{Arc, Condvar, Mutex};

use block2::RcBlock;
use dispatch2::{DispatchQueue, GlobalQueueIdentifier, DispatchQueueGlobalPriority};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{AnyThread, DefinedClass, MainThreadMarker, define_class, msg_send};
use objc2_app_kit::{
    NSBitmapImageFileType, NSBitmapImageRep, NSPasteboard, NSPasteboardItem, NSPasteboardItemDataProvider, NSPasteboardType,
    NSPasteboardTypePNG, NSWorkspace,
};
use objc2_core_foundation::{CFRetained, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{CGImage, CGPreflightScreenCaptureAccess, CGRequestScreenCaptureAccess};
use objc2_foundation::{NSArray, NSData, NSDictionary, NSError, NSObject, NSObjectProtocol, NSURL};
use objc2_screen_capture_kit::{
    SCCaptureResolutionType, SCContentFilter, SCScreenshotManager, SCShareableContent, SCStreamConfiguration,
};

use super::{MainOnly, ns, on_main};

pub type Image = CFRetained<CGImage>;

pub fn has_permission() -> bool {
    CGPreflightScreenCaptureAccess()
}

/// Shows the system prompt the first time; afterwards macOS only lists the app
/// in System Settings.
pub fn request_permission() -> bool {
    CGRequestScreenCaptureAccess()
}

pub fn open_privacy_settings(mtm: MainThreadMarker) {
    let _ = mtm;
    if let Some(url) = NSURL::URLWithString(&ns(
        "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture",
    )) {
        NSWorkspace::sharedWorkspace().openURL(&url);
    }
}

fn error_text(err: *mut NSError, fallback: &str) -> String {
    // SAFETY: SCK passes either null or a valid NSError.
    match unsafe { err.as_ref() } {
        Some(e) => e.localizedDescription().to_string(),
        None => fallback.to_string(),
    }
}

/// Fetches displays/windows/apps; `done` runs on the main thread.
pub fn fetch_content(done: impl FnOnce(Result<Retained<SCShareableContent>, String>) + 'static) {
    let done = MainOnly(Cell::new(Some(Box::new(done) as Box<dyn FnOnce(_)>)));
    let block = RcBlock::new(move |content: *mut SCShareableContent, err: *mut NSError| {
        // SAFETY: a non-null content pointer is a valid object for the block's duration.
        let result = match unsafe { Retained::retain(content) } {
            Some(c) => Ok(MainOnly(c)),
            None => Err(error_text(err, "Screen content unavailable")),
        };
        let done = MainOnly(done.0.take());
        on_main(move |_| {
            let done = done;
            if let Some(f) = done.0 {
                f(result.map(|c| c.0));
            }
        });
    });
    // SAFETY: the block matches the expected signature and is retained by SCK.
    unsafe {
        SCShareableContent::getShareableContentExcludingDesktopWindows_onScreenWindowsOnly_completionHandler(
            false, true, &block,
        )
    };
}

fn screenshot(
    filter: &SCContentFilter,
    config: &SCStreamConfiguration,
    done: impl FnOnce(Result<Image, String>) + 'static,
) {
    let done = MainOnly(Cell::new(Some(Box::new(done) as Box<dyn FnOnce(_)>)));
    let block = RcBlock::new(move |img: *mut CGImage, err: *mut NSError| {
        // SAFETY: a non-null image is valid for the block's duration; we retain it.
        let result = match NonNull::new(img) {
            Some(p) => Ok(MainOnly(unsafe { CFRetained::retain(p) })),
            None => Err(error_text(err, "Capture failed")),
        };
        let done = MainOnly(done.0.take());
        on_main(move |_| {
            let done = done;
            if let Some(f) = done.0 {
                f(result.map(|i| i.0));
            }
        });
    });
    // SAFETY: valid filter/config; block signature matches.
    unsafe {
        SCScreenshotManager::captureImageWithFilter_configuration_completionHandler(filter, config, Some(&block))
    };
}

fn base_config(width: f64, height: f64) -> Retained<SCStreamConfiguration> {
    // SAFETY: plain property setters on a fresh configuration.
    unsafe {
        let c = SCStreamConfiguration::new();
        c.setWidth(width.round().max(1.0) as usize);
        c.setHeight(height.round().max(1.0) as usize);
        c.setShowsCursor(false);
        c.setCaptureResolution(SCCaptureResolutionType::Best);
        c
    }
}

/// Captures `rect` (points, top-left origin, relative to the display) of a
/// display, leaving snapit's own windows out of the image.
pub fn capture_region(
    content: &SCShareableContent,
    display_id: u32,
    rect: CGRect,
    done: impl FnOnce(Result<(Image, f64), String>) + 'static,
) {
    // SAFETY: read-only accessors on SCK objects.
    unsafe {
        let Some(display) = content.displays().iter().find(|d| d.displayID() == display_id) else {
            return done(Err("That display is no longer available".into()));
        };
        let me = std::process::id() as i32;
        let apps: Vec<_> = content.applications().iter().filter(|a| a.processID() == me).collect();
        let filter = SCContentFilter::initWithDisplay_excludingApplications_exceptingWindows(
            SCContentFilter::alloc(),
            &display,
            &NSArray::from_retained_slice(&apps),
            &NSArray::new(),
        );
        let scale = filter.pointPixelScale() as f64;
        let config = base_config(rect.size.width * scale, rect.size.height * scale);
        config.setSourceRect(rect);
        screenshot(&filter, &config, move |r| done(r.map(|i| (i, scale))));
    }
}

/// Captures a single window on its own, without its shadow.
pub fn capture_window(
    content: &SCShareableContent,
    window_id: u32,
    done: impl FnOnce(Result<(Image, f64), String>) + 'static,
) {
    // SAFETY: read-only accessors on SCK objects.
    unsafe {
        let Some(window) = content.windows().iter().find(|w| w.windowID() == window_id) else {
            return done(Err("That window is no longer available".into()));
        };
        let filter = SCContentFilter::initWithDesktopIndependentWindow(SCContentFilter::alloc(), &window);
        let scale = filter.pointPixelScale() as f64;
        let r = filter.contentRect();
        let config = base_config(r.size.width * scale, r.size.height * scale);
        config.setIgnoreShadowsSingleWindow(true);
        screenshot(&filter, &config, move |res| done(res.map(|i| (i, scale))));
    }
}

fn encode_png(img: &CGImage) -> Option<Retained<NSData>> {
    let rep = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), img);
    // SAFETY: empty properties dictionary is valid.
    unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new()) }
}

enum Png {
    Encoding,
    Ready(Retained<NSData>),
    Taken,
    Failed,
}

// SAFETY: NSData is immutable and thread-safe; the encoder hands it over once.
unsafe impl Send for Png {}

/// The PNG behind a clipboard promise. The encoder fills it in on a background
/// queue; a paste that arrives first waits for it.
pub struct PendingPng {
    png: Mutex<Png>,
    ready: Condvar,
}

impl PendingPng {
    fn new() -> Arc<PendingPng> {
        Arc::new(PendingPng { png: Mutex::new(Png::Encoding), ready: Condvar::new() })
    }

    fn fulfil(&self, png: Option<Retained<NSData>>) {
        *self.png.lock().unwrap() = png.map_or(Png::Failed, Png::Ready);
        self.ready.notify_all();
    }

    /// Waits for the encode, then hands the PNG over once. Later calls, and
    /// calls after a failed encode, get `None`.
    fn take(&self) -> Option<Retained<NSData>> {
        let mut png = self.ready.wait_while(self.png.lock().unwrap(), |p| matches!(p, Png::Encoding)).unwrap();
        match std::mem::replace(&mut *png, Png::Taken) {
            Png::Ready(data) => Some(data),
            Png::Failed => {
                *png = Png::Failed;
                None
            }
            _ => None,
        }
    }

    fn failed(&self) -> bool {
        matches!(*self.png.lock().unwrap(), Png::Failed)
    }
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements; no Drop impl.
    #[unsafe(super(NSObject))]
    #[ivars = Arc<PendingPng>]
    struct PngProvider;

    unsafe impl NSObjectProtocol for PngProvider {}

    unsafe impl NSPasteboardItemDataProvider for PngProvider {
        #[unsafe(method(pasteboard:item:provideDataForType:))]
        fn provide(&self, _pb: Option<&NSPasteboard>, item: &NSPasteboardItem, ty: &NSPasteboardType) {
            if let Some(png) = self.ivars().take() {
                item.setData_forType(&png, ty);
            }
        }
    }
);

/// Replaces the pasteboard's contents with a promise of `pending`'s PNG and
/// returns the change count it was written at. Pasting before the encode
/// finishes blocks the paste, not the capture.
fn promise_png(pb: &NSPasteboard, pending: &Arc<PendingPng>) -> isize {
    // SAFETY: NSObject's init on a freshly allocated instance with ivars set.
    let provider: Retained<PngProvider> =
        unsafe { msg_send![super(PngProvider::alloc().set_ivars(pending.clone())), init] };
    let item = NSPasteboardItem::new();
    // SAFETY: NSPasteboardTypePNG is a valid static type string.
    let types = NSArray::from_slice(&[unsafe { NSPasteboardTypePNG }]);
    item.setDataProvider_forTypes(ProtocolObject::from_ref(&*provider), &types);
    pb.clearContents();
    pb.writeObjects(&NSArray::from_retained_slice(&[ProtocolObject::from_retained(item)]));
    pb.changeCount()
}

/// Once the encode is done and nobody has pasted yet, swaps the promise for
/// the real data so snapit stops holding the PNG (and the clipboard survives
/// snapit quitting). Does nothing if the pasteboard has changed since.
fn settle(pb: &NSPasteboard, count: isize, pending: &PendingPng) {
    if pb.changeCount() != count {
        return;
    }
    if let Some(png) = pending.take() {
        pb.clearContents();
        // SAFETY: NSPasteboardTypePNG is a valid static type string.
        pb.setData_forType(Some(&png), unsafe { NSPasteboardTypePNG });
    } else if pending.failed() {
        pb.clearContents();
    }
}

/// Why an export didn't fully succeed.
#[derive(Debug)]
pub enum ExportError {
    /// The PNG couldn't be encoded: nothing reached the clipboard or the disk,
    /// and the clipboard promise has been withdrawn.
    Encode,
    /// The clipboard (if asked for) has the PNG, but writing the file failed.
    Save(String),
}

impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            ExportError::Encode => f.write_str("Couldn't encode the image as PNG"),
            ExportError::Save(e) => f.write_str(e),
        }
    }
}

/// Puts a promise of `img` as PNG on the pasteboard straight away if
/// `clipboard`, then encodes off the main thread and writes it to `path` if
/// given. `done` runs on the main thread with the outcome. The clipboard is
/// ready when this returns, so callers can give feedback then, and must report
/// an [`ExportError::Encode`] as the copy having failed.
pub fn export(img: Image, clipboard: bool, path: Option<PathBuf>, done: impl FnOnce(Result<(), ExportError>) + 'static) {
    let pending = PendingPng::new();
    let promise = clipboard.then(|| promise_png(&NSPasteboard::generalPasteboard(), &pending));
    let img = MainOnly(img);
    let done = MainOnly(done);
    DispatchQueue::global_queue(GlobalQueueIdentifier::Priority(DispatchQueueGlobalPriority::High)).exec_async(move || {
        // CGImage and NSData are immutable and thread-safe.
        let (img, done) = (img, done);
        let png = encode_png(&img.0);
        drop(img);
        // Hand the PNG to any waiting paste before touching the disk.
        pending.fulfil(png.clone());
        let saved = match (&png, &path) {
            (None, _) => Err(ExportError::Encode),
            (Some(data), Some(p)) => {
                if data.writeToFile_atomically(&ns(&p.to_string_lossy()), true) {
                    Ok(())
                } else {
                    Err(ExportError::Save(format!("Couldn't write {}", p.display())))
                }
            }
            (Some(_), None) => Ok(()),
        };
        on_main(move |_| {
            let done = done;
            if let Some(count) = promise {
                settle(&NSPasteboard::generalPasteboard(), count, &pending);
            }
            (done.0)(saved);
        });
    });
}

#[cfg(test)]
pub fn encode_png_for_test(img: &CGImage) -> Option<Vec<u8>> {
    encode_png(img).map(|d| d.to_vec())
}

/// Writes an (unfulfilled) PNG promise to `pb`: the clipboard half of an export.
#[cfg(test)]
pub fn promise_for_test(pb: &NSPasteboard) -> Arc<PendingPng> {
    let pending = PendingPng::new();
    promise_png(pb, &pending);
    pending
}

pub fn size(img: &CGImage) -> CGSize {
    CGSize::new(CGImage::width(Some(img)) as f64, CGImage::height(Some(img)) as f64)
}

pub fn rect(x: f64, y: f64, w: f64, h: f64) -> CGRect {
    CGRect::new(CGPoint::new(x, y), CGSize::new(w, h))
}

#[cfg(test)]
mod tests {
    use super::*;
    use objc2_core_graphics::{CGBitmapContextCreate, CGBitmapContextCreateImage, CGColorSpace, CGImageAlphaInfo, kCGColorSpaceSRGB};
    use std::time::{Duration, Instant};

    fn image() -> Image {
        let space = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB })).unwrap();
        let ctx = unsafe {
            CGBitmapContextCreate(std::ptr::null_mut(), 64, 48, 8, 0, Some(&space), CGImageAlphaInfo::PremultipliedLast.0)
        }
        .unwrap();
        CGBitmapContextCreateImage(Some(&ctx)).unwrap()
    }

    fn png_type() -> &'static NSPasteboardType {
        // SAFETY: NSPasteboardTypePNG is a valid static type string.
        unsafe { NSPasteboardTypePNG }
    }

    #[test]
    fn paste_waits_for_encode() {
        let pb = NSPasteboard::pasteboardWithUniqueName();
        let pending = PendingPng::new();
        promise_png(&pb, &pending);
        let expected = encode_png(&image()).unwrap().to_vec();

        let (bg, img) = (pending.clone(), MainOnly(image()));
        let encoder = std::thread::spawn(move || {
            let img = img;
            std::thread::sleep(Duration::from_millis(100));
            bg.fulfil(encode_png(&img.0));
        });
        let t = Instant::now();
        let pasted = pb.dataForType(png_type()).expect("promised PNG");
        assert!(t.elapsed() >= Duration::from_millis(90), "paste didn't wait: {:?}", t.elapsed());
        assert_eq!(pasted.to_vec(), expected);
        encoder.join().unwrap();

        // Already provided: settling leaves the pasteboard alone.
        let count = pb.changeCount();
        settle(&pb, count, &pending);
        assert_eq!(pb.changeCount(), count);
        assert_eq!(pb.dataForType(png_type()).unwrap().to_vec(), expected);
    }

    #[test]
    fn settle_swaps_promise_for_data() {
        let pb = NSPasteboard::pasteboardWithUniqueName();
        let pending = PendingPng::new();
        let count = promise_png(&pb, &pending);
        pending.fulfil(encode_png(&image()));
        settle(&pb, count, &pending);
        assert!(matches!(*pending.png.lock().unwrap(), Png::Taken), "snapit still holds the PNG");
        assert!(pb.dataForType(png_type()).is_some());
    }

    #[test]
    fn failed_encode_clears_promise() {
        let pb = NSPasteboard::pasteboardWithUniqueName();
        let pending = PendingPng::new();
        let count = promise_png(&pb, &pending);
        pending.fulfil(None);
        settle(&pb, count, &pending);
        assert!(pb.dataForType(png_type()).is_none());
    }

    #[test]
    fn settle_skips_changed_pasteboard() {
        let pb = NSPasteboard::pasteboardWithUniqueName();
        let pending = PendingPng::new();
        let count = promise_png(&pb, &pending);
        pb.clearContents(); // the user copied something else
        pending.fulfil(encode_png(&image()));
        settle(&pb, count, &pending);
        assert!(pb.dataForType(png_type()).is_none());
    }
}
