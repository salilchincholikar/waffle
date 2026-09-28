import AppKit
import WaffleBridge

/// Small sheet-modal dialogs built on NSAlert.
enum Dialogs {
    static func label(_ s: String) -> NSTextField {
        let l = NSTextField(labelWithString: s)
        l.alignment = .right
        return l
    }

    static func grid(_ rows: [[NSView]]) -> NSGridView {
        let g = NSGridView(views: rows)
        g.rowSpacing = 8
        g.columnSpacing = 8
        g.column(at: 0).xPlacement = .trailing
        g.frame.size = g.fittingSize
        return g
    }

    static func popup(_ items: [String], selected: Int = 0) -> NSPopUpButton {
        let p = NSPopUpButton(frame: .zero, pullsDown: false)
        p.addItems(withTitles: items)
        p.selectItem(at: selected)
        return p
    }

    static func run(_ window: NSWindow, title: String, message: String = "", accessory: NSView?, ok: String = "OK", done: @escaping () -> Void) {
        let a = NSAlert()
        a.messageText = title
        a.informativeText = message
        a.accessoryView = accessory
        a.addButton(withTitle: ok)
        a.addButton(withTitle: "Cancel")
        a.beginSheetModal(for: window) { r in
            if r == .alertFirstButtonReturn { done() }
        }
    }

    static func askText(_ window: NSWindow, title: String, message: String = "", value: String, ok: String = "OK", done: @escaping (String) -> Void) {
        let f = NSTextField(string: value)
        f.frame = NSRect(x: 0, y: 0, width: 260, height: 24)
        let a = NSAlert()
        a.messageText = title
        a.informativeText = message
        a.accessoryView = f
        a.addButton(withTitle: ok)
        a.addButton(withTitle: "Cancel")
        a.window.initialFirstResponder = f
        a.beginSheetModal(for: window) { r in
            if r == .alertFirstButtonReturn { done(f.stringValue) }
        }
        DispatchQueue.main.async { f.selectText(nil) }
    }

    static func info(_ window: NSWindow?, _ title: String, _ message: String = "") {
        let a = NSAlert()
        a.messageText = title
        a.informativeText = message
        if let window { a.beginSheetModal(for: window) } else { a.runModal() }
    }

    // ---- sort --------------------------------------------------------------------

    static func sort(_ window: NSWindow, columns: [String], initial: Int, done: @escaping (_ keys: [(Int, Bool)], _ header: Bool) -> Void) {
        var rows: [[NSView]] = []
        var pops: [(NSPopUpButton, NSPopUpButton)] = []
        for i in 0..<3 {
            let col = popup((i == 0 ? [] : ["(none)"]) + columns, selected: i == 0 ? initial : 0)
            let dir = popup(["A → Z (ascending)", "Z → A (descending)"])
            rows.append([label(i == 0 ? "Sort by" : "Then by"), col, dir])
            pops.append((col, dir))
        }
        let header = NSButton(checkboxWithTitle: "First row is a header", target: nil, action: nil)
        header.state = .on
        rows.append([NSView(), header, NSView()])
        let g = grid(rows)
        run(window, title: "Sort", message: "Rows move together; formulas are adjusted.", accessory: g, ok: "Sort") {
            var keys: [(Int, Bool)] = []
            for (i, (c, d)) in pops.enumerated() {
                let idx = c.indexOfSelectedItem - (i == 0 ? 0 : 1)
                if idx >= 0 { keys.append((idx, d.indexOfSelectedItem == 0)) }
            }
            done(keys, header.state == .on)
        }
    }

    // ---- cleanup --------------------------------------------------------------------

    static let dateFormats = ["yyyy-mm-dd", "dd/mm/yyyy", "mm/dd/yyyy", "dd-mmm-yyyy", "d mmm yyyy", "mmm d, yyyy", "dd.mm.yyyy", "yyyy-mm-dd hh:mm:ss"]

    static func dates(_ window: NSWindow, done: @escaping (_ order: Int, _ format: String) -> Void) {
        let order = popup(["Day / Month / Year (05/03/2024 = 5 March)", "Month / Day / Year (05/03/2024 = May 3)", "Year / Month / Day"])
        let fmt = NSComboBox()
        fmt.addItems(withObjectValues: dateFormats)
        fmt.stringValue = dateFormats[0]
        fmt.frame.size.width = 220
        let g = grid([[label("Existing dates are written"), order], [label("Show dates as"), fmt]])
        run(window, title: "Standardize Dates", message: "Text that looks like a date in the selection becomes a real date in one format.", accessory: g, ok: "Standardize") {
            done(order.indexOfSelectedItem, fmt.stringValue.isEmpty ? dateFormats[0] : fmt.stringValue)
        }
    }

    static let amountFormats: [(String, String)] = [
        ("1,234.56", "#,##0.00"),
        ("1234.56", "0.00"),
        ("1,234", "#,##0"),
        ("₹ 1,23,456.78 (Indian grouping)", "[$₹-4009] [>=10000000]##\\,##\\,##\\,##0.00;[$₹-4009] [>=100000]##\\,##\\,##0.00;[$₹-4009] ##,##0.00"),
        ("₹ 1,234.56", "[$₹-4009] #,##0.00"),
        ("$1,234.56", "\"$\"#,##0.00"),
        ("(1,234.56) for negatives", "#,##0.00;(#,##0.00)"),
    ]

    static func amounts(_ window: NSWindow, done: @escaping (_ decimalComma: Bool, _ format: String) -> Void) {
        let dec = popup(["1,234.56 — dot is the decimal", "1.234,56 — comma is the decimal"])
        let fmt = popup(amountFormats.map(\.0))
        let g = grid([[label("Amounts are written"), dec], [label("Show amounts as"), fmt]])
        run(window, title: "Standardize Amounts", message: "Currency symbols, grouping, (negatives), CR/DR and stray spaces are cleaned up.", accessory: g, ok: "Standardize") {
            done(dec.indexOfSelectedItem == 1, amountFormats[fmt.indexOfSelectedItem].1)
        }
    }

    static func textToColumns(_ window: NSWindow, done: @escaping (String) -> Void) {
        let delims: [(String, String)] = [("Comma  ,", ","), ("Semicolon  ;", ";"), ("Tab", "\t"), ("Space", " "), ("Pipe  |", "|"), ("Dash  -", "-")]
        let p = popup(delims.map(\.0) + ["Other…"])
        let other = NSTextField(string: "")
        other.placeholderString = "Custom delimiter"
        other.frame.size.width = 120
        let g = grid([[label("Split at"), p], [label("Custom"), other]])
        run(window, title: "Split Text into Columns", message: "New columns are inserted to the right of the selected column.", accessory: g, ok: "Split") {
            let i = p.indexOfSelectedItem
            done(i < delims.count ? delims[i].1 : other.stringValue)
        }
    }

    static func duplicates(_ window: NSWindow, columns: [String], done: @escaping (_ cols: [Int], _ header: Bool) -> Void) {
        let stack = NSStackView()
        stack.orientation = .vertical
        stack.alignment = .leading
        let header = NSButton(checkboxWithTitle: "First row is a header (keep it)", target: nil, action: nil)
        header.state = .on
        stack.addArrangedSubview(header)
        stack.addArrangedSubview(NSTextField(labelWithString: "Rows are duplicates when these columns match:"))
        var boxes: [NSButton] = []
        for c in columns.prefix(40) {
            let b = NSButton(checkboxWithTitle: c, target: nil, action: nil)
            b.state = .on
            boxes.append(b)
            stack.addArrangedSubview(b)
        }
        let scroll = NSScrollView(frame: NSRect(x: 0, y: 0, width: 300, height: min(320, CGFloat(boxes.count + 2) * 22 + 10)))
        scroll.hasVerticalScroller = true
        scroll.documentView = stack
        stack.frame.size = stack.fittingSize
        run(window, title: "Remove Duplicate Rows", accessory: scroll, ok: "Remove") {
            let cols = boxes.enumerated().filter { $0.element.state == .on }.map(\.offset)
            done(cols.count == boxes.count ? [] : cols, header.state == .on)
        }
    }
}
