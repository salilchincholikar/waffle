import AppKit
import WaffleBridge

protocol CellEditorOwner: AnyObject {
    /// Commit text for a cell and optionally move the selection.
    func editorCommit(_ editor: CellEditor, text: String, move: (dr: Int, dc: Int)?)
    func editorCancel(_ editor: CellEditor)
    func editorTextChanged(_ editor: CellEditor, text: String)
}

/// The in-cell text field.
final class CellEditor: NSTextField, NSTextFieldDelegate {
    let pos: CellPos
    /// Started by typing: arrow keys commit and move (Excel's "Enter" mode).
    var enterMode: Bool
    weak var owner: CellEditorOwner?
    private var finished = false
    private var baseRect = NSRect.zero

    init(pos: CellPos, text: String, font: NSFont, enterMode: Bool) {
        self.pos = pos
        self.enterMode = enterMode
        super.init(frame: .zero)
        stringValue = text
        self.font = font
        isBordered = false
        isBezeled = false
        drawsBackground = true
        focusRingType = .none
        usesSingleLineMode = false
        cell?.wraps = false
        cell?.isScrollable = true
        lineBreakMode = .byClipping
        delegate = self
        wantsLayer = true
        layer?.borderWidth = 2
        layer?.borderColor = NSColor.controlAccentColor.cgColor
    }

    required init?(coder: NSCoder) { fatalError() }

    func apply(_ palette: SheetPalette) {
        backgroundColor = NSColor(cgColor: palette.background)
        textColor = palette.text
    }

    func place(in rect: NSRect, canvas: GridCanvas) {
        baseRect = rect
        grow()
    }

    private func grow() {
        let text = stringValue as NSString
        let f = font ?? .systemFont(ofSize: 13)
        let lines = max(1, stringValue.components(separatedBy: "\n").count)
        let w = text.size(withAttributes: [.font: f]).width + 16
        let lineH = f.ascender - f.descender + f.leading
        let h = max(baseRect.height, CGFloat(lines) * lineH + 6)
        let maxW = (superview?.bounds.width ?? 2000) - baseRect.minX - 4
        frame = NSRect(x: baseRect.minX, y: baseRect.minY, width: min(max(baseRect.width, w), max(baseRect.width, maxW)), height: h)
    }

    func controlTextDidChange(_ obj: Notification) {
        grow()
        owner?.editorTextChanged(self, text: stringValue)
    }

    func commit(move: (dr: Int, dc: Int)?) {
        guard !finished else { return }
        finished = true
        owner?.editorCommit(self, text: stringValue, move: move)
    }

    func cancel() {
        guard !finished else { return }
        finished = true
        owner?.editorCancel(self)
    }

    func controlTextDidEndEditing(_ obj: Notification) {
        // Focus moved elsewhere (click in the grid, a control…): keep what was typed.
        commit(move: nil)
    }

    func control(_ control: NSControl, textView: NSTextView, doCommandBy sel: Selector) -> Bool {
        let flags = NSApp.currentEvent?.modifierFlags ?? []
        switch sel {
        case #selector(insertNewline(_:)):
            if flags.contains(.option) || flags.contains(.control) {
                textView.insertNewlineIgnoringFieldEditor(nil)
                grow()
                return true
            }
            commit(move: flags.contains(.shift) ? (-1, 0) : (1, 0))
            return true
        case #selector(insertNewlineIgnoringFieldEditor(_:)):
            textView.insertNewlineIgnoringFieldEditor(nil)
            grow()
            return true
        case #selector(insertTab(_:)):
            commit(move: (0, 1)); return true
        case #selector(insertBacktab(_:)):
            commit(move: (0, -1)); return true
        case #selector(cancelOperation(_:)):
            cancel(); return true
        case #selector(moveUp(_:)) where enterMode:
            commit(move: (-1, 0)); return true
        case #selector(moveDown(_:)) where enterMode:
            commit(move: (1, 0)); return true
        case #selector(moveLeft(_:)) where enterMode:
            commit(move: (0, -1)); return true
        case #selector(moveRight(_:)) where enterMode:
            commit(move: (0, 1)); return true
        default:
            return false
        }
    }
}
