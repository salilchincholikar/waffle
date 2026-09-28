@_exported import CWaffle
import Foundation

/// A cell position (0-based).
public struct CellPos: Hashable {
    public var r: Int
    public var c: Int
    public init(r: Int, c: Int) { self.r = r; self.c = c }
}

/// Inclusive rectangle of cells. Whole rows/columns extend to the sheet limits.
public struct CellRect: Equatable {
    public var r0: Int, c0: Int, r1: Int, c1: Int

    /// "Every row" marker for whole-column selections. A sheet's real limit is `Book.maxRows(_:)`
    /// (Excel's 1,048,576 for workbooks; unbounded for CSV).
    public static let maxRows = Int(Int32.max)
    public static let maxCols = 16_384

    public init(r0: Int, c0: Int, r1: Int, c1: Int) {
        self.r0 = min(r0, r1); self.r1 = max(r0, r1)
        self.c0 = min(c0, c1); self.c1 = max(c0, c1)
    }
    public init(_ p: CellPos) { self.init(r0: p.r, c0: p.c, r1: p.r, c1: p.c) }
    public init(_ a: CellPos, _ b: CellPos) { self.init(r0: a.r, c0: a.c, r1: b.r, c1: b.c) }

    public var isSingle: Bool { r0 == r1 && c0 == c1 }
    public var isFullRows: Bool { c0 == 0 && c1 >= CellRect.maxCols - 1 }
    public var isFullCols: Bool { r0 == 0 && r1 >= CellRect.maxRows - 1 }
    public var rows: Int { r1 - r0 + 1 }
    public var cols: Int { c1 - c0 + 1 }
    public func contains(_ p: CellPos) -> Bool { p.r >= r0 && p.r <= r1 && p.c >= c0 && p.c <= c1 }
    public func intersects(_ o: CellRect) -> Bool { r0 <= o.r1 && o.r0 <= r1 && c0 <= o.c1 && o.c0 <= c1 }
    public func union(_ o: CellRect) -> CellRect {
        CellRect(r0: min(r0, o.r0), c0: min(c0, o.c0), r1: max(r1, o.r1), c1: max(c1, o.c1))
    }
    public var xc: WfRect { WfRect(r0: UInt32(r0), c0: UInt32(c0), r1: UInt32(r1), c1: UInt32(c1)) }
    public init(_ x: WfRect) { self.init(r0: Int(x.r0), c0: Int(x.c0), r1: Int(x.r1), c1: Int(x.c1)) }
}

public enum FileKind: UInt32 {
    case xlsx = 0, xlsm, csv, importOnly, new
}

public func columnName(_ c: Int) -> String {
    var n = c
    var s = ""
    repeat {
        s = String(UnicodeScalar(UInt8(65 + n % 26))) + s
        n = n / 26 - 1
    } while n >= 0
    return s
}

public func cellName(_ p: CellPos) -> String { "\(columnName(p.c))\(p.r + 1)" }

/// Parse "B12", "b12", "$B$12" → position.
public func parseCellName(_ s: String) -> CellPos? {
    let t = s.trimmingCharacters(in: .whitespaces).uppercased().replacingOccurrences(of: "$", with: "")
    var letters = 0, i = t.startIndex
    while i < t.endIndex, let a = t[i].asciiValue, a >= 65, a <= 90 {
        letters = letters * 26 + Int(a - 64); i = t.index(after: i)
    }
    guard letters > 0, letters <= CellRect.maxCols, let row = Int(t[i...]), row >= 1, row <= CellRect.maxRows else { return nil }
    return CellPos(r: row - 1, c: letters - 1)
}

/// Swift face of a Rust document. All calls are cheap and synchronous.
public final class Book {
    public let ptr: OpaquePointer

    public init(ptr: OpaquePointer) { self.ptr = ptr }
    deinit { wf_close(ptr) }

    public static func open(_ url: URL) throws -> Book {
        guard let p = url.path.withCString({ wf_open($0) }) else {
            let msg = String(cString: wf_open_error())
            throw NSError(domain: "Waffle", code: 1, userInfo: [NSLocalizedDescriptionKey: msg.isEmpty ? "This file could not be opened." : msg])
        }
        return Book(ptr: p)
    }
    public static func empty() -> Book { Book(ptr: wf_new()) }

    private func str(_ p: UnsafePointer<CChar>?) -> String? { p.map { String(cString: $0) } }

    public var progress: Double { Double(wf_progress(ptr)) / 1000 }
    public var isLoaded: Bool { wf_loaded(ptr) }
    public func takeLoadError() -> String? { str(wf_take_load_error(ptr)) }
    public var lastError: String { str(wf_error(ptr)) ?? "Something went wrong." }
    public var kind: FileKind { FileKind(rawValue: wf_format(ptr)) ?? .xlsx }
    public var edited: Bool { wf_edited(ptr) }
    public var memory: UInt64 { wf_memory(ptr) }

    public func save(to url: URL, csv: Bool) throws {
        let ok = url.path.withCString { wf_save(ptr, $0, csv) }
        if !ok { throw NSError(domain: "Waffle", code: 2, userInfo: [NSLocalizedDescriptionKey: lastError]) }
    }

    // sheets
    public var sheetCount: Int { Int(wf_sheet_count(ptr)) }
    public func sheetName(_ i: Int) -> String { str(wf_sheet_name(ptr, UInt32(i))) ?? "" }
    public func sheetHidden(_ i: Int) -> Bool { wf_sheet_visibility(ptr, UInt32(i)) != 0 }
    public func sheetLoaded(_ i: Int) -> Bool { wf_sheet_loaded(ptr, UInt32(i)) }
    public var activeSheet: Int {
        get { Int(wf_active_sheet(ptr)) }
        set { wf_set_active_sheet(ptr, UInt32(newValue)) }
    }
    public func addSheet(at: Int) -> Int? { let i = wf_add_sheet(ptr, UInt32(at)); return i < 0 ? nil : Int(i) }
    public func renameSheet(_ i: Int, _ name: String) -> Bool { name.withCString { wf_rename_sheet(ptr, UInt32(i), $0) } }
    public func deleteSheet(_ i: Int) -> Bool { wf_delete_sheet(ptr, UInt32(i)) }
    public func moveSheet(_ from: Int, _ to: Int) -> Bool { wf_move_sheet(ptr, UInt32(from), UInt32(to)) }
    public func setSheetHidden(_ i: Int, _ h: Bool) -> Bool { wf_set_sheet_hidden(ptr, UInt32(i), h) }

    // geometry
    public func maxRows(_ s: Int) -> Int { Int(wf_max_rows(ptr, UInt32(s))) }
    public func rows(_ s: Int) -> Int { Int(wf_rows(ptr, UInt32(s))) }
    public func cols(_ s: Int) -> Int { Int(wf_cols(ptr, UInt32(s))) }
    public func dataRows(_ s: Int) -> Int { Int(wf_data_rows(ptr, UInt32(s))) }
    public func dataCols(_ s: Int) -> Int { Int(wf_data_cols(ptr, UInt32(s))) }
    public func rowY(_ s: Int, _ r: Int) -> Double { wf_row_y(ptr, UInt32(s), UInt32(clamping: r)) }
    public func colX(_ s: Int, _ c: Int) -> Double { wf_col_x(ptr, UInt32(s), UInt32(clamping: c)) }
    public func rowAt(_ s: Int, _ y: Double) -> Int { Int(wf_row_at(ptr, UInt32(s), y)) }
    public func colAt(_ s: Int, _ x: Double) -> Int { Int(wf_col_at(ptr, UInt32(s), x)) }
    public func totalHeight(_ s: Int) -> Double { wf_total_height(ptr, UInt32(s)) }
    public func totalWidth(_ s: Int) -> Double { wf_total_width(ptr, UInt32(s)) }
    /// The sheet hides gridlines (Excel's View ▸ Gridlines off).
    public func hidesGridlines(_ s: Int) -> Bool { wf_hide_gridlines(ptr, UInt32(s)) }
    public func freeze(_ s: Int) -> (rows: Int, cols: Int) { (Int(wf_freeze_rows(ptr, UInt32(s))), Int(wf_freeze_cols(ptr, UInt32(s)))) }
    public func rowHidden(_ s: Int, _ r: Int) -> Bool { wf_row_hidden(ptr, UInt32(s), UInt32(clamping: r)) }
    public func colHidden(_ s: Int, _ c: Int) -> Bool { wf_col_hidden(ptr, UInt32(s), UInt32(clamping: c)) }
    public func rowFiltered(_ s: Int, _ r: Int) -> Bool { wf_row_filtered(ptr, UInt32(s), UInt32(clamping: r)) }
    public func hasFormula(_ s: Int, _ p: CellPos) -> Bool { wf_has_formula(ptr, UInt32(s), UInt32(p.r), UInt32(p.c)) }
    public func isBlank(_ s: Int, _ p: CellPos) -> Bool { wf_is_blank(ptr, UInt32(s), UInt32(p.r), UInt32(p.c)) }

    private var mergeBuf = [WfRect](repeating: WfRect(), count: 512)
    public func merges(_ s: Int, in area: CellRect) -> [CellRect] {
        let n = mergeBuf.withUnsafeMutableBufferPointer { wf_merges(ptr, UInt32(s), area.xc, $0.baseAddress, 512) }
        return (0..<Int(n)).map { CellRect(mergeBuf[$0]) }
    }
    public func merge(at p: CellPos, sheet s: Int) -> CellRect? {
        var r = WfRect()
        return wf_merge_at(ptr, UInt32(s), UInt32(p.r), UInt32(p.c), &r) ? CellRect(r) : nil
    }

    /// Visible cells, row-major. The buffers are valid until the next fetch.
    public func fetch(_ s: Int, _ area: CellRect) -> (UnsafeBufferPointer<WfCell>, UnsafePointer<UInt8>?) {
        var cells: UnsafePointer<WfCell>?
        var text: UnsafePointer<UInt8>?
        let n = wf_fetch(ptr, UInt32(s), area.xc, &cells, &text)
        return (UnsafeBufferPointer(start: cells, count: Int(n)), text)
    }

    public func editText(_ s: Int, _ p: CellPos) -> String { str(wf_edit_text(ptr, UInt32(s), UInt32(p.r), UInt32(p.c))) ?? "" }
    public func displayText(_ s: Int, _ p: CellPos) -> String { str(wf_display_text(ptr, UInt32(s), UInt32(p.r), UInt32(p.c))) ?? "" }
    public func jump(_ s: Int, from p: CellPos, dr: Int, dc: Int) -> CellPos {
        var r: UInt32 = UInt32(p.r), c: UInt32 = UInt32(p.c)
        wf_jump(ptr, UInt32(s), UInt32(p.r), UInt32(p.c), Int32(dr), Int32(dc), &r, &c)
        return CellPos(r: Int(r), c: Int(c))
    }

    /// Rich-text runs for a cell: (text, run, font name).
    public func runs(_ s: Int, _ p: CellPos, text: String) -> [(String, WfRun, String?)] {
        var buf = [WfRun](repeating: WfRun(), count: 64)
        let n = Int(buf.withUnsafeMutableBufferPointer { wf_runs(ptr, UInt32(s), UInt32(p.r), UInt32(p.c), $0.baseAddress, 64) })
        let bytes = Array(text.utf8)
        return (0..<n).compactMap { i in
            let r = buf[i]
            let a = Int(r.start), b = min(Int(r.end), bytes.count)
            guard a < b else { return nil }
            let font = str(wf_run_font(ptr, UInt32(s), UInt32(p.r), UInt32(p.c), UInt32(i)))
            return (String(decoding: bytes[a..<b], as: UTF8.self), r, font)
        }
    }

    // drawings
    public func drawings(_ s: Int) -> [(WfDrawing, String)] {
        let n = Int(wf_drawing_count(ptr, UInt32(s)))
        return (0..<n).compactMap { i in
            var d = WfDrawing()
            guard wf_drawing(ptr, UInt32(s), UInt32(i), &d) else { return nil }
            return (d, str(wf_drawing_ref(ptr, UInt32(s), UInt32(i))) ?? "")
        }
    }
    public func partBytes(_ path: String) -> Data? {
        var len = 0
        guard let p = path.withCString({ wf_part_bytes(ptr, $0, &len) }) else { return nil }
        return Data(bytes: p, count: len)
    }

    // styles
    public var styleGeneration: UInt32 { wf_style_generation(ptr) }
    public var styleCount: Int { Int(wf_style_count(ptr)) }
    public func style(_ id: Int) -> WfStyle { var s = WfStyle(); wf_style(ptr, UInt32(id), &s); return s }
    public func fontName(_ id: Int) -> String { str(wf_style_font_name(ptr, UInt32(id))) ?? "Calibri" }
    public func numberFormat(_ id: Int) -> String { str(wf_style_numfmt(ptr, UInt32(id))) ?? "General" }
    public func cellStyle(_ s: Int, _ p: CellPos) -> Int { Int(wf_cell_style(ptr, UInt32(s), UInt32(p.r), UInt32(p.c))) }

    public enum StyleKind: UInt32 {
        case bold = 0, italic, underline, strike, fontSize, fontName, textColor, fill, numberFormat, hAlign, vAlign, wrap, borders
    }
    @discardableResult
    public func applyStyle(_ s: Int, _ rects: [CellRect], _ kind: StyleKind, num: Double = 0, text: String = "", color: Double = -1) -> Bool {
        var xs = rects.map(\.xc)
        return text.withCString { t in xs.withUnsafeMutableBufferPointer { wf_apply_style(ptr, UInt32(s), $0.baseAddress, UInt32($0.count), kind.rawValue, num, t, color) } }
    }

    // edits
    public func setInput(_ s: Int, _ p: CellPos, _ text: String) -> Bool { text.withCString { wf_set_input(ptr, UInt32(s), UInt32(p.r), UInt32(p.c), $0) } }
    public func setInput(_ s: Int, _ rect: CellRect, _ text: String) -> Bool { text.withCString { wf_set_input_range(ptr, UInt32(s), rect.xc, $0) } }
    public enum ClearWhat: UInt32 { case contents = 0, formats, all }
    public func clear(_ s: Int, _ rects: [CellRect], _ what: ClearWhat) -> Bool {
        var xs = rects.map(\.xc)
        return xs.withUnsafeMutableBufferPointer { wf_clear(ptr, UInt32(s), $0.baseAddress, UInt32($0.count), what.rawValue) }
    }
    public func insertRows(_ s: Int, at: Int, count: Int) -> Bool { wf_insert_rows(ptr, UInt32(s), UInt32(at), UInt32(count)) }
    public func deleteRows(_ s: Int, at: Int, count: Int) -> Bool { wf_delete_rows(ptr, UInt32(s), UInt32(at), UInt32(count)) }
    public func insertCols(_ s: Int, at: Int, count: Int) -> Bool { wf_insert_cols(ptr, UInt32(s), UInt32(at), UInt32(count)) }
    public func deleteCols(_ s: Int, at: Int, count: Int) -> Bool { wf_delete_cols(ptr, UInt32(s), UInt32(at), UInt32(count)) }
    public func setColWidth(_ s: Int, _ c0: Int, _ c1: Int, px: Double) -> Bool { wf_set_col_width(ptr, UInt32(s), UInt32(c0), UInt32(c1), px) }
    public func setRowHeight(_ s: Int, _ r0: Int, _ r1: Int, px: Double) -> Bool { wf_set_row_height(ptr, UInt32(s), UInt32(r0), UInt32(r1), px) }
    public func setHidden(_ s: Int, rows: Bool, _ a0: Int, _ a1: Int, _ hidden: Bool) -> Bool { wf_set_hidden(ptr, UInt32(s), rows ? 0 : 1, UInt32(a0), UInt32(a1), hidden) }
    public func setFreeze(_ s: Int, rows: Int, cols: Int) -> Bool { wf_set_freeze(ptr, UInt32(s), UInt32(rows), UInt32(cols)) }
    public func merge(_ s: Int, _ r: CellRect) -> Bool { wf_merge(ptr, UInt32(s), r.xc) }
    public func unmerge(_ s: Int, _ r: CellRect) -> Bool { wf_unmerge(ptr, UInt32(s), r.xc) }
    public func fill(_ s: Int, _ r: CellRect, down: Bool) -> Bool { wf_fill(ptr, UInt32(s), r.xc, down) }
    public func fillFrom(_ s: Int, _ src: CellRect, _ target: CellRect) -> Bool { wf_fill_from(ptr, UInt32(s), src.xc, target.xc) }

    // undo
    public func undo() -> Int? { var s: UInt32 = 0; return wf_undo(ptr, &s) ? Int(s) : nil }
    public func redo() -> Int? { var s: UInt32 = 0; return wf_redo(ptr, &s) ? Int(s) : nil }
    public var undoLabel: String? { str(wf_undo_label(ptr)) }
    public var redoLabel: String? { str(wf_redo_label(ptr)) }

    // data
    public func sort(_ s: Int, r0: Int, r1: Int, keys: [(col: Int, ascending: Bool)]) -> Bool {
        var cols = keys.map { UInt32($0.col) }
        var asc = keys.map(\.ascending)
        return cols.withUnsafeMutableBufferPointer { c in asc.withUnsafeMutableBufferPointer { a in
            wf_sort(ptr, UInt32(s), UInt32(r0), UInt32(r1), c.baseAddress, a.baseAddress, UInt32(keys.count))
        } }
    }
    public func removeEmptyRows(_ s: Int, r0: Int, r1: Int) -> Int { Int(wf_remove_empty_rows(ptr, UInt32(s), UInt32(r0), UInt32(r1))) }
    public func removeDuplicates(_ s: Int, r0: Int, r1: Int, cols: [Int]) -> Int {
        var cs = cols.map { UInt32($0) }
        return Int(cs.withUnsafeMutableBufferPointer { wf_remove_duplicates(ptr, UInt32(s), UInt32(r0), UInt32(r1), $0.baseAddress, UInt32($0.count)) })
    }
    public enum Transform: UInt32 { case trim = 0, upper, lower, title }
    public func transform(_ s: Int, _ rects: [CellRect], _ how: Transform) -> Int {
        var xs = rects.map(\.xc)
        return Int(xs.withUnsafeMutableBufferPointer { wf_transform(ptr, UInt32(s), $0.baseAddress, UInt32($0.count), how.rawValue) })
    }
    public func normalizeDates(_ s: Int, _ rects: [CellRect], order: Int, format: String) -> Int {
        var xs = rects.map(\.xc)
        return Int(format.withCString { f in xs.withUnsafeMutableBufferPointer { wf_normalize_dates(ptr, UInt32(s), $0.baseAddress, UInt32($0.count), UInt32(order), f) } })
    }
    public func normalizeAmounts(_ s: Int, _ rects: [CellRect], decimalComma: Bool, format: String) -> Int {
        var xs = rects.map(\.xc)
        return Int(format.withCString { f in xs.withUnsafeMutableBufferPointer { wf_normalize_amounts(ptr, UInt32(s), $0.baseAddress, UInt32($0.count), decimalComma, f) } })
    }
    public func textToColumns(_ s: Int, col: Int, r0: Int, r1: Int, delimiter: String) -> Int {
        Int(delimiter.withCString { wf_text_to_columns(ptr, UInt32(s), UInt32(col), UInt32(r0), UInt32(r1), $0) })
    }

    public struct FindFlags: OptionSet {
        public let rawValue: UInt32
        public init(rawValue: UInt32) { self.rawValue = rawValue }
        public static let matchCase = FindFlags(rawValue: 1)
        public static let wholeCell = FindFlags(rawValue: 2)
        public static let formulas = FindFlags(rawValue: 4)
    }
    public func find(_ s: Int, from p: CellPos, _ q: String, _ f: FindFlags, forward: Bool) -> CellPos? {
        var r: UInt32 = 0, c: UInt32 = 0
        let ok = q.withCString { wf_find(ptr, UInt32(s), UInt32(p.r), UInt32(p.c), $0, f.rawValue, forward, &r, &c) }
        return ok ? CellPos(r: Int(r), c: Int(c)) : nil
    }
    public func findAll(_ s: Int, _ q: String, _ f: FindFlags, limit: Int) -> [CellPos] {
        var buf = [UInt32](repeating: 0, count: limit * 2)
        let n = Int(q.withCString { qq in buf.withUnsafeMutableBufferPointer { wf_find_all(ptr, UInt32(s), qq, f.rawValue, $0.baseAddress, UInt32(limit)) } })
        return (0..<n).map { CellPos(r: Int(buf[$0 * 2]), c: Int(buf[$0 * 2 + 1])) }
    }
    public func countMatches(_ s: Int, _ q: String, _ f: FindFlags) -> Int { Int(q.withCString { wf_count_matches(ptr, UInt32(s), $0, f.rawValue) }) }
    public func replaceOne(_ s: Int, _ p: CellPos, _ q: String, _ w: String, _ f: FindFlags) -> Bool {
        q.withCString { qq in w.withCString { wf_replace_one(ptr, UInt32(s), UInt32(p.r), UInt32(p.c), qq, $0, f.rawValue) } }
    }
    public func replaceAll(_ s: Int, _ q: String, _ w: String, _ f: FindFlags) -> Int {
        Int(q.withCString { qq in w.withCString { wf_replace_all(ptr, UInt32(s), qq, $0, f.rawValue) } })
    }

    public func filterValues(_ s: Int, header: Int, col: Int, limit: Int = 1000) -> [(String, Int)] {
        let raw = str(wf_filter_values(ptr, UInt32(s), UInt32(header), UInt32(col), UInt32(limit))) ?? ""
        return raw.split(separator: "\n", omittingEmptySubsequences: true).map {
            let parts = $0.split(separator: "\t", omittingEmptySubsequences: false)
            return (String(parts.first ?? ""), Int(parts.last ?? "0") ?? 0)
        }
    }
    public enum FilterMode: UInt32 { case clear = 0, values, contains, blanks, nonBlanks }
    @discardableResult
    public func setFilter(_ s: Int, header: Int, col: Int, _ mode: FilterMode, _ payload: String = "") -> Int {
        Int(payload.withCString { wf_set_filter(ptr, UInt32(s), UInt32(header), UInt32(col), mode.rawValue, $0) })
    }
    public func clearFilters(_ s: Int) { wf_clear_filters(ptr, UInt32(s)) }
    public func filterActive(_ s: Int, col: Int) -> Bool { wf_filter_active(ptr, UInt32(s), UInt32(col)) }

    public func stats(_ s: Int, _ rects: [CellRect]) -> WfStats {
        var out = WfStats()
        var xs = rects.map(\.xc)
        xs.withUnsafeMutableBufferPointer { wf_stats(ptr, UInt32(s), $0.baseAddress, UInt32($0.count), &out) }
        return out
    }

    // clipboard
    public func copy(_ s: Int, _ r: CellRect) -> String { str(wf_copy(ptr, UInt32(s), r.xc)) ?? "" }
    public enum PasteWhat: UInt32 { case all = 0, values, formats }
    public func pasteClip(_ s: Int, _ target: CellRect, _ what: PasteWhat) -> CellRect? {
        var out = WfRect()
        return wf_paste_clip(ptr, UInt32(s), target.xc, what.rawValue, &out) ? CellRect(out) : nil
    }
    public func pasteText(_ s: Int, at p: CellPos, _ text: String) -> CellRect? {
        var out = WfRect()
        return text.withCString { wf_paste_text(ptr, UInt32(s), UInt32(p.r), UInt32(p.c), $0, &out) } ? CellRect(out) : nil
    }
}
