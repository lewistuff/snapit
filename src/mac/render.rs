//! Draws annotation items into the current NSGraphicsContext. The caller sets
//! up a flipped context in image-pixel coordinates; the canvas and the
//! exported image share this code, so what you see is what you get.

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{
    NSBezierPath, NSColor, NSFont, NSFontAttributeName, NSFontWeightBold, NSForegroundColorAttributeName,
    NSLineCapStyle, NSLineJoinStyle, NSStringDrawing,
};
use objc2_foundation::{NSDictionary, NSPoint, NSRect, NSSize, NSString};

use super::ns;
use crate::core::annotate::{Item, Kind, NUM_COLORS};

/// sRGB palette: red, yellow, green, blue, black, white.
pub const PALETTE: [(f64, f64, f64); NUM_COLORS] =
    [(1.0, 0.23, 0.19), (1.0, 0.8, 0.0), (0.2, 0.78, 0.35), (0.0, 0.48, 1.0), (0.0, 0.0, 0.0), (1.0, 1.0, 1.0)];

pub const COLOR_NAMES: [&str; NUM_COLORS] = ["Red", "Yellow", "Green", "Blue", "Black", "White"];

pub fn color(i: usize) -> Retained<NSColor> {
    let (r, g, b) = PALETTE[i.min(NUM_COLORS - 1)];
    NSColor::colorWithSRGBRed_green_blue_alpha(r, g, b, 1.0)
}

/// Whether text drawn on top of this colour should be dark.
fn is_light(i: usize) -> bool {
    let (r, g, b) = PALETTE[i.min(NUM_COLORS - 1)];
    0.299 * r + 0.587 * g + 0.114 * b > 0.6
}

pub fn font(size: f64) -> Retained<NSFont> {
    // SAFETY: NSFontWeightBold is a valid static.
    NSFont::systemFontOfSize_weight(size, unsafe { NSFontWeightBold })
}

pub fn text_attrs(font_px: f64, color: &NSColor) -> Retained<NSDictionary<NSString, AnyObject>> {
    let f = font(font_px);
    // SAFETY: valid attribute keys with values of the documented types.
    unsafe {
        NSDictionary::from_slices(
            &[NSFontAttributeName, NSForegroundColorAttributeName],
            &[&*f as &AnyObject, color as &AnyObject],
        )
    }
}

/// Rendered size of text, in the same units as `font_px`.
pub fn measure(text: &str, font_px: f64) -> (f64, f64) {
    let attrs = text_attrs(font_px, &NSColor::blackColor());
    // SAFETY: valid attributes.
    let s = unsafe { ns(text).sizeWithAttributes(Some(&attrs)) };
    (s.width.ceil(), s.height.ceil())
}

fn pt(x: f64, y: f64) -> NSPoint {
    NSPoint::new(x, y)
}

fn norm(a: NSPoint, b: NSPoint) -> NSRect {
    NSRect::new(pt(a.x.min(b.x), a.y.min(b.y)), NSSize::new((a.x - b.x).abs(), (a.y - b.y).abs()))
}

fn stroked(path: &NSBezierPath, width: f64) {
    path.setLineWidth(width);
    path.setLineCapStyle(NSLineCapStyle::Round);
    path.setLineJoinStyle(NSLineJoinStyle::Round);
    path.stroke();
}

/// Draws `items` bottom to top. `hairline` is one screen point in image
/// pixels, used for selection outlines (which exports leave out by clearing
/// `selected`).
pub fn draw(items: &[Item], hairline: f64) {
    for it in items {
        let (a, b) = (pt(it.a.x, it.a.y), pt(it.b.x, it.b.y));
        let c = color(it.color);
        c.setStroke();
        c.setFill();
        let bounds = match it.kind {
            Kind::Line => {
                let p = NSBezierPath::bezierPath();
                p.moveToPoint(a);
                p.lineToPoint(b);
                stroked(&p, it.stroke);
                norm(a, b)
            }
            Kind::Arrow => {
                draw_arrow(a, b, it.stroke);
                norm(a, b)
            }
            Kind::Rect => {
                let r = norm(a, b);
                stroked(&NSBezierPath::bezierPathWithRect(r), it.stroke);
                r
            }
            Kind::Ellipse => {
                let r = norm(a, b);
                stroked(&NSBezierPath::bezierPathWithOvalInRect(r), it.stroke);
                r
            }
            Kind::Text => {
                let attrs = text_attrs(it.font_size, &c);
                // SAFETY: valid attributes.
                unsafe { ns(&it.text).drawAtPoint_withAttributes(a, Some(&attrs)) };
                norm(a, b)
            }
            Kind::Step => {
                let r = it.radius;
                let circle = NSRect::new(pt(a.x - r, a.y - r), NSSize::new(2.0 * r, 2.0 * r));
                let disc = NSBezierPath::bezierPathWithOvalInRect(circle);
                disc.fill();
                NSColor::whiteColor().colorWithAlphaComponent(0.9).setStroke();
                disc.setLineWidth(r * 0.12);
                disc.stroke();
                let fg = if is_light(it.color) { NSColor::blackColor() } else { NSColor::whiteColor() };
                let attrs = text_attrs(it.font_size, &fg);
                let label = ns(&it.number.to_string());
                // SAFETY: valid attributes.
                unsafe {
                    let s = label.sizeWithAttributes(Some(&attrs));
                    label.drawAtPoint_withAttributes(pt(a.x - s.width / 2.0, a.y - s.height / 2.0), Some(&attrs));
                }
                circle
            }
        };
        if it.selected {
            draw_selection(bounds, it.stroke, hairline);
        }
    }
}

fn draw_arrow(a: NSPoint, b: NSPoint, stroke: f64) {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len = dx.hypot(dy);
    if len < 1.0 {
        return;
    }
    let (ux, uy) = (dx / len, dy / len);
    let head = (stroke * 4.0 + 6.0).min(len * 0.6);
    let half = head * 0.5;
    let base = pt(b.x - ux * head, b.y - uy * head);

    let shaft = NSBezierPath::bezierPath();
    shaft.moveToPoint(a);
    shaft.lineToPoint(pt(b.x - ux * head * 0.6, b.y - uy * head * 0.6));
    stroked(&shaft, stroke);

    let tri = NSBezierPath::bezierPath();
    tri.moveToPoint(b);
    tri.lineToPoint(pt(base.x - uy * half, base.y + ux * half));
    tri.lineToPoint(pt(base.x + uy * half, base.y - ux * half));
    tri.closePath();
    tri.setLineJoinStyle(NSLineJoinStyle::Round);
    tri.setLineWidth(stroke * 0.5);
    tri.fill();
    tri.stroke();
}

fn draw_selection(bounds: NSRect, stroke: f64, hairline: f64) {
    let pad = stroke / 2.0 + 4.0 * hairline;
    let r = NSRect::new(
        pt(bounds.origin.x - pad, bounds.origin.y - pad),
        NSSize::new(bounds.size.width + 2.0 * pad, bounds.size.height + 2.0 * pad),
    );
    let path = NSBezierPath::bezierPathWithRect(r);
    path.setLineWidth(1.5 * hairline);
    let dash = [4.0 * hairline, 3.0 * hairline];
    // SAFETY: `dash` outlives the call and has `count` elements.
    unsafe { path.setLineDash_count_phase(dash.as_ptr(), dash.len() as isize, 0.0) };
    NSColor::whiteColor().setStroke();
    NSBezierPath::bezierPathWithRect(r).stroke();
    NSColor::systemBlueColor().setStroke();
    path.stroke();
}
