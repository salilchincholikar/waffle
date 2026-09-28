import AppKit
import WaffleBridge
import CWaffle

extension SheetWindowController {
    // ---- find (shared across all tabs/windows) ---------------------------------------------------

    var fc: FindCenter { FindCenter.shared }

    @objc func showFind(_ sender: Any?) { openFind(replace: false) }
    @objc func showFindReplace(_ sender: Any?) { openFind(replace: true) }

    func openFind(replace: Bool) {
        fc.visible = true
        // Seed the search with the selected cell's text when the field is empty.
        if fc.query.isEmpty, !grid.selection.primary.isSingle || !book.isBlank(sheet, grid.selection.active) {
            let t = book.displayText(sheet, grid.selection.active)
            if !t.isEmpty && t.count < 60 { fc.query = t }
        }
        fc.post()
        fc.refresh(from: self)
        window?.makeFirstResponder(findBar.search)
        findBar.search.selectText(nil)
        if replace { findBar.openReplace() }
    }

    @objc func findNext(_ sender: Any?) { find(forward: true) }
    @objc func findPrevious(_ sender: Any?) { find(forward: false) }

    func find(forward: Bool) {
        commitEditing()
        if findBar.search.stringValue != fc.query { fc.query = findBar.search.stringValue }
        guard !fc.query.isEmpty else { showFind(nil); return }
        fc.step(from: self, forward: forward)
    }

    func replaceCurrent() {
        guard !fc.query.isEmpty else { return }
        if let h = fc.currentHit, let d = h.doc, let wc = d.windowControllers.first as? SheetWindowController, let b = d.book {
            if b.replaceOne(h.sheet, h.pos, fc.query, fc.replacement, fc.flags) { wc.edited() }
        }
        fc.invalidate()
        find(forward: true)
    }

    func replaceAll() {
        guard !fc.query.isEmpty else { return }
        let n = fc.replaceAll(from: self)
        findBar.count.stringValue = n == 0 ? "No matches" : "\(n) replaced"
    }

    func closeFind() {
        fc.visible = false
        fc.post()
        window?.makeFirstResponder(grid.canvas)
    }

    /// Cells changed (edit, undo, redo): the matches may have too. Find again right away
    /// while a find is showing, so the count and highlights stay current.
    func refindAfterChange() {
        guard !fc.query.isEmpty else { return }
        fc.invalidate()
        guard fc.visible else { return }
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            fc.refresh(from: self)
        }
    }

    /// Mirror the shared find state into this window (bar contents, visibility, highlights).
    @objc func findChanged(_ n: Notification) {
        findBar.sync(from: fc)
        let (all, cur) = fc.matches(in: doc, sheet: sheet)
        grid.canvas.highlights = fc.visible ? all : []
        grid.canvas.currentHighlight = fc.visible ? cur : nil
    }
}
