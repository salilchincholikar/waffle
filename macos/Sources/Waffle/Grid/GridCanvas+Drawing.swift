import AppKit
import WaffleBridge
import CWaffle

extension GridCanvas {
    // ---- drawing -------------------------------------------------------------------


    override func draw(_ dirtyRect: NSRect) {
        guard let ctx = NSGraphicsContext.current?.cgContext else { return }
        ctx.setFillColor(palette.background)
        ctx.fill(bounds)
        guard let book, sheet < book.sheetCount else { return }

        let fz = frozenSize
        let f = frozen
        let main = mainArea
        let off = scrollOffset
        let cellsLeft = rowHeaderWidth, cellsTop = headerHeight

        // Visible column / row ranges for the scrolling area.
        let lastCol = max(0, book.cols(sheet) - 1), lastRow = max(0, book.rows(sheet) - 1)
        let c0 = max(f.cols, book.colAt(sheet, Double((fz.width + max(0, off.x)) / zoom)))
        let c1 = min(lastCol, book.colAt(sheet, Double((fz.width + max(0, off.x) + main.width) / zoom)))
        let r0 = max(f.rows, book.rowAt(sheet, Double((fz.height + max(0, off.y)) / zoom)))
        let r1 = min(lastRow, book.rowAt(sheet, Double((fz.height + max(0, off.y) + main.height) / zoom)))

        // Panes: (rows, cols, clip)
        var panes: [(ClosedRange<Int>, ClosedRange<Int>, NSRect)] = []
        if r0 <= r1 && c0 <= c1 { panes.append((r0...r1, c0...c1, main)) }
        if f.rows > 0 && c0 <= c1 {
            panes.append((0...(f.rows - 1), c0...c1, NSRect(x: main.minX, y: cellsTop, width: main.width, height: fz.height)))
        }
        if f.cols > 0 && r0 <= r1 {
            panes.append((r0...r1, 0...(f.cols - 1), NSRect(x: cellsLeft, y: main.minY, width: fz.width, height: main.height)))
        }
        if f.rows > 0 && f.cols > 0 {
            panes.append((0...(f.rows - 1), 0...(f.cols - 1), NSRect(x: cellsLeft, y: cellsTop, width: fz.width, height: fz.height)))
        }
        for (rows, cols, clip) in panes where clip.intersects(dirtyRect) {
            ctx.saveGState()
            ctx.clip(to: clip)
            drawCells(ctx, rows: rows, cols: cols, clip: clip)
            drawDrawings(ctx)
            drawSelection(ctx)
            ctx.restoreGState()
        }
        drawHeaders(ctx, c0: c0, c1: c1, r0: r0, r1: r1)

        if let g = resizeGuide {
            ctx.setFillColor(accent.cgColor)
            if g.vertical { ctx.fill(CGRect(x: g.at - 1, y: 0, width: 2, height: bounds.height)) } else { ctx.fill(CGRect(x: 0, y: g.at - 1, width: bounds.width, height: 2)) }
        }

        // Frozen dividers.
        ctx.setFillColor(palette.frozenLine)
        if f.rows > 0 { ctx.fill(CGRect(x: 0, y: cellsTop + fz.height - 1, width: bounds.width, height: 1)) }
        if f.cols > 0 { ctx.fill(CGRect(x: cellsLeft + fz.width - 1, y: 0, width: 1, height: bounds.height)) }
    }

    func drawCells(_ ctx: CGContext, rows: ClosedRange<Int>, cols: ClosedRange<Int>, clip: NSRect) {
        guard let book else { return }
        // Fetch a margin of columns so overflowing text knows its neighbours.
        let margin = 12
        let fc0 = max(0, cols.lowerBound - margin)
        let fc1 = min(book.cols(sheet) - 1, cols.upperBound + margin)
        let area = CellRect(r0: rows.lowerBound, c0: fc0, r1: rows.upperBound, c1: fc1)
        let merges = book.merges(sheet, in: area)
        let (cells, text) = book.fetch(sheet, area)
        guard cells.count == area.rows * area.cols else { return }
        let width = area.cols

        // Column edges once, all measured the way this pane moves: the extra columns fetched
        // past a frozen pane's edge must not take the scroll offset (that shrank the frozen
        // columns by the scroll amount), nor frozen ones in the scrolling pane lose it.
        let dx = cols.lowerBound >= frozen.cols ? scrollOffset.x : 0
        var xs = [CGFloat](repeating: 0, count: width + 1)
        for i in 0...width { xs[i] = rowHeaderWidth + book.colX(sheet, fc0 + i) * zoom - dx }
        var hiddenCol = [Bool](repeating: false, count: width)
        for i in 0..<width { hiddenCol[i] = xs[i + 1] - xs[i] < 0.5 }

        func inMerge(_ r: Int, _ c: Int) -> CellRect? {
            merges.first { $0.contains(CellPos(r: r, c: c)) }
        }

        // 1. Fills
        for r in rows {
            let y0 = yOf(r), y1 = yOfEnd(r)
            if y1 - y0 < 0.5 { continue }
            for i in (cols.lowerBound - fc0)...(cols.upperBound - fc0) where !hiddenCol[i] {
                let cell = cells[(r - rows.lowerBound) * width + i]
                let rect = CGRect(x: xs[i], y: y0, width: xs[i + 1] - xs[i], height: y1 - y0)
                if cell.cf_fill != 0 {
                    ctx.setFillColor(NSColor(rgb: cell.cf_fill).cgColor)
                    ctx.fill(rect)
                } else if cell.style != 0, let fill = style(Int(cell.style)).fill {
                    ctx.setFillColor(fill)
                    ctx.fill(rect)
                }
                if cell.bar != 0 {
                    let w = (rect.width - 4) * CGFloat(cell.bar) / 1000
                    let bar = CGRect(x: rect.minX + 2, y: rect.minY + 2, width: w, height: rect.height - 4)
                    let base = NSColor(rgb: cell.bar_color)
                    if let g = CGGradient(colorsSpace: nil, colors: [base.withAlphaComponent(0.85).cgColor, base.withAlphaComponent(0.15).cgColor] as CFArray, locations: [0, 1]) {
                        ctx.saveGState()
                        ctx.clip(to: bar)
                        ctx.drawLinearGradient(g, start: CGPoint(x: bar.minX, y: 0), end: CGPoint(x: bar.maxX, y: 0), options: [])
                        ctx.restoreGState()
                    }
                    ctx.setStrokeColor(base.cgColor)
                    ctx.setLineWidth(0.5)
                    ctx.stroke(bar.insetBy(dx: 0.25, dy: 0.25))
                }
            }
        }

        // 1b. Search matches
        if !highlights.isEmpty {
            for r in rows {
                let y0 = yOf(r), y1 = yOfEnd(r)
                if y1 - y0 < 0.5 { continue }
                for i in (cols.lowerBound - fc0)...(cols.upperBound - fc0) where !hiddenCol[i] {
                    let p = CellPos(r: r, c: fc0 + i)
                    guard highlights.contains(p) else { continue }
                    let current = p == currentHighlight
                    ctx.setFillColor(current ? CGColor(srgbRed: 1, green: 0.62, blue: 0.1, alpha: 0.55) : CGColor(srgbRed: 1, green: 0.9, blue: 0.2, alpha: 0.45))
                    ctx.fill(CGRect(x: xs[i], y: y0, width: xs[i + 1] - xs[i], height: y1 - y0))
                }
            }
        }

        // 2. Text layout (spill extents) so gridlines can be skipped under overflowing text.
        struct Draw { let rect: CGRect; let clip: CGRect; let cell: WfCell; let str: String; var pos = CellPos(r: 0, c: 0) }
        var draws: [Draw] = []
        var spilled = Set<Int>() // key: row * 65536 + boundary column index
        for r in rows {
            let y0 = yOf(r), y1 = yOfEnd(r)
            if y1 - y0 < 0.5 { continue }
            for i in 0..<width where !hiddenCol[i] {
                let c = fc0 + i
                let cell = cells[(r - rows.lowerBound) * width + i]
                if cell.text_len == 0 { continue }
                if inMerge(r, c) != nil { continue }
                let str = String(decoding: UnsafeBufferPointer(start: text!.advanced(by: Int(cell.text_off)), count: Int(cell.text_len)), as: UTF8.self)
                let st = style(Int(cell.style))
                var rect = CGRect(x: xs[i], y: y0, width: xs[i + 1] - xs[i], height: y1 - y0)
                var clipRect = rect
                if cell.kind == 2 && !st.wrap {
                    // Overflow into empty neighbours.
                    let needed = CGFloat(CTLineGetTypographicBounds(line(str, style: Int(cell.style), color: cell.color), nil, nil, nil)) + 6 + st.indent
                    if needed > rect.width {
                        var left = i, right = i
                        let spillRight = cell.align != 2, spillLeft = cell.align != 0
                        func empty(_ j: Int) -> Bool { j >= 0 && j < width && cells[(r - rows.lowerBound) * width + j].kind == 0 && inMerge(r, fc0 + j) == nil }
                        while (xs[right + 1] - xs[left]) < needed {
                            var grew = false
                            if spillRight, empty(right + 1) { right += 1; grew = true }
                            if spillLeft, (xs[right + 1] - xs[left]) < needed, empty(left - 1) { left -= 1; grew = true }
                            if !grew { break }
                        }
                        if left != i || right != i {
                            for b in left..<right { spilled.insert(r * 65536 + b + 1) }
                            clipRect = CGRect(x: xs[left], y: y0, width: xs[right + 1] - xs[left], height: y1 - y0)
                            if cell.align == 1 { rect = clipRect } else if cell.align == 0 { rect.size.width = clipRect.maxX - rect.minX } else { rect = CGRect(x: clipRect.minX, y: y0, width: rect.maxX - clipRect.minX, height: y1 - y0) }
                        }
                    }
                }
                if c < cols.lowerBound - 0 && clipRect.maxX <= xs[cols.lowerBound - fc0] { continue }
                if c > cols.upperBound && clipRect.minX >= xs[cols.upperBound - fc0 + 1] { continue }
                draws.append(Draw(rect: rect, clip: clipRect, cell: cell, str: str, pos: CellPos(r: r, c: c)))
            }
        }

        // 3. Gridlines
        ctx.setFillColor(palette.grid)
        let visC0 = cols.lowerBound - fc0, visC1 = cols.upperBound - fc0
        for r in rows {
            let y0 = yOf(r), y1 = yOfEnd(r)
            if y1 - y0 < 0.5 { continue }
            // horizontal (bottom edge)
            ctx.fill(CGRect(x: xs[visC0], y: y1 - 0.5, width: xs[visC1 + 1] - xs[visC0], height: 0.5))
            // vertical (right edges)
            for i in visC0...visC1 where !hiddenCol[i] {
                if spilled.contains(r * 65536 + i + 1) { continue }
                ctx.fill(CGRect(x: xs[i + 1] - 0.5, y: y0, width: 0.5, height: y1 - y0))
            }
        }

        // 4. Merged cells: cover interior lines, draw their content.
        for m in merges {
            let rect = rectOf(m)
            let cell = cellValue(r: m.r0, c: m.c0, area: area, cells: cells, width: width)
            let fill = cell.map { style(Int($0.style)).fill } ?? nil
            ctx.setFillColor(fill ?? palette.background)
            ctx.fill(rect.insetBy(dx: 0.25, dy: 0.25).offsetBy(dx: -0.25, dy: -0.25))
            ctx.setFillColor(palette.grid)
            ctx.fill(CGRect(x: rect.minX, y: rect.maxY - 0.5, width: rect.width, height: 0.5))
            ctx.fill(CGRect(x: rect.maxX - 0.5, y: rect.minY, width: 0.5, height: rect.height))
            if let cell, cell.text_len > 0 {
                let str = String(decoding: UnsafeBufferPointer(start: text!.advanced(by: Int(cell.text_off)), count: Int(cell.text_len)), as: UTF8.self)
                draws.append(Draw(rect: rect, clip: rect, cell: cell, str: str, pos: CellPos(r: m.r0, c: m.c0)))
            } else if cell == nil {
                let t = book.displayText(sheet, CellPos(r: m.r0, c: m.c0))
                if !t.isEmpty {
                    let id = book.cellStyle(sheet, CellPos(r: m.r0, c: m.c0))
                    var fake = WfCell(); fake.style = UInt32(id); fake.kind = 2; fake.align = 0
                    draws.append(Draw(rect: rect, clip: rect, cell: fake, str: t))
                }
            }
        }

        // 5. Text
        for d in draws {
            if d.cell.flags & 4 != 0, let rich = richLine(d.str, cell: d.cell, pos: d.pos) {
                drawText(ctx, d.str, cell: d.cell, rect: d.rect, clip: d.clip, preset: rich)
            } else {
                drawText(ctx, d.str, cell: d.cell, rect: d.rect, clip: d.clip)
            }
        }

        // 6. Borders
        for r in rows {
            let y0 = yOf(r), y1 = yOfEnd(r)
            if y1 - y0 < 0.5 { continue }
            for i in visC0...visC1 where !hiddenCol[i] {
                let cell = cells[(r - rows.lowerBound) * width + i]
                if cell.style == 0 { continue }
                let st = style(Int(cell.style))
                if st.borders.allSatisfy({ $0.style == 0 }) { continue }
                var rect = CGRect(x: xs[i], y: y0, width: xs[i + 1] - xs[i], height: y1 - y0)
                if let m = inMerge(r, fc0 + i) { rect = rectOf(m) }
                drawBorders(ctx, st, rect)
            }
        }
    }

    func cellValue(r: Int, c: Int, area: CellRect, cells: UnsafeBufferPointer<WfCell>, width: Int) -> WfCell? {
        guard area.contains(CellPos(r: r, c: c)) else { return nil }
        return cells[(r - area.r0) * width + (c - area.c0)]
    }

    /// A line built from a cell's rich-text runs.
    func richLine(_ str: String, cell: WfCell, pos: CellPos) -> CTLine? {
        guard let book else { return nil }
        let runs = book.runs(sheet, pos, text: str)
        guard !runs.isEmpty else { return nil }
        let st = style(Int(cell.style))
        let out = NSMutableAttributedString()
        let fm = NSFontManager.shared
        for (text, r, fontName) in runs {
            var font = st.font
            if let fontName, let f = NSFont(name: fontName, size: font.pointSize) { font = f }
            if r.size > 0 {
                let px = CGFloat(r.size) * 4 / 3 * zoom * (NSFont(name: fontName ?? "", size: 10) == nil ? 0.87 : 1)
                font = NSFont(descriptor: font.fontDescriptor, size: px) ?? font
            }
            if r.bold == 1 { font = fm.convert(font, toHaveTrait: .boldFontMask) } else if r.bold == 0 { font = fm.convert(font, toNotHaveTrait: .boldFontMask) }
            if r.italic == 1 { font = fm.convert(font, toHaveTrait: .italicFontMask) } else if r.italic == 0 { font = fm.convert(font, toNotHaveTrait: .italicFontMask) }
            var attrs: [NSAttributedString.Key: Any] = [.font: font, .foregroundColor: palette.text(r.color, over: backgroundFill(cell, st), auto: st.color)]
            if r.underline == 1 || (r.underline == -1 && st.underline) { attrs[.underlineStyle] = NSUnderlineStyle.single.rawValue }
            if r.strike == 1 || (r.strike == -1 && st.strike) { attrs[.strikethroughStyle] = NSUnderlineStyle.single.rawValue }
            out.append(NSAttributedString(string: text.replacingOccurrences(of: "\n", with: " "), attributes: attrs))
        }
        return CTLineCreateWithAttributedString(out)
    }

    func drawText(_ ctx: CGContext, _ str: String, cell: WfCell, rect: CGRect, clip: CGRect, preset: CTLine? = nil) {
        let st = style(Int(cell.style))
        let pad: CGFloat = 3 * zoom
        ctx.saveGState()
        ctx.clip(to: clip.insetBy(dx: 0.5, dy: 0))
        if st.wrap || str.contains("\n") && rect.height > st.font.boundingRectForFont.height * 1.8 {
            drawWrapped(ctx, str, cell: cell, st: st, rect: rect.insetBy(dx: pad, dy: 1))
            ctx.restoreGState()
            return
        }
        var s = str
        if s.contains("\n") { s = s.replacingOccurrences(of: "\n", with: " ") }
        let textColor: UInt32 = cell.flags & 2 != 0 ? 0xFF808080 : (cell.cf_font != 0 ? cell.cf_font : cell.color)
        var l = preset ?? line(s, style: Int(cell.style), color: textColor, cf: cell.cf_flags, fill: backgroundFill(cell, st))
        var ascent: CGFloat = 0, descent: CGFloat = 0
        var w = CGFloat(CTLineGetTypographicBounds(l, &ascent, &descent, nil))
        // Numbers that don't fit show ### like Excel.
        if cell.kind == 1 && w + pad * 2 > rect.width && cell.flags & 2 == 0 {
            let hashW = CGFloat(CTLineGetTypographicBounds(line("#", style: Int(cell.style), color: cell.color), nil, nil, nil))
            let n = max(1, Int((rect.width - pad * 2) / max(hashW, 1)))
            s = String(repeating: "#", count: n)
            l = line(s, style: Int(cell.style), color: cell.color)
            w = CGFloat(CTLineGetTypographicBounds(l, &ascent, &descent, nil))
        }
        var x: CGFloat
        switch cell.align {
        case 1: x = rect.midX - w / 2
        case 2: x = rect.maxX - pad - w - st.indent
        default: x = rect.minX + pad + st.indent
        }
        let lineH = ascent + descent
        var baseline: CGFloat
        switch st.valign {
        case 1: baseline = rect.midY + (ascent - descent) / 2
        case 2: baseline = rect.minY + 1 + ascent
        default: baseline = rect.maxY - max(2, (rect.height - lineH) > 4 ? 3 * zoom : 1) - descent
        }
        if rect.height < lineH { baseline = rect.minY + ascent }
        x = x.rounded(); baseline = baseline.rounded()
        ctx.textMatrix = CGAffineTransform(scaleX: 1, y: -1)
        ctx.textPosition = CGPoint(x: x, y: baseline)
        CTLineDraw(l, ctx)
        ctx.restoreGState()
    }

    func drawWrapped(_ ctx: CGContext, _ str: String, cell: WfCell, st: CellStyle, rect: CGRect) {
        let para = NSMutableParagraphStyle()
        para.alignment = cell.align == 1 ? .center : cell.align == 2 ? .right : .left
        para.lineBreakMode = .byWordWrapping
        var attrs: [NSAttributedString.Key: Any] = [.font: st.font, .foregroundColor: palette.text(cell.color, over: backgroundFill(cell, st), auto: st.color), .paragraphStyle: para]
        if st.underline { attrs[.underlineStyle] = NSUnderlineStyle.single.rawValue }
        let a = NSAttributedString(string: str, attributes: attrs)
        let bounding = a.boundingRect(with: NSSize(width: rect.width, height: .greatestFiniteMagnitude), options: [.usesLineFragmentOrigin])
        var r = rect
        switch st.valign {
        case 1: r.origin.y = rect.midY - bounding.height / 2
        case 2: break
        default: r.origin.y = max(rect.minY, rect.maxY - bounding.height)
        }
        r.size.height = max(bounding.height, rect.height)
        NSGraphicsContext.saveGraphicsState()
        a.draw(with: r, options: [.usesLineFragmentOrigin])
        NSGraphicsContext.restoreGraphicsState()
    }

    func drawBorders(_ ctx: CGContext, _ st: CellStyle, _ rect: CGRect) {
        for (i, b) in st.borders.enumerated() where b.style != 0 {
            let w: CGFloat = [0, 1, 2, 1, 1, 3, 3, 0.5, 2, 1, 2, 1, 2, 2][Int(min(b.style, 13))]
            ctx.setStrokeColor(b.color)
            ctx.setLineWidth(w)
            switch b.style {
            case 3, 8: ctx.setLineDash(phase: 0, lengths: [3, 2])
            case 4: ctx.setLineDash(phase: 0, lengths: [1, 1])
            case 9, 10: ctx.setLineDash(phase: 0, lengths: [4, 2, 1, 2])
            case 11, 12: ctx.setLineDash(phase: 0, lengths: [4, 2, 1, 2, 1, 2])
            default: ctx.setLineDash(phase: 0, lengths: [])
            }
            let (a, z): (CGPoint, CGPoint)
            switch i {
            case 0: (a, z) = (CGPoint(x: rect.minX, y: rect.minY), CGPoint(x: rect.minX, y: rect.maxY))
            case 1: (a, z) = (CGPoint(x: rect.maxX, y: rect.minY), CGPoint(x: rect.maxX, y: rect.maxY))
            case 2: (a, z) = (CGPoint(x: rect.minX, y: rect.minY), CGPoint(x: rect.maxX, y: rect.minY))
            default: (a, z) = (CGPoint(x: rect.minX, y: rect.maxY), CGPoint(x: rect.maxX, y: rect.maxY))
            }
            if b.style == 6 {
                // double
                let off: CGPoint = i < 2 ? CGPoint(x: 1, y: 0) : CGPoint(x: 0, y: 1)
                ctx.setLineWidth(0.75)
                ctx.strokeLineSegments(between: [CGPoint(x: a.x - off.x, y: a.y - off.y), CGPoint(x: z.x - off.x, y: z.y - off.y), CGPoint(x: a.x + off.x, y: a.y + off.y), CGPoint(x: z.x + off.x, y: z.y + off.y)])
            } else {
                ctx.strokeLineSegments(between: [a, z])
            }
        }
        ctx.setLineDash(phase: 0, lengths: [])
    }


    func drawDrawings(_ ctx: CGContext) {
        guard let book else { return }
        if drawingCache == nil || drawingCache!.sheet != sheet { drawingCache = (sheet, 0, book.drawings(sheet)) }
        let items = drawingCache!.items
        guard !items.isEmpty else { return }
        NSGraphicsContext.saveGraphicsState()
        for (d, ref) in items {
            let x0 = xOf(Int(d.col0)) + CGFloat(d.dx0) * zoom
            let y0 = yOf(Int(d.row0)) + CGFloat(d.dy0) * zoom
            var rect: CGRect
            if d.two_cell {
                let x1 = xOf(Int(d.col1)) + CGFloat(d.dx1) * zoom
                let y1 = yOf(Int(d.row1)) + CGFloat(d.dy1) * zoom
                rect = CGRect(x: x0, y: y0, width: max(4, x1 - x0), height: max(4, y1 - y0))
            } else {
                rect = CGRect(x: x0, y: y0, width: CGFloat(d.width) * zoom, height: CGFloat(d.height) * zoom)
            }
            guard rect.intersects(bounds) else { continue }
            if d.kind == 0 {
                if images[ref] == nil, let data = book.partBytes(ref) { images[ref] = NSImage(data: data) ?? NSImage() }
                images[ref]?.draw(in: rect, from: .zero, operation: .sourceOver, fraction: 1, respectFlipped: true, hints: [.interpolation: NSImageInterpolation.high])
            } else {
                // Charts aren't rendered yet: show a quiet placeholder card.
                let path = CGPath(roundedRect: rect.insetBy(dx: 0.5, dy: 0.5), cornerWidth: 6, cornerHeight: 6, transform: nil)
                ctx.setFillColor(palette.cardFill)
                ctx.addPath(path); ctx.fillPath()
                ctx.setStrokeColor(palette.cardStroke)
                ctx.setLineWidth(1)
                ctx.addPath(path); ctx.strokePath()
                let icon = NSImage(systemSymbolName: "chart.bar.xaxis", accessibilityDescription: "Chart")
                let title = (ref.isEmpty ? "Chart" : ref) as NSString
                let attrs: [NSAttributedString.Key: Any] = [.font: NSFont.systemFont(ofSize: 13 * zoom, weight: .semibold), .foregroundColor: palette.cardTitle]
                let sub: [NSAttributedString.Key: Any] = [.font: NSFont.systemFont(ofSize: 11 * zoom), .foregroundColor: palette.cardNote]
                let tsz = title.size(withAttributes: attrs)
                let iconSize = 28 * zoom
                icon?.draw(in: CGRect(x: rect.midX - iconSize / 2, y: rect.midY - iconSize - 6, width: iconSize, height: iconSize), from: .zero, operation: .sourceOver, fraction: 0.45, respectFlipped: true, hints: nil)
                title.draw(at: CGPoint(x: rect.midX - tsz.width / 2, y: rect.midY + 2), withAttributes: attrs)
                let note = "Chart (kept in the file)" as NSString
                let nsz = note.size(withAttributes: sub)
                note.draw(at: CGPoint(x: rect.midX - nsz.width / 2, y: rect.midY + 4 + tsz.height), withAttributes: sub)
            }
        }
        NSGraphicsContext.restoreGraphicsState()
    }

    /// Call after structural edits / undo so drawings re-read their anchors.
    func invalidateDrawings() { drawingCache = nil }


    func drawSelection(_ ctx: CGContext) {
        guard let book else { return }
        let accentCG = accent.cgColor
        for (i, sel) in selection.ranges.enumerated() {
            var r = sel
            r.r1 = min(r.r1, book.rows(sheet) - 1)
            r.c1 = min(r.c1, book.cols(sheet) - 1)
            let rect = rectOf(r)
            if !(r.isSingle && selection.ranges.count == 1) {
                ctx.setFillColor(accent.withAlphaComponent(0.12).cgColor)
                ctx.fill(rect)
            }
            if i == selection.ranges.count - 1 {
                ctx.setStrokeColor(accentCG)
                ctx.setLineWidth(r.isSingle ? 2 : 1)
                ctx.stroke(rect.insetBy(dx: r.isSingle ? 1 : 0.5, dy: r.isSingle ? 1 : 0.5))
            }
        }
        // Active cell outline (merged area if merged).
        let act = book.merge(at: selection.active, sheet: sheet) ?? CellRect(selection.active)
        let ar = rectOf(act)
        ctx.setStrokeColor(accentCG)
        ctx.setLineWidth(2)
        ctx.stroke(ar.insetBy(dx: 1, dy: 1))
        // Fill handle
        let p = rectOf(selection.primary)
        ctx.setFillColor(accentCG)
        ctx.fill(CGRect(x: p.maxX - 3, y: p.maxY - 3, width: 6, height: 6))
        ctx.setFillColor(.white)
        ctx.fill(CGRect(x: p.maxX - 3.5, y: p.maxY - 3.5, width: 1, height: 7))
        if let cut = cutRect {
            ctx.setStrokeColor(accentCG)
            ctx.setLineWidth(1.5)
            ctx.setLineDash(phase: 0, lengths: [4, 3])
            ctx.stroke(rectOf(cut).insetBy(dx: 1, dy: 1))
            ctx.setLineDash(phase: 0, lengths: [])
        }
    }

    func drawHeaders(_ ctx: CGContext, c0: Int, c1: Int, r0: Int, r1: Int) {
        guard let book else { return }
        let f = frozen
        let headerBG = palette.headerBG
        let selBG = accent.withAlphaComponent(0.18).cgColor
        let lineColor = palette.headerLine
        let font = NSFont.monospacedDigitSystemFont(ofSize: 11 * max(zoom, 0.8), weight: .regular)
        let fontSel = NSFont.monospacedDigitSystemFont(ofSize: 11 * max(zoom, 0.8), weight: .semibold)
        let textColor = palette.headerText

        func selectedCol(_ c: Int) -> Bool { selection.ranges.contains { c >= $0.c0 && c <= $0.c1 } }
        func selectedRow(_ r: Int) -> Bool { selection.ranges.contains { r >= $0.r0 && r <= $0.r1 } }
        func fullCol(_ c: Int) -> Bool { selection.ranges.contains { $0.isFullCols && c >= $0.c0 && c <= $0.c1 } }
        func fullRow(_ r: Int) -> Bool { selection.ranges.contains { $0.isFullRows && r >= $0.r0 && r <= $0.r1 } }

        // Column header strip
        ctx.saveGState()
        ctx.clip(to: CGRect(x: rowHeaderWidth, y: 0, width: bounds.width - rowHeaderWidth, height: headerHeight))
        ctx.setFillColor(headerBG)
        ctx.fill(CGRect(x: 0, y: 0, width: bounds.width, height: headerHeight))
        var colList: [Int] = Array(0..<f.cols)
        if c0 <= c1 { colList += Array(c0...c1) }
        for c in colList {
            let x0 = xOf(c), x1 = xOfEnd(c)
            if x1 - x0 < 0.5 { continue }
            if c >= f.cols && x1 <= rowHeaderWidth + frozenSize.width { continue }
            // A column scrolled partly under frozen ones is cut at the freeze line.
            let underFrozen = c >= f.cols && f.cols > 0
            if underFrozen {
                ctx.saveGState()
                ctx.clip(to: CGRect(x: rowHeaderWidth + frozenSize.width, y: 0, width: bounds.width, height: headerHeight))
            }
            defer { if underFrozen { ctx.restoreGState() } }
            if selectedCol(c) {
                ctx.setFillColor(fullCol(c) ? accent.withAlphaComponent(0.35).cgColor : selBG)
                ctx.fill(CGRect(x: x0, y: 0, width: x1 - x0, height: headerHeight))
            }
            ctx.setFillColor(lineColor)
            ctx.fill(CGRect(x: x1 - 0.5, y: 0, width: 0.5, height: headerHeight))
            let name = columnName(c) as NSString
            let attrs: [NSAttributedString.Key: Any] = [.font: selectedCol(c) ? fontSel : font, .foregroundColor: textColor]
            let sz = name.size(withAttributes: attrs)
            if sz.width < x1 - x0 - 2 {
                name.draw(at: NSPoint(x: (x0 + x1 - sz.width) / 2, y: (headerHeight - sz.height) / 2), withAttributes: attrs)
            }
            if filterMode {
                let active = book.filterActive(sheet, col: c)
                let box = CGRect(x: x1 - 15, y: (headerHeight - 13) / 2, width: 13, height: 13)
                if x1 - x0 > 30 {
                    ctx.setFillColor(active ? accent.cgColor : palette.filterBox)
                    let path = CGPath(roundedRect: box, cornerWidth: 3, cornerHeight: 3, transform: nil)
                    ctx.addPath(path); ctx.fillPath()
                    ctx.setFillColor(active ? .white : palette.filterGlyph)
                    ctx.move(to: CGPoint(x: box.minX + 3.5, y: box.minY + 5))
                    ctx.addLine(to: CGPoint(x: box.maxX - 3.5, y: box.minY + 5))
                    ctx.addLine(to: CGPoint(x: box.midX, y: box.maxY - 4))
                    ctx.fillPath()
                }
            }
        }
        ctx.restoreGState()
        ctx.setFillColor(lineColor)
        ctx.fill(CGRect(x: 0, y: headerHeight - 0.5, width: bounds.width, height: 0.5))

        // Row header strip
        ctx.saveGState()
        ctx.clip(to: CGRect(x: 0, y: headerHeight, width: rowHeaderWidth, height: bounds.height - headerHeight))
        ctx.setFillColor(headerBG)
        ctx.fill(CGRect(x: 0, y: headerHeight, width: rowHeaderWidth, height: bounds.height))
        var rowList: [Int] = Array(0..<f.rows)
        if r0 <= r1 { rowList += Array(r0...r1) }
        for r in rowList {
            let y0 = yOf(r), y1 = yOfEnd(r)
            if y1 - y0 < 0.5 { continue }
            if r >= f.rows && y1 <= headerHeight + frozenSize.height { continue }
            // A row scrolled partly under frozen ones is cut at the freeze line.
            let underFrozen = r >= f.rows && f.rows > 0
            if underFrozen {
                ctx.saveGState()
                ctx.clip(to: CGRect(x: 0, y: headerHeight + frozenSize.height, width: rowHeaderWidth, height: bounds.height))
            }
            defer { if underFrozen { ctx.restoreGState() } }
            if selectedRow(r) {
                ctx.setFillColor(fullRow(r) ? accent.withAlphaComponent(0.35).cgColor : selBG)
                ctx.fill(CGRect(x: 0, y: y0, width: rowHeaderWidth, height: y1 - y0))
            }
            ctx.setFillColor(lineColor)
            ctx.fill(CGRect(x: 0, y: y1 - 0.5, width: rowHeaderWidth, height: 0.5))
            let filtered = book.rowFiltered(sheet, r + 1) || (r > 0 && book.rowFiltered(sheet, r - 1))
            let name = String(r + 1) as NSString
            let attrs: [NSAttributedString.Key: Any] = [.font: selectedRow(r) ? fontSel : font, .foregroundColor: filtered ? accent : textColor]
            let sz = name.size(withAttributes: attrs)
            if sz.height < y1 - y0 + 2 {
                name.draw(at: NSPoint(x: rowHeaderWidth - sz.width - 6, y: y0 + (y1 - y0 - sz.height) / 2), withAttributes: attrs)
            }
        }
        ctx.restoreGState()
        ctx.setFillColor(lineColor)
        ctx.fill(CGRect(x: rowHeaderWidth - 0.5, y: 0, width: 0.5, height: bounds.height))

        // Corner
        ctx.setFillColor(headerBG)
        ctx.fill(CGRect(x: 0, y: 0, width: rowHeaderWidth - 0.5, height: headerHeight - 0.5))
        ctx.setFillColor(palette.corner)
        ctx.move(to: CGPoint(x: rowHeaderWidth - 5, y: headerHeight - 5))
        ctx.addLine(to: CGPoint(x: rowHeaderWidth - 5, y: headerHeight - 13))
        ctx.addLine(to: CGPoint(x: rowHeaderWidth - 13, y: headerHeight - 5))
        ctx.fillPath()
    }
}
