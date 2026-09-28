import AppKit
import WaffleBridge
import CWaffle

/// Selection: one or more rectangles, an active cell and the anchor for Shift-extension.
struct Selection {
    var active = CellPos(r: 0, c: 0)
    var anchor = CellPos(r: 0, c: 0)
    var ranges: [CellRect] = [CellRect(CellPos(r: 0, c: 0))]

    var primary: CellRect { ranges.last ?? CellRect(active) }
    mutating func set(_ p: CellPos) {
        active = p; anchor = p; ranges = [CellRect(p)]
    }
}

protocol GridViewDelegate: AnyObject {
    func gridSelectionChanged(_ grid: GridView)
    func gridDidEdit(_ grid: GridView)
    func gridBeginEditing(_ grid: GridView, initial: String?, select: Bool)
    func gridContextMenu(_ grid: GridView, for event: NSEvent, header: GridCanvas.Hit) -> NSMenu?
    func gridFilterClicked(_ grid: GridView, column: Int, at rect: NSRect)
}

/// The document view of the scroll view: empty, just sized to the scrollable content.
final class SpacerView: NSView {
    override var isFlipped: Bool { true }
}

/// Owns the scroll view and the canvas that draws the sheet.
final class GridView: NSView {
    let scrollView = NSScrollView()
    let spacer = SpacerView()
    let canvas = GridCanvas()
    weak var delegate: GridViewDelegate?

    var book: Book? { didSet { canvas.book = book; reload() } }
    var sheet = 0 { didSet { canvas.sheet = sheet; reload() } }
    var selection: Selection {
        get { canvas.selection }
        set { canvas.selection = newValue }
    }

    override init(frame: NSRect) {
        super.init(frame: frame)
        scrollView.hasVerticalScroller = true
        scrollView.hasHorizontalScroller = true
        scrollView.autohidesScrollers = false
        scrollView.scrollerStyle = .overlay
        scrollView.drawsBackground = true
        scrollView.backgroundColor = NSColor(cgColor: canvas.palette.background) ?? .white
        scrollView.automaticallyAdjustsContentInsets = false
        scrollView.documentView = spacer
        scrollView.contentView.postsBoundsChangedNotifications = true
        scrollView.verticalScrollElasticity = .allowed
        scrollView.horizontalScrollElasticity = .allowed
        scrollView.allowsMagnification = false
        addSubview(scrollView)
        scrollView.addSubview(canvas, positioned: .above, relativeTo: scrollView.contentView)
        canvas.grid = self
        canvas.updatePalette()
        NotificationCenter.default.addObserver(self, selector: #selector(scrolled), name: NSView.boundsDidChangeNotification, object: scrollView.contentView)
        NotificationCenter.default.addObserver(self, selector: #selector(tiled), name: NSScrollView.didLiveScrollNotification, object: scrollView)
    }

    required init?(coder: NSCoder) { fatalError() }

    override func layout() {
        super.layout()
        scrollView.frame = bounds
        tiled()
        updateInsets()
    }

    @objc private func tiled() {
        setCanvasFrame(scrollView.contentView.frame)
    }

    /// The canvas only redraws on request, so a size change must ask for it.
    private func setCanvasFrame(_ f: NSRect) {
        guard canvas.frame != f else { return }
        canvas.frame = f
        canvas.needsDisplay = true
    }

    @objc private func scrolled() {
        canvas.needsDisplay = true
        canvas.repositionEditor()
    }

    /// Recompute content size and insets after geometry changes.
    func reload() {
        canvas.invalidateStyles()
        updateInsets()
        canvas.needsDisplay = true
    }

    func updateInsets() {
        guard let book else { return }
        let z = canvas.zoom
        let (fr, fc) = book.freeze(sheet)
        let fh = book.rowY(sheet, fr) * z
        let fw = book.colX(sheet, fc) * z
        canvas.updateHeaderMetrics()
        let top = canvas.headerHeight + fh
        let left = canvas.rowHeaderWidth + fw
        let insets = NSEdgeInsets(top: top, left: left, bottom: 0, right: 0)
        if scrollView.contentInsets.top != insets.top || scrollView.contentInsets.left != insets.left {
            let origin = canvas.scrollOffset
            scrollView.contentInsets = insets
            scrollView.contentView.scroll(to: NSPoint(x: origin.x - left, y: origin.y - top))
        }
        let w = max(0, (book.totalWidth(sheet) * z) - fw)
        let h = max(0, (book.totalHeight(sheet) * z) - fh)
        spacer.frame.size = NSSize(width: w, height: h)
        scrollView.reflectScrolledClipView(scrollView.contentView)
        setCanvasFrame(scrollView.contentView.frame)
    }

    /// Scroll so that a cell is fully visible.
    func scrollToVisible(_ p: CellPos) {
        guard let book else { return }
        let z = canvas.zoom
        let (fr, fc) = book.freeze(sheet)
        let fx = book.colX(sheet, fc), fy = book.rowY(sheet, fr)
        var off = canvas.scrollOffset
        let viewW = canvas.mainArea.width, viewH = canvas.mainArea.height
        // Before the first layout there is no viewport to scroll within.
        guard viewW > 0, viewH > 0 else { return }
        if p.c >= fc {
            let x0 = (book.colX(sheet, p.c) - fx) * z, x1 = (book.colX(sheet, p.c + 1) - fx) * z
            if x0 < off.x { off.x = x0 } else if x1 > off.x + viewW { off.x = min(x0, x1 - viewW) }
        }
        if p.r >= fr {
            let y0 = (book.rowY(sheet, p.r) - fy) * z, y1 = (book.rowY(sheet, p.r + 1) - fy) * z
            if y0 < off.y { off.y = y0 } else if y1 > off.y + viewH { off.y = min(y0, y1 - viewH) }
        }
        if off != canvas.scrollOffset { setScrollOffset(off) }
    }

    func setScrollOffset(_ p: NSPoint) {
        let ins = scrollView.contentInsets
        let maxX = max(0, spacer.frame.width - canvas.mainArea.width)
        let maxY = max(0, spacer.frame.height - canvas.mainArea.height)
        let x = min(max(0, p.x), maxX), y = min(max(0, p.y), maxY)
        scrollView.contentView.scroll(to: NSPoint(x: x - ins.left, y: y - ins.top))
        scrollView.reflectScrolledClipView(scrollView.contentView)
    }

    func setZoom(_ z: CGFloat) {
        let z = min(4, max(0.25, z))
        guard z != canvas.zoom else { return }
        let old = canvas.zoom
        let off = canvas.scrollOffset
        canvas.zoom = z
        canvas.invalidateStyles()
        updateInsets()
        setScrollOffset(NSPoint(x: off.x * z / old, y: off.y * z / old))
        canvas.needsDisplay = true
    }
}
