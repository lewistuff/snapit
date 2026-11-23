//! Micro-benchmarks for the work snapit does between a mouse release and a
//! ready clipboard (SC-007), plus the editor's per-event model work. They are
//! `#[ignore]`d so `cargo test` stays fast; run them with `make bench`, which
//! builds in release mode and runs them one at a time.
//!
//! Capture itself (ScreenCaptureKit) and hotkey → overlay latency (SC-003)
//! need Screen Recording access and real input, so they stay manual checkpoints.

use std::hint::black_box;
use std::time::{Duration, Instant};

use objc2_app_kit::{NSPasteboard, NSPasteboardTypePNG};
use objc2_core_foundation::CGRect;
use objc2_core_graphics::{
    CGBitmapContextCreate, CGBitmapContextCreateImage, CGBitmapContextGetBytesPerRow, CGBitmapContextGetData,
    CGColor, CGColorSpace, CGContext, CGImageAlphaInfo, kCGColorSpaceSRGB,
};
use objc2_foundation::{NSData, NSPoint, NSSize};

use crate::core::annotate::{Doc, Tool, pt};
use crate::mac::capture::{self, Image};
use crate::mac::editor;

/// Built-in Retina display (14" MacBook Pro) and a 5K display, in pixels.
const RETINA: (usize, usize) = (3024, 1964);
const FIVE_K: (usize, usize) = (5120, 2880);

/// Runs `f` once to warm up, then `iters` times, and prints median, p95 and min.
fn bench(name: &str, iters: usize, mut f: impl FnMut()) {
    f();
    let mut times: Vec<Duration> = (0..iters)
        .map(|_| {
            let t = Instant::now();
            f();
            t.elapsed()
        })
        .collect();
    times.sort();
    let p95 = times[(iters * 95).div_ceil(100) - 1];
    println!("{name:<44} median {:>9.3?}   p95 {:>9.3?}   min {:>9.3?}", times[iters / 2], p95, times[0]);
}

#[derive(Clone, Copy)]
enum Content {
    /// Flat window chrome: large single-colour areas with a few panels.
    /// Compresses well, like most UI screenshots.
    Ui,
    /// Pseudo-random pixels everywhere: the PNG worst case (photos, video).
    Noise,
}

fn image((w, h): (usize, usize), content: Content) -> Image {
    let space = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB })).unwrap();
    // SAFETY: null data lets CG allocate; parameters describe 8-bit RGBA.
    let ctx = unsafe {
        CGBitmapContextCreate(std::ptr::null_mut(), w, h, 8, 0, Some(&space), CGImageAlphaInfo::PremultipliedLast.0)
    }
    .unwrap();
    let fill = |r, g, b, x, y, pw, ph| {
        CGContext::set_fill_color_with_color(Some(&ctx), Some(&CGColor::new_srgb(r, g, b, 1.0)));
        CGContext::fill_rect(Some(&ctx), CGRect::new(NSPoint::new(x, y), NSSize::new(pw, ph)));
    };
    let (fw, fh) = (w as f64, h as f64);
    match content {
        Content::Ui => {
            fill(0.96, 0.96, 0.97, 0.0, 0.0, fw, fh);
            fill(0.88, 0.89, 0.91, 0.0, 0.0, fw * 0.22, fh);
            fill(1.0, 1.0, 1.0, fw * 0.26, fh * 0.1, fw * 0.7, fh * 0.8);
            for i in 0..40 {
                // Rows of "text": thin dark bars of varying length.
                let y = fh * 0.12 + i as f64 * fh * 0.018;
                fill(0.2, 0.2, 0.25, fw * 0.28, y, fw * (0.2 + 0.4 * ((i * 7 % 10) as f64 / 10.0)), fh * 0.006);
            }
        }
        Content::Noise => {
            let data = CGBitmapContextGetData(Some(&ctx)) as *mut u8;
            let bpr = CGBitmapContextGetBytesPerRow(Some(&ctx));
            // SAFETY: CG allocated bpr * h bytes for this context.
            let bytes = unsafe { std::slice::from_raw_parts_mut(data, bpr * h) };
            let mut s: u32 = 0x9e37_79b9;
            for px in bytes.as_chunks_mut::<4>().0 {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                px[..3].copy_from_slice(&s.to_le_bytes()[..3]);
                px[3] = 255;
            }
        }
    }
    CGBitmapContextCreateImage(Some(&ctx)).unwrap()
}

/// A document with `n` annotations spread over a 5K, 2× canvas: shapes,
/// arrows, steps and the odd text label, as a heavily marked-up capture.
fn doc(n: usize, measure: bool) -> Doc {
    let (w, h) = (FIVE_K.0 as f64, FIVE_K.1 as f64);
    let m: Option<crate::core::annotate::MeasureFn> = if measure { Some(Box::new(editor::measure)) } else { None };
    let mut d = Doc::new(w, h, 2.0, m);
    let tools = [Tool::Rect, Tool::Arrow, Tool::Ellipse, Tool::Line, Tool::Step, Tool::Text];
    for i in 0..n {
        let x = 100.0 + (i * 397 % 4600) as f64;
        let y = 100.0 + (i * 211 % 2500) as f64;
        d.set_color(i % 6);
        d.set_tool(tools[i % tools.len()]);
        let eff = d.press(pt(x, y), 1);
        if tools[i % tools.len()] == Tool::Text {
            d.commit_text(0, eff.text_at, "Click here, then Save");
        } else {
            d.drag_to(pt(x + 180.0, y + 120.0));
            d.release(pt(x + 180.0, y + 120.0));
        }
        d.deselect();
    }
    d
}

#[test]
#[ignore]
fn bench_export() {
    println!();
    for (label, dims) in [("retina", RETINA), ("5k", FIVE_K)] {
        for (kind, content) in [("ui", Content::Ui), ("noise", Content::Noise)] {
            let img = image(dims, content);
            let mut len = 0;
            bench(&format!("png encode        {label:<6} {kind}"), 10, || {
                len = black_box(capture::encode_png_for_test(&img).unwrap()).len();
            });
            println!("{:<44} {} KB", "", len / 1024);
        }
    }

    // The clipboard half of a quick capture: a private pasteboard so the
    // benchmark doesn't clobber the user's clipboard.
    let png = capture::encode_png_for_test(&image(FIVE_K, Content::Ui)).unwrap();
    let data = NSData::with_bytes(&png);
    let pb = NSPasteboard::pasteboardWithUniqueName();
    bench("pasteboard write  5k     ui", 20, || {
        pb.clearContents();
        // SAFETY: NSPasteboardTypePNG is a valid static type string.
        black_box(pb.setData_forType(Some(&data), unsafe { NSPasteboardTypePNG }));
    });
}

#[test]
#[ignore]
fn bench_flatten() {
    println!();
    let img = image(FIVE_K, Content::Ui);
    for n in [0, 20, 100] {
        let items = doc(n, true).render();
        bench(&format!("flatten           5k     {n} annotations"), 10, || {
            black_box(editor::flatten(&img, &items).unwrap());
        });
    }
}

#[test]
#[ignore]
fn bench_doc() {
    println!();
    for n in [20, 200] {
        let mut d = doc(n, false);
        bench(&format!("render list       {n} annotations"), 1000, || {
            black_box(d.render());
        });
        bench(&format!("hit test (miss)   {n} annotations"), 1000, || {
            black_box(d.hit_test(pt(5000.0, 2800.0)));
        });
        // One mouse-move while drawing a new arrow: what the canvas pays per event.
        d.set_tool(Tool::Arrow);
        d.press(pt(10.0, 10.0), 1);
        let mut i = 0.0;
        bench(&format!("drag + render     {n} annotations"), 1000, || {
            i += 1.0;
            d.drag_to(pt(200.0 + i % 500.0, 200.0));
            black_box(d.render());
        });
        d.release(pt(700.0, 200.0));
        bench(&format!("undo + redo       {n} annotations"), 1000, || {
            d.undo();
            d.redo();
        });
    }
    bench("build doc         200 annotations", 20, || {
        black_box(doc(200, false));
    });
}
