import AppKit
import WaffleBridge

enum UTI {
    static let xlsx = "org.openxmlformats.spreadsheetml.sheet"
    static let xlsm = "org.openxmlformats.spreadsheetml.sheet.macroenabled"
    static let xls = "com.microsoft.excel.xls"
    static let xlsb = "com.microsoft.excel.sheet.binary.macroenabled"
    static let ods = "org.oasis-open.opendocument.spreadsheet"
    static let csv = "public.comma-separated-values-text"
    static let tsv = "public.tab-separated-values-text"
    static let txt = "public.plain-text"

    static let importOnly: Set<String> = [xls, xlsb, ods]
    static let text: Set<String> = [csv, tsv, txt]
}

@objc(Document)
final class Document: NSDocument {
    var book: Book?
    /// Name of the xls/xlsb/ods file this untitled workbook was imported from.
    var importedName: String?

    override init() {
        super.init()
        hasUndoManager = false
    }

    // Never write to the original file behind the user's back.
    override class var autosavesInPlace: Bool { false }
    override class var autosavesDrafts: Bool { false }
    override class var preservesVersions: Bool { false }
    override class func canConcurrentlyReadDocuments(ofType typeName: String) -> Bool { false }

    override func makeWindowControllers() {
        if book == nil { book = Book.empty() }
        let wc = SheetWindowController(document: self)
        addWindowController(wc)
        wc.window?.setFrameAutosaveName("WaffleDocument")
        wc.attach()
    }

    /// Set while macOS restores this document's window from the last session.
    var restoring = false

    /// Windows from the last session join their tab group in the background (WindowTabs).
    override func restoreWindow(withIdentifier identifier: NSUserInterfaceItemIdentifier, state: NSCoder, completionHandler: @escaping (NSWindow?, (any Error)?) -> Void) {
        restoring = true  // the window may not exist yet, so the document carries the mark
        super.restoreWindow(withIdentifier: identifier, state: state) { [weak self] w, e in
            completionHandler(w, e)
            // Restored windows are put on screen after this; clear the mark once they are.
            DispatchQueue.main.asyncAfter(deadline: .now() + 2) { self?.restoring = false }
        }
    }

    override var displayName: String! {
        get {
            if fileURL == nil, let n = importedName { return n }
            return super.displayName
        }
        set { super.displayName = newValue }
    }

    override func read(from url: URL, ofType typeName: String) throws {
        book = try Book.open(url)
    }

    /// Revert re-reads the file into a new book; point the window at it.
    override func revert(toContentsOf url: URL, ofType typeName: String) throws {
        (windowControllers.first as? SheetWindowController)?.discardEditing()
        try super.revert(toContentsOf: url, ofType: typeName)
        for case let wc as SheetWindowController in windowControllers { wc.attach() }
    }

    override func write(to url: URL, ofType typeName: String) throws {
        guard let book else { return }
        (windowControllers.first as? SheetWindowController)?.commitEditing()
        try book.save(to: url, csv: UTI.text.contains(typeName) || ["csv", "tsv", "txt"].contains(url.pathExtension.lowercased()))
    }

    override func writableTypes(for op: NSDocument.SaveOperationType) -> [String] {
        switch book?.kind {
        case .xlsm: return [UTI.xlsm, UTI.xlsx, UTI.csv, UTI.tsv]
        case .csv: return fileType == UTI.tsv ? [UTI.tsv, UTI.csv, UTI.xlsx] : [UTI.csv, UTI.tsv, UTI.xlsx]
        default: return [UTI.xlsx, UTI.csv, UTI.tsv]
        }
    }

    override func fileNameExtension(forType typeName: String, saveOperation: NSDocument.SaveOperationType) -> String? {
        switch typeName {
        case UTI.xlsx: return "xlsx"
        case UTI.xlsm: return "xlsm"
        case UTI.csv: return "csv"
        case UTI.tsv: return "tsv"
        case UTI.txt: return "txt"
        default: return super.fileNameExtension(forType: typeName, saveOperation: saveOperation)
        }
    }

    override func prepareSavePanel(_ savePanel: NSSavePanel) -> Bool {
        savePanel.isExtensionHidden = false
        savePanel.canSelectHiddenExtension = true
        if fileURL == nil, let n = importedName {
            savePanel.nameFieldStringValue = (n as NSString).deletingPathExtension + ".xlsx"
        }
        return true
    }

    override func save(to url: URL, ofType typeName: String, for saveOperation: NSDocument.SaveOperationType, completionHandler: @escaping (Error?) -> Void) {
        // CSV holds one sheet and no formatting: say so before losing anything.
        if UTI.text.contains(typeName), let book, book.kind != .csv {
            let key = "WaffleCSVWarned"
            if !UserDefaults.standard.bool(forKey: key), let w = windowControllers.first?.window {
                let a = NSAlert()
                a.messageText = "Save as CSV?"
                a.informativeText = "CSV keeps only the values of the current sheet (“\(book.sheetName(book.activeSheet))”). Formatting, formulas and other sheets are not saved."
                a.addButton(withTitle: "Save CSV")
                a.addButton(withTitle: "Cancel")
                a.showsSuppressionButton = true
                a.beginSheetModal(for: w) { r in
                    if a.suppressionButton?.state == .on { UserDefaults.standard.set(true, forKey: key) }
                    if r == .alertFirstButtonReturn {
                        super.save(to: url, ofType: typeName, for: saveOperation, completionHandler: completionHandler)
                    } else {
                        completionHandler(CocoaError(.userCancelled))
                    }
                }
                return
            }
        }
        super.save(to: url, ofType: typeName, for: saveOperation, completionHandler: completionHandler)
    }

    override func canClose(withDelegate delegate: Any, shouldClose shouldCloseSelector: Selector?, contextInfo: UnsafeMutableRawPointer?) {
        (windowControllers.first as? SheetWindowController)?.commitEditing()
        super.canClose(withDelegate: delegate, shouldClose: shouldCloseSelector, contextInfo: contextInfo)
    }
}

/// Opens xls/xlsb/ods as untitled workbooks (they can't be written back).
final class DocumentController: NSDocumentController {
    override func makeDocument(withContentsOf url: URL, ofType typeName: String) throws -> NSDocument {
        if UTI.importOnly.contains(typeName) || ["xls", "xlsb", "ods"].contains(url.pathExtension.lowercased()) {
            let doc = Document()
            doc.fileType = UTI.xlsx
            try doc.read(from: url, ofType: typeName)
            doc.importedName = url.lastPathComponent
            return doc
        }
        return try super.makeDocument(withContentsOf: url, ofType: typeName)
    }

    override func typeForContents(of url: URL) throws -> String {
        switch url.pathExtension.lowercased() {
        case "xlsx": return UTI.xlsx
        case "xlsm": return UTI.xlsm
        case "xls": return UTI.xls
        case "xlsb": return UTI.xlsb
        case "ods": return UTI.ods
        case "csv": return UTI.csv
        case "tsv", "tab": return UTI.tsv
        case "txt": return UTI.txt
        default: return try super.typeForContents(of: url)
        }
    }

    override var defaultType: String? { UTI.xlsx }
}
