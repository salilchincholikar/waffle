import AppKit
import WaffleBridge

/// Search state shared by every window/tab, so one search spans all open files.
final class FindCenter {
    static let shared = FindCenter()
    static let changed = Notification.Name("WaffleFindChanged")

    struct Hit {
        weak var doc: Document?
        let sheet: Int
        let pos: CellPos
    }

    var query = "" { didSet { if query != oldValue { invalidate() } } }
    var replacement = ""
    var flags: Book.FindFlags = [] { didSet { if flags != oldValue { invalidate() } } }
    var visible = false

    private(set) var hits: [Hit] = []
    private(set) var index = -1
    private var stale = true
    /// The window that ran the last search (its tab order comes first).
    private weak var origin: SheetWindowController?

    private let perSheetLimit = 5_000
    private let totalLimit = 20_000

    func invalidate() {
        stale = true
        hits = []
        index = -1
        post()
    }

    func post() { NotificationCenter.default.post(name: FindCenter.changed, object: self) }

    /// Find as you type: run the search now if the query or options changed, so the count
    /// and highlights show before stepping to a match.
    func refresh(from wc: SheetWindowController) {
        if stale || origin == nil { search(from: wc) }
    }

    /// Open documents in tab order: the front window's tab group first, then the rest.
    private func orderedDocuments(from wc: SheetWindowController) -> [Document] {
        var out: [Document] = []
        let windows = wc.window.map { WindowTabs.shared.tabs(of: $0) } ?? []
        for w in windows {
            if let d = (w.windowController as? SheetWindowController)?.document as? Document, !out.contains(where: { $0 === d }) { out.append(d) }
        }
        for case let d as Document in NSDocumentController.shared.documents where !out.contains(where: { $0 === d }) {
            out.append(d)
        }
        return out
    }

    private func search(from wc: SheetWindowController) {
        hits = []
        index = -1
        stale = false
        origin = wc
        guard !query.isEmpty else { post(); return }
        let docs = orderedDocuments(from: wc)  // Find always covers every open file
        outer: for d in docs {
            guard let book = d.book, book.isLoaded else { continue }
            let sheets = (0..<book.sheetCount).filter { !book.sheetHidden($0) }
            for s in sheets {
                for p in book.findAll(s, query, flags, limit: perSheetLimit) {
                    hits.append(Hit(doc: d, sheet: s, pos: p))
                    if hits.count >= totalLimit { break outer }
                }
            }
        }
        post()
    }

    /// Move to the next/previous match relative to where the user is.
    func step(from wc: SheetWindowController, forward: Bool) {
        if stale || origin == nil { search(from: wc) }
        hits.removeAll { $0.doc == nil }
        guard !hits.isEmpty else { index = -1; post(); NSSound.beep(); return }
        if index < 0 || !isCurrent(hits[index], in: wc) {
            // Start from the user's position: first hit after the active cell in this file/sheet.
            let here = wc.grid.selection.active
            let docIdx = hits.firstIndex { $0.doc === wc.doc && ($0.sheet > wc.sheet || $0.sheet == wc.sheet && ($0.pos.r, $0.pos.c) > (here.r, here.c)) }
            index = forward ? (docIdx ?? 0) : ((docIdx ?? 0) - 1 + hits.count) % hits.count
        } else {
            index = (index + (forward ? 1 : -1) + hits.count) % hits.count
        }
        reveal(hits[index])
        post()
    }

    private func isCurrent(_ h: Hit, in wc: SheetWindowController) -> Bool {
        h.doc === wc.doc && h.sheet == wc.sheet && h.pos == wc.grid.selection.active
    }

    func reveal(_ h: Hit) {
        guard let doc = h.doc, let wc = doc.windowControllers.first as? SheetWindowController, let w = wc.window else { return }
        if !w.isKeyWindow {
            // Another tab: switch in place (no window animation); another window: bring it up.
            if WindowTabs.shared.group(of: w)?.selected !== w { WindowTabs.shared.select(w) } else { w.makeKeyAndOrderFront(nil) }
        }
        if wc.sheet != h.sheet { wc.switchSheet(h.sheet) }
        wc.grid.selection.set(h.pos)
        wc.grid.scrollToVisible(h.pos)
        wc.gridSelectionChanged(wc.grid)
    }

    /// Matches in one document's sheet (for highlighting).
    func matches(in doc: Document, sheet: Int) -> (all: Set<CellPos>, current: CellPos?) {
        var set = Set<CellPos>()
        for h in hits where h.doc === doc && h.sheet == sheet { set.insert(h.pos) }
        let cur = index >= 0 && index < hits.count && hits[index].doc === doc && hits[index].sheet == sheet ? hits[index].pos : nil
        return (set, cur)
    }

    var status: String {
        if query.isEmpty { return "" }
        if stale { return "" }
        if hits.isEmpty { return "No matches" }
        let more = hits.count >= totalLimit ? "+" : ""
        return index >= 0 ? "\(index + 1) of \(hits.count)\(more)" : "\(hits.count)\(more) found"
    }

    /// The document of the current match, for "Replace".
    var currentHit: Hit? { index >= 0 && index < hits.count ? hits[index] : nil }

    func replaceAll(from wc: SheetWindowController) -> Int {
        guard !query.isEmpty else { return 0 }
        let docs = orderedDocuments(from: wc)  // Find always covers every open file
        var total = 0
        for d in docs {
            guard let book = d.book, book.isLoaded else { continue }
            let sheets = (0..<book.sheetCount).filter { !book.sheetHidden($0) }
            var changed = 0
            for s in sheets {
                let n = book.replaceAll(s, query, replacement, flags)
                if n > 0 { changed += n }
            }
            if changed > 0, let c = d.windowControllers.first as? SheetWindowController {
                c.edited()
            }
            total += changed
        }
        invalidate()
        return total
    }
}
