import AppKit
import WaffleBridge

#if WAFFLE_DEBUG_HOOKS
extension AppDelegate {
    /// Env-var driven hooks: WAFFLE_MENUDUMP prints the File/Edit menus; WAFFLE_TABTEST=query
    /// reports tab grouping and steps a cross-file search.
    func runDebugHooks() {
        if ProcessInfo.processInfo.environment["WAFFLE_MENUDUMP"] != nil {
            DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) {
                for top in NSApp.mainMenu?.items ?? [] where ["File", "Edit"].contains(top.title) {
                    print("\(top.title):")
                    for i in top.submenu?.items ?? [] {
                        let key = i.keyEquivalent.isEmpty ? "" : "  [\(i.keyEquivalentModifierMask.contains(.shift) ? "⇧" : "")\(i.keyEquivalentModifierMask.contains(.command) ? "⌘" : "")\(i.keyEquivalent.uppercased())]"
                        print(i.isSeparatorItem ? "  ---" : "  \(i.title)\(i.submenu != nil ? " ▸" : "")\(key)")
                    }
                }
                fflush(stdout)
                exit(0)
            }
        }
        if let q = ProcessInfo.processInfo.environment["WAFFLE_TABTEST"] {
            DispatchQueue.main.asyncAfter(deadline: .now() + 3) { Self.tabTest(query: q) }
        }
    }

    /// Debug aid: open several files, then report tab grouping and a cross-file search.
    static func tabTest(query: String) {
        let docs = NSDocumentController.shared.documents.compactMap { $0 as? Document }
        print("documents: \(docs.count)")
        for d in docs {
            let b = d.book!
            print("  \(d.displayName ?? "?"): sheets \((0..<b.sheetCount).map { b.sheetName($0) })")
        }
        let windows = docs.compactMap { $0.windowControllers.first?.window }
        let tabs = windows.first.map { WindowTabs.shared.tabs(of: $0).count } ?? 1
        print("windows: \(windows.count), tab groups: \(WindowTabs.shared.groups.count), tabs in group: \(tabs)")
        guard let wc = docs.first?.windowControllers.first as? SheetWindowController else { exit(1) }
        let fc = FindCenter.shared
        fc.query = query
        var seen: [String] = []
        for i in 0..<8 {
            // Step from the tab on show, as clicking its Find arrows would.
            let shown = wc.window.flatMap { WindowTabs.shared.group(of: $0)?.selected }?.windowController as? SheetWindowController ?? wc
            fc.step(from: shown, forward: i >= 4 ? true : false)
            if let h = fc.currentHit, let d = h.doc {
                seen.append("\(d.displayName ?? "?") › \(d.book!.sheetName(h.sheet)) › \(cellName(h.pos))  [\(fc.status)]")
            }
        }
        print("search '\(query)' across open files:")
        seen.forEach { print("  " + $0) }
        let front = docs.first { $0.windowControllers.first?.window?.isKeyWindow == true }
        print("key tab now: \(front?.displayName ?? "none")")
        fflush(stdout)
        exit(0)
    }
}
#else
extension AppDelegate {
    func runDebugHooks() {}
}
#endif
