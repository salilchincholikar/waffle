import AppKit
import WaffleBridge

protocol SheetTabsDelegate: AnyObject {
    func tabsSelect(_ index: Int)
    func tabsAdd()
    func tabsRename(_ index: Int)
    func tabsMenu(for index: Int) -> NSMenu?
    func tabsMove(from: Int, to: Int)
}

/// A label that reports changes (so layout can react when the status text appears).
final class StatusLabel: NSTextField {
    var onChange: (() -> Void)?
    override var stringValue: String {
        didSet { if stringValue != oldValue { onChange?() } }
    }
}

/// Bottom bar: sheet tabs (a system segmented control) on the left, status and load
/// progress on the right. A plain bar under the grid, like Numbers and Finder.
final class BottomBar: NSView {
    static let height: CGFloat = 34
    /// Clear of the window's rounded bottom corners.
    static let inset: CGFloat = 14

    let tabs = SheetTabs()
    let status = StatusLabel(labelWithString: "")
    let progress = NSProgressIndicator()
    private let add = NSButton()

    override init(frame: NSRect) {
        super.init(frame: frame)
        add.image = NSImage(systemSymbolName: "plus", accessibilityDescription: "Add Sheet")?
            .withSymbolConfiguration(.init(pointSize: 13, weight: .medium))
        add.isBordered = false
        add.bezelStyle = .regularSquare
        add.contentTintColor = .secondaryLabelColor
        add.toolTip = "Add Sheet"
        add.target = self
        add.action = #selector(addSheet)
        status.font = .monospacedDigitSystemFont(ofSize: NSFont.smallSystemFontSize, weight: .regular)
        status.textColor = .secondaryLabelColor
        status.alignment = .right
        status.lineBreakMode = .byTruncatingHead
        status.isSelectable = true
        status.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        progress.style = .bar
        progress.controlSize = .small
        progress.isIndeterminate = false
        progress.minValue = 0
        progress.maxValue = 1
        progress.isHidden = true
        let separator = NSBox()
        separator.boxType = .separator
        let background = NSVisualEffectView.windowChrome()
        addSubview(background)
        NSLayoutConstraint.activate([
            background.topAnchor.constraint(equalTo: topAnchor),
            background.bottomAnchor.constraint(equalTo: bottomAnchor),
            background.leadingAnchor.constraint(equalTo: leadingAnchor),
            background.trailingAnchor.constraint(equalTo: trailingAnchor),
        ])
        // Status and progress share a stack so a hidden progress bar takes no space.
        let right = NSStackView(views: [status, progress])
        right.spacing = 8
        right.detachesHiddenViews = true
        for v in [separator, add, tabs, right] as [NSView] {
            v.translatesAutoresizingMaskIntoConstraints = false
            addSubview(v)
        }
        NSLayoutConstraint.activate([
            separator.topAnchor.constraint(equalTo: topAnchor),
            separator.leadingAnchor.constraint(equalTo: leadingAnchor),
            separator.trailingAnchor.constraint(equalTo: trailingAnchor),
            add.leadingAnchor.constraint(equalTo: leadingAnchor, constant: Self.inset),
            add.centerYAnchor.constraint(equalTo: centerYAnchor),
            add.widthAnchor.constraint(equalToConstant: 22),
            add.heightAnchor.constraint(equalToConstant: 22),
            tabs.leadingAnchor.constraint(equalTo: add.trailingAnchor, constant: 10),
            tabs.centerYAnchor.constraint(equalTo: centerYAnchor),
            tabs.heightAnchor.constraint(equalToConstant: SheetTabs.controlHeight),
            tabs.trailingAnchor.constraint(lessThanOrEqualTo: right.leadingAnchor, constant: -16),
            right.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -Self.inset),
            right.centerYAnchor.constraint(equalTo: centerYAnchor),
            progress.widthAnchor.constraint(equalToConstant: 110),
            heightAnchor.constraint(equalToConstant: Self.height),
        ])
    }

    required init?(coder: NSCoder) { fatalError() }

    @objc private func addSheet() { tabs.delegate?.tabsAdd() }

    func showProgress(_ p: Double?) {
        progress.isHidden = p == nil
        if let p { progress.doubleValue = p }
    }
}

extension NSVisualEffectView {
    /// The translucent window material used behind the bars (desktop shows through).
    static func windowChrome() -> NSVisualEffectView {
        let v = NSVisualEffectView()
        v.material = .sidebar
        v.blendingMode = .behindWindow
        v.state = .followsWindowActiveState
        v.translatesAutoresizingMaskIntoConstraints = false
        return v
    }
}

/// Sheet tabs as a native segmented control (scrolls when there are many sheets).
/// Hidden sheets are left out; double-click renames, right-click shows the sheet menu.
final class SheetTabs: NSView {
    /// Natural height of a regular segmented control.
    static let controlHeight: CGFloat = NSSegmentedControl(labels: ["X"], trackingMode: .selectOne, target: nil, action: nil).fittingSize.height
    weak var delegate: SheetTabsDelegate?
    private let scroll = NSScrollView()
    private let control = TabSegments()
    /// Segment → sheet index (hidden sheets are skipped).
    private var sheetOf: [Int] = []
    private var widthConstraint: NSLayoutConstraint?
    private(set) var names: [String] = []

    var selected = 0 {
        didSet {
            if let seg = sheetOf.firstIndex(of: selected) {
                control.selectedSegment = seg
                scrollToSelected()
            }
        }
    }

    override init(frame: NSRect) {
        super.init(frame: frame)
        control.segmentStyle = .automatic
        control.trackingMode = .selectOne
        control.controlSize = .regular
        // Neutral selection, like tab strips in macOS (the accent fill reads as a mode switch).
        control.selectedSegmentBezelColor = .quaternaryLabelColor
        control.target = self
        control.action = #selector(changed)
        control.onDoubleClick = { [weak self] seg in
            guard let self, seg < self.sheetOf.count else { return }
            self.delegate?.tabsRename(self.sheetOf[seg])
        }
        control.menuFor = { [weak self] seg in
            guard let self, seg < self.sheetOf.count else { return nil }
            self.delegate?.tabsSelect(self.sheetOf[seg])
            return self.delegate?.tabsMenu(for: self.sheetOf[seg])
        }
        scroll.documentView = control
        scroll.hasHorizontalScroller = false
        scroll.drawsBackground = false
        scroll.horizontalScrollElasticity = .allowed
        scroll.translatesAutoresizingMaskIntoConstraints = false
        addSubview(scroll)
        NSLayoutConstraint.activate([
            scroll.leadingAnchor.constraint(equalTo: leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: trailingAnchor),
            scroll.topAnchor.constraint(equalTo: topAnchor),
            scroll.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])
    }

    required init?(coder: NSCoder) { fatalError() }

    func set(names: [String], hidden: [Bool], selected: Int) {
        self.names = names
        sheetOf = names.indices.filter { !hidden[$0] }
        control.segmentCount = sheetOf.count
        let font = NSFont.systemFont(ofSize: NSFont.systemFontSize)
        for (seg, sheet) in sheetOf.enumerated() {
            control.setLabel(names[sheet], forSegment: seg)
            control.setWidth(ceil((names[sheet] as NSString).size(withAttributes: [.font: font]).width) + 28, forSegment: seg)
        }
        control.sizeToFit()
        control.frame = NSRect(x: 0, y: 0, width: control.frame.width, height: Self.controlHeight)
        widthConstraint?.isActive = false
        widthConstraint = widthAnchor.constraint(equalToConstant: control.frame.width).withPriority(.defaultHigh)
        widthConstraint?.isActive = true
        needsLayout = true
        self.selected = selected
    }


    private func scrollToSelected() {
        let seg = control.selectedSegment
        guard seg >= 0 else { return }
        var x: CGFloat = 0
        for i in 0..<seg { x += control.width(forSegment: i) }
        control.scrollToVisible(NSRect(x: x, y: 0, width: control.width(forSegment: seg), height: control.frame.height))
    }

    @objc private func changed() {
        let seg = control.selectedSegment
        guard seg >= 0, seg < sheetOf.count else { return }
        delegate?.tabsSelect(sheetOf[seg])
    }

    /// Move the selected sheet one place left/right (sheet menu).
    func moveSelected(by delta: Int) {
        guard let seg = sheetOf.firstIndex(of: selected), seg + delta >= 0, seg + delta < sheetOf.count else {
            NSSound.beep()
            return
        }
        delegate?.tabsMove(from: sheetOf[seg], to: sheetOf[seg + delta])
    }
}

/// Segmented control that also reports double-clicks and asks for a per-segment context menu.
final class TabSegments: NSSegmentedControl {
    var onDoubleClick: ((Int) -> Void)?
    var menuFor: ((Int) -> NSMenu?)?

    private func segment(at event: NSEvent) -> Int? {
        let x = convert(event.locationInWindow, from: nil).x
        var edge: CGFloat = 0
        for i in 0..<segmentCount {
            edge += width(forSegment: i)
            if x < edge { return i }
        }
        return nil
    }

    override func mouseDown(with event: NSEvent) {
        if event.clickCount == 2, let i = segment(at: event) {
            onDoubleClick?(i)
            return
        }
        super.mouseDown(with: event)
    }

    override func menu(for event: NSEvent) -> NSMenu? {
        guard let i = segment(at: event) else { return nil }
        return menuFor?(i)
    }
}

extension NSLayoutConstraint {
    func withPriority(_ p: NSLayoutConstraint.Priority) -> NSLayoutConstraint {
        priority = p
        return self
    }
}
