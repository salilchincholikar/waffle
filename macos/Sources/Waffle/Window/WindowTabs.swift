import AppKit

/// Waffle's own window tabs. macOS tabbing always adds a tab-bar row once a window has two
/// tabs, so Waffle groups document windows itself: a group shows one window (the selected
/// tab) and keeps the others ordered out at the same frame. The tabs are drawn in each
/// window's title row (TitleTabs).
final class WindowTabs {
    static let shared = WindowTabs()
    static let changed = Notification.Name("WaffleWindowTabsChanged")

    final class Group {
        var windows: [NSWindow] = []
        weak var selected: NSWindow?
    }

    private(set) var groups: [Group] = []
    /// The group last worked in: new files join it.
    private weak var active: Group?


    func group(of w: NSWindow) -> Group? { groups.first { $0.windows.contains { $0 === w } } }

    /// The tabs shown alongside `w` (just `w` when it isn't grouped yet).
    func tabs(of w: NSWindow) -> [NSWindow] { group(of: w)?.windows ?? [w] }

    /// A document window about to be shown: it joins the active group as the selected tab
    /// (after the current one), or starts a new group. Returns false when it starts a group,
    /// so the caller shows it normally.
    @discardableResult
    func adopt(_ w: NSWindow, background: Bool = false) -> Bool {
        guard group(of: w) == nil else { return false }
        if let g = active, let current = g.selected, current !== w {
            let i = g.windows.firstIndex { $0 === current }.map { $0 + 1 } ?? g.windows.count
            g.windows.insert(w, at: i)
            // A window macOS restores joins in the background; a file you open comes forward.
            if !background || !current.isVisible { show(w, replacing: current, in: g) }
            post()
            return true
        }
        let g = Group()
        g.windows = [w]
        g.selected = w
        groups.append(g)
        active = g
        post()
        return false
    }

    func select(_ w: NSWindow) {
        guard let g = group(of: w), let current = g.selected, current !== w else { return }
        show(w, replacing: current, in: g)
        post()
    }

    /// Next (+1) or previous (-1) tab, wrapping around.
    func step(from w: NSWindow, by d: Int) {
        guard let g = group(of: w), g.windows.count > 1, let i = g.windows.firstIndex(where: { $0 === w }) else { return }
        select(g.windows[(i + d + g.windows.count) % g.windows.count])
    }

    /// Called when a window becomes key (also via the Window menu or ⌘`): keep its group
    /// active and make sure it is the tab on show.
    func didBecomeKey(_ w: NSWindow) {
        guard let g = group(of: w) else { return }
        active = g
        if g.selected !== w { select(w) }
    }

    /// A window is closing: drop it, and show its neighbour if it was on show.
    func remove(_ w: NSWindow) {
        guard let g = group(of: w), let i = g.windows.firstIndex(where: { $0 === w }) else { return }
        g.windows.remove(at: i)
        if g.windows.isEmpty {
            groups.removeAll { $0 === g }
        } else if g.selected === w || g.selected == nil {
            let next = g.windows[min(i, g.windows.count - 1)]
            next.setFrame(w.frame, display: false)
            g.selected = next
            next.makeKeyAndOrderFront(nil)
        }
        post()
    }

    /// Window ▸ Move Tab to New Window.
    func moveToNewWindow(_ w: NSWindow) {
        guard let g = group(of: w), g.windows.count > 1, let i = g.windows.firstIndex(where: { $0 === w }) else { return }
        g.windows.remove(at: i)
        let next = g.windows[min(i, g.windows.count - 1)]
        next.setFrame(w.frame, display: false)
        g.selected = next
        next.orderFront(nil)
        let n = Group()
        n.windows = [w]
        n.selected = w
        groups.append(n)
        active = n
        w.setFrameTopLeftPoint(NSPoint(x: w.frame.minX + 28, y: w.frame.maxY - 28))
        w.makeKeyAndOrderFront(nil)
        post()
    }

    /// Window ▸ Merge All Windows: every tab joins `w`'s group.
    func mergeAll(into w: NSWindow) {
        guard let target = group(of: w) else { return }
        for g in groups where g !== target {
            for x in g.windows {
                x.orderOut(nil)
                target.windows.append(x)
            }
        }
        groups = [target]
        post()
    }

    /// Swap the tab on show in place: no window animation (it read as the app bouncing),
    /// and the new window goes directly above the old one before the old one leaves.
    private func show(_ w: NSWindow, replacing current: NSWindow, in g: Group) {
        let (wAnim, cAnim) = (w.animationBehavior, current.animationBehavior)
        w.animationBehavior = .none
        current.animationBehavior = .none
        w.setFrame(current.frame, display: true)
        g.selected = w
        active = g
        w.order(.above, relativeTo: current.windowNumber)
        w.makeKey()
        current.orderOut(nil)
        w.animationBehavior = wAnim
        current.animationBehavior = cAnim
    }

    fileprivate func post() { NotificationCenter.default.post(name: Self.changed, object: self) }
}

/// A document window: joins a tab group the moment it is first put on screen, however that
/// happens (opening, Finder, several files at once, windows macOS restores at launch).
final class DocumentWindow: NSWindow {
    /// Being restored from the last session (its document is marked while that happens).
    var restoring: Bool { (windowController?.document as? Document)?.restoring == true }

    /// A hidden tab asked to come forward some other way (Find jumping to a match in
    /// another file, the Window menu): switch tabs in place, without the window animation.
    private func switchedToHiddenTab() -> Bool {
        guard !restoring, let g = WindowTabs.shared.group(of: self), g.selected !== self else { return false }
        WindowTabs.shared.select(self)
        return true
    }

    override func makeKeyAndOrderFront(_ sender: Any?) {
        if !switchedToHiddenTab() { super.makeKeyAndOrderFront(sender) }
    }

    override func orderFront(_ sender: Any?) {
        if !switchedToHiddenTab() { super.orderFront(sender) }
    }

    override func order(_ place: NSWindow.OrderingMode, relativeTo otherWin: Int) {
        let tabs = WindowTabs.shared
        if place != .out {
            if let g = tabs.group(of: self) {
                // A hidden tab brought forward (Window menu, ⌘`, the app itself): it becomes
                // the tab on show and the current one hides.
                if g.selected !== self {
                    // Restored tabs, or windows ordered behind others, stay in the background.
                    if !restoring && place == .above { tabs.select(self) }
                    return
                }
            } else if tabs.adopt(self, background: restoring || place == .below) {
                return  // shown as the group's selected tab
            }
        }
        super.order(place, relativeTo: otherWin)
    }
}
