import AppKit
import WaffleBridge
import CWaffle

extension SheetWindowController {
    // ---- structure -----------------------------------------------------------------------------

    var rowSpan: (Int, Int) { let r = grid.selection.primary; return (r.r0, r.isFullCols ? 1 : r.rows) }
    var colSpan: (Int, Int) { let r = grid.selection.primary; return (r.c0, r.isFullRows ? 1 : r.cols) }

    @objc func insertRowsAbove(_ sender: Any?) { let (a, n) = rowSpan; run { book.insertRows(sheet, at: a, count: n) } }
    @objc func insertRowsBelow(_ sender: Any?) {
        let r = grid.selection.primary
        let (_, n) = rowSpan
        run { book.insertRows(sheet, at: r.r1 + 1, count: n) }
    }
    @objc func insertColumnsLeft(_ sender: Any?) { let (a, n) = colSpan; run { book.insertCols(sheet, at: a, count: n) } }
    @objc func insertColumnsRight(_ sender: Any?) {
        let r = grid.selection.primary
        let (_, n) = colSpan
        run { book.insertCols(sheet, at: r.c1 + 1, count: n) }
    }
    @objc func deleteRows(_ sender: Any?) {
        let r = grid.selection.primary
        run { book.deleteRows(sheet, at: r.r0, count: min(r.rows, max(1, book.dataRows(sheet) - r.r0))) }
    }
    @objc func deleteColumns(_ sender: Any?) {
        let r = grid.selection.primary
        run { book.deleteCols(sheet, at: r.c0, count: min(r.cols, max(1, book.dataCols(sheet) - r.c0))) }
    }
    @objc func hideRows(_ sender: Any?) { let r = grid.selection.primary; run { book.setHidden(sheet, rows: true, r.r0, min(r.r1, book.rows(sheet) - 1), true) } }
    @objc func unhideRows(_ sender: Any?) {
        let r = grid.selection.primary
        let a = max(0, r.r0 - 1), b = min(r.r1 + 1, book.rows(sheet) - 1)
        run { book.setHidden(sheet, rows: true, r.isFullCols ? 0 : a, r.isFullCols ? book.rows(sheet) - 1 : b, false) }
    }
    @objc func hideColumns(_ sender: Any?) { let r = grid.selection.primary; run { book.setHidden(sheet, rows: false, r.c0, min(r.c1, book.cols(sheet) - 1), true) } }
    @objc func unhideColumns(_ sender: Any?) {
        let r = grid.selection.primary
        let a = max(0, r.c0 - 1), b = min(r.c1 + 1, book.cols(sheet) - 1)
        run { book.setHidden(sheet, rows: false, r.isFullRows ? 0 : a, r.isFullRows ? book.cols(sheet) - 1 : b, false) }
    }
    @objc func autofitColumns(_ sender: Any?) {
        let r = grid.selection.primary
        grid.canvas.autofitColumns(r.c0...min(r.c1, max(r.c0, book.dataCols(sheet) - 1)))
    }
    @objc func mergeCells(_ sender: Any?) { run { book.merge(sheet, dataRectOr(grid.selection.primary)) } }
    @objc func unmergeCells(_ sender: Any?) { run { book.unmerge(sheet, dataRectOr(grid.selection.primary)) } }

    @objc func freezeTopRow(_ sender: Any?) { run { book.setFreeze(sheet, rows: 1, cols: 0) }; grid.reload() }
    @objc func freezeFirstColumn(_ sender: Any?) { run { book.setFreeze(sheet, rows: 0, cols: 1) }; grid.reload() }
    @objc func freezeAtSelection(_ sender: Any?) {
        let p = grid.selection.active
        run { book.setFreeze(sheet, rows: p.r, cols: p.c) }
        grid.reload()
    }
    @objc func unfreeze(_ sender: Any?) { run { book.setFreeze(sheet, rows: 0, cols: 0) }; grid.reload() }

    func run(_ f: () -> Bool) {
        commitEditing()
        if f() { edited() } else { fail() }
    }
}
