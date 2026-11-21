//! Screen Recording permission, ScreenCaptureKit screenshots, and export
//! (PNG to the pasteboard and/or a file). Pixels stay in CGImage/NSData; the
//! PNG is encoded on a background queue.

use std::cell::Cell;
use std::path::PathBuf;
use std::ptr::NonNull;

use block2::RcBlock;
use dispatch2::{DispatchQueue, GlobalQueueIdentifier, DispatchQueueGlobalPriority};
use objc2::rc::Retained;
use objc2::{AnyThread, MainThreadMarker};
use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSPasteboard, NSPasteboardTypePNG, NSWorkspace};
use objc2_core_foundation::{CFRetained, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{CGImage, CGPreflightScreenCaptureAccess, CGRequestScreenCaptureAccess};
use objc2_foundation::{NSArray, NSData, NSDictionary, NSError, NSURL};
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

/// Encodes `img` as PNG off the main thread, writes it to `path` if given, and
/// puts it on the pasteboard if `clipboard`. `done` runs on the main thread
/// with the save error, if any. The pasteboard is set even when saving fails.
pub fn export(img: Image, clipboard: bool, path: Option<PathBuf>, done: impl FnOnce(Result<(), String>) + 'static) {
    let img = MainOnly(img);
    let done = MainOnly(done);
    DispatchQueue::global_queue(GlobalQueueIdentifier::Priority(DispatchQueueGlobalPriority::High)).exec_async(move || {
        // CGImage and NSData are immutable and thread-safe.
        let (img, done) = (img, done);
        let png = encode_png(&img.0);
        let saved = match (&png, &path) {
            (None, _) => Err("Couldn't encode the image".to_string()),
            (Some(data), Some(p)) => {
                if data.writeToFile_atomically(&ns(&p.to_string_lossy()), true) {
                    Ok(())
                } else {
                    Err(format!("Couldn't write {}", p.display()))
                }
            }
            (Some(_), None) => Ok(()),
        };
        let png = MainOnly(png);
        on_main(move |mtm| {
            let (png, done) = (png, done);
            if clipboard && let Some(data) = &png.0 {
                let pb = NSPasteboard::generalPasteboard();
                pb.clearContents();
                // SAFETY: NSPasteboardTypePNG is a valid static type string.
                pb.setData_forType(Some(data), unsafe { NSPasteboardTypePNG });
            }
            let _ = mtm;
            (done.0)(saved);
        });
    });
}

#[cfg(test)]
pub fn encode_png_for_test(img: &CGImage) -> Option<Vec<u8>> {
    encode_png(img).map(|d| d.to_vec())
}

pub fn size(img: &CGImage) -> CGSize {
    CGSize::new(CGImage::width(Some(img)) as f64, CGImage::height(Some(img)) as f64)
}

pub fn rect(x: f64, y: f64, w: f64, h: f64) -> CGRect {
    CGRect::new(CGPoint::new(x, y), CGSize::new(w, h))
}
