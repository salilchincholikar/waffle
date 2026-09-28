import AppKit
import WaffleBridge
import CWaffle

extension SheetWindowController {
    // ---- title bar --------------------------------------------------------------------------
    // No NSToolbar (it can't get thinner than ~38 pt): our own row sits in the title-bar
    // area next to the traffic lights, like Helium/Chrome: file tabs at the left, then Find
    // and the sheet tools at the right. The formula row with the formatting is below it.

    func configureTitleBar() {
        guard let w = window else { return }
        w.titleVisibility = .hidden
        w.titlebarAppearsTransparent = true
        w.titlebarSeparatorStyle = .none
    }

    /// Height of the standard title bar (the row's height).
    static var titleBarHeight: CGFloat {
        NSWindow.frameRect(forContentRect: NSRect(x: 0, y: 0, width: 100, height: 100), styleMask: [.titled]).height - 100
    }

    func symbol(_ name: String, _ desc: String) -> NSImage {
        NSImage(systemSymbolName: name, accessibilityDescription: desc) ?? NSImage()
    }

    func menuItem(_ title: String, _ sel: Selector, _ symbolName: String? = nil) -> NSMenuItem {
        let m = NSMenuItem(title: title, action: sel, keyEquivalent: "")
        if let symbolName { m.image = symbol(symbolName, title) }
        return m
    }

    /// Sheet-wide tools: sort, filter, freeze, clean up.
    func buildToolControls() -> [NSView] {
        let sort = popDown("Sort", symbol("arrow.up.arrow.down", "Sort"), [
            menuItem("Sort A → Z", #selector(sortAscending(_:)), "arrow.up"),
            menuItem("Sort Z → A", #selector(sortDescending(_:)), "arrow.down"),
            menuItem("Custom Sort…", #selector(customSort(_:))),
        ])
        let filter = NSButton(image: symbol("line.3.horizontal.decrease.circle", "Filter"), target: self, action: #selector(toggleFilter(_:)))
        filter.toolTip = "Filter rows by column values"
        filter.bezelStyle = .accessoryBarAction
        let freeze = popDown("Freeze", symbol("pin", "Freeze"), [
            menuItem("Freeze Top Row", #selector(freezeTopRow(_:))),
            menuItem("Freeze First Column", #selector(freezeFirstColumn(_:))),
            menuItem("Freeze Above & Left of Selection", #selector(freezeAtSelection(_:))),
            menuItem("Unfreeze", #selector(unfreeze(_:))),
        ])
        let caseMenu = NSMenuItem(title: "Change Case", action: nil, keyEquivalent: "")
        let cm = NSMenu()
        cm.addItem(menuItem("UPPERCASE", #selector(upperCase(_:))))
        cm.addItem(menuItem("lowercase", #selector(lowerCase(_:))))
        cm.addItem(menuItem("Title Case", #selector(titleCase(_:))))
        caseMenu.submenu = cm
        let clean = popDown("Clean Up", symbol("wand.and.stars", "Clean Up"), [
            menuItem("Trim Extra Spaces", #selector(trimSpaces(_:))),
            menuItem("Remove Empty Rows", #selector(removeEmptyRows(_:))),
            menuItem("Remove Duplicate Rows…", #selector(removeDuplicates(_:))),
            caseMenu,
            .separator(),
            menuItem("Standardize Dates…", #selector(standardizeDates(_:))),
            menuItem("Standardize Amounts…", #selector(standardizeAmounts(_:))),
            menuItem("Split Text into Columns…", #selector(textToColumns(_:))),
        ])
        return [sort, filter, freeze, clean]
    }
}
