import AppKit
import WaffleBridge
import CWaffle

/// Draws headers, frozen panes, cells and selection; handles mouse and keyboard.
final class GridCanvas: NSView, NSMenuItemValidation {
    weak var grid: GridView?
    var book: Book?
    var sheet = 0
    var zoom: CGFloat = 1
    var selection = Selection() { didSet { needsDisplay = true } }
    var filterMode = false { didSet { needsDisplay = true } }
    var filterHeaderRow = 0
    var cutRect: CellRect? { didSet { needsDisplay = true } }
    /// Search matches on this sheet, and the one the user is on.
    var highlights: Set<CellPos> = [] { didSet { if highlights != oldValue { needsDisplay = true } } }
    var currentHighlight: CellPos? { didSet { if currentHighlight != oldValue { needsDisplay = true } } }
    var showFormulas = false

    private(set) var headerHeight: CGFloat = 22
    private(set) var rowHeaderWidth: CGFloat = 44

    var styles: [Int: CellStyle] = [:]
    var styleGeneration: UInt32 = .max
    var lines: [LineKey: CTLine] = [:]

    struct LineKey: Hashable {
        let text: String
        let style: Int
        let color: UInt32
        let cf: UInt8
        let lightFill: Bool
    }

    /// The fill drawn behind a cell: a conditional format or table band, else its own fill.
    func backgroundFill(_ cell: WfCell, _ st: CellStyle) -> CGColor? {
        cell.cf_fill != 0 ? palette.tone(NSColor(rgb: cell.cf_fill)) : st.fill
    }

    override var isFlipped: Bool { true }
    override var isOpaque: Bool { true }
    override var acceptsFirstResponder: Bool { true }
    override var wantsUpdateLayer: Bool { false }

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        layerContentsRedrawPolicy = .onSetNeedsDisplay
        NotificationCenter.default.addObserver(self, selector: #selector(sheetAppearanceChanged), name: SheetAppearance.changed, object: nil)
    }
    required init?(coder: NSCoder) { fatalError() }

    // ---- geometry ------------------------------------------------------------

    var scrollOffset: NSPoint {
        guard let sv = grid?.scrollView else { return .zero }
        let b = sv.contentView.bounds.origin
        let ins = sv.contentInsets
        return NSPoint(x: b.x + ins.left, y: b.y + ins.top)
    }

    var frozen: (rows: Int, cols: Int) { book?.freeze(sheet) ?? (0, 0) }
    var frozenSize: NSSize {
        guard let book else { return .zero }
        let f = frozen
        return NSSize(width: book.colX(sheet, f.cols) * zoom, height: book.rowY(sheet, f.rows) * zoom)
    }

    /// The scrolling cell area in canvas coordinates.
    var mainArea: NSRect {
        let fz = frozenSize
        let x = rowHeaderWidth + fz.width, y = headerHeight + fz.height
        return NSRect(x: x, y: y, width: max(0, bounds.width - x), height: max(0, bounds.height - y))
    }

    func updateHeaderMetrics() {
        headerHeight = (20 * max(zoom, 0.8)).rounded()
        guard let book else { return }
        let digits = String(max(book.rows(sheet), 1000)).count
        let font = NSFont.monospacedDigitSystemFont(ofSize: 11 * max(zoom, 0.8), weight: .regular)
        let w = ("8" as NSString).size(withAttributes: [.font: font]).width
        rowHeaderWidth = (CGFloat(digits) * w + 16).rounded()
    }

    /// Canvas x of a column's left edge.
    func xOf(_ c: Int) -> CGFloat {
        guard let book else { return 0 }
        let fc = frozen.cols
        let x = book.colX(sheet, c) * zoom
        if c < fc { return rowHeaderWidth + x }
        return rowHeaderWidth + x - scrollOffset.x
    }

    func yOf(_ r: Int) -> CGFloat {
        guard let book else { return 0 }
        let fr = frozen.rows
        let y = book.rowY(sheet, r) * zoom
        if r < fr { return headerHeight + y }
        return headerHeight + y - scrollOffset.y
    }

    /// Canvas rect for a cell range as seen in the pane that holds its top-left.
    func rectOf(_ r: CellRect) -> NSRect {
        let x0 = xOf(r.c0), y0 = yOf(r.r0)
        let x1 = xOfEnd(r.c1), y1 = yOfEnd(r.r1)
        return NSRect(x: x0, y: y0, width: max(0, x1 - x0), height: max(0, y1 - y0))
    }

    /// Right edge of column c (in c's own pane).
    func xOfEnd(_ c: Int) -> CGFloat {
        guard let book else { return 0 }
        let w = (book.colX(sheet, c + 1) - book.colX(sheet, c)) * zoom
        return xOf(c) + w
    }
    func yOfEnd(_ r: Int) -> CGFloat {
        guard let book else { return 0 }
        let h = (book.rowY(sheet, r + 1) - book.rowY(sheet, r)) * zoom
        return yOf(r) + h
    }

    enum Hit: Equatable {
        case corner
        case colHeader(Int, resize: Bool)
        case rowHeader(Int, resize: Bool)
        case cell(CellPos)
        case none
    }

    /// Column under a canvas x (cells area).
    func colAt(_ x: CGFloat) -> Int {
        guard let book else { return 0 }
        let fz = frozenSize
        let local = x - rowHeaderWidth
        if local < fz.width { return book.colAt(sheet, Double(max(0, local) / zoom)) }
        return book.colAt(sheet, Double((local + scrollOffset.x) / zoom))
    }
    func rowAt(_ y: CGFloat) -> Int {
        guard let book else { return 0 }
        let fz = frozenSize
        let local = y - headerHeight
        if local < fz.height { return book.rowAt(sheet, Double(max(0, local) / zoom)) }
        return book.rowAt(sheet, Double((local + scrollOffset.y) / zoom))
    }

    func hit(_ p: NSPoint) -> Hit {
        if p.x < rowHeaderWidth && p.y < headerHeight { return .corner }
        if p.y < headerHeight {
            let c = colAt(p.x)
            // Near the right edge of a column (or left edge of the next) → resize.
            let right = xOfEnd(c), left = xOf(c)
            if abs(p.x - right) <= 4 { return .colHeader(c, resize: true) }
            if abs(p.x - left) <= 3, c > 0 { return .colHeader(previousVisibleCol(c), resize: true) }
            return .colHeader(c, resize: false)
        }
        if p.x < rowHeaderWidth {
            let r = rowAt(p.y)
            let bottom = yOfEnd(r), top = yOf(r)
            if abs(p.y - bottom) <= 3 { return .rowHeader(r, resize: true) }
            if abs(p.y - top) <= 2, r > 0 { return .rowHeader(previousVisibleRow(r), resize: true) }
            return .rowHeader(r, resize: false)
        }
        return .cell(CellPos(r: rowAt(p.y), c: colAt(p.x)))
    }

    func previousVisibleCol(_ c: Int) -> Int {
        var i = c - 1
        while i > 0, book?.colHidden(sheet, i) == true { i -= 1 }
        return max(0, i)
    }
    func previousVisibleRow(_ r: Int) -> Int {
        var i = r - 1
        while i > 0, book?.rowHidden(sheet, i) == true { i -= 1 }
        return max(0, i)
    }

    // ---- styles ----------------------------------------------------------------

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        updatePalette()
    }

    @objc private func sheetAppearanceChanged() { updatePalette() }

    /// Re-resolve the sheet colours (setting or system appearance changed).
    func updatePalette() {
        let p = SheetPalette.resolve(for: self)
        grid?.scrollView.backgroundColor = NSColor(cgColor: p.background) ?? .white
        guard p.isDark != palette.isDark else { return }
        palette = p
        invalidateStyles()
        editor?.apply(p)
        needsDisplay = true
    }

    func invalidateStyles() {
        drawingCache = nil
        styles.removeAll(keepingCapacity: true)
        lines.removeAll(keepingCapacity: true)
        styleGeneration = .max
    }

    func style(_ id: Int) -> CellStyle {
        if let book, book.styleGeneration != styleGeneration {
            styles.removeAll(keepingCapacity: true)
            lines.removeAll(keepingCapacity: true)
            styleGeneration = book.styleGeneration
        }
        if let s = styles[id] { return s }
        let s = CellStyle(book: book!, id: id, zoom: zoom, palette: palette)
        styles[id] = s
        return s
    }

    func line(_ text: String, style id: Int, color: UInt32, cf: UInt8 = 0, fill: CGColor? = nil) -> CTLine {
        let st = style(id)
        let bg = fill ?? st.fill
        let key = LineKey(text: text, style: id, color: color, cf: cf, lightFill: palette.isLight(bg))
        if let l = lines[key] { return l }
        if lines.count > 20_000 { lines.removeAll(keepingCapacity: true) }
        var font = st.font
        if cf & 1 != 0 { font = NSFontManager.shared.convert(font, toHaveTrait: .boldFontMask) }
        if cf & 2 != 0 { font = NSFontManager.shared.convert(font, toHaveTrait: .italicFontMask) }
        var attrs: [NSAttributedString.Key: Any] = [.font: font, .foregroundColor: palette.text(color, over: bg, auto: st.color)]
        if st.underline || cf & 8 != 0 { attrs[.underlineStyle] = NSUnderlineStyle.single.rawValue }
        if st.strike || cf & 4 != 0 { attrs[.strikethroughStyle] = NSUnderlineStyle.single.rawValue }
        let l = CTLineCreateWithAttributedString(NSAttributedString(string: text, attributes: attrs))
        lines[key] = l
        return l
    }


    // State used by the drawing / input extensions.
    enum Drag {
        case cells(start: CellPos, additive: Bool)
        case rows(start: Int)
        case cols(start: Int)
        case resizeCol(Int, startX: CGFloat, startWidth: CGFloat)
        case resizeRow(Int, startY: CGFloat, startHeight: CGFloat)
        case fillHandle(CellRect)
    }
    var palette = SheetPalette.light
    var images: [String: NSImage] = [:]
    var drawingCache: (sheet: Int, version: Int, items: [(WfDrawing, String)])?
    var accent: NSColor { NSColor.controlAccentColor.usingColorSpace(.sRGB) ?? .systemBlue }
    var editor: CellEditor?
    var drag: Drag?
    var fillPreview: CellRect?
    var resizeGuide: (vertical: Bool, at: CGFloat)?
    var autoscrollTimer: Timer?
    var lastDragEvent: NSEvent?
}
