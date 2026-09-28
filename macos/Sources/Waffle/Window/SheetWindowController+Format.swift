import AppKit
import WaffleBridge
import CWaffle

extension SheetWindowController {
    // ---- formatting ----------------------------------------------------------------------------

    var activeStyle: WfStyle { book.style(book.cellStyle(sheet, grid.selection.active)) }

    func applyStyle(_ kind: Book.StyleKind, num: Double = 0, text: String = "", color: Double = -1) {
        commitEditing()
        if book.applyStyle(sheet, rects, kind, num: num, text: text, color: color) { edited(); updateFormatControls() } else { fail() }
    }

    @objc func toggleBold(_ sender: Any?) { applyStyle(.bold, num: activeStyle.bold != 0 ? 0 : 1) }
    @objc func toggleItalic(_ sender: Any?) { applyStyle(.italic, num: activeStyle.italic != 0 ? 0 : 1) }
    @objc func toggleUnderline(_ sender: Any?) { applyStyle(.underline, num: activeStyle.underline != 0 ? 0 : 1) }
    @objc func toggleStrike(_ sender: Any?) { applyStyle(.strike, num: activeStyle.strike != 0 ? 0 : 1) }
    @objc func biggerFont(_ sender: Any?) { applyStyle(.fontSize, num: Double(activeStyle.font_size) + 1) }
    @objc func smallerFont(_ sender: Any?) { applyStyle(.fontSize, num: max(6, Double(activeStyle.font_size) - 1)) }
    @objc func alignLeft(_ sender: Any?) { applyStyle(.hAlign, num: 1) }
    @objc func alignCenter(_ sender: Any?) { applyStyle(.hAlign, num: 2) }
    @objc func alignRight(_ sender: Any?) { applyStyle(.hAlign, num: 3) }
    @objc func alignGeneral(_ sender: Any?) { applyStyle(.hAlign, num: 0) }
    @objc func alignTop(_ sender: Any?) { applyStyle(.vAlign, num: 2) }
    @objc func alignMiddle(_ sender: Any?) { applyStyle(.vAlign, num: 1) }
    @objc func alignBottom(_ sender: Any?) { applyStyle(.vAlign, num: 0) }
    @objc func toggleWrap(_ sender: Any?) { applyStyle(.wrap, num: activeStyle.wrap != 0 ? 0 : 1) }
    @objc func noFill(_ sender: Any?) { applyStyle(.fill, num: -1) }
    @objc func automaticTextColor(_ sender: Any?) { applyStyle(.textColor, num: -1) }

    static let numberFormats: [(String, String)] = [
        ("General", "General"),
        ("Number  1,234.56", "#,##0.00"),
        ("Whole Number  1,234", "#,##0"),
        ("Rupee  ₹ 1,234.56", "[$₹-4009] #,##0.00"),
        ("Dollar  $1,234.56", "\"$\"#,##0.00"),
        ("Accounting", "_(\"$\"* #,##0.00_);_(\"$\"* \\(#,##0.00\\);_(\"$\"* \"-\"??_);_(@_)"),
        ("Percent  12%", "0%"),
        ("Percent  12.34%", "0.00%"),
        ("Scientific  1.23E+04", "0.00E+00"),
        ("Date  2024-03-05", "yyyy-mm-dd"),
        ("Date  05/03/2024", "dd/mm/yyyy"),
        ("Date  03/05/2024", "mm/dd/yyyy"),
        ("Date  05-Mar-2024", "dd-mmm-yyyy"),
        ("Date & Time", "yyyy-mm-dd hh:mm"),
        ("Time  14:30", "hh:mm"),
        ("Text", "@"),
    ]

    @objc func setNumberFormat(_ sender: NSMenuItem) {
        guard let code = sender.representedObject as? String else { return }
        applyStyle(.numberFormat, text: code)
    }
    @objc func customNumberFormat(_ sender: Any?) {
        let cur = book.numberFormat(book.cellStyle(sheet, grid.selection.active))
        Dialogs.askText(window!, title: "Custom Number Format", message: "Excel format code, e.g. #,##0.00;[Red]-#,##0.00", value: cur, ok: "Apply") { [weak self] code in
            self?.applyStyle(.numberFormat, text: code)
        }
    }
    @objc func increaseDecimals(_ sender: Any?) { changeDecimals(1) }
    @objc func decreaseDecimals(_ sender: Any?) { changeDecimals(-1) }
    func changeDecimals(_ d: Int) {
        var code = book.numberFormat(book.cellStyle(sheet, grid.selection.active))
        if code == "General" { code = d > 0 ? "0.0" : "0"; applyStyle(.numberFormat, text: code); return }
        // Adjust the first section's decimals.
        var sections = code.components(separatedBy: ";")
        var s = sections[0]
        if let dot = s.range(of: ".") {
            var i = dot.upperBound
            var n = 0
            while i < s.endIndex, s[i] == "0" || s[i] == "#" { n += 1; i = s.index(after: i) }
            let newN = max(0, n + d)
            s.replaceSubrange(dot.lowerBound..<i, with: newN == 0 ? "" : "." + String(repeating: "0", count: newN))
        } else if d > 0, let last = s.lastIndex(where: { $0 == "0" || $0 == "#" }) {
            s.insert(contentsOf: ".0", at: s.index(after: last))
        }
        sections[0] = s
        applyStyle(.numberFormat, text: sections.joined(separator: ";"))
    }

    @objc func borderAll(_ sender: Any?) { applyStyle(.borders, num: 1, text: "lrtb", color: -1) }
    @objc func borderBottom(_ sender: Any?) {
        commitEditing()
        let r = dataRectOr(grid.selection.primary)
        if book.applyStyle(sheet, [CellRect(r0: r.r1, c0: r.c0, r1: r.r1, c1: r.c1)], .borders, num: 1, text: "b") { edited() }
    }
    @objc func borderThickBottom(_ sender: Any?) {
        commitEditing()
        let r = dataRectOr(grid.selection.primary)
        if book.applyStyle(sheet, [CellRect(r0: r.r1, c0: r.c0, r1: r.r1, c1: r.c1)], .borders, num: 5, text: "b") { edited() }
    }
    @objc func borderOutline(_ sender: Any?) {
        commitEditing()
        let r = dataRectOr(grid.selection.primary)
        var ok = book.applyStyle(sheet, [CellRect(r0: r.r0, c0: r.c0, r1: r.r0, c1: r.c1)], .borders, num: 1, text: "t")
        ok = book.applyStyle(sheet, [CellRect(r0: r.r1, c0: r.c0, r1: r.r1, c1: r.c1)], .borders, num: 1, text: "b") && ok
        ok = book.applyStyle(sheet, [CellRect(r0: r.r0, c0: r.c0, r1: r.r1, c1: r.c0)], .borders, num: 1, text: "l") && ok
        ok = book.applyStyle(sheet, [CellRect(r0: r.r0, c0: r.c1, r1: r.r1, c1: r.c1)], .borders, num: 1, text: "r") && ok
        if ok { edited() } else { fail() }
    }
    @objc func borderNone(_ sender: Any?) { applyStyle(.borders, num: 0, text: "lrtb") }
}
