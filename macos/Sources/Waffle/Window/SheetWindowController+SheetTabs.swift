import AppKit
import WaffleBridge
import CWaffle

extension SheetWindowController {
    // ---- tabs ------------------------------------------------------------------------------

    func tabsSelect(_ index: Int) { switchSheet(index) }

    func tabsAdd() {
        commitEditing()
        guard let i = book.addSheet(at: sheet + 1) else { fail(); return }
        doc.updateChangeCount(.changeDone)
        refreshTabs()
        switchSheet(i)
    }

    func tabsRename(_ index: Int) {
        Dialogs.askText(window!, title: "Rename Sheet", value: book.sheetName(index), ok: "Rename") { [weak self] name in
            guard let self else { return }
            if self.book.renameSheet(index, name) { self.doc.updateChangeCount(.changeDone); self.refreshTabs(); self.updateFormulaBar() } else { self.fail() }
        }
    }

    func tabsMenu(for index: Int) -> NSMenu? {
        let m = NSMenu()
        let rename = m.addItem(withTitle: "Rename…", action: #selector(renameSheet(_:)), keyEquivalent: "")
        rename.tag = index
        let del = m.addItem(withTitle: "Delete Sheet", action: #selector(deleteSheet(_:)), keyEquivalent: "")
        del.tag = index
        m.addItem(.separator())
        let hide = m.addItem(withTitle: "Hide Sheet", action: #selector(hideSheet(_:)), keyEquivalent: "")
        hide.tag = index
        let hiddenSheets = (0..<book.sheetCount).filter { book.sheetHidden($0) }
        if !hiddenSheets.isEmpty {
            let un = NSMenu()
            for i in hiddenSheets {
                let it = un.addItem(withTitle: book.sheetName(i), action: #selector(unhideSheet(_:)), keyEquivalent: "")
                it.tag = i
            }
            m.addItem(withTitle: "Unhide", action: nil, keyEquivalent: "").submenu = un
        }
        m.addItem(.separator())
        m.addItem(withTitle: "Move Left", action: #selector(moveSheetLeft(_:)), keyEquivalent: "")
        m.addItem(withTitle: "Move Right", action: #selector(moveSheetRight(_:)), keyEquivalent: "")
        m.addItem(.separator())
        m.addItem(withTitle: "Insert Sheet", action: #selector(insertSheet(_:)), keyEquivalent: "")
        return m
    }

    func tabsMove(from: Int, to: Int) {
        if book.moveSheet(from, to) {
            doc.updateChangeCount(.changeDone)
            grid.sheet = to
            refreshTabs()
        }
    }

    @objc func renameSheet(_ sender: Any?) {
        tabsRename((sender as? NSMenuItem)?.tag ?? sheet)
    }
    @objc func deleteSheet(_ sender: Any?) {
        let i = (sender as? NSMenuItem)?.tag ?? sheet
        let a = NSAlert()
        a.messageText = "Delete “\(book.sheetName(i))”?"
        a.informativeText = "You can undo this."
        a.addButton(withTitle: "Delete")
        a.addButton(withTitle: "Cancel")
        a.buttons.first?.hasDestructiveAction = true
        a.beginSheetModal(for: window!) { [weak self] r in
            guard let self, r == .alertFirstButtonReturn else { return }
            if self.book.deleteSheet(i) {
                self.doc.updateChangeCount(.changeDone)
                let next = self.book.activeSheet
                self.grid.sheet = next
                self.refreshTabs()
                self.updateFormulaBar()
            } else { self.fail() }
        }
    }
    @objc func hideSheet(_ sender: Any?) {
        let i = (sender as? NSMenuItem)?.tag ?? sheet
        if book.setSheetHidden(i, true) {
            doc.updateChangeCount(.changeDone)
            if let v = (0..<book.sheetCount).first(where: { !book.sheetHidden($0) }) { switchSheet(v) }
            refreshTabs()
        } else { fail() }
    }
    @objc func unhideSheet(_ sender: NSMenuItem) {
        if book.setSheetHidden(sender.tag, false) { doc.updateChangeCount(.changeDone); refreshTabs(); switchSheet(sender.tag) }
    }
    @objc func insertSheet(_ sender: Any?) { tabsAdd() }
    @objc func nextSheet(_ sender: Any?) {
        let n = book.sheetCount
        var i = sheet
        repeat { i = (i + 1) % n } while book.sheetHidden(i) && i != sheet
        switchSheet(i)
    }
    @objc func previousSheet(_ sender: Any?) {
        let n = book.sheetCount
        var i = sheet
        repeat { i = (i - 1 + n) % n } while book.sheetHidden(i) && i != sheet
        switchSheet(i)
    }

    @objc func moveSheetLeft(_ sender: Any?) { bottom.tabs.moveSelected(by: -1) }
    @objc func moveSheetRight(_ sender: Any?) { bottom.tabs.moveSelected(by: 1) }
}
