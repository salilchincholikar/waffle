import AppKit
import WaffleBridge

let app = NSApplication.shared
// The first NSDocumentController created becomes the shared one.
_ = DocumentController()
let delegate = AppDelegate()
app.delegate = delegate
app.setActivationPolicy(.regular)
app.run()
