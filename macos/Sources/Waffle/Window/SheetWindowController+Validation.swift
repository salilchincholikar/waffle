import AppKit
import WaffleBridge
import CWaffle

extension SheetWindowController {
    // ---- view ------------------------------------------------------------------------------------

    @objc func zoomIn(_ sender: Any?) { grid.setZoom(grid.canvas.zoom * 1.15) }
    @objc func zoomOut(_ sender: Any?) { grid.setZoom(grid.canvas.zoom / 1.15) }
    @objc func actualSize(_ sender: Any?) { grid.setZoom(1) }

    // ---- menu validation --------------------------------------------------------------------------

    func validateMenuItem(_ item: NSMenuItem) -> Bool {
        guard doc.book != nil else { return false }
        let loaded = book.isLoaded
        let csv = book.kind == .csv
        switch item.action {
        case #selector(undo(_:)):
            item.title = book.undoLabel.map { "Undo \($0)" } ?? "Undo"
            return book.undoLabel != nil
        case #selector(redo(_:)):
            item.title = book.redoLabel.map { "Redo \($0)" } ?? "Redo"
            return book.redoLabel != nil
        case #selector(toggleBold(_:)):
            item.state = activeStyle.bold != 0 ? .on : .off; return loaded && !csv
        case #selector(toggleItalic(_:)):
            item.state = activeStyle.italic != 0 ? .on : .off; return loaded && !csv
        case #selector(toggleUnderline(_:)):
            item.state = activeStyle.underline != 0 ? .on : .off; return loaded && !csv
        case #selector(toggleStrike(_:)):
            item.state = activeStyle.strike != 0 ? .on : .off; return loaded && !csv
        case #selector(toggleWrap(_:)):
            item.state = activeStyle.wrap != 0 ? .on : .off; return loaded && !csv
        case #selector(toggleFilter(_:)):
            item.state = grid.canvas.filterMode ? .on : .off; return loaded
        case #selector(biggerFont(_:)), #selector(smallerFont(_:)), #selector(alignLeft(_:)), #selector(alignCenter(_:)), #selector(alignRight(_:)),
             #selector(alignGeneral(_:)), #selector(alignTop(_:)), #selector(alignMiddle(_:)), #selector(alignBottom(_:)), #selector(setNumberFormat(_:)),
             #selector(customNumberFormat(_:)), #selector(increaseDecimals(_:)), #selector(decreaseDecimals(_:)), #selector(borderAll(_:)),
             #selector(borderBottom(_:)), #selector(borderThickBottom(_:)), #selector(borderOutline(_:)), #selector(borderNone(_:)), #selector(noFill(_:)),
             #selector(automaticTextColor(_:)), #selector(pickColor(_:)), #selector(moreColors(_:)), #selector(clearFormats(_:)), #selector(mergeCells(_:)), #selector(unmergeCells(_:)), #selector(pasteFormats(_:)),
             #selector(insertSheet(_:)):
            return loaded && !csv
        case #selector(showInFinder(_:)):
            return doc.fileURL != nil
        case #selector(unfreeze(_:)):
            let f = book.freeze(sheet); return f.rows > 0 || f.cols > 0
        case #selector(clearAllFilters(_:)):
            return grid.canvas.filterMode
        case #selector(showNextFileTab(_:)), #selector(showPreviousFileTab(_:)), #selector(moveFileTabToNewWindow(_:)):
            return (window.map { WindowTabs.shared.tabs(of: $0).count } ?? 1) > 1
        case #selector(mergeAllFileWindows(_:)):
            return WindowTabs.shared.groups.count > 1
        default:
            return loaded || item.action == #selector(showFind(_:)) || item.action == #selector(zoomIn(_:)) || item.action == #selector(zoomOut(_:)) || item.action == #selector(actualSize(_:))
        }
    }
}
