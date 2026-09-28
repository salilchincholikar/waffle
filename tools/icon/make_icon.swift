// Waffle logo, wordmark lockups and app icon generator.
//
// Usage:
//   swift make_icon.swift [outDir]
//     writes logo.png, lockup-light.png, lockup-dark.png, preview.png and AppIcon.iconset
//
// Then: iconutil -c icns <outDir>/AppIcon.iconset -o AppIcon.icns   (or: make icon)
//
// The mark is a waffle block: a flat, slightly tilted face with real thickness (its orange
// side shows at the right and bottom), a 3x3 grid of sunken pockets, and a diagonal of green
// cells from dark to light - a waffle that is also a spreadsheet. Flat illustration style:
// solid fills, soft inner shadows, no gloss.
//
// logo.png        the mark on transparency
// lockup-*.png    the mark above the "waffle" wordmark (SF Pro Rounded, heavy); -light has
//                 dark text for light backgrounds, -dark has light text for dark ones
// AppIcon         the mark with no tile of its own: macOS 26 puts it on a system tile that
//                 follows the Dock's icon style. preview.png shows it on a light tile.

import AppKit

func rgb(_ hex: UInt32, _ a: CGFloat = 1) -> CGColor {
    CGColor(srgbRed: CGFloat((hex >> 16) & 0xff) / 255, green: CGFloat((hex >> 8) & 0xff) / 255,
            blue: CGFloat(hex & 0xff) / 255, alpha: a)
}

func rounded(_ r: CGRect, _ c: CGFloat) -> CGPath { CGPath(roundedRect: r, cornerWidth: c, cornerHeight: c, transform: nil) }

// Colours
let faceColor = rgb(0xFAD06E), faceRim = rgb(0xFDE4A6)
let sideColor = rgb(0xEE8B2F), sideShade = rgb(0xDE7424)
let pocketColor = rgb(0xF39B3E), pocketShadow = rgb(0xD9771F)
let greens: [Int: (CGColor, CGColor)] = [          // pocket index: (fill, inner shadow)
    0: (rgb(0x1F5B33), rgb(0x123D21)),
    4: (rgb(0x5B9B45), rgb(0x3F7A30)),
    8: (rgb(0x93C87D), rgb(0x6FA85C)),
]
let inkDark = NSColor(srgbRed: 0x12 / 255.0, green: 0x30 / 255.0, blue: 0x1F / 255.0, alpha: 1)
let inkLight = NSColor(srgbRed: 0xF6 / 255.0, green: 0xEE / 255.0, blue: 0xDC / 255.0, alpha: 1)

// Geometry: a 1024 design space, the block centred at (0, 0), y down.
let face = CGRect(x: -300, y: -300, width: 600, height: 600)
let faceRadius: CGFloat = 116
let depth = CGSize(width: 64, height: 50)      // extrusion toward the bottom-right, on screen
let tilt: CGFloat = 5 * .pi / 180              // clockwise

func pockets() -> [CGRect] {
    let margin: CGFloat = 60, ridge: CGFloat = 44
    let pk = (face.width - 2 * margin - 2 * ridge) / 3
    return (0..<9).map { i in
        CGRect(x: face.minX + margin + CGFloat(i % 3) * (pk + ridge), y: face.minY + margin + CGFloat(i / 3) * (pk + ridge), width: pk, height: pk)
    }
}

func drawMark(_ cg: CGContext) {
    // Thickness: sweep the face outline toward the bottom-right, darker further back.
    let steps = 24
    for i in stride(from: steps, through: 1, by: -1) {
        let t = CGFloat(i) / CGFloat(steps)
        cg.saveGState()
        cg.translateBy(x: depth.width * t, y: depth.height * t)
        cg.rotate(by: tilt)
        cg.addPath(rounded(face, faceRadius))
        cg.setFillColor(t > 0.55 ? sideShade : sideColor)
        cg.fillPath()
        cg.restoreGState()
    }
    // Face
    cg.saveGState()
    cg.rotate(by: tilt)
    let outline = rounded(face, faceRadius)
    cg.addPath(outline); cg.setFillColor(faceColor); cg.fillPath()
    cg.addPath(rounded(face.insetBy(dx: 16, dy: 16), faceRadius - 16))
    cg.setStrokeColor(faceRim); cg.setLineWidth(8); cg.strokePath()
    // Pockets: flat fill with a soft inner shadow along the top and left walls.
    for (i, p) in pockets().enumerated() {
        let (fill, shadow) = greens[i] ?? (pocketColor, pocketShadow)
        let path = rounded(p, 28)
        cg.saveGState()
        cg.addPath(path); cg.clip()
        cg.setFillColor(fill); cg.fill(p)
        cg.setStrokeColor(shadow); cg.setLineWidth(30)
        cg.addPath(rounded(p.offsetBy(dx: 9, dy: 13), 28)); cg.strokePath()
        cg.restoreGState()
    }
    cg.restoreGState()
}

func bitmap(_ w: Int, _ h: Int) -> (NSBitmapImageRep, NSGraphicsContext) {
    let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: w, pixelsHigh: h, bitsPerSample: 8, samplesPerPixel: 4,
                               hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
    return (rep, NSGraphicsContext(bitmapImageRep: rep)!)
}

/// The mark alone (tile: on a light reference tile) in a square of `pixels`.
func renderMark(pixels: Int, tile: Bool) -> NSBitmapImageRep {
    let (rep, ctx) = bitmap(pixels, pixels)
    let cg = ctx.cgContext
    let s = CGFloat(pixels) / 1024
    cg.translateBy(x: 0, y: CGFloat(pixels))
    cg.scaleBy(x: s, y: -s)  // 1024 design units, top-left origin
    if tile {
        let t = rounded(CGRect(x: 100, y: 100, width: 824, height: 824), 185)
        cg.addPath(t); cg.setFillColor(rgb(0xF5F5F7)); cg.fillPath()
        cg.translateBy(x: 500, y: 495)
        cg.scaleBy(x: 0.9, y: 0.9)
    } else {
        cg.translateBy(x: 490, y: 485)
        cg.scaleBy(x: 1.3, y: 1.3)  // fill the canvas; macOS insets it on its tile
    }
    drawMark(cg)
    ctx.flushGraphics()
    return rep
}

/// The mark above the wordmark, on transparency.
func renderLockup(ink: NSColor) -> NSBitmapImageRep {
    let W = 1200, H = 1240
    let (rep, ctx) = bitmap(W, H)
    NSGraphicsContext.current = ctx
    let cg = ctx.cgContext
    cg.saveGState()
    cg.translateBy(x: 0, y: CGFloat(H))
    cg.scaleBy(x: 1, y: -1)
    cg.translateBy(x: CGFloat(W) / 2 - 24, y: 440)
    cg.scaleBy(x: 1.1, y: 1.1)
    drawMark(cg)
    cg.restoreGState()
    // Wordmark (AppKit text, y up).
    let base = NSFont.systemFont(ofSize: 300, weight: .heavy)
    let font = base.fontDescriptor.withDesign(.rounded).flatMap { NSFont(descriptor: $0, size: 300) } ?? base
    let word = NSAttributedString(string: "waffle", attributes: [.font: font, .foregroundColor: ink, .kern: -6])
    let size = word.size()
    word.draw(at: NSPoint(x: (CGFloat(W) - size.width) / 2, y: 40))
    NSGraphicsContext.current = nil
    return rep
}

func write(_ rep: NSBitmapImageRep, _ path: String) {
    try! rep.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: path))
}

let outDir = CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "."
let fm = FileManager.default
let iconset = outDir + "/AppIcon.iconset"
try? fm.removeItem(atPath: iconset)
try! fm.createDirectory(atPath: iconset, withIntermediateDirectories: true)

write(renderMark(pixels: 1024, tile: false), "\(outDir)/logo.png")
write(renderMark(pixels: 1024, tile: true), "\(outDir)/preview.png")
write(renderLockup(ink: inkDark), "\(outDir)/lockup-light.png")
write(renderLockup(ink: inkLight), "\(outDir)/lockup-dark.png")
for base in [16, 32, 128, 256, 512] {
    write(renderMark(pixels: base, tile: false), "\(iconset)/icon_\(base)x\(base).png")
    write(renderMark(pixels: base * 2, tile: false), "\(iconset)/icon_\(base)x\(base)@2x.png")
}
print("wrote logo.png, lockup-light.png, lockup-dark.png, preview.png, \(iconset)")
