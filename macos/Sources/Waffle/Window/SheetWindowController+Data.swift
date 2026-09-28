import AppKit
import WaffleBridge
import CWaffle

extension SheetWindowController {
    // ---- data -----------------------------------------------------------------------------------

    func sortRange(header: Bool) -> (Int, Int) {
        let r = grid.selection.primary
        let rows = book.dataRows(sheet)
        var r0 = r.isSingle || r.isFullCols ? 0 : r.r0
        let r1 = r.isSingle || r.isFullCols ? rows - 1 : min(r.r1, rows - 1)
        if header && (r.isSingle || r.isFullCols || r.r0 == 0) { r0 += 1 }
        return (r0, r1)
    }

    func headerNames() -> [String] {
        let cols = max(1, book.dataCols(sheet))
        return (0..<cols).map { c in
            let v = book.displayText(sheet, CellPos(r: 0, c: c))
            return v.isEmpty ? "Column \(columnName(c))" : "\(columnName(c)) — \(v)"
        }
    }

    @objc func sortAscending(_ sender: Any?) { quickSort(true) }
    @objc func sortDescending(_ sender: Any?) { quickSort(false) }
    func quickSort(_ asc: Bool) {
        let (r0, r1) = sortRange(header: looksLikeHeader())
        run { book.sort(sheet, r0: r0, r1: r1, keys: [(grid.selection.active.c, asc)]) }
    }

    /// First row is text while the second row has a number in the same column.
    func looksLikeHeader() -> Bool {
        let c = grid.selection.active.c
        let a = book.fetch(sheet, CellRect(r0: 0, c0: c, r1: 1, c1: c)).0
        guard a.count == 2 else { return true }
        return a[0].kind == 2 && (a[1].kind != 2 || grid.selection.primary.isSingle)
    }

    @objc func customSort(_ sender: Any?) {
        Dialogs.sort(window!, columns: headerNames(), initial: grid.selection.active.c) { [weak self] keys, header in
            guard let self, !keys.isEmpty else { return }
            let (r0, r1) = self.sortRange(header: header)
            self.run { self.book.sort(self.sheet, r0: r0, r1: r1, keys: keys.map { ($0.0, $0.1) }) }
        }
    }

    @objc func toggleFilter(_ sender: Any?) {
        let c = grid.canvas
        if c.filterMode {
            book.clearFilters(sheet)
            c.filterMode = false
            grid.updateInsets()
        } else {
            let fr = book.freeze(sheet).rows
            filterHeader = fr > 0 ? fr - 1 : (grid.selection.primary.isSingle ? 0 : grid.selection.primary.r0)
            c.filterHeaderRow = filterHeader
            c.filterMode = true
        }
        updateStatus()
    }

    @objc func clearAllFilters(_ sender: Any?) {
        book.clearFilters(sheet)
        grid.updateInsets()
        grid.canvas.needsDisplay = true
    }

    @objc func filterBySelection(_ sender: Any?) {
        let p = grid.selection.active
        if !grid.canvas.filterMode { toggleFilter(nil) }
        let v = book.displayText(sheet, p)
        book.setFilter(sheet, header: filterHeader, col: p.c, .values, v)
        grid.updateInsets(); grid.canvas.needsDisplay = true
    }

    func report(_ n: Int, _ what: String) {
        if n < 0 { fail(); return }
        if n > 0 { edited() }
        bottom.status.stringValue = n == 0 ? "Nothing to change" : "\(n) \(what)"
    }

    var cleanupRects: [CellRect] { rects.map { ($0.isFullCols || $0.isFullRows || $0.isSingle) ? CellRect(r0: $0.isSingle ? 0 : $0.r0, c0: $0.isSingle ? 0 : $0.c0, r1: $0.isSingle ? CellRect.maxRows - 1 : $0.r1, c1: $0.isSingle ? CellRect.maxCols - 1 : $0.c1) : $0 } }

    @objc func trimSpaces(_ sender: Any?) { commitEditing(); report(book.transform(sheet, cleanupRects, .trim), "cells trimmed") }
    @objc func upperCase(_ sender: Any?) { commitEditing(); report(book.transform(sheet, rects, .upper), "cells changed") }
    @objc func lowerCase(_ sender: Any?) { commitEditing(); report(book.transform(sheet, rects, .lower), "cells changed") }
    @objc func titleCase(_ sender: Any?) { commitEditing(); report(book.transform(sheet, rects, .title), "cells changed") }
    @objc func removeEmptyRows(_ sender: Any?) {
        commitEditing()
        let r = grid.selection.primary
        let (r0, r1) = r.isSingle || r.isFullCols ? (0, book.dataRows(sheet) - 1) : (r.r0, r.r1)
        report(book.removeEmptyRows(sheet, r0: r0, r1: r1), "empty rows removed")
    }
    @objc func removeDuplicates(_ sender: Any?) {
        commitEditing()
        Dialogs.duplicates(window!, columns: headerNames()) { [weak self] cols, header in
            guard let self else { return }
            let r = self.grid.selection.primary
            var (r0, r1) = r.isSingle || r.isFullCols ? (0, self.book.dataRows(self.sheet) - 1) : (r.r0, r.r1)
            if header { r0 += 1 }
            _ = r1
            r1 = min(r1, self.book.dataRows(self.sheet) - 1)
            self.report(self.book.removeDuplicates(self.sheet, r0: r0, r1: r1, cols: cols), "duplicate rows removed")
        }
    }
    @objc func standardizeDates(_ sender: Any?) {
        commitEditing()
        Dialogs.dates(window!) { [weak self] order, fmt in
            guard let self else { return }
            self.report(self.book.normalizeDates(self.sheet, self.cleanupRects, order: order, format: fmt), "dates standardized")
        }
    }
    @objc func standardizeAmounts(_ sender: Any?) {
        commitEditing()
        Dialogs.amounts(window!) { [weak self] comma, fmt in
            guard let self else { return }
            self.report(self.book.normalizeAmounts(self.sheet, self.cleanupRects, decimalComma: comma, format: fmt), "amounts standardized")
        }
    }
    @objc func textToColumns(_ sender: Any?) {
        commitEditing()
        let col = grid.selection.active.c
        Dialogs.textToColumns(window!) { [weak self] delim in
            guard let self else { return }
            let n = self.book.textToColumns(self.sheet, col: col, r0: 0, r1: self.book.dataRows(self.sheet) - 1, delimiter: delim)
            if n < 0 { self.fail() } else if n == 0 { self.bottom.status.stringValue = "Nothing to split" } else { self.edited(); self.bottom.status.stringValue = "Split into \(n) columns" }
        }
    }
}
