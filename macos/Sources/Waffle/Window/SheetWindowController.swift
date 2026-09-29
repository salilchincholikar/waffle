import AppKit
import WaffleBridge
import CWaffle

final class SheetWindowController: NSWindowController, NSWindowDelegate, GridViewDelegate, CellEditorOwner, SheetTabsDelegate, NSTextFieldDelegate, NSMenuItemValidation {
    let grid = GridView()
    let bottom = BottomBar()
    let nameBox = NSTextField()
    let formula = NSTextField()
    let findBar = FindBar()
    var loadTimer: Timer?
    var filterHeader = 0
    var statsWork: DispatchWorkItem?

    var doc: Document { document as! Document }
    var book: Book { doc.book! }
    var sheet: Int { grid.sheet }

    init(document: Document) {
        let window = DocumentWindow(contentRect: NSRect(x: 0, y: 0, width: 1200, height: 780), styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView], backing: .buffered, defer: false)
        window.minSize = NSSize(width: 640, height: 360)
        // Waffle's own tabs (WindowTabs); macOS tabbing would add a tab-bar row.
        window.tabbingMode = .disallowed
        super.init(window: window)
        window.delegate = self
        shouldCascadeWindows = true
        configureTitleBar()
        buildUI()
        NotificationCenter.default.addObserver(self, selector: #selector(findChanged(_:)), name: FindCenter.changed, object: nil)
    }

    // Clipboard and formatting-control state (used by extensions).
    let clipToken = UUID().uuidString
    var cutSource: (sheet: Int, rect: CellRect)?
    let styleSeg = NSSegmentedControl()
    let alignSeg = NSSegmentedControl()
    /// Which colour ("More Colors…" panel) is being picked: 0 text, 1 fill.
    var pendingColorKind = 0
    /// Colour pull-downs on the formula row (their icon shows the last colour used).
    var textColorButton: NSPopUpButton?
    var fillColorButton: NSPopUpButton?
    /// Open files as tabs at the left of the title row (one file: just its name).
    let titleTabs = TitleTabs()
    private var titleObservation: NSKeyValueObservation?

    // ---- tabs ------------------------------------------------------------------------------

    /// The "+" after the file tabs: open files as tabs of this window.
    @objc override func newWindowForTab(_ sender: Any?) {
        NSDocumentController.shared.openDocument(sender)
    }

    // ---- file tabs (WindowTabs) ---------------------------------------------------------------

    @objc func showNextFileTab(_ sender: Any?) { if let w = window { WindowTabs.shared.step(from: w, by: 1) } }
    @objc func showPreviousFileTab(_ sender: Any?) { if let w = window { WindowTabs.shared.step(from: w, by: -1) } }
    @objc func moveFileTabToNewWindow(_ sender: Any?) { if let w = window { WindowTabs.shared.moveToNewWindow(w) } }
    @objc func mergeAllFileWindows(_ sender: Any?) { if let w = window { WindowTabs.shared.mergeAll(into: w) } }

    func windowDidBecomeKey(_ notification: Notification) {
        if let w = window { WindowTabs.shared.didBecomeKey(w) }
        titleTabs.reload()
        findChanged(Notification(name: FindCenter.changed))
    }

    required init?(coder: NSCoder) { fatalError() }

    override func windowTitle(forDocumentDisplayName displayName: String) -> String { displayName }

    // ---- layout -------------------------------------------------------------------

    func buildUI() {
        guard let content = window?.contentView else { return }
        // Formula bar
        nameBox.placeholderString = "A1"
        nameBox.alignment = .center
        nameBox.font = .monospacedDigitSystemFont(ofSize: 12, weight: .regular)
        nameBox.bezelStyle = .roundedBezel
        nameBox.delegate = self
        nameBox.toolTip = "Cell reference — type one (e.g. B12) and press Return to jump"
        let fx = NSTextField(labelWithString: "ƒx")
        fx.font = .systemFont(ofSize: 13, weight: .medium)
        fx.textColor = .tertiaryLabelColor
        formula.font = .systemFont(ofSize: 13)
        formula.isBordered = false
        formula.drawsBackground = false
        formula.focusRingType = .none
        formula.delegate = self
        formula.placeholderString = "Value or =formula"
        formula.lineBreakMode = .byTruncatingTail
        formula.cell?.usesSingleLineMode = true
        // Formula bar: standard text fields in a plain bar, like Numbers.
        formula.isBordered = true
        formula.isBezeled = true
        formula.bezelStyle = .roundedBezel
        formula.drawsBackground = true
        formula.focusRingType = .default
        // Title row (standard title-bar height, like Helium): file tabs at the left, then
        // Find and the sheet tools at the right.
        let find = findBar.view
        // 400 wide (its toggles and count sit inside), narrowing to 300 before it would
        // crowd the tabs.
        find.widthAnchor.constraint(equalToConstant: 400).withPriority(.init(740)).isActive = true
        find.widthAnchor.constraint(greaterThanOrEqualToConstant: 300).isActive = true
        findBar.controller = self
        let titleRow = NSStackView(views: [find])
        titleRow.spacing = 8
        let tools = NSStackView(views: buildToolControls())
        tools.spacing = 6
        for v in tools.arrangedSubviews { (v as? NSControl)?.controlSize = .small }

        // Formula row: reference, formula, then the cell formatting controls.
        let format = buildFormatControls()
        let fbar = NSStackView(views: [nameBox, fx, formula] + format)
        formula.setContentHuggingPriority(.defaultLow, for: .horizontal)
        formula.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        formula.widthAnchor.constraint(greaterThanOrEqualToConstant: 180).isActive = true
        fbar.spacing = 8
        // .fill: the formula field gives up width; gravity areas would overlap the buttons.
        fbar.distribution = .fill
        // Narrow windows drop the least-used controls first (still in the Format menu)
        // instead of the row setting a minimum window width.
        fbar.setClippingResistancePriority(.defaultLow, for: .horizontal)
        for (v, p) in zip(format, [900, 700, 700, 800, 500, 600, 400, 300] as [Float]) {
            fbar.setVisibilityPriority(NSStackView.VisibilityPriority(rawValue: p), for: v)
        }
        fbar.edgeInsets = NSEdgeInsets(top: 3, left: BottomBar.inset, bottom: 3, right: BottomBar.inset)
        nameBox.widthAnchor.constraint(equalToConstant: 80).isActive = true

        grid.delegate = self
        bottom.tabs.delegate = self

        let sep = NSBox()
        sep.boxType = .separator
        let stack = NSStackView(views: [fbar, sep, grid, bottom])
        stack.orientation = .vertical
        stack.spacing = 0
        // .fill lets the grid absorb height changes; the default gravity areas clip and
        // overlap views instead (the grid could end up under the sheet tabs).
        stack.distribution = .fill
        stack.alignment = .leading
        stack.setHuggingPriority(.defaultLow, for: .horizontal)
        stack.translatesAutoresizingMaskIntoConstraints = false
        // Translucent chrome behind the title and formula rows (the desktop shows through,
        // like Finder and Safari). The sheet itself stays opaque.
        let chrome = NSVisualEffectView.windowChrome()
        content.addSubview(chrome)
        content.addSubview(stack)
        titleTabs.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        titleObservation = window?.observe(\.title, options: [.initial, .new]) { [weak self] _, _ in
            self?.titleTabs.reload()
        }
        for v in [titleTabs, titleRow, tools] as [NSView] {
            v.translatesAutoresizingMaskIntoConstraints = false
            content.addSubview(v)
        }
        // Clear of the traffic lights.
        let lights = (window?.standardWindowButton(.zoomButton)?.frame.maxX ?? 68) + 14
        for v in [fbar, sep, grid, bottom] as [NSView] {
            v.translatesAutoresizingMaskIntoConstraints = false
            v.widthAnchor.constraint(equalTo: stack.widthAnchor).isActive = true
        }
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: content.leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: content.trailingAnchor),
            stack.topAnchor.constraint(equalTo: content.safeAreaLayoutGuide.topAnchor),
            stack.bottomAnchor.constraint(equalTo: content.bottomAnchor),
            chrome.topAnchor.constraint(equalTo: content.topAnchor),
            chrome.leadingAnchor.constraint(equalTo: content.leadingAnchor),
            chrome.trailingAnchor.constraint(equalTo: content.trailingAnchor),
            chrome.bottomAnchor.constraint(equalTo: grid.topAnchor),
            titleRow.centerYAnchor.constraint(equalTo: content.topAnchor, constant: Self.titleBarHeight / 2),
            titleRow.trailingAnchor.constraint(equalTo: tools.leadingAnchor, constant: -12),

            titleTabs.leadingAnchor.constraint(equalTo: content.leadingAnchor, constant: lights),
            titleTabs.centerYAnchor.constraint(equalTo: titleRow.centerYAnchor),
            titleTabs.trailingAnchor.constraint(lessThanOrEqualTo: titleRow.leadingAnchor, constant: -12),
            tools.centerYAnchor.constraint(equalTo: titleRow.centerYAnchor),
            tools.trailingAnchor.constraint(equalTo: content.trailingAnchor, constant: -BottomBar.inset),

        ])
        grid.setContentHuggingPriority(.defaultLow, for: .vertical)
        // Typing goes to the sheet, not the Find field.
        window?.initialFirstResponder = grid.canvas
    }

    override func windowDidLoad() { super.windowDidLoad() }

    /// Called by the document once the book is ready.
    func attach() {
        loadTimer?.invalidate()
        loadTimer = nil
        grid.canvas.filterMode = false
        grid.selection = Selection()
        grid.book = book
        grid.sheet = book.activeSheet
        syncSavedFilter()
        refreshTabs()
        updateFormulaBar()
        // The window is in its tab group by now (it may open without becoming key).
        DispatchQueue.main.async { [weak self] in self?.titleTabs.reload() }
        window?.makeFirstResponder(grid.canvas)
        findChanged(Notification(name: FindCenter.changed))
        if !book.isLoaded {
            bottom.showProgress(book.progress)
            loadTimer = Timer.scheduledTimer(withTimeInterval: 0.12, repeats: true) { [weak self] _ in self?.loadTick() }
        } else {
            snapshotIfRequested()
        }
        updateStatus()
    }

    func loadTick() {
        let done = book.isLoaded
        bottom.showProgress(done ? nil : book.progress)
        grid.updateInsets()
        grid.canvas.needsDisplay = true
        if done {
            loadTimer?.invalidate(); loadTimer = nil
            syncSavedFilter()   // a sheet's saved filter is known once it has loaded
            refreshTabs()
            updateFormulaBar()
            updateStatus()
            if let err = book.takeLoadError() {
                Dialogs.info(window, "Part of this file couldn't be read.", err)
            }
            snapshotIfRequested()
        } else {
            bottom.status.stringValue = "Loading… \(Int(book.progress * 100))%"
        }
    }

    func windowWillClose(_ notification: Notification) {
        loadTimer?.invalidate()
        loadTimer = nil
        if let w = window { WindowTabs.shared.remove(w) }
    }

    // ---- refresh helpers --------------------------------------------------------------

    func refreshTabs() {
        let n = book.sheetCount
        bottom.tabs.set(names: (0..<n).map { book.sheetName($0) }, hidden: (0..<n).map { book.sheetHidden($0) }, selected: sheet)
    }

    func switchSheet(_ i: Int) {
        guard i != sheet || grid.book == nil else { return }
        commitEditing()
        book.activeSheet = i
        grid.sheet = i
        grid.selection = Selection()
        syncSavedFilter()
        grid.setScrollOffset(.zero)
        bottom.tabs.selected = i
        updateFormulaBar()
        updateStatus()
        findChanged(Notification(name: FindCenter.changed))
    }

    func edited() {
        refindAfterChange()
        grid.canvas.invalidateDrawings()
        doc.updateChangeCount(.changeDone)
        grid.updateInsets()
        grid.canvas.needsDisplay = true
        updateFormulaBar()
        updateStatus()
    }

    func fail() {
        NSSound.beep()
        let msg = book.lastError
        Dialogs.info(window, msg)
    }

    func updateFormulaBar() {
        guard doc.book != nil else { return }
        let sel = grid.selection
        let r = sel.primary
        if r.isSingle || book.merge(at: sel.active, sheet: sheet) == r {
            nameBox.stringValue = cellName(sel.active)
        } else if r.isFullCols {
            nameBox.stringValue = r.c0 == r.c1 ? columnName(r.c0) : "\(columnName(r.c0)):\(columnName(r.c1))"
        } else if r.isFullRows {
            nameBox.stringValue = "\(r.r0 + 1):\(r.r1 + 1)"
        } else {
            nameBox.stringValue = "\(r.rows)R × \(r.cols)C"
        }
        if grid.canvas.editor == nil { formula.stringValue = book.editText(sheet, sel.active) }
    }

    func updateStatus() {
        guard doc.book != nil, book.isLoaded else { return }
        statsWork?.cancel()
        let ranges = grid.selection.ranges
        let s = sheet
        let work = DispatchWorkItem { [weak self] in
            guard let self else { return }
            let st = self.book.stats(s, ranges)
            let fmt = NumberFormatter()
            fmt.numberStyle = .decimal
            fmt.maximumFractionDigits = 2
            var parts: [String] = []
            if st.count > 1 {
                if st.numbers > 0 {
                    parts.append("Sum \(fmt.string(from: NSNumber(value: st.sum)) ?? "")")
                    parts.append("Avg \(fmt.string(from: NSNumber(value: st.sum / Double(st.numbers))) ?? "")")
                    parts.append("Min \(fmt.string(from: NSNumber(value: st.min)) ?? "")")
                    parts.append("Max \(fmt.string(from: NSNumber(value: st.max)) ?? "")")
                }
                parts.append("Count \(st.count)")
            }
            self.bottom.status.stringValue = parts.joined(separator: "   ")
        }
        statsWork = work
        // Big selections: wait until the selection settles.
        let big = ranges.contains { $0.rows * min($0.cols, 20000) > 200_000 }
        DispatchQueue.main.asyncAfter(deadline: .now() + (big ? 0.25 : 0.0), execute: work)
    }

    var rects: [CellRect] { grid.selection.ranges }

    /// Selection clamped to the data area (for whole-column/row operations).
    var dataRect: CellRect {
        let r = grid.selection.primary
        let rows = max(1, book.dataRows(sheet)), cols = max(1, book.dataCols(sheet))
        return CellRect(r0: r.r0, c0: r.c0, r1: min(r.r1, max(r.r0, rows - 1)), c1: min(r.c1, max(r.c0, cols - 1)))
    }

    // ---- grid delegate ------------------------------------------------------------------

    func gridSelectionChanged(_ grid: GridView) {
        updateFormulaBar()
        updateStatus()
        updateFormatControls()
    }

    func gridDidEdit(_ grid: GridView) { edited() }

    func gridBeginEditing(_ grid: GridView, initial: String?, select: Bool) {
        guard book.isLoaded else { NSSound.beep(); return }
        let c = grid.canvas
        let pos = grid.selection.active
        commitEditing()
        let text = initial ?? book.editText(sheet, pos)
        let st = c.style(book.cellStyle(sheet, pos))
        let ed = CellEditor(pos: pos, text: text, font: st.font, enterMode: initial != nil)
        ed.owner = self
        ed.apply(c.palette)
        c.editor = ed
        c.addSubview(ed)
        grid.scrollToVisible(pos)
        c.repositionEditor()
        window?.makeFirstResponder(ed)
        if let fe = ed.currentEditor() {
            fe.selectedRange = NSRange(location: (text as NSString).length, length: 0)
        }
        formula.stringValue = text
    }

    func gridContextMenu(_ grid: GridView, for event: NSEvent, header: GridCanvas.Hit) -> NSMenu? {
        let m = NSMenu()
        func add(_ title: String, _ sel: Selector, _ key: String = "") {
            m.addItem(withTitle: title, action: sel, keyEquivalent: key)
        }
        add("Cut", #selector(cut(_:))); add("Copy", #selector(copy(_:))); add("Paste", #selector(paste(_:)))
        add("Paste Values Only", #selector(pasteValues(_:)))
        m.addItem(.separator())
        switch header {
        case .colHeader:
            add("Insert Column Left", #selector(insertColumnsLeft(_:)))
            add("Insert Column Right", #selector(insertColumnsRight(_:)))
            add("Delete Column", #selector(deleteColumns(_:)))
            m.addItem(.separator())
            add("Sort A → Z", #selector(sortAscending(_:)))
            add("Sort Z → A", #selector(sortDescending(_:)))
            add("Autofit Width", #selector(autofitColumns(_:)))
            add("Hide Column", #selector(hideColumns(_:)))
            add("Unhide Columns", #selector(unhideColumns(_:)))
        case .rowHeader:
            add("Insert Row Above", #selector(insertRowsAbove(_:)))
            add("Insert Row Below", #selector(insertRowsBelow(_:)))
            add("Delete Row", #selector(deleteRows(_:)))
            m.addItem(.separator())
            add("Hide Row", #selector(hideRows(_:)))
            add("Unhide Rows", #selector(unhideRows(_:)))
        default:
            add("Insert Row Above", #selector(insertRowsAbove(_:)))
            add("Insert Column Left", #selector(insertColumnsLeft(_:)))
            add("Delete Row", #selector(deleteRows(_:)))
            add("Delete Column", #selector(deleteColumns(_:)))
            m.addItem(.separator())
            add("Sort A → Z", #selector(sortAscending(_:)))
            add("Sort Z → A", #selector(sortDescending(_:)))
            add("Filter by This Value", #selector(filterBySelection(_:)))
        }
        m.addItem(.separator())
        add("Clear Contents", #selector(delete(_:)))
        add("Clear Formatting", #selector(clearFormats(_:)))
        return m
    }

    func gridFilterClicked(_ grid: GridView, column: Int, at rect: NSRect) {
        let values = book.filterValues(sheet, header: filterHeader, col: column)
        let vc = FilterPopover(values: values)
        vc.onApply = { [weak self] vals in
            guard let self else { return }
            adoptSavedFilter()
            if let vals { self.book.setFilter(self.sheet, header: self.filterHeader, col: column, .values, vals.joined(separator: "\n")) } else { self.book.setFilter(self.sheet, header: self.filterHeader, col: column, .clear) }
            self.grid.updateInsets()
            self.grid.canvas.needsDisplay = true
            self.updateStatus()
        }
        let pop = NSPopover()
        pop.contentViewController = vc
        pop.behavior = .transient
        pop.show(relativeTo: rect, of: grid.canvas, preferredEdge: .maxY)
    }

    // ---- editing ------------------------------------------------------------------------

    func commitEditing() {
        grid.canvas.editor?.commit(move: nil)
    }

    /// Drop an in-progress cell edit (e.g. before reverting the file).
    func discardEditing() {
        grid.canvas.editor?.cancel()
    }

    @objc func showInFinder(_ sender: Any?) {
        guard let url = doc.fileURL else { NSSound.beep(); return }
        NSWorkspace.shared.activateFileViewerSelecting([url])
    }

    func editorCommit(_ editor: CellEditor, text: String, move: (dr: Int, dc: Int)?) {
        let c = grid.canvas
        c.editor = nil
        editor.removeFromSuperview()
        let old = book.editText(sheet, editor.pos)
        let cmdEnter = NSApp.currentEvent?.modifierFlags.contains(.command) == true && !grid.selection.primary.isSingle
        if cmdEnter {
            if book.setInput(sheet, grid.selection.primary, text) { edited() } else { fail() }
        } else if text != old {
            if book.setInput(sheet, editor.pos, text) { edited() } else { fail() }
        }
        window?.makeFirstResponder(c)
        if let m = move { c.advance(dr: m.dr, dc: m.dc) }
        updateFormulaBar()
    }

    func editorCancel(_ editor: CellEditor) {
        grid.canvas.editor = nil
        editor.removeFromSuperview()
        window?.makeFirstResponder(grid.canvas)
        updateFormulaBar()
    }

    func editorTextChanged(_ editor: CellEditor, text: String) {
        formula.stringValue = text
    }

    // Formula bar & name box
    func control(_ control: NSControl, textView: NSTextView, doCommandBy sel: Selector) -> Bool {
        if control === nameBox, sel == #selector(insertNewline(_:)) {
            goTo(nameBox.stringValue)
            return true
        }
        if control === formula {
            switch sel {
            case #selector(insertNewline(_:)):
                if NSApp.currentEvent?.modifierFlags.contains(.option) == true {
                    textView.insertNewlineIgnoringFieldEditor(nil); return true
                }
                let pos = grid.selection.active
                if formula.stringValue != book.editText(sheet, pos) {
                    if book.setInput(sheet, pos, formula.stringValue) { edited() } else { fail() }
                }
                window?.makeFirstResponder(grid.canvas)
                grid.canvas.advance(dr: 1, dc: 0)
                return true
            case #selector(cancelOperation(_:)):
                formula.stringValue = book.editText(sheet, grid.selection.active)
                window?.makeFirstResponder(grid.canvas)
                return true
            default: return false
            }
        }
        return false
    }

    func controlTextDidChange(_ obj: Notification) {
        if (obj.object as? NSTextField) === formula, let ed = grid.canvas.editor {
            ed.stringValue = formula.stringValue
        }
    }

    func goTo(_ ref: String) {
        let parts = ref.split(separator: ":").map(String.init)
        guard var a = parseCellName(parts.first ?? "") else { NSSound.beep(); return }
        var b = parts.count > 1 ? parseCellName(parts[1]) ?? a : a
        let limit = book.maxRows(sheet) - 1
        a.r = min(a.r, limit)
        b.r = min(b.r, limit)
        grid.selection = Selection(active: a, anchor: a, ranges: [CellRect(a, b)])
        grid.scrollToVisible(a)
        window?.makeFirstResponder(grid.canvas)
        gridSelectionChanged(grid)
    }

}
