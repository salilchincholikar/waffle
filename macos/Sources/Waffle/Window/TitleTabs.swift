import AppKit

/// Open files as compact tabs in the title row (like Safari's compact tabs); the grouping
/// lives in WindowTabs. With one file it reads as a plain title.
final class TitleTabs: NSView {
    private let stack = NSStackView()
    private let add = NSButton()

    override init(frame: NSRect) {
        super.init(frame: frame)
        stack.spacing = 4
        stack.distribution = .fillEqually
        stack.translatesAutoresizingMaskIntoConstraints = false
        add.image = NSImage(systemSymbolName: "plus", accessibilityDescription: "New Tab")?
            .withSymbolConfiguration(.init(pointSize: 11, weight: .semibold))
        add.isBordered = false
        add.contentTintColor = .secondaryLabelColor
        add.toolTip = "Open a file in a new tab"
        add.target = nil
        add.action = #selector(NSResponder.newWindowForTab(_:))
        add.translatesAutoresizingMaskIntoConstraints = false
        addSubview(stack)
        addSubview(add)
        NotificationCenter.default.addObserver(self, selector: #selector(reload), name: WindowTabs.changed, object: nil)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: leadingAnchor),
            stack.topAnchor.constraint(equalTo: topAnchor),
            stack.bottomAnchor.constraint(equalTo: bottomAnchor),
            add.leadingAnchor.constraint(equalTo: stack.trailingAnchor, constant: 6),
            add.centerYAnchor.constraint(equalTo: centerYAnchor),
            add.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor),
            add.widthAnchor.constraint(equalToConstant: 20),
        ])
    }

    required init?(coder: NSCoder) { fatalError() }

    /// Rebuild from the window's tab group (tabs changed, or this window retitled).
    @objc func reload() {
        guard let window else { return }
        let windows = WindowTabs.shared.tabs(of: window)
        let multi = windows.count > 1
        stack.arrangedSubviews.forEach { $0.removeFromSuperview() }
        for w in windows {
            let t = TitleTab(title: w.title, selected: w === window, pill: multi)
            t.onSelect = { [weak w] in
                if let w { WindowTabs.shared.select(w) }
            }
            t.onClose = { [weak w] in w?.performClose(nil) }
            stack.addArrangedSubview(t)
        }
        add.isHidden = !multi
    }
}

/// One tab: file name, and a close button while hovered or selected.
private final class TitleTab: NSView {
    var onSelect: (() -> Void)?
    var onClose: (() -> Void)?
    private let label: NSTextField
    private let close = NSButton()
    private let selected: Bool
    private let pill: Bool
    private var hovering = false { didSet { update() } }

    init(title: String, selected: Bool, pill: Bool) {
        self.selected = selected
        self.pill = pill
        label = NSTextField(labelWithString: title)
        super.init(frame: .zero)
        wantsLayer = true
        layer?.cornerRadius = 7
        label.font = .systemFont(ofSize: NSFont.systemFontSize, weight: selected ? .semibold : .regular)
        label.lineBreakMode = .byTruncatingMiddle
        label.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        close.image = NSImage(systemSymbolName: "xmark", accessibilityDescription: "Close Tab")?
            .withSymbolConfiguration(.init(pointSize: 9, weight: .bold))
        close.isBordered = false
        close.contentTintColor = .secondaryLabelColor
        close.target = self
        close.action = #selector(closeTab)
        close.toolTip = "Close Tab"
        for v in [label, close] as [NSView] {
            v.translatesAutoresizingMaskIntoConstraints = false
            addSubview(v)
        }
        let pad: CGFloat = pill ? 10 : 0
        NSLayoutConstraint.activate([
            heightAnchor.constraint(equalToConstant: 24),
            close.leadingAnchor.constraint(equalTo: leadingAnchor, constant: pill ? 6 : 0),
            close.centerYAnchor.constraint(equalTo: centerYAnchor),
            close.widthAnchor.constraint(equalToConstant: pill ? 14 : 0),
            label.leadingAnchor.constraint(equalTo: close.trailingAnchor, constant: pill ? 4 : 0),
            label.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -pad),
            label.centerYAnchor.constraint(equalTo: centerYAnchor),
            widthAnchor.constraint(lessThanOrEqualToConstant: pill ? 220 : 480),
        ])
        if pill {
            widthAnchor.constraint(greaterThanOrEqualToConstant: 80).isActive = true
            // Roomy by default; tabs only narrow when there are many.
            widthAnchor.constraint(equalToConstant: 200).withPriority(.init(260)).isActive = true
        }
        toolTip = title
        update()
    }

    required init?(coder: NSCoder) { fatalError() }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        trackingAreas.forEach(removeTrackingArea)
        addTrackingArea(NSTrackingArea(rect: bounds, options: [.mouseEnteredAndExited, .activeInActiveApp, .inVisibleRect], owner: self))
    }

    override func mouseEntered(with event: NSEvent) { hovering = true }
    override func mouseExited(with event: NSEvent) { hovering = false }
    override func mouseDown(with event: NSEvent) { if !selected { onSelect?() } }
    // Clicks on a tab select it rather than dragging the window.
    override var mouseDownCanMoveWindow: Bool { !pill }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        update()
    }

    private func update() {
        close.isHidden = !pill || !(selected || hovering)
        label.textColor = selected || !pill ? .labelColor : .secondaryLabelColor
        effectiveAppearance.performAsCurrentDrawingAppearance {
            let bg: NSColor? = !pill ? nil : selected ? .quaternaryLabelColor : hovering ? NSColor.quaternaryLabelColor.withAlphaComponent(0.5) : nil
            layer?.backgroundColor = bg?.cgColor
        }
    }

    @objc private func closeTab() { onClose?() }
}
