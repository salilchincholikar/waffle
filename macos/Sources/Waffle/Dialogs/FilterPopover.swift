import AppKit
import WaffleBridge

final class FilterPopover: NSViewController, NSTableViewDataSource, NSTableViewDelegate, NSSearchFieldDelegate {
    private let all: [(String, Int)]
    private var shown: [Int] = []
    private var checked: [Bool]
    private let table = NSTableView()
    private let search = NSSearchField()
    var onApply: ((_ values: [String]?) -> Void)?

    init(values: [(String, Int)], active: Bool) {
        all = values
        checked = Array(repeating: true, count: values.count)
        super.init(nibName: nil, bundle: nil)
    }
    required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        let v = NSView(frame: NSRect(x: 0, y: 0, width: 280, height: 360))
        search.placeholderString = "Search values"
        search.delegate = self
        let col = NSTableColumn(identifier: .init("v"))
        col.width = 250
        table.addTableColumn(col)
        table.headerView = nil
        table.rowHeight = 20
        table.dataSource = self
        table.delegate = self
        table.style = .plain
        let scroll = NSScrollView()
        scroll.documentView = table
        scroll.hasVerticalScroller = true
        scroll.borderType = .bezelBorder
        let selectAll = NSButton(title: "Select All", target: self, action: #selector(checkAll))
        let none = NSButton(title: "None", target: self, action: #selector(checkNone))
        let clear = NSButton(title: "Clear Filter", target: self, action: #selector(clearFilter))
        let apply = NSButton(title: "Apply", target: self, action: #selector(apply))
        apply.keyEquivalent = "\r"
        apply.bezelColor = .controlAccentColor
        for b in [selectAll, none, clear, apply] { b.bezelStyle = .rounded; b.controlSize = .small }
        let buttons = NSStackView(views: [selectAll, none, NSView(), clear, apply])
        buttons.distribution = .fill
        let stack = NSStackView(views: [search, scroll, buttons])
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 8
        stack.edgeInsets = NSEdgeInsets(top: 10, left: 10, bottom: 10, right: 10)
        stack.translatesAutoresizingMaskIntoConstraints = false
        v.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: v.leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: v.trailingAnchor),
            stack.topAnchor.constraint(equalTo: v.topAnchor),
            stack.bottomAnchor.constraint(equalTo: v.bottomAnchor),
            search.widthAnchor.constraint(equalTo: stack.widthAnchor, constant: -20),
            scroll.widthAnchor.constraint(equalTo: stack.widthAnchor, constant: -20),
            buttons.widthAnchor.constraint(equalTo: stack.widthAnchor, constant: -20),
        ])
        view = v
        refilter()
    }

    private func refilter() {
        let q = search.stringValue.lowercased()
        shown = all.indices.filter { q.isEmpty || all[$0].0.lowercased().contains(q) }
        table.reloadData()
    }

    func controlTextDidChange(_ obj: Notification) { refilter() }

    func numberOfRows(in tableView: NSTableView) -> Int { shown.count }

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        let i = shown[row]
        let title = all[i].0.isEmpty ? "(Blanks)" : all[i].0
        let b = NSButton(checkboxWithTitle: "\(title)  (\(all[i].1))", target: self, action: #selector(toggle(_:)))
        b.tag = i
        b.state = checked[i] ? .on : .off
        b.lineBreakMode = .byTruncatingTail
        return b
    }

    @objc private func toggle(_ b: NSButton) { checked[b.tag] = b.state == .on }
    @objc private func checkAll() { for i in shown { checked[i] = true }; table.reloadData() }
    @objc private func checkNone() { for i in shown { checked[i] = false }; table.reloadData() }
    @objc private func clearFilter() { onApply?(nil); dismiss(nil) }
    @objc private func apply() {
        if checked.allSatisfy({ $0 }) { onApply?(nil) } else { onApply?(all.indices.filter { checked[$0] }.map { all[$0].0 }) }
        dismiss(nil)
    }
}
