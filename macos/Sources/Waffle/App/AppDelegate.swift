import AppKit
import WaffleBridge

final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationWillFinishLaunching(_ notification: Notification) {
        NSApp.mainMenu = buildMenu()
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSWindow.allowsAutomaticWindowTabbing = false  // Waffle draws its own tabs
        runDebugHooks()
        // Launched on its own (Dock, Spotlight), not to open files: show the Open panel,
        // like Numbers. Launches from Finder's Open / Open With aren't "default" launches.
        if notification.userInfo?[NSApplication.launchIsDefaultUserInfoKey] as? Bool == true {
            DispatchQueue.main.async {
                if NSDocumentController.shared.documents.isEmpty { NSDocumentController.shared.openDocument(nil) }
            }
        }
    }

    /// View ▸ Dark Sheet in Dark Mode (off by default: the sheet stays white).
    @objc func toggleDarkSheet(_ sender: Any?) { SheetAppearance.darkInDarkMode.toggle() }

    func validateMenuItem(_ item: NSMenuItem) -> Bool {
        if item.action == #selector(toggleDarkSheet(_:)) { item.state = SheetAppearance.darkInDarkMode ? .on : .off }
        return true
    }

    func applicationShouldOpenUntitledFile(_ sender: NSApplication) -> Bool { false }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        if !flag { NSDocumentController.shared.openDocument(nil) }
        return true
    }


}
