import AppKit
import WaffleBridge

/// Find & Replace in the title row: a Find field shaped like the formula field (like a
/// browser's address bar) with the match options as toggles inside it, the match count and
/// previous/next while finding, and a Replace bar that drops down under it. Find always covers every open
/// file; the state lives in FindCenter, so every tab shows the same find.
final class FindBar: NSObject, NSTextFieldDelegate {
    weak var controller: SheetWindowController?
    /// The title-row control.
    let view = FindField()
    var search: NSTextField { view.field }
    var count: NSTextField { view.count }
    /// Replace, a slim bar under the Find field (⇄ or ⌥⌘F; Esc closes).
    let replaceBar = ReplaceBar()
    var replace: NSTextField { replaceBar.field }
    private var pendingRefresh: DispatchWorkItem?

    /// Search shortly after typing pauses (a search covers every open file).
    private func scheduleRefresh() {
        pendingRefresh?.cancel()
        let work = DispatchWorkItem { [weak self] in
            guard let wc = self?.controller else { return }
            FindCenter.shared.refresh(from: wc)
        }
        pendingRefresh = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.15, execute: work)
    }

    override init() {
        super.init()
        search.delegate = self
        view.previous.target = self
        view.previous.action = #selector(previous)
        view.next.target = self
        view.next.action = #selector(next)
        view.replaceButton.target = self
        view.replaceButton.action = #selector(openReplace)
        for t in view.toggles {
            t.target = self
            t.action = #selector(optionsChanged)
        }

        replace.delegate = self
        replaceBar.one.target = self
        replaceBar.one.action = #selector(replaceOne)
        replaceBar.all.target = self
        replaceBar.all.action = #selector(replaceAll)
    }

    /// ⇄ / ⌥⌘F: show the Replace bar under the Find field (again: hide it).
    @objc func openReplace() {
        guard let content = view.window?.contentView else { return }
        if replaceBar.superview == nil {
            replaceBar.translatesAutoresizingMaskIntoConstraints = false
            content.addSubview(replaceBar)  // on top of the formula row and sheet
            NSLayoutConstraint.activate([
                replaceBar.topAnchor.constraint(equalTo: view.bottomAnchor, constant: 6),
                replaceBar.leadingAnchor.constraint(equalTo: view.leadingAnchor),
                replaceBar.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            ])
        } else if !replaceBar.isHidden, NSApp.currentEvent?.type == .leftMouseUp {
            closeReplace()  // the ⇄ button toggles
            return
        }
        replaceBar.isHidden = false
        view.window?.makeFirstResponder(replace)
    }

    private func closeReplace() {
        replaceBar.isHidden = true
        view.window?.makeFirstResponder(search)
    }

    /// Update from the shared state without clobbering a field that's being typed in.
    func sync(from fc: FindCenter) {
        if search.currentEditor() == nil && search.stringValue != fc.query { search.stringValue = fc.query }
        if replace.currentEditor() == nil && replace.stringValue != fc.replacement { replace.stringValue = fc.replacement }
        count.stringValue = fc.visible ? fc.status : ""
        view.matchCase.state = fc.flags.contains(.matchCase) ? .on : .off
        view.wholeCell.state = fc.flags.contains(.wholeCell) ? .on : .off
        view.inFormulas.state = fc.flags.contains(.formulas) ? .on : .off
        view.finding = fc.visible && !fc.query.isEmpty
        if !fc.visible && !replaceBar.isHidden { replaceBar.isHidden = true }
    }

    func controlTextDidChange(_ obj: Notification) {
        let fc = FindCenter.shared
        if (obj.object as? NSTextField) === replace {
            fc.replacement = replace.stringValue
        } else if (obj.object as? NSTextField) === search {
            fc.query = search.stringValue
            if !fc.visible && !search.stringValue.isEmpty {
                fc.visible = true
                fc.post()
            }
            view.finding = fc.visible && !search.stringValue.isEmpty
            scheduleRefresh()
        }
    }

    @objc private func optionsChanged() {
        var f: Book.FindFlags = []
        if view.matchCase.state == .on { f.insert(.matchCase) }
        if view.wholeCell.state == .on { f.insert(.wholeCell) }
        if view.inFormulas.state == .on { f.insert(.formulas) }
        FindCenter.shared.flags = f
        FindCenter.shared.post()
        scheduleRefresh()
    }

    @objc private func next() { controller?.find(forward: true) }
    @objc private func previous() { controller?.find(forward: false) }
    @objc private func replaceOne() { FindCenter.shared.replacement = replace.stringValue; controller?.replaceCurrent() }
    @objc private func replaceAll() { FindCenter.shared.replacement = replace.stringValue; controller?.replaceAll() }
    @objc private func close() { controller?.closeFind() }

    func control(_ control: NSControl, textView: NSTextView, doCommandBy sel: Selector) -> Bool {
        if sel == #selector(NSResponder.cancelOperation(_:)) {
            if control === replace { closeReplace() } else { close() }
            return true
        }
        if sel == #selector(NSResponder.insertNewline(_:)) {
            if control === replace { replaceOne() } else if NSApp.currentEvent?.modifierFlags.contains(.shift) == true { previous() } else { next() }
            return true
        }
        return false
    }
}

/// The Find field: a rounded field like the formula field. Inside it: a magnifier at the
/// left; at the right the match count and previous/next (while finding), the match
/// toggles (Aa match case, ab whole cell, ƒx in formulas) and Replace.
final class FindField: NSView {
    let field = FindTextField()
    let count = NSTextField(labelWithString: "")
    let previous = NSButton()
    let next = NSButton()
    let matchCase = FindToggle(title: "Aa", tip: "Match case")
    let wholeCell = FindToggle(title: "ab", tip: "Whole cell only", underline: true)
    let inFormulas = FindToggle(title: "ƒx", tip: "Look in formulas, not just values")
    let replaceButton = NSButton()
    var toggles: [FindToggle] { [matchCase, wholeCell, inFormulas] }
    private let stepper: NSStackView
    private let accessories: NSStackView
    private let cell = InsetTextFieldCell(textCell: "")

    /// Shows the count and previous/next.
    var finding = false {
        didSet { if finding != oldValue { stepper.isHidden = !finding } }
    }

    override init(frame: NSRect) {
        stepper = NSStackView(views: [count, previous, next])
        let divider = NSBox()
        divider.boxType = .separator
        divider.heightAnchor.constraint(equalToConstant: 14).isActive = true
        accessories = NSStackView(views: [stepper, matchCase, wholeCell, inFormulas, divider, replaceButton])
        super.init(frame: frame)
        field.cell = cell
        cell.isEditable = true
        cell.isSelectable = true
        cell.isScrollable = true
        cell.usesSingleLineMode = true
        cell.lineBreakMode = .byClipping
        field.isBezeled = true
        field.bezelStyle = .roundedBezel
        field.drawsBackground = true
        field.font = .systemFont(ofSize: 13)
        field.placeholderString = "Find"
        field.toolTip = "Find in all open files (⌘F) · Replace (⌥⌘F)"
        cell.leftInset = 24

        func glyph(_ b: NSButton, _ name: String, _ tip: String) {
            b.image = NSImage(systemSymbolName: name, accessibilityDescription: tip)?
                .withSymbolConfiguration(.init(pointSize: 11, weight: .medium))
            b.isBordered = false
            b.contentTintColor = .secondaryLabelColor
            b.toolTip = tip
            b.setContentHuggingPriority(.required, for: .horizontal)
        }
        glyph(previous, "chevron.up", "Previous match (⇧⌘G)")
        glyph(next, "chevron.down", "Next match (⌘G)")
        glyph(replaceButton, "arrow.2.squarepath", "Replace… (⌥⌘F)")
        for b in [previous, next, replaceButton] { b.heightAnchor.constraint(equalToConstant: 18).isActive = true }
        let magnifier = NSImageView(image: NSImage(systemSymbolName: "magnifyingglass", accessibilityDescription: nil)!
            .withSymbolConfiguration(.init(pointSize: 12, weight: .medium))!)
        magnifier.contentTintColor = .secondaryLabelColor
        count.font = .monospacedDigitSystemFont(ofSize: NSFont.smallSystemFontSize, weight: .regular)
        count.textColor = .secondaryLabelColor
        count.alignment = .right
        stepper.spacing = 4
        stepper.isHidden = true
        accessories.spacing = 3
        accessories.setCustomSpacing(8, after: stepper)
        accessories.setCustomSpacing(6, after: inFormulas)
        accessories.setCustomSpacing(6, after: divider)

        for v in [field, magnifier, accessories] as [NSView] {
            v.translatesAutoresizingMaskIntoConstraints = false
            addSubview(v)
        }
        // Typed text stays clear of the controls at the right, count included, so the
        // text area doesn't shift when a find starts ("999 of 999" fits).
        let countWidth = ("999 of 999" as NSString).size(withAttributes: [.font: count.font!]).width + 2
        count.widthAnchor.constraint(equalToConstant: countWidth).isActive = true
        stepper.isHidden = false
        accessories.layoutSubtreeIfNeeded()
        cell.rightInset = accessories.fittingSize.width + 10
        stepper.isHidden = true
        NSLayoutConstraint.activate([
            field.leadingAnchor.constraint(equalTo: leadingAnchor),
            field.trailingAnchor.constraint(equalTo: trailingAnchor),
            field.centerYAnchor.constraint(equalTo: centerYAnchor),
            heightAnchor.constraint(equalTo: field.heightAnchor),
            magnifier.leadingAnchor.constraint(equalTo: field.leadingAnchor, constant: 8),
            magnifier.centerYAnchor.constraint(equalTo: field.centerYAnchor),
            accessories.trailingAnchor.constraint(equalTo: field.trailingAnchor, constant: -6),
            accessories.centerYAnchor.constraint(equalTo: field.centerYAnchor),
        ])
    }

    required init?(coder: NSCoder) { fatalError() }

    // Controls over the field get the arrow (the field's I-beam covers only the text).
    override func resetCursorRects() {
        addCursorRect(accessories.frame, cursor: .arrow)
        addCursorRect(NSRect(x: 0, y: 0, width: cell.leftInset, height: bounds.height), cursor: .arrow)
    }
}

/// The Find text field: the I-beam only over the text area, not the controls laid over it.
final class FindTextField: NSTextField {
    override func resetCursorRects() {
        guard let cell else { return super.resetCursorRects() }
        addCursorRect(cell.drawingRect(forBounds: bounds), cursor: .iBeam)
    }
}

/// A small on/off text toggle inside the Find field (Aa, ab, ƒx); accent-tinted when on.
final class FindToggle: NSButton {
    private let label: String
    private let underline: Bool

    init(title: String, tip: String, underline: Bool = false) {
        label = title
        self.underline = underline
        super.init(frame: .zero)
        setButtonType(.pushOnPushOff)
        isBordered = false
        wantsLayer = true
        layer?.cornerRadius = 4
        toolTip = tip
        setAccessibilityLabel(tip)
        widthAnchor.constraint(equalToConstant: 22).isActive = true
        heightAnchor.constraint(equalToConstant: 18).isActive = true
        refresh()
    }

    required init?(coder: NSCoder) { fatalError() }

    override var state: NSControl.StateValue { didSet { refresh() } }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        refresh()
    }

    private func refresh() {
        let on = state == .on
        var attrs: [NSAttributedString.Key: Any] = [
            .font: NSFont.systemFont(ofSize: 11, weight: .semibold),
            .foregroundColor: on ? NSColor.controlAccentColor : NSColor.secondaryLabelColor,
        ]
        if underline { attrs[.underlineStyle] = NSUnderlineStyle.single.rawValue }
        attributedTitle = NSAttributedString(string: label, attributes: attrs)
        effectiveAppearance.performAsCurrentDrawingAppearance {
            layer?.backgroundColor = on ? NSColor.controlAccentColor.withAlphaComponent(0.18).cgColor : nil
        }
    }
}

/// A text field cell that leaves room at the sides for views laid over the field.
final class InsetTextFieldCell: NSTextFieldCell {
    var leftInset: CGFloat = 0
    var rightInset: CGFloat = 0

    private func inset(_ r: NSRect) -> NSRect {
        var r = r
        r.origin.x += leftInset
        r.size.width = max(0, r.width - leftInset - rightInset)
        return r
    }

    // The one place the inset applies: AppKit derives both the drawn text and the typing
    // (field editor) area from this, so insetting edit/select frames too would inset twice.
    override func drawingRect(forBounds rect: NSRect) -> NSRect { super.drawingRect(forBounds: inset(rect)) }
}

/// Replace, as a slim bar under the Find field: a field shaped like Find's, with Replace
/// and Replace All inside it at the right, on a floating card.
final class ReplaceBar: NSView {
    let field = FindTextField()
    let one = NSButton()
    let all = NSButton()
    private let cell = InsetTextFieldCell(textCell: "")
    private let buttons: NSStackView

    override init(frame: NSRect) {
        buttons = NSStackView(views: [one, all])
        super.init(frame: frame)
        isHidden = true
        let card = NSVisualEffectView()
        card.material = .menu
        card.state = .active
        card.wantsLayer = true
        card.layer?.cornerRadius = 10
        card.layer?.masksToBounds = true
        wantsLayer = true
        shadow = NSShadow()
        layer?.shadowOpacity = 0.25
        layer?.shadowRadius = 10
        layer?.shadowOffset = CGSize(width: 0, height: -3)

        field.cell = cell
        cell.isEditable = true
        cell.isSelectable = true
        cell.isScrollable = true
        cell.usesSingleLineMode = true
        cell.lineBreakMode = .byClipping
        field.isBezeled = true
        field.bezelStyle = .roundedBezel
        field.drawsBackground = true
        field.font = .systemFont(ofSize: 13)
        field.placeholderString = "Replace with"
        cell.leftInset = 24
        let icon = NSImageView(image: NSImage(systemSymbolName: "arrow.2.squarepath", accessibilityDescription: nil)!
            .withSymbolConfiguration(.init(pointSize: 11, weight: .medium))!)
        icon.contentTintColor = .secondaryLabelColor

        func textButton(_ b: NSButton, _ title: String, _ tip: String, accent: Bool) {
            b.isBordered = false
            b.attributedTitle = NSAttributedString(string: title, attributes: [
                .font: NSFont.systemFont(ofSize: 12, weight: .semibold),
                .foregroundColor: accent ? NSColor.controlAccentColor : NSColor.secondaryLabelColor,
            ])
            b.toolTip = tip
            b.setContentHuggingPriority(.required, for: .horizontal)
            b.heightAnchor.constraint(equalToConstant: 18).isActive = true
        }
        textButton(one, "Replace", "Replace this match (↩)", accent: false)
        textButton(all, "Replace All", "Replace every match in all open files (⌥↩)", accent: true)
        all.keyEquivalent = "\r"
        all.keyEquivalentModifierMask = .option
        buttons.spacing = 10

        for v in [card, field, icon, buttons] as [NSView] {
            v.translatesAutoresizingMaskIntoConstraints = false
            addSubview(v)
        }
        buttons.layoutSubtreeIfNeeded()
        cell.rightInset = buttons.fittingSize.width + 14
        NSLayoutConstraint.activate([
            card.topAnchor.constraint(equalTo: topAnchor),
            card.bottomAnchor.constraint(equalTo: bottomAnchor),
            card.leadingAnchor.constraint(equalTo: leadingAnchor),
            card.trailingAnchor.constraint(equalTo: trailingAnchor),
            field.topAnchor.constraint(equalTo: topAnchor, constant: 6),
            field.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -6),
            field.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 6),
            field.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -6),
            icon.leadingAnchor.constraint(equalTo: field.leadingAnchor, constant: 8),
            icon.centerYAnchor.constraint(equalTo: field.centerYAnchor),
            buttons.trailingAnchor.constraint(equalTo: field.trailingAnchor, constant: -8),
            buttons.centerYAnchor.constraint(equalTo: field.centerYAnchor),
        ])
    }

    required init?(coder: NSCoder) { fatalError() }

    override func resetCursorRects() {
        addCursorRect(convert(buttons.bounds, from: buttons), cursor: .arrow)
    }
}
