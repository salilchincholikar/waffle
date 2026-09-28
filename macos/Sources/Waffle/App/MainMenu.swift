import AppKit
import WaffleBridge

extension AppDelegate {
    // ---- menu ---------------------------------------------------------------------------

    func item(_ title: String, _ action: Selector?, _ key: String = "", _ mods: NSEvent.ModifierFlags = .command) -> NSMenuItem {
        let i = NSMenuItem(title: title, action: action, keyEquivalent: key)
        i.keyEquivalentModifierMask = mods
        return i
    }

    func menu(_ title: String, _ items: [NSMenuItem]) -> NSMenuItem {
        let top = NSMenuItem(title: title, action: nil, keyEquivalent: "")
        let m = NSMenu(title: title)
        items.forEach { m.addItem($0) }
        top.submenu = m
        return top
    }

    func sub(_ title: String, _ items: [NSMenuItem]) -> NSMenuItem {
        let i = NSMenuItem(title: title, action: nil, keyEquivalent: "")
        let m = NSMenu(title: title)
        items.forEach { m.addItem($0) }
        i.submenu = m
        return i
    }

    func buildMenu() -> NSMenu {
        let W = SheetWindowController.self
        _ = W
        let main = NSMenu()
        let appName = "Waffle"

        let services = NSMenu()
        NSApp.servicesMenu = services
        let servicesItem = NSMenuItem(title: "Services", action: nil, keyEquivalent: "")
        servicesItem.submenu = services
        main.addItem(menu(appName, [
            item("About \(appName)", #selector(NSApplication.orderFrontStandardAboutPanel(_:)), ""),
            .separator(),
            servicesItem,
            .separator(),
            item("Hide \(appName)", #selector(NSApplication.hide(_:)), "h"),
            item("Hide Others", #selector(NSApplication.hideOtherApplications(_:)), "h", [.command, .option]),
            item("Show All", #selector(NSApplication.unhideAllApplications(_:))),
            .separator(),
            item("Quit \(appName)", #selector(NSApplication.terminate(_:)), "q"),
        ]))

        // "Open Recent" is inserted by NSDocumentController after "Open…" (it also keeps it filled).
        main.addItem(menu("File", [
            item("New", #selector(NSDocumentController.newDocument(_:)), "n"),
            item("Open…", #selector(NSDocumentController.openDocument(_:)), "o"),
            .separator(),
            item("Close Tab", #selector(NSWindow.performClose(_:)), "w"),
            item("Save", #selector(NSDocument.save(_:)), "s"),
            item("Save As…", #selector(NSDocument.saveAs(_:)), "s", [.command, .shift]),
            item("Export a Copy…", #selector(NSDocument.saveTo(_:)), "e", [.command, .shift]),
            item("Revert to Saved", #selector(NSDocument.revertToSaved(_:))),
            .separator(),
            item("Show in Finder", #selector(SheetWindowController.showInFinder(_:)), "r", [.command, .shift]),
        ]))

        main.addItem(menu("Edit", [
            item("Undo", #selector(SheetWindowController.undo(_:)), "z"),
            item("Redo", #selector(SheetWindowController.redo(_:)), "z", [.command, .shift]),
            .separator(),
            item("Cut", #selector(NSText.cut(_:)), "x"),
            item("Copy", #selector(NSText.copy(_:)), "c"),
            item("Paste", #selector(NSText.paste(_:)), "v"),
            item("Paste Values Only", #selector(SheetWindowController.pasteValues(_:)), "v", [.command, .shift]),
            item("Paste Formatting Only", #selector(SheetWindowController.pasteFormats(_:)), "v", [.command, .option]),
            item("Clear Contents", #selector(SheetWindowController.delete(_:)), ""),
            item("Clear All", #selector(SheetWindowController.clearAll(_:)), ""),
            item("Select All", #selector(NSText.selectAll(_:)), "a"),
            .separator(),
            item("Edit Cell", #selector(SheetWindowController.editCell(_:)), "u", [.control]),
            item("Fill Down", #selector(SheetWindowController.fillDown(_:)), "d"),
            item("Fill Right", #selector(SheetWindowController.fillRight(_:)), "r"),
            .separator(),
            sub("Find", [
                item("Find…", #selector(SheetWindowController.showFind(_:)), "f"),
                item("Find and Replace…", #selector(SheetWindowController.showFindReplace(_:)), "f", [.command, .option]),
                item("Find Next", #selector(SheetWindowController.findNext(_:)), "g"),
                item("Find Previous", #selector(SheetWindowController.findPrevious(_:)), "g", [.command, .shift]),
            ]),
        ]))

        main.addItem(menu("Insert", [
            item("Row Above", #selector(SheetWindowController.insertRowsAbove(_:)), "+", [.command, .shift]),
            item("Row Below", #selector(SheetWindowController.insertRowsBelow(_:))),
            item("Column Left", #selector(SheetWindowController.insertColumnsLeft(_:))),
            item("Column Right", #selector(SheetWindowController.insertColumnsRight(_:))),
            .separator(),
            item("Sheet", #selector(SheetWindowController.insertSheet(_:)), "t", [.command, .shift]),
        ]))

        let numberItems: [NSMenuItem] = SheetWindowController.numberFormats.map { (t, code) in
            let m = item(t, #selector(SheetWindowController.setNumberFormat(_:)))
            m.representedObject = code
            return m
        } + [.separator(), item("Increase Decimals", #selector(SheetWindowController.increaseDecimals(_:))), item("Decrease Decimals", #selector(SheetWindowController.decreaseDecimals(_:))), item("Custom…", #selector(SheetWindowController.customNumberFormat(_:)))]

        main.addItem(menu("Format", [
            item("Bold", #selector(SheetWindowController.toggleBold(_:)), "b"),
            item("Italic", #selector(SheetWindowController.toggleItalic(_:)), "i"),
            item("Underline", #selector(SheetWindowController.toggleUnderline(_:)), "u"),
            item("Strikethrough", #selector(SheetWindowController.toggleStrike(_:)), "x", [.command, .shift]),
            item("Bigger", #selector(SheetWindowController.biggerFont(_:)), ">", [.command, .shift]),
            item("Smaller", #selector(SheetWindowController.smallerFont(_:)), "<", [.command, .shift]),
            item("No Fill", #selector(SheetWindowController.noFill(_:))),
            item("Automatic Text Color", #selector(SheetWindowController.automaticTextColor(_:))),
            .separator(),
            sub("Number", numberItems),
            sub("Alignment", [
                item("Left", #selector(SheetWindowController.alignLeft(_:)), "{"),
                item("Center", #selector(SheetWindowController.alignCenter(_:)), "|"),
                item("Right", #selector(SheetWindowController.alignRight(_:)), "}"),
                item("Automatic", #selector(SheetWindowController.alignGeneral(_:))),
                .separator(),
                item("Top", #selector(SheetWindowController.alignTop(_:))),
                item("Middle", #selector(SheetWindowController.alignMiddle(_:))),
                item("Bottom", #selector(SheetWindowController.alignBottom(_:))),
            ]),
            item("Wrap Text", #selector(SheetWindowController.toggleWrap(_:))),
            sub("Borders", [
                item("All Borders", #selector(SheetWindowController.borderAll(_:))),
                item("Outline", #selector(SheetWindowController.borderOutline(_:))),
                item("Bottom", #selector(SheetWindowController.borderBottom(_:))),
                item("Thick Bottom", #selector(SheetWindowController.borderThickBottom(_:))),
                item("None", #selector(SheetWindowController.borderNone(_:))),
            ]),
            .separator(),
            item("Merge Cells", #selector(SheetWindowController.mergeCells(_:))),
            item("Unmerge Cells", #selector(SheetWindowController.unmergeCells(_:))),
            .separator(),
            item("Autofit Column Width", #selector(SheetWindowController.autofitColumns(_:))),
            sub("Rows & Columns", [
                item("Hide Rows", #selector(SheetWindowController.hideRows(_:)), "9", [.control]),
                item("Unhide Rows", #selector(SheetWindowController.unhideRows(_:)), "9", [.control, .shift]),
                item("Hide Columns", #selector(SheetWindowController.hideColumns(_:)), "0", [.control]),
                item("Unhide Columns", #selector(SheetWindowController.unhideColumns(_:)), "0", [.control, .shift]),
                .separator(),
                item("Delete Rows", #selector(SheetWindowController.deleteRows(_:)), "-", [.command]),
                item("Delete Columns", #selector(SheetWindowController.deleteColumns(_:))),
            ]),
            .separator(),
            item("Clear Formatting", #selector(SheetWindowController.clearFormats(_:))),
        ]))

        main.addItem(menu("Data", [
            item("Sort A → Z", #selector(SheetWindowController.sortAscending(_:))),
            item("Sort Z → A", #selector(SheetWindowController.sortDescending(_:))),
            item("Custom Sort…", #selector(SheetWindowController.customSort(_:))),
            .separator(),
            item("Filter", #selector(SheetWindowController.toggleFilter(_:)), "l", [.command, .shift]),
            item("Filter by Selected Value", #selector(SheetWindowController.filterBySelection(_:))),
            item("Clear All Filters", #selector(SheetWindowController.clearAllFilters(_:))),
            .separator(),
            item("Trim Extra Spaces", #selector(SheetWindowController.trimSpaces(_:))),
            item("Remove Empty Rows", #selector(SheetWindowController.removeEmptyRows(_:))),
            item("Remove Duplicate Rows…", #selector(SheetWindowController.removeDuplicates(_:))),
            sub("Change Case", [
                item("UPPERCASE", #selector(SheetWindowController.upperCase(_:))),
                item("lowercase", #selector(SheetWindowController.lowerCase(_:))),
                item("Title Case", #selector(SheetWindowController.titleCase(_:))),
            ]),
            .separator(),
            item("Standardize Dates…", #selector(SheetWindowController.standardizeDates(_:))),
            item("Standardize Amounts…", #selector(SheetWindowController.standardizeAmounts(_:))),
            item("Split Text into Columns…", #selector(SheetWindowController.textToColumns(_:))),
        ]))

        main.addItem(menu("View", [
            item("Zoom In", #selector(SheetWindowController.zoomIn(_:)), "="),
            item("Zoom Out", #selector(SheetWindowController.zoomOut(_:)), "-", [.command, .option]),
            item("Actual Size", #selector(SheetWindowController.actualSize(_:)), "0"),
            .separator(),
            sub("Freeze Panes", [
                item("Freeze Top Row", #selector(SheetWindowController.freezeTopRow(_:))),
                item("Freeze First Column", #selector(SheetWindowController.freezeFirstColumn(_:))),
                item("Freeze Above & Left of Selection", #selector(SheetWindowController.freezeAtSelection(_:))),
                item("Unfreeze", #selector(SheetWindowController.unfreeze(_:))),
            ]),
            .separator(),
            item("Dark Sheet in Dark Mode", #selector(AppDelegate.toggleDarkSheet(_:))),
            .separator(),
            item("Enter Full Screen", #selector(NSWindow.toggleFullScreen(_:)), "f", [.command, .control]),
        ]))

        main.addItem(menu("Sheet", [
            item("Next Sheet", #selector(SheetWindowController.nextSheet(_:)), "}", [.command, .option]),
            item("Previous Sheet", #selector(SheetWindowController.previousSheet(_:)), "{", [.command, .option]),
            .separator(),
            item("Rename…", #selector(SheetWindowController.renameSheet(_:))),
            item("Delete", #selector(SheetWindowController.deleteSheet(_:))),
            item("Hide", #selector(SheetWindowController.hideSheet(_:))),
        ]))

        let windowMenu = menu("Window", [
            item("Minimize", #selector(NSWindow.performMiniaturize(_:)), "m"),
            item("Zoom", #selector(NSWindow.performZoom(_:))),
            .separator(),
            item("Show Previous Tab", #selector(SheetWindowController.showPreviousFileTab(_:)), "[", [.command, .shift]),
            item("Show Next Tab", #selector(SheetWindowController.showNextFileTab(_:)), "]", [.command, .shift]),
            item("Move Tab to New Window", #selector(SheetWindowController.moveFileTabToNewWindow(_:))),
            item("Merge All Windows", #selector(SheetWindowController.mergeAllFileWindows(_:))),
            .separator(),
            item("Bring All to Front", #selector(NSApplication.arrangeInFront(_:))),
        ])
        NSApp.windowsMenu = windowMenu.submenu
        main.addItem(windowMenu)
        let help = menu("Help", [])
        NSApp.helpMenu = help.submenu
        main.addItem(help)
        return main
    }
}
