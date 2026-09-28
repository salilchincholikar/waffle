import AppKit
import WaffleBridge
import CWaffle

extension GridCanvas {
    // ---- undo / redo ------------------------------------------------------------------------
    // NSWindow answers undo:/redo: itself (via its own, empty undo manager) before the
    // window controller is asked, so the first responder has to claim them.

    var controller: SheetWindowController? { window?.windowController as? SheetWindowController }

    @objc func undo(_ sender: Any?) { controller?.undo(sender) }
    @objc func redo(_ sender: Any?) { controller?.redo(sender) }

    func validateMenuItem(_ item: NSMenuItem) -> Bool {
        if item.action == #selector(undo(_:)) || item.action == #selector(redo(_:)) {
            return controller?.validateMenuItem(item) ?? false
        }
        return true
    }

    // ---- keyboard --------------------------------------------------------------------------

    func extend(to pos: CellPos) {
        let r = expandForMerges(CellRect(selection.anchor, pos))
        if selection.ranges.isEmpty { selection.ranges = [r] } else { selection.ranges[selection.ranges.count - 1] = r }
    }

    /// Move the active cell, skipping hidden rows/columns and merged interiors.
    func move(dr: Int, dc: Int, extendSelection: Bool, jump: Bool = false) {
        guard let book else { return }
        let from: CellPos = extendSelection ? selectionEdge(dr: dr, dc: dc) : selection.active
        var p = from
        if jump {
            p = book.jump(sheet, from: from, dr: dr.signum(), dc: dc.signum())
        } else {
            if let m = book.merge(at: from, sheet: sheet), !extendSelection {
                if dr > 0 { p.r = m.r1 } else if dc > 0 { p.c = m.c1 }
            }
            p.r = max(0, min(book.rows(sheet) - 1, p.r + dr))
            p.c = max(0, min(book.cols(sheet) - 1, p.c + dc))
            while dr != 0, p.r > 0, p.r < book.rows(sheet) - 1, book.rowHidden(sheet, p.r) { p.r += dr.signum() }
            while dc != 0, p.c > 0, p.c < book.cols(sheet) - 1, book.colHidden(sheet, p.c) { p.c += dc.signum() }
        }
        if extendSelection {
            extend(to: p)
        } else {
            let m = book.merge(at: p, sheet: sheet)
            let a = m.map { CellPos(r: $0.r0, c: $0.c0) } ?? p
            selection = Selection(active: a, anchor: a, ranges: [m ?? CellRect(a)])
        }
        grid?.scrollToVisible(extendSelection ? p : selection.active)
        notifySelection()
    }

    /// The far corner of the current range, used when extending with Shift+arrow.
    func selectionEdge(dr: Int, dc: Int) -> CellPos {
        let r = selection.primary
        let a = selection.anchor
        return CellPos(r: a.r == r.r0 ? r.r1 : r.r0, c: a.c == r.c0 ? r.c1 : r.c0)
    }

    /// Enter/Tab movement within a multi-cell selection (wraps like Excel).
    func advance(dr: Int, dc: Int) {
        let r = selection.primary
        if r.isSingle || r.isFullCols || r.isFullRows {
            move(dr: dr, dc: dc, extendSelection: false)
            return
        }
        var p = selection.active
        if dc != 0 {
            p.c += dc
            if p.c > r.c1 { p.c = r.c0; p.r += 1 } else if p.c < r.c0 { p.c = r.c1; p.r -= 1 }
            if p.r > r.r1 { p.r = r.r0 } else if p.r < r.r0 { p.r = r.r1 }
        } else {
            p.r += dr
            if p.r > r.r1 { p.r = r.r0; p.c += 1 } else if p.r < r.r0 { p.r = r.r1; p.c -= 1 }
            if p.c > r.c1 { p.c = r.c0 } else if p.c < r.c0 { p.c = r.c1 }
        }
        selection.active = p
        grid?.scrollToVisible(p)
        notifySelection()
    }

    override func keyDown(with event: NSEvent) {
        let flags = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
        let shift = flags.contains(.shift)
        let cmd = flags.contains(.command)
        let opt = flags.contains(.option)
        let pageRows = max(1, Int(mainArea.height / (20 * zoom)) - 1)
        switch event.keyCode {
        case 126: move(dr: -1, dc: 0, extendSelection: shift, jump: cmd); return          // up
        case 125: move(dr: 1, dc: 0, extendSelection: shift, jump: cmd); return           // down
        case 123: move(dr: 0, dc: -1, extendSelection: shift, jump: cmd); return          // left
        case 124: move(dr: 0, dc: 1, extendSelection: shift, jump: cmd); return           // right
        case 116: if opt { move(dr: 0, dc: -pageRows / 3, extendSelection: shift) } else { move(dr: -pageRows, dc: 0, extendSelection: shift) }; return // page up
        case 121: if opt { move(dr: 0, dc: pageRows / 3, extendSelection: shift) } else { move(dr: pageRows, dc: 0, extendSelection: shift) }; return  // page down
        case 115: // home
            if cmd { selection.set(CellPos(r: 0, c: 0)) } else { selection.set(CellPos(r: selection.active.r, c: 0)) }
            grid?.scrollToVisible(selection.active); notifySelection(); return
        case 119: // end
            if let book {
                let p = CellPos(r: max(0, book.dataRows(sheet) - 1), c: max(0, book.dataCols(sheet) - 1))
                selection.set(cmd ? p : CellPos(r: selection.active.r, c: p.c))
                grid?.scrollToVisible(selection.active); notifySelection()
            }
            return
        case 36, 76: // return / enter
            if cmd { return }
            advance(dr: shift ? -1 : 1, dc: 0); return
        case 48: // tab (Ctrl+Tab switches window tabs, like a browser)
            if flags.contains(.control) {
                if shift { window?.selectPreviousTab(nil) } else { window?.selectNextTab(nil) }
                return
            }
            advance(dr: 0, dc: shift ? -1 : 1); return
        case 51, 117: // delete / forward delete
            if let book, book.clear(sheet, selection.ranges, .contents) { didEdit() }
            return
        case 53: // escape
            if cutRect != nil { cutRect = nil }
            return
        case 120: // F2
            grid?.delegate?.gridBeginEditing(grid!, initial: nil, select: false); return
        default: break
        }
        if flags.contains(.control), event.charactersIgnoringModifiers == "u" {
            grid?.delegate?.gridBeginEditing(grid!, initial: nil, select: false); return
        }
        if !cmd && !flags.contains(.control), let chars = event.characters, !chars.isEmpty,
           let scalar = chars.unicodeScalars.first, scalar.value >= 32, scalar.value != 127, !(0xF700...0xF8FF).contains(scalar.value) {
            // Typing replaces the cell: start the editor and let it handle this key (incl. IME).
            grid?.delegate?.gridBeginEditing(grid!, initial: "", select: false)
            if let ed = editor, let fe = ed.currentEditor() {
                fe.interpretKeyEvents([event])
            }
            return
        }
        super.keyDown(with: event)
    }
}
