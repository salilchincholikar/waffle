import AppKit

/// Whether the sheet itself (cells, headers) turns dark in dark mode. The window chrome
/// always follows the system; by default the sheet stays white, like Excel.
enum SheetAppearance {
    static let key = "DarkSheetInDarkMode"
    static let changed = Notification.Name("WaffleSheetAppearanceChanged")

    static var darkInDarkMode: Bool {
        get { UserDefaults.standard.bool(forKey: key) }
        set {
            UserDefaults.standard.set(newValue, forKey: key)
            NotificationCenter.default.post(name: changed, object: nil)
        }
    }
}

/// Colours the grid draws with. Colours set in the file are drawn as-is on top of these.
struct SheetPalette {
    let isDark: Bool
    let background: CGColor
    /// "Automatic" text and border colour.
    let text: NSColor
    let grid: CGColor
    let frozenLine: CGColor
    let headerBG: CGColor
    let headerLine: CGColor
    let headerText: NSColor
    let corner: CGColor
    let filterBox: CGColor
    let filterGlyph: CGColor
    let cardFill: CGColor
    let cardStroke: CGColor
    let cardTitle: NSColor
    let cardNote: NSColor

    static let light = SheetPalette(
        isDark: false, background: CGColor(gray: 1, alpha: 1), text: .black,
        grid: CGColor(gray: 0.86, alpha: 1), frozenLine: CGColor(gray: 0.62, alpha: 1),
        headerBG: CGColor(gray: 0.965, alpha: 1), headerLine: CGColor(gray: 0.8, alpha: 1),
        headerText: NSColor(white: 0.35, alpha: 1), corner: CGColor(gray: 0.72, alpha: 1),
        filterBox: CGColor(gray: 0.88, alpha: 1), filterGlyph: CGColor(gray: 0.3, alpha: 1),
        cardFill: CGColor(gray: 0.985, alpha: 1), cardStroke: CGColor(gray: 0.8, alpha: 1),
        cardTitle: NSColor(white: 0.3, alpha: 1), cardNote: NSColor(white: 0.55, alpha: 1))

    static let dark = SheetPalette(
        isDark: true, background: CGColor(gray: 0.12, alpha: 1), text: NSColor(white: 0.9, alpha: 1),
        grid: CGColor(gray: 0.24, alpha: 1), frozenLine: CGColor(gray: 0.45, alpha: 1),
        headerBG: CGColor(gray: 0.16, alpha: 1), headerLine: CGColor(gray: 0.3, alpha: 1),
        headerText: NSColor(white: 0.65, alpha: 1), corner: CGColor(gray: 0.42, alpha: 1),
        filterBox: CGColor(gray: 0.3, alpha: 1), filterGlyph: CGColor(gray: 0.8, alpha: 1),
        cardFill: CGColor(gray: 0.15, alpha: 1), cardStroke: CGColor(gray: 0.32, alpha: 1),
        cardTitle: NSColor(white: 0.85, alpha: 1), cardNote: NSColor(white: 0.6, alpha: 1))

    /// Find highlights: every match, and the current one. On a dark sheet they are deep
    /// amber (a see-through yellow turns olive there and hides light text).
    var match: CGColor { isDark ? CGColor(srgbRed: 0.45, green: 0.34, blue: 0.08, alpha: 0.95) : CGColor(srgbRed: 1, green: 0.9, blue: 0.2, alpha: 0.45) }
    var currentMatch: CGColor { isDark ? CGColor(srgbRed: 0.66, green: 0.36, blue: 0.05, alpha: 0.95) : CGColor(srgbRed: 1, green: 0.62, blue: 0.1, alpha: 0.55) }

    /// Text colour for a file colour (0 = automatic), drawn over `fill` (nil = the sheet).
    /// On a dark sheet: text on a light fill is drawn as on a white sheet; elsewhere black
    /// means "default text" (files store it that way) and very dark colours are lightened.
    func ink(_ v: UInt32, on fill: CGColor? = nil) -> NSColor {
        guard isDark else { return v == 0 ? text : NSColor(rgb: v) }
        if let fill, Self.luminance(fill) > 0.5 { return v == 0 ? .black : NSColor(rgb: v) }
        if v & 0xFFFFFF == 0 { return text }
        // Dark file colours (dark green, navy…) are lifted until they read on the dark sheet.
        let c = NSColor(rgb: v).cgColor
        let l = Self.luminance(c)
        guard l < 0.45 else { return NSColor(cgColor: c) ?? .white }
        return NSColor(cgColor: Self.mix(c, CGColor(gray: 1, alpha: 1), min(0.7, 0.45 - l + 0.25))) ?? .white
    }

    /// Text colour over `fill`, the colour actually drawn behind the cell (its own fill, a
    /// table style's band or a conditional format): automatic text is dark on a light fill.
    func text(_ v: UInt32, over fill: CGColor?, auto: NSColor) -> NSColor {
        if v != 0 { return ink(v, on: fill) }
        return isDark && isLight(fill) ? .black : auto
    }

    func isLight(_ fill: CGColor?) -> Bool { fill.map { Self.luminance($0) > 0.5 } ?? false }

    private static func luminance(_ c: CGColor) -> CGFloat {
        guard let s = c.converted(to: CGColorSpace(name: CGColorSpace.sRGB)!, intent: .defaultIntent, options: nil)?.components, s.count >= 3 else { return 1 }
        return 0.2126 * s[0] + 0.7152 * s[1] + 0.0722 * s[2]
    }

    /// Fill for a file colour (0 = none). On a dark sheet a plain white fill counts as none.
    func fill(_ v: UInt32) -> CGColor? {
        guard v != 0, !(isDark && v & 0xFFFFFF == 0xFFFFFF) else { return nil }
        return tone(NSColor(rgb: v))
    }

    /// A fill as drawn on this sheet. On a dark sheet, file colours are toned down toward
    /// the background (a light green becomes a deep green) instead of glowing at full
    /// brightness; text over them then reads as over a dark cell.
    func tone(_ c: NSColor) -> CGColor {
        guard isDark else { return c.cgColor }
        // Paler colours go further: stripes and tints nearly vanish, strong colours keep a hue.
        return Self.mix(c.cgColor, background, 0.8 + 0.15 * Self.luminance(c.cgColor))
    }

    /// `a` moved `t` of the way to `b`, in sRGB.
    static func mix(_ a: CGColor, _ b: CGColor, _ t: CGFloat) -> CGColor {
        let srgb = CGColorSpace(name: CGColorSpace.sRGB)!
        guard let x = a.converted(to: srgb, intent: .defaultIntent, options: nil)?.components, x.count >= 3,
              let y = b.converted(to: srgb, intent: .defaultIntent, options: nil)?.components, y.count >= 3 else { return a }
        return CGColor(srgbRed: x[0] + (y[0] - x[0]) * t, green: x[1] + (y[1] - x[1]) * t, blue: x[2] + (y[2] - x[2]) * t, alpha: 1)
    }

    /// The palette for a view, given the setting and the view's effective appearance.
    static func resolve(for view: NSView) -> SheetPalette {
        guard SheetAppearance.darkInDarkMode else { return .light }
        return view.effectiveAppearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua ? .dark : .light
    }
}
