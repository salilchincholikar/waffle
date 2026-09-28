import AppKit
import WaffleBridge
import CWaffle

/// Resolved drawing attributes for one xf.
final class CellStyle {
    let font: NSFont
    let color: NSColor
    let fill: CGColor?
    let underline: Bool
    let strike: Bool
    let halign: UInt8
    let valign: UInt8
    let wrap: Bool
    let indent: CGFloat
    let borders: [(style: UInt8, color: CGColor)]
    let rotation: Int16

    init(book: Book, id: Int, zoom: CGFloat, palette: SheetPalette) {
        let s = book.style(id)
        let name = book.fontName(id)
        // Excel sizes are points at 96 dpi: 11pt renders ~14.7px.
        let px = CGFloat(s.font_size > 0 ? s.font_size : 11) * 4 / 3 * zoom
        var font = NSFont(name: name, size: px)
        if font == nil {
            // Calibri & friends are rarely installed on macOS; the system font reads larger, so scale it down.
            font = NSFont.systemFont(ofSize: px * 0.87)
        }
        var f = font!
        let fm = NSFontManager.shared
        if s.bold != 0 { f = fm.convert(f, toHaveTrait: .boldFontMask) }
        if s.italic != 0 { f = fm.convert(f, toHaveTrait: .italicFontMask) }
        self.font = f
        fill = palette.fill(s.fill_color)
        color = palette.ink(s.text_color, on: fill)
        underline = s.underline != 0
        strike = s.strike != 0
        halign = s.halign
        valign = s.valign
        wrap = s.wrap != 0
        indent = CGFloat(s.indent) * 9 * zoom
        rotation = s.rotation
        var b: [(UInt8, CGColor)] = []
        for i in 0..<4 {
            let st = withUnsafeBytes(of: s.border_style) { $0[i] }
            let col = withUnsafeBytes(of: s.border_color) { $0.load(fromByteOffset: i * 4, as: UInt32.self) }
            b.append((st, palette.ink(col).cgColor))
        }
        borders = b
    }
}

extension NSColor {
    convenience init(rgb: UInt32) {
        self.init(srgbRed: CGFloat((rgb >> 16) & 0xFF) / 255, green: CGFloat((rgb >> 8) & 0xFF) / 255, blue: CGFloat(rgb & 0xFF) / 255, alpha: 1)
    }
    var rgbValue: UInt32 {
        guard let c = usingColorSpace(.sRGB) else { return 0 }
        return (UInt32(c.redComponent * 255) << 16) | (UInt32(c.greenComponent * 255) << 8) | UInt32(c.blueComponent * 255)
    }
}
