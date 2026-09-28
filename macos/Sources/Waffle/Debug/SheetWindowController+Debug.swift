import AppKit
import WaffleBridge
import CWaffle

// Debug-only hooks driven by environment variables (see docs/development.md).
// Compiled into debug builds only (`WAFFLE_DEBUG_HOOKS`); release builds get no-ops.

#if WAFFLE_DEBUG_HOOKS
/// Hooks run once per launch (attach() runs again after Revert, which the self-test uses).
private var debugHooksRan = false

extension SheetWindowController {
    /// Debug aid: WAFFLE_SELFTEST=/out.xlsx drives common commands through the real
    /// controller paths, saves, prints a report and quits.
    func selfTestIfRequested() -> Bool {
        let env = ProcessInfo.processInfo.environment
        guard let out = env["WAFFLE_SELFTEST"] else { return false }
        var log: [String] = []
        func check(_ name: String, _ ok: Bool) { log.append((ok ? "ok   " : "FAIL ") + name) }
        // Anything asynchronous (Find refreshes, loading) is waited for, never slept for:
        // CI machines are slower than a laptop. The deadline turns a hang into a FAIL.
        func waitFor(_ seconds: TimeInterval = 5, _ ok: () -> Bool) -> Bool {
            let end = Date().addingTimeInterval(seconds)
            while !ok() && Date() < end { RunLoop.main.run(until: Date().addingTimeInterval(0.02)) }
            return ok()
        }
        // Menu routing: who actually receives Edit ▸ Undo / Redo from the grid?
        window?.makeFirstResponder(grid.canvas)
        // Walk the chain the way AppKit does: responders, then the window's delegate.
        func route(_ sel: Selector) -> AnyObject? {
            var r: NSResponder? = grid.canvas
            while let x = r {
                if x.responds(to: sel) { return x }
                r = x.nextResponder
            }
            if let d = window?.delegate as? NSObject, d.responds(to: sel) { return d }
            return nil
        }

        for sel in [#selector(undo(_:)), #selector(redo(_:)), #selector(copy(_:)), #selector(paste(_:)), #selector(selectAll(_:)), #selector(delete(_:)), #selector(toggleBold(_:))] {
            let t = route(sel)
            check("route \(sel) -> \(t.map { String(describing: type(of: $0)) } ?? "nil")", t is SheetWindowController || t is GridCanvas)
        }
        let c = grid.canvas
        // Undo/redo exactly as the Edit menu delivers them.
        grid.selection.set(CellPos(r: 30, c: 5))
        _ = book.setInput(sheet, CellPos(r: 30, c: 5), "menu-undo")
        edited()
        let item = NSMenuItem(title: "Undo", action: #selector(undo(_:)), keyEquivalent: "z")
        check("undo menu enabled", (route(#selector(undo(_:))) as? NSMenuItemValidation)?.validateMenuItem(item) == true && item.title == "Undo Typing")
        _ = (route(#selector(undo(_:))) as? NSObject)?.perform(#selector(undo(_:)), with: nil)
        check("undo via menu", book.displayText(sheet, CellPos(r: 30, c: 5)).isEmpty)
        _ = (route(#selector(redo(_:))) as? NSObject)?.perform(#selector(redo(_:)), with: nil)
        check("redo via menu", book.displayText(sheet, CellPos(r: 30, c: 5)) == "menu-undo")
        // Typing through the editor path.
        grid.selection.set(CellPos(r: 0, c: 0))
        gridBeginEditing(grid, initial: "", select: false)
        c.editor?.stringValue = "hello"
        c.editor?.commit(move: (1, 0))
        check("type A1", book.displayText(sheet, CellPos(r: 0, c: 0)) == "hello")
        check("moved down", grid.selection.active == CellPos(r: 1, c: 0))
        gridBeginEditing(grid, initial: "", select: false)
        c.editor?.stringValue = "1,234.5"
        c.editor?.commit(move: nil)
        check("number parse", book.displayText(sheet, CellPos(r: 1, c: 0)) == "1,234.50" || book.kind == .csv)
        // Formatting
        if book.kind != .csv {
            toggleBold(nil)
            check("bold", activeStyle.bold != 0)
            undo(nil)
            check("undo bold", activeStyle.bold == 0)
            redo(nil)
            check("redo bold", activeStyle.bold != 0)
        }
        // Clipboard
        grid.selection = Selection(active: CellPos(r: 0, c: 0), anchor: CellPos(r: 0, c: 0), ranges: [CellRect(r0: 0, c0: 0, r1: 1, c1: 0)])
        copy(nil)
        grid.selection.set(CellPos(r: 5, c: 3))
        paste(nil)
        check("paste", book.displayText(sheet, CellPos(r: 5, c: 3)) == "hello")
        // Structure
        grid.selection.set(CellPos(r: 0, c: 0))
        insertRowsAbove(nil)
        check("insert row", book.displayText(sheet, CellPos(r: 1, c: 0)) == "hello")
        undo(nil)
        check("undo insert", book.displayText(sheet, CellPos(r: 0, c: 0)) == "hello")
        // Sort + find + filter
        grid.selection.set(CellPos(r: 0, c: 3))
        sortAscending(nil)
        findBar.search.stringValue = "hello"
        grid.selection.set(CellPos(r: 0, c: 0))
        find(forward: true)
        check("find", book.displayText(sheet, grid.selection.active) == "hello")
        // Find stays live across edits: with Find showing, a new match counts right away.
        let fcs = FindCenter.shared
        fcs.query = "hello"
        fcs.visible = true
        fcs.refresh(from: self)
        let before = fcs.hits.count
        _ = book.setInput(sheet, CellPos(r: 40, c: 5), "hello")
        edited()
        check("find refreshes after edit", waitFor { fcs.hits.count == before + 1 && !fcs.status.isEmpty })
        undo(nil)
        check("find refreshes after undo", waitFor { fcs.hits.count == before })
        findChanged(Notification(name: FindCenter.changed))
        let lit = grid.canvas.highlights.count
        _ = book.setColWidth(sheet, 0, 0, px: 150)
        gridDidEdit(grid)
        let kept = waitFor { grid.canvas.highlights.count == lit }
        check("find highlights survive a column resize (\(lit) → \(grid.canvas.highlights.count))", lit > 0 && kept)
        fcs.visible = false
        toggleFilter(nil)
        book.setFilter(sheet, header: 0, col: 0, .values, "hello")
        check("filter hides", (1..<min(50, book.dataRows(sheet))).contains { book.rowHidden(sheet, $0) } || book.dataRows(sheet) < 3)
        toggleFilter(nil)
        grid.canvas.display()
        // Keyboard navigation
        grid.selection.set(CellPos(r: 0, c: 0))
        c.move(dr: 0, dc: 1, extendSelection: false, jump: true)
        c.move(dr: 1, dc: 0, extendSelection: true)
        check("keyboard", grid.selection.primary.rows == 2)
        // Revert to Saved must show the file again, not the edited data.
        if let url = doc.fileURL, let type = doc.fileType, let original = try? Book.open(url) {
            check("reference copy loads", waitFor(30) { original.isLoaded })
            let expected = original.displayText(0, CellPos(r: 0, c: 0))
            _ = book.setInput(sheet, CellPos(r: 0, c: 0), "changed-before-revert")
            edited()
            do {
                try doc.revert(toContentsOf: url, ofType: type)
                _ = waitFor(30) { self.book.isLoaded }
                let now = book.displayText(0, CellPos(r: 0, c: 0))
                check("revert restores file (A1 = \(now.prefix(20)))", now == expected && grid.book === doc.book && !doc.isDocumentEdited)
            } catch { check("revert: \(error.localizedDescription)", false) }
        }
        do {
            try book.save(to: URL(fileURLWithPath: out), csv: out.hasSuffix(".csv"))
            check("save", true)
        } catch { check("save: \(error.localizedDescription)", false) }
        print(log.joined(separator: "\n"))
        fflush(stdout)
        exit(0)
    }

    /// Debug aid: WAFFLE_SNAPSHOT=/path.png renders the window to a PNG and quits.
    /// WAFFLE_SNAPSHOT_SELECT=B2:D6 selects a range first; WAFFLE_SNAPSHOT_SCROLL=x,y scrolls.
    func snapshotIfRequested() {
        if debugHooksRan { return }
        debugHooksRan = true
        if selfTestIfRequested() { return }
        if ProcessInfo.processInfo.environment["WAFFLE_BENCH"] != nil {
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) { [self] in
                let c = grid.canvas
                var times: [Double] = []
                for i in 0..<40 {
                    grid.setScrollOffset(NSPoint(x: Double(i % 7) * 900, y: Double(i) * 3000))
                    let t = CFAbsoluteTimeGetCurrent()
                    c.display()
                    times.append((CFAbsoluteTimeGetCurrent() - t) * 1000)
                }
                times.sort()
                print(String(format: "full redraw of %.0fx%.0f pt: median %.1f ms, p90 %.1f ms, max %.1f ms", c.bounds.width, c.bounds.height, times[20], times[36], times[39]))
                exit(0)
            }
            return
        }
        guard ProcessInfo.processInfo.environment["WAFFLE_SNAPSHOT"] != nil else { return }
        // With several files, the settings go to the tab on show (the last file opened),
        // not the first file to finish loading.
        debugHooksRan = false
        DispatchQueue.main.asyncAfter(deadline: .now() + 1) { [weak self] in
            guard let self, !debugHooksRan, let w = window else { return }
            if let g = WindowTabs.shared.group(of: w), g.selected !== w { return }
            debugHooksRan = true
            applySnapshotSettings()
        }
    }

    private func applySnapshotSettings() {
        let env = ProcessInfo.processInfo.environment
        guard let path = env["WAFFLE_SNAPSHOT"] else { return }
        if let name = env["WAFFLE_SNAPSHOT_SHEET"], let i = (0..<book.sheetCount).first(where: { book.sheetName($0) == name }) { switchSheet(i) }
        if let sel = env["WAFFLE_SNAPSHOT_SELECT"] {
            goTo(sel)
            // Again once loading has settled (a recalculation on open can reset it).
            DispatchQueue.main.asyncAfter(deadline: .now() + 1) { [weak self] in self?.goTo(sel) }
        }
        if let sc = env["WAFFLE_SNAPSHOT_SCROLL"] {
            let p = sc.split(separator: ",").compactMap { Double($0) }
            if p.count == 2 { grid.setScrollOffset(NSPoint(x: p[0], y: p[1])) }
        }
        if let z = env["WAFFLE_SNAPSHOT_ZOOM"], let v = Double(z) { grid.setZoom(v) }
        // WAFFLE_SNAPSHOT_APPEARANCE=light|dark, WAFFLE_SNAPSHOT_FIND=1 (Find and Replace open)
        switch env["WAFFLE_SNAPSHOT_APPEARANCE"] {
        case "light": NSApp.appearance = NSAppearance(named: .aqua)  // app-wide: every tab
        case "dark": NSApp.appearance = NSAppearance(named: .darkAqua)
        default: break
        }
        if env["WAFFLE_SNAPSHOT_FIND"] != nil { showFindReplace(nil) }
        // WAFFLE_SNAPSHOT_FINDTYPE=text: type into the Find field (checks the field editor).
        if let t = env["WAFFLE_SNAPSHOT_FINDTYPE"] {
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) { [weak self] in
                guard let self else { return }
                window?.makeFirstResponder(findBar.search)
                (findBar.search.currentEditor() as? NSTextView)?.insertText(t, replacementRange: NSRange(location: NSNotFound, length: 0))
                // WAFFLE_SNAPSHOT_COLW=px: then resize column A, as dragging its edge would.
                if let w = env["WAFFLE_SNAPSHOT_COLW"].flatMap(Double.init) {
                    DispatchQueue.main.asyncAfter(deadline: .now() + 0.4) {
                        if self.book.setColWidth(self.sheet, 0, 0, px: w) { self.gridDidEdit(self.grid) }
                    }
                }
            }
        }
        // After the saved window frame has been restored, which would override it.
        if let size = env["WAFFLE_SNAPSHOT_SIZE"]?.split(separator: "x").compactMap({ Double($0) }), size.count == 2 {
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) { [weak self] in
                self?.window?.setContentSize(NSSize(width: size[0], height: size[1]))
            }
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.8) { [weak self] in
            guard let w = self?.window, let v = w.contentView, let rep = v.bitmapImageRepForCachingDisplay(in: v.bounds) else { return }
            v.cacheDisplay(in: v.bounds, to: rep)
            // cacheDisplay skips the window background; composite it underneath.
            let out = NSImage(size: v.bounds.size)
            out.lockFocus()
            w.effectiveAppearance.performAsCurrentDrawingAppearance {
                NSColor.windowBackgroundColor.setFill()
                NSRect(origin: .zero, size: v.bounds.size).fill()
            }
            rep.draw(in: NSRect(origin: .zero, size: v.bounds.size))
            out.unlockFocus()
            if let tiff = out.tiffRepresentation, let bmp = NSBitmapImageRep(data: tiff) {
                try? bmp.representation(using: .png, properties: [:])?.write(to: URL(fileURLWithPath: path))
            }
            if env["WAFFLE_SNAPSHOT_STAY"] == nil { NSApp.terminate(nil) }
        }
    }
}
#else
extension SheetWindowController {
    func snapshotIfRequested() {}
}
#endif
