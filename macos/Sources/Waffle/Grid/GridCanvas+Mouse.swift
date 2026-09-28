import AppKit
import WaffleBridge
import CWaffle

extension GridCanvas {
    // ---- editor positioning --------------------------------------------------------------

    func repositionEditor() {
        guard let editor, let book else { return }
        let area = book.merge(at: editor.pos, sheet: sheet) ?? CellRect(editor.pos)
        editor.place(in: rectOf(area), canvas: self)
    }

    // ---- mouse ---------------------------------------------------------------------------


    override func resetCursorRects() {
        super.resetCursorRects()
    }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        for t in trackingAreas { removeTrackingArea(t) }
        addTrackingArea(NSTrackingArea(rect: .zero, options: [.mouseMoved, .activeInKeyWindow, .inVisibleRect, .cursorUpdate], owner: self))
    }

    override func mouseMoved(with event: NSEvent) {
        let p = convert(event.locationInWindow, from: nil)
        switch hit(p) {
        case .colHeader(_, resize: true): NSCursor.resizeLeftRight.set()
        case .rowHeader(_, resize: true): NSCursor.resizeUpDown.set()
        case .cell:
            let pr = rectOf(selection.primary)
            if abs(p.x - pr.maxX) < 5 && abs(p.y - pr.maxY) < 5 { NSCursor.crosshair.set() } else { NSCursor.arrow.set() }
        default: NSCursor.arrow.set()
        }
    }

    override func cursorUpdate(with event: NSEvent) { mouseMoved(with: event) }

    override func scrollWheel(with event: NSEvent) {
        grid?.scrollView.scrollWheel(with: event)
    }

    override func magnify(with event: NSEvent) {
        grid?.setZoom(zoom * (1 + event.magnification))
    }

    override func mouseDown(with event: NSEvent) {
        guard let book else { return }
        window?.makeFirstResponder(self)
        let p = convert(event.locationInWindow, from: nil)
        let shift = event.modifierFlags.contains(.shift)
        let cmd = event.modifierFlags.contains(.command)
        if event.modifierFlags.contains(.control) {
            rightMouseDown(with: event)
            return
        }
        commitEditorIfNeeded()
        let h = hit(p)
        switch h {
        case .corner:
            selectAll()
        case let .colHeader(c, resize):
            if resize {
                drag = .resizeCol(c, startX: p.x, startWidth: xOfEnd(c) - xOf(c))
                if event.clickCount == 2 { autofitColumns(c...c); drag = nil }
                return
            }
            if filterMode, p.x > xOfEnd(c) - 17 {
                let r = NSRect(x: xOf(c), y: 0, width: xOfEnd(c) - xOf(c), height: headerHeight)
                grid?.delegate?.gridFilterClicked(grid!, column: c, at: r)
                return
            }
            let full = CellRect(r0: 0, c0: c, r1: CellRect.maxRows - 1, c1: c)
            if shift, let last = selection.ranges.last, last.isFullCols {
                selection.ranges[selection.ranges.count - 1] = CellRect(r0: 0, c0: min(selection.anchor.c, c), r1: CellRect.maxRows - 1, c1: max(selection.anchor.c, c))
            } else if cmd {
                selection.ranges.append(full); selection.anchor = CellPos(r: 0, c: c)
                selection.active = CellPos(r: firstVisibleRow(), c: c)
            } else {
                selection = Selection(active: CellPos(r: firstVisibleRow(), c: c), anchor: CellPos(r: 0, c: c), ranges: [full])
            }
            drag = .cols(start: c)
        case let .rowHeader(r, resize):
            if resize {
                drag = .resizeRow(r, startY: p.y, startHeight: yOfEnd(r) - yOf(r))
                if event.clickCount == 2 { _ = book.setRowHeight(sheet, r, r, px: 20); didEdit(); drag = nil }
                return
            }
            let full = CellRect(r0: r, c0: 0, r1: r, c1: CellRect.maxCols - 1)
            if shift, let last = selection.ranges.last, last.isFullRows {
                selection.ranges[selection.ranges.count - 1] = CellRect(r0: min(selection.anchor.r, r), c0: 0, r1: max(selection.anchor.r, r), c1: CellRect.maxCols - 1)
            } else if cmd {
                selection.ranges.append(full); selection.anchor = CellPos(r: r, c: 0); selection.active = CellPos(r: r, c: 0)
            } else {
                selection = Selection(active: CellPos(r: r, c: 0), anchor: CellPos(r: r, c: 0), ranges: [full])
            }
            drag = .rows(start: r)
        case let .cell(pos):
            let pr = rectOf(selection.primary)
            if abs(p.x - pr.maxX) < 5 && abs(p.y - pr.maxY) < 5 {
                drag = .fillHandle(selection.primary)
                return
            }
            if event.clickCount == 2 {
                selection.set(pos)
                grid?.delegate?.gridBeginEditing(grid!, initial: nil, select: false)
                return
            }
            if shift {
                extend(to: pos)
            } else if cmd {
                selection.ranges.append(CellRect(pos)); selection.active = pos; selection.anchor = pos
            } else {
                let m = book.merge(at: pos, sheet: sheet)
                selection = Selection(active: m.map { CellPos(r: $0.r0, c: $0.c0) } ?? pos, anchor: pos, ranges: [m ?? CellRect(pos)])
            }
            drag = .cells(start: selection.anchor, additive: cmd)
        case .none:
            break
        }
        notifySelection()
    }

    override func mouseDragged(with event: NSEvent) {
        lastDragEvent = event
        handleDrag(event)
        let p = convert(event.locationInWindow, from: nil)
        let main = mainArea
        let outside = p.x > bounds.maxX - 4 || p.y > bounds.maxY - 4 || (p.x < main.minX && frozen.cols == 0 && p.x > rowHeaderWidth) || (p.y < main.minY && frozen.rows == 0 && p.y > headerHeight)
        if outside, autoscrollTimer == nil, drag != nil {
            autoscrollTimer = Timer.scheduledTimer(withTimeInterval: 0.05, repeats: true) { [weak self] _ in self?.autoscrollTick() }
        } else if !outside {
            autoscrollTimer?.invalidate(); autoscrollTimer = nil
        }
    }

    func autoscrollTick() {
        guard let ev = lastDragEvent, let grid else { return }
        let p = convert(ev.locationInWindow, from: nil)
        var off = scrollOffset
        let step: CGFloat = 20 * zoom
        if p.x > bounds.maxX - 4 { off.x += step } else if p.x < mainArea.minX { off.x -= step }
        if p.y > bounds.maxY - 4 { off.y += step } else if p.y < mainArea.minY { off.y -= step }
        grid.setScrollOffset(off)
        handleDrag(ev)
    }

    func handleDrag(_ event: NSEvent) {
        guard let drag, let book else { return }
        let p = convert(event.locationInWindow, from: nil)
        let clampedP = NSPoint(x: min(max(p.x, rowHeaderWidth + 1), bounds.maxX - 1), y: min(max(p.y, headerHeight + 1), bounds.maxY - 1))
        switch drag {
        case let .cells(start, _):
            let pos = CellPos(r: rowAt(clampedP.y), c: colAt(clampedP.x))
            var rect = CellRect(start, pos)
            rect = expandForMerges(rect)
            selection.ranges[selection.ranges.count - 1] = rect
            notifySelection()
        case let .cols(start):
            let c = colAt(clampedP.x)
            selection.ranges[selection.ranges.count - 1] = CellRect(r0: 0, c0: min(start, c), r1: CellRect.maxRows - 1, c1: max(start, c))
            notifySelection()
        case let .rows(start):
            let r = rowAt(clampedP.y)
            selection.ranges[selection.ranges.count - 1] = CellRect(r0: min(start, r), c0: 0, r1: max(start, r), c1: CellRect.maxCols - 1)
            notifySelection()
        case let .resizeCol(c, startX, startWidth):
            _ = book
            resizeGuide = (true, xOf(c) + max(0, startWidth + (p.x - startX)))
            needsDisplay = true
        case let .resizeRow(r, startY, startHeight):
            resizeGuide = (false, yOf(r) + max(0, startHeight + (p.y - startY)))
            needsDisplay = true
        case let .fillHandle(src):
            let pos = CellPos(r: rowAt(clampedP.y), c: colAt(clampedP.x))
            let down = abs(pos.r - src.r1) >= abs(pos.c - src.c1)
            let target = down ? CellRect(r0: src.r0, c0: src.c0, r1: max(src.r1, pos.r), c1: src.c1) : CellRect(r0: src.r0, c0: src.c0, r1: src.r1, c1: max(src.c1, pos.c))
            fillPreview = target
            selection.ranges[selection.ranges.count - 1] = target
        }
    }

    override func mouseUp(with event: NSEvent) {
        autoscrollTimer?.invalidate(); autoscrollTimer = nil
        defer { drag = nil; fillPreview = nil }
        guard let drag, let book else { return }
        let p = convert(event.locationInWindow, from: nil)
        switch drag {
        case let .resizeCol(c, startX, startWidth):
            resizeGuide = nil
            let w = max(0, startWidth + (p.x - startX)) / zoom
            let cols = selectedFullColumns(containing: c) ?? (c...c)
            if abs(p.x - startX) > 0.5, book.setColWidth(sheet, cols.lowerBound, cols.upperBound, px: w.rounded()) { didEdit() }
        case let .resizeRow(r, startY, startHeight):
            resizeGuide = nil
            let h = max(0, startHeight + (p.y - startY)) / zoom
            let rows = selectedFullRows(containing: r) ?? (r...r)
            if abs(p.y - startY) > 0.5, book.setRowHeight(sheet, rows.lowerBound, rows.upperBound, px: h.rounded()) { didEdit() }
        case let .fillHandle(src):
            if let t = fillPreview, t != src {
                let down = t.rows > src.rows
                let target = down ? CellRect(r0: src.r1 + 1, c0: src.c0, r1: t.r1, c1: src.c1) : CellRect(r0: src.r0, c0: src.c1 + 1, r1: src.r1, c1: t.c1)
                if book.fillFrom(sheet, src, target) { didEdit() }
            }
        default: break
        }
        notifySelection()
    }

    override func rightMouseDown(with event: NSEvent) {
        let p = convert(event.locationInWindow, from: nil)
        let h = hit(p)
        switch h {
        case let .cell(pos) where !selection.ranges.contains(where: { $0.contains(pos) }):
            selection.set(pos); notifySelection()
        case let .colHeader(c, _) where !selection.ranges.contains(where: { $0.isFullCols && c >= $0.c0 && c <= $0.c1 }):
            selection = Selection(active: CellPos(r: 0, c: c), anchor: CellPos(r: 0, c: c), ranges: [CellRect(r0: 0, c0: c, r1: CellRect.maxRows - 1, c1: c)]); notifySelection()
        case let .rowHeader(r, _) where !selection.ranges.contains(where: { $0.isFullRows && r >= $0.r0 && r <= $0.r1 }):
            selection = Selection(active: CellPos(r: r, c: 0), anchor: CellPos(r: r, c: 0), ranges: [CellRect(r0: r, c0: 0, r1: r, c1: CellRect.maxCols - 1)]); notifySelection()
        default: break
        }
        if let menu = grid?.delegate?.gridContextMenu(grid!, for: event, header: h) {
            NSMenu.popUpContextMenu(menu, with: event, for: self)
        }
    }

    func selectedFullColumns(containing c: Int) -> ClosedRange<Int>? {
        selection.ranges.first { $0.isFullCols && c >= $0.c0 && c <= $0.c1 }.map { $0.c0...$0.c1 }
    }
    func selectedFullRows(containing r: Int) -> ClosedRange<Int>? {
        selection.ranges.first { $0.isFullRows && r >= $0.r0 && r <= $0.r1 }.map { $0.r0...$0.r1 }
    }

    func expandForMerges(_ r: CellRect) -> CellRect {
        guard let book else { return r }
        var rect = r
        var changed = true
        var guardCount = 0
        while changed && guardCount < 8 {
            changed = false; guardCount += 1
            for m in book.merges(sheet, in: rect) where !(rect.contains(CellPos(r: m.r0, c: m.c0)) && rect.contains(CellPos(r: m.r1, c: m.c1))) {
                rect = rect.union(m); changed = true
            }
        }
        return rect
    }

    func firstVisibleRow() -> Int {
        var r = 0
        while r < 1000, book?.rowHidden(sheet, r) == true { r += 1 }
        return r
    }

    func notifySelection() {
        needsDisplay = true
        grid?.delegate?.gridSelectionChanged(grid!)
    }

    func didEdit() {
        drawingCache = nil
        grid?.updateInsets()
        needsDisplay = true
        grid?.delegate?.gridDidEdit(grid!)
    }

    func commitEditorIfNeeded() {
        editor?.commit(move: nil)
    }

    func selectAll() {
        selection = Selection(active: CellPos(r: 0, c: 0), anchor: CellPos(r: 0, c: 0), ranges: [CellRect(r0: 0, c0: 0, r1: CellRect.maxRows - 1, c1: CellRect.maxCols - 1)])
        notifySelection()
    }

    func autofitColumns(_ cols: ClosedRange<Int>) {
        guard let book else { return }
        let rows = min(book.dataRows(sheet), 5000)
        guard rows > 0 else { return }
        for c in cols {
            let (cells, text) = book.fetch(sheet, CellRect(r0: 0, c0: c, r1: rows - 1, c1: c))
            var w: CGFloat = 0
            for cell in cells where cell.text_len > 0 {
                let s = String(decoding: UnsafeBufferPointer(start: text!.advanced(by: Int(cell.text_off)), count: Int(cell.text_len)), as: UTF8.self)
                let st = style(Int(cell.style))
                let lw = (s as NSString).size(withAttributes: [.font: st.font]).width
                w = max(w, lw / zoom)
            }
            if w > 0 { _ = book.setColWidth(sheet, c, c, px: min(600, (w + 12).rounded())) }
        }
        didEdit()
    }
}
