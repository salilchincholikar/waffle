import AppKit
import WaffleBridge
import CWaffle

extension SheetWindowController {
    // ---- undo -----------------------------------------------------------------------------

    @objc func undo(_ sender: Any?) {
        commitEditing()
        guard let s = book.undo() else { NSSound.beep(); return }
        doc.updateChangeCount(.changeUndone)
        afterHistory(s)
    }

    @objc func redo(_ sender: Any?) {
        commitEditing()
        guard let s = book.redo() else { NSSound.beep(); return }
        doc.updateChangeCount(.changeRedone)
        afterHistory(s)
    }

    func afterHistory(_ s: Int) {
        refreshTabs()
        if s != sheet, s < book.sheetCount { book.activeSheet = s; grid.sheet = s; bottom.tabs.selected = s }
        syncSavedFilter()
        grid.reload()
        updateFormulaBar()
        updateStatus()
        refindAfterChange()
    }

    // ---- clipboard --------------------------------------------------------------------------

    static let clipType = NSPasteboard.PasteboardType("com.salilchincholikar.waffle.clip")

    @objc func copy(_ sender: Any?) {
        commitEditing()
        let r = dataRectForCopy()
        let tsv = book.copy(sheet, r)
        let pb = NSPasteboard.general
        pb.clearContents()
        pb.setString(tsv, forType: .string)
        pb.setString(clipToken, forType: Self.clipType)
        cutSource = nil
        grid.canvas.cutRect = nil
    }

    @objc func cut(_ sender: Any?) {
        copy(sender)
        cutSource = (sheet, dataRectForCopy())
        grid.canvas.cutRect = dataRectForCopy()
    }

    func dataRectForCopy() -> CellRect {
        let r = grid.selection.primary
        if r.isFullCols || r.isFullRows { return dataRect }
        return r
    }

    func pasteRect(_ what: Book.PasteWhat) {
        commitEditing()
        let pb = NSPasteboard.general
        let target = grid.selection.primary
        let ours = pb.string(forType: Self.clipType) == clipToken
        var result: CellRect?
        if ours {
            let t = target.isSingle ? CellRect(r0: target.r0, c0: target.c0, r1: target.r0, c1: target.c0) : target
            result = book.pasteClip(sheet, t, what)
            if let cut = cutSource, result != nil, cut.sheet == sheet, !(cut.rect.intersects(result!)) {
                _ = book.clear(sheet, [cut.rect], .all)
            }
            cutSource = nil
            grid.canvas.cutRect = nil
        } else if let s = pb.string(forType: .string) {
            result = book.pasteText(sheet, at: CellPos(r: target.r0, c: target.c0), s)
        }
        guard let r = result else { if ours || pb.string(forType: .string) != nil { fail() }; return }
        grid.selection = Selection(active: CellPos(r: r.r0, c: r.c0), anchor: CellPos(r: r.r0, c: r.c0), ranges: [r])
        edited()
    }

    @objc func paste(_ sender: Any?) { pasteRect(.all) }
    @objc func pasteValues(_ sender: Any?) { pasteRect(.values) }
    @objc func pasteFormats(_ sender: Any?) { pasteRect(.formats) }

    @objc func delete(_ sender: Any?) {
        if book.clear(sheet, rects, .contents) { edited() } else { fail() }
    }
    @objc func clearFormats(_ sender: Any?) {
        if book.clear(sheet, rects, .formats) { edited() } else { fail() }
    }
    @objc func clearAll(_ sender: Any?) {
        if book.clear(sheet, rects, .all) { edited() } else { fail() }
    }

    @objc override func selectAll(_ sender: Any?) { grid.canvas.selectAll() }

    @objc func fillDown(_ sender: Any?) {
        let r = grid.selection.primary
        if r.rows < 2 { NSSound.beep(); return }
        if book.fill(sheet, dataRectOr(r), down: true) { edited() } else { fail() }
    }
    @objc func fillRight(_ sender: Any?) {
        let r = grid.selection.primary
        if r.cols < 2 { NSSound.beep(); return }
        if book.fill(sheet, dataRectOr(r), down: false) { edited() } else { fail() }
    }
    func dataRectOr(_ r: CellRect) -> CellRect { (r.isFullCols || r.isFullRows) ? dataRect : r }

    @objc func editCell(_ sender: Any?) { gridBeginEditing(grid, initial: nil, select: false) }
}
