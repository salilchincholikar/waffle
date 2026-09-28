import AppKit
import WaffleBridge

/// Cell formatting controls, on the formula bar's row (the title bar keeps search and the
/// sheet-wide tools). Menus and actions are the same ones the Format menu uses.
extension SheetWindowController {
    func buildFormatControls() -> [NSView] {
        styleSeg.segmentCount = 3
        styleSeg.trackingMode = .momentary
        for (i, (s, d)) in [("bold", "Bold"), ("italic", "Italic"), ("underline", "Underline")].enumerated() {
            styleSeg.setImage(symbol(s, d), forSegment: i)
            styleSeg.setToolTip("\(d) (⌘\(d.first!))", forSegment: i)
            styleSeg.setWidth(28, forSegment: i)
        }
        styleSeg.target = self
        styleSeg.action = #selector(styleSegment(_:))

        alignSeg.segmentCount = 3
        alignSeg.trackingMode = .momentary
        for (i, (s, d)) in [("text.alignleft", "Align Left"), ("text.aligncenter", "Center"), ("text.alignright", "Align Right")].enumerated() {
            alignSeg.setImage(symbol(s, d), forSegment: i)
            alignSeg.setToolTip(d, forSegment: i)
            alignSeg.setWidth(28, forSegment: i)
        }
        alignSeg.target = self
        alignSeg.action = #selector(alignSegment(_:))

        let wrap = NSButton(image: symbol("text.justify.left", "Wrap Text"), target: self, action: #selector(toggleWrap(_:)))
        wrap.toolTip = "Wrap Text"
        wrap.bezelStyle = .accessoryBarAction
        wrap.setButtonType(.momentaryPushIn)

        var formats = SheetWindowController.numberFormats.map { (t, code) -> NSMenuItem in
            let m = NSMenuItem(title: t, action: #selector(setNumberFormat(_:)), keyEquivalent: "")
            m.representedObject = code
            return m
        }
        formats += [.separator(),
                    menuItem("Increase Decimals", #selector(increaseDecimals(_:))),
                    menuItem("Decrease Decimals", #selector(decreaseDecimals(_:))),
                    menuItem("Custom Format…", #selector(customNumberFormat(_:)))]
        let number = popDown("Number Format", symbol("number", "Number Format"), formats)
        let borders = popDown("Borders", symbol("square.grid.3x3", "Borders"), [
            menuItem("All Borders", #selector(borderAll(_:)), "square.grid.3x3"),
            menuItem("Outline", #selector(borderOutline(_:)), "square"),
            menuItem("Bottom Border", #selector(borderBottom(_:)), "square.bottomhalf.filled"),
            menuItem("Thick Bottom Border", #selector(borderThickBottom(_:))),
            menuItem("No Borders", #selector(borderNone(_:)), "square.dashed"),
        ])
        let merge = popDown("Merge", symbol("rectangle.split.2x1", "Merge"), [
            menuItem("Merge Cells", #selector(mergeCells(_:))),
            menuItem("Unmerge Cells", #selector(unmergeCells(_:))),
        ])
        return [styleSeg, colorButton(.text), colorButton(.fill), alignSeg, wrap, number, borders, merge]
    }

    /// A pull-down button showing an icon; the menu's first item is the (hidden) title.
    func popDown(_ label: String, _ image: NSImage, _ items: [NSMenuItem]) -> NSPopUpButton {
        let b = NSPopUpButton(frame: .zero, pullsDown: true)
        b.bezelStyle = .accessoryBarAction
        b.toolTip = label
        // Icon + chevron only; the default width leaves the chevron far from the icon.
        b.widthAnchor.constraint(equalToConstant: 46).isActive = true
        let head = NSMenuItem(title: "", action: nil, keyEquivalent: "")
        head.image = image
        b.menu?.addItem(head)
        for i in items { b.menu?.addItem(i) }
        b.setAccessibilityLabel(label)
        return b
    }

    func colorButton(_ kind: ColorKind) -> NSPopUpButton {
        let label = kind == .text ? "Text Color" : "Fill Color"
        var items: [NSMenuItem] = []
        let first = NSMenuItem(title: kind == .text ? "Automatic" : "No Fill", action: #selector(pickColor(_:)), keyEquivalent: "")
        first.image = Self.swatch(kind == .text ? 0x000000 : nil)
        first.tag = kind.rawValue
        first.representedObject = -1
        items += [first, .separator()]
        for (name, rgb) in kind == .text ? Self.textPalette : Self.fillPalette {
            let it = NSMenuItem(title: name, action: #selector(pickColor(_:)), keyEquivalent: "")
            it.image = Self.swatch(rgb)
            it.tag = kind.rawValue
            it.representedObject = Int(rgb)
            items.append(it)
        }
        let more = NSMenuItem(title: "More Colors…", action: #selector(moreColors(_:)), keyEquivalent: "")
        more.tag = kind.rawValue
        items += [.separator(), more]
        let b = popDown(label, Self.colorIcon(kind == .text ? "character" : "highlighter", color: kind == .text ? .labelColor : NSColor(rgb: 0xFFF2A8)), items)
        if kind == .text { textColorButton = b } else { fillColorButton = b }
        return b
    }
}

// ---- formatting actions ------------------------------------------------------------------

extension SheetWindowController {
    @objc func styleSegment(_ s: NSSegmentedControl) {
        switch s.selectedSegment {
        case 0: toggleBold(nil)
        case 1: toggleItalic(nil)
        default: toggleUnderline(nil)
        }
    }

    @objc func alignSegment(_ s: NSSegmentedControl) {
        switch s.selectedSegment {
        case 0: alignLeft(nil)
        case 1: alignCenter(nil)
        default: alignRight(nil)
        }
    }

    /// CSV files can't store formatting: disable the formatting controls for them.
    func updateFormatControls() {
        guard doc.book != nil else { return }
        let enabled = book.kind != .csv
        for c in [styleSeg, alignSeg, textColorButton, fillColorButton] as [NSControl?] { c?.isEnabled = enabled }
    }

    // ---- colour pickers --------------------------------------------------------------

    enum ColorKind: Int { case text = 0, fill = 1 }

    static let textPalette: [(String, UInt32)] = [
        ("Black", 0x000000), ("Dark Gray", 0x595959), ("Gray", 0x8C8C8C), ("Red", 0xC00000), ("Orange", 0xE36C09),
        ("Green", 0x00803C), ("Blue", 0x0B5CD5), ("Purple", 0x7030A0),
    ]
    static let fillPalette: [(String, UInt32)] = [
        ("Yellow", 0xFFF2A8), ("Green", 0xD6F5D6), ("Blue", 0xD9E8FB), ("Red", 0xFADADD), ("Orange", 0xFDE3C8),
        ("Purple", 0xE8DEF8), ("Gray", 0xEDEDED),
    ]

    /// Icon (SF Symbol) with a colour bar underneath, like Pages/Numbers.
    static func colorIcon(_ symbolName: String, color: NSColor?) -> NSImage {
        let size = NSSize(width: 20, height: 18)
        let img = NSImage(size: size, flipped: false) { rect in
            let cfg = NSImage.SymbolConfiguration(pointSize: 12, weight: .semibold)
            if let sym = NSImage(systemSymbolName: symbolName, accessibilityDescription: nil)?.withSymbolConfiguration(cfg) {
                let s = sym.size
                NSColor.labelColor.set()
                sym.draw(in: NSRect(x: (rect.width - s.width) / 2, y: 5 + (13 - s.height) / 2, width: s.width, height: s.height))
            }
            let bar = NSRect(x: 3, y: 0.5, width: rect.width - 6, height: 3.5)
            if let color {
                color.setFill()
                NSBezierPath(roundedRect: bar, xRadius: 1.5, yRadius: 1.5).fill()
            } else {
                NSColor.tertiaryLabelColor.setStroke()
                let p = NSBezierPath(roundedRect: bar.insetBy(dx: 0.5, dy: 0.5), xRadius: 1.5, yRadius: 1.5)
                p.lineWidth = 1
                p.stroke()
            }
            return true
        }
        img.isTemplate = false
        return img
    }

    static func swatch(_ rgb: UInt32?) -> NSImage {
        NSImage(size: NSSize(width: 14, height: 14), flipped: false) { rect in
            let path = NSBezierPath(roundedRect: rect.insetBy(dx: 0.5, dy: 0.5), xRadius: 3, yRadius: 3)
            if let rgb { NSColor(rgb: rgb).setFill(); path.fill() }
            NSColor.separatorColor.setStroke()
            path.stroke()
            if rgb == nil {
                NSColor.systemRed.setStroke()
                let slash = NSBezierPath()
                slash.move(to: NSPoint(x: 2, y: 2)); slash.line(to: NSPoint(x: 12, y: 12))
                slash.stroke()
            }
            return true
        }
    }

    @objc func pickColor(_ sender: NSMenuItem) {
        guard let rgb = sender.representedObject as? Int else { return }
        let kind = ColorKind(rawValue: sender.tag) ?? .text
        applyStyle(kind == .text ? .textColor : .fill, num: Double(rgb))
        updateColorIcon(kind, rgb: rgb < 0 ? nil : UInt32(rgb))
    }

    @objc func moreColors(_ sender: NSMenuItem) {
        pendingColorKind = sender.tag
        let panel = NSColorPanel.shared
        panel.setTarget(self)
        panel.setAction(#selector(colorPanelChanged(_:)))
        panel.showsAlpha = false
        panel.orderFront(nil)
    }

    @objc func colorPanelChanged(_ panel: NSColorPanel) {
        let kind = ColorKind(rawValue: pendingColorKind) ?? .text
        let rgb = panel.color.rgbValue
        applyStyle(kind == .text ? .textColor : .fill, num: Double(rgb))
        updateColorIcon(kind, rgb: rgb)
    }

    /// The colour button's bar shows the last colour used.
    func updateColorIcon(_ kind: ColorKind, rgb: UInt32?) {
        let b = kind == .text ? textColorButton : fillColorButton
        b?.item(at: 0)?.image = Self.colorIcon(kind == .text ? "character" : "highlighter", color: rgb.map { NSColor(rgb: $0) } ?? (kind == .text ? .labelColor : nil))
    }
}
