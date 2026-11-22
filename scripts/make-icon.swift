// Renders assets/AppIcon.icns: a white six-blade aperture on a blue-violet
// rounded square, following the macOS icon grid (824 pt tile on a 1024 canvas).
// Run from the repo root: swift scripts/make-icon.swift
// The menu bar uses the same aperture geometry (src/mac/status.rs).
import AppKit

func aperture(_ ctx: CGContext, center c: CGPoint, radius R: CGFloat) {
    let r = R * 0.36, gap = R * 0.075, start = CGFloat.pi / 2
    let v = (0..<6).map { k -> CGPoint in
        let a = start + CGFloat(k) * .pi / 3
        return CGPoint(x: c.x + r * cos(a), y: c.y + r * sin(a))
    }
    ctx.saveGState()
    ctx.addEllipse(in: CGRect(x: c.x - R, y: c.y - R, width: 2 * R, height: 2 * R))
    ctx.clip()
    ctx.beginTransparencyLayer(auxiliaryInfo: nil)
    ctx.setFillColor(.white)
    ctx.fill(CGRect(x: c.x - R, y: c.y - R, width: 2 * R, height: 2 * R))
    // Each blade edge runs along one hexagon side and on out past the rim.
    ctx.setBlendMode(.clear)
    ctx.setLineWidth(gap)
    ctx.setLineCap(.butt)
    for k in 0..<6 {
        let a = v[k], b = v[(k + 1) % 6]
        let len = hypot(b.x - a.x, b.y - a.y)
        let d = CGPoint(x: (b.x - a.x) / len, y: (b.y - a.y) / len)
        ctx.move(to: a)
        ctx.addLine(to: CGPoint(x: a.x + d.x * 3 * R, y: a.y + d.y * 3 * R))
        ctx.strokePath()
    }
    ctx.addLines(between: v)
    ctx.closePath()
    ctx.fillPath()
    ctx.endTransparencyLayer()
    ctx.restoreGState()
}

func render(_ px: Int) -> Data {
    let s = CGFloat(px) / 1024
    let ctx = CGContext(data: nil, width: px, height: px, bitsPerComponent: 8, bytesPerRow: 0,
                        space: CGColorSpace(name: CGColorSpace.sRGB)!,
                        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
    ctx.scaleBy(x: s, y: s)
    let tile = CGRect(x: 100, y: 100, width: 824, height: 824)
    let path = CGPath(roundedRect: tile, cornerWidth: 185, cornerHeight: 185, transform: nil)

    ctx.saveGState()
    ctx.setShadow(offset: CGSize(width: 0, height: -10), blur: 24, color: CGColor(gray: 0, alpha: 0.35))
    ctx.addPath(path)
    ctx.setFillColor(.black)
    ctx.fillPath()
    ctx.restoreGState()

    ctx.saveGState()
    ctx.addPath(path)
    ctx.clip()
    let colors = [CGColor(srgbRed: 0.42, green: 0.36, blue: 1.0, alpha: 1),
                  CGColor(srgbRed: 0.13, green: 0.20, blue: 0.62, alpha: 1)] as CFArray
    let grad = CGGradient(colorsSpace: nil, colors: colors, locations: [0, 1])!
    ctx.drawLinearGradient(grad, start: CGPoint(x: 0, y: 924), end: CGPoint(x: 0, y: 100), options: [])
    ctx.restoreGState()

    aperture(ctx, center: CGPoint(x: 512, y: 512), radius: 260)

    let rep = NSBitmapImageRep(cgImage: ctx.makeImage()!)
    return rep.representation(using: .png, properties: [:])!
}

let fm = FileManager.default
let set = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("AppIcon.iconset")
try? fm.removeItem(at: set)
try! fm.createDirectory(at: set, withIntermediateDirectories: true)
for pt in [16, 32, 128, 256, 512] {
    try! render(pt).write(to: set.appendingPathComponent("icon_\(pt)x\(pt).png"))
    try! render(pt * 2).write(to: set.appendingPathComponent("icon_\(pt)x\(pt)@2x.png"))
}
try! fm.createDirectory(atPath: "assets", withIntermediateDirectories: true)
let p = Process()
p.executableURL = URL(fileURLWithPath: "/usr/bin/iconutil")
p.arguments = ["-c", "icns", set.path, "-o", "assets/AppIcon.icns"]
try! p.run()
p.waitUntilExit()
try! render(1024).write(to: URL(fileURLWithPath: "assets/AppIcon.png"))
print("wrote assets/AppIcon.icns")
