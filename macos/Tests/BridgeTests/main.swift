// Bridge tests: exercise the Swift ↔ Rust layer the app is built on.
// A plain executable (no XCTest / Swift Testing needed), so it runs with just the
// Command Line Tools and in CI:  make test-swift
import Foundation
import WaffleBridge

var failures = 0
var passed = 0
func check(_ ok: @autoclosure () -> Bool, _ what: String, line: Int = #line) {
    if ok() { passed += 1 } else { failures += 1; print("FAIL (line \(line)): \(what)") }
}
func test(_ name: String, _ body: () throws -> Void) {
    do { try body() } catch { failures += 1; print("FAIL \(name): \(error)") }
}

let corpus = URL(fileURLWithPath: #filePath)
    .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
    .appendingPathComponent("testdata/files")

func openLoaded(_ name: String) throws -> Book {
    let book = try Book.open(corpus.appendingPathComponent(name))
    let deadline = Date().addingTimeInterval(30)  // generous: CI machines are slow
    while !book.isLoaded && Date() < deadline { Thread.sleep(forTimeInterval: 0.01) }
    check(book.isLoaded, "\(name) finished loading")
    return book
}

test("cell names") {
    check(columnName(0) == "A", "A")
    check(columnName(25) == "Z", "Z")
    check(columnName(26) == "AA", "AA")
    check(columnName(16383) == "XFD", "XFD")
    check(cellName(CellPos(r: 11, c: 1)) == "B12", "B12")
    check(parseCellName("$b$12") == CellPos(r: 11, c: 1), "parse $b$12")
    check(parseCellName("A0") == nil, "A0 is invalid")
    check(parseCellName("XFE1") == nil, "XFE is past the last column")
}

test("cell rects") {
    let r = CellRect(r0: 5, c0: 3, r1: 1, c1: 0)
    check(r.r0 == 1 && r.r1 == 5 && r.c0 == 0 && r.c1 == 3, "normalised corners")
    check(r.contains(CellPos(r: 2, c: 2)), "contains inside")
    check(!r.contains(CellPos(r: 6, c: 2)), "excludes outside")
    check(CellRect(r0: 0, c0: 2, r1: CellRect.maxRows - 1, c1: 2).isFullCols, "whole column")
}

test("open, edit, undo") {
    let book = try openLoaded("styled.xlsx")
    check(book.sheetName(0) == "Styled", "sheet name")
    check(book.displayText(0, CellPos(r: 20, c: 1)) == "1,234,567.89", "number format")
    check(book.setInput(0, CellPos(r: 50, c: 0), "hello"), "set input")
    check(book.displayText(0, CellPos(r: 50, c: 0)) == "hello", "value shows")
    check(book.undoLabel == "Typing", "undo label")
    check(book.undo() == 0, "undo returns the sheet")
    check(book.displayText(0, CellPos(r: 50, c: 0)).isEmpty, "undo clears")
}

test("formulas recalculate") {
    let book = try openLoaded("formulas.xlsx")
    check(book.setInput(0, CellPos(r: 1, c: 0), "100"), "edit input")
    check(book.displayText(0, CellPos(r: 1, c: 1)) == "200", "dependent formula updated")
}

test("fetch visible cells") {
    let book = try openLoaded("comma.csv")
    let (cells, text) = book.fetch(0, CellRect(r0: 0, c0: 0, r1: 2, c1: 2))
    check(cells.count == 9, "3×3 cells")
    let first = String(decoding: UnsafeBufferPointer(start: text!.advanced(by: Int(cells[0].text_off)), count: Int(cells[0].text_len)), as: UTF8.self)
    check(first == "id", "A1 text")
}

test("CSV saves byte-for-byte") {
    let book = try openLoaded("quoted.csv")
    let out = FileManager.default.temporaryDirectory.appendingPathComponent("waffle-bridge-test.csv")
    try book.save(to: out, csv: true)
    let saved = try Data(contentsOf: out), original = try Data(contentsOf: corpus.appendingPathComponent("quoted.csv"))
    check(saved == original, "identical bytes")
}

test("find in a sheet") {
    let book = try openLoaded("rich.xlsx")
    let data = (0..<book.sheetCount).first { book.sheetName($0) == "Data" }!
    check(book.findAll(data, "North", [], limit: 100).count == 5, "5 matches")
}

print(failures == 0 ? "✓ bridge tests: \(passed) checks passed" : "✗ bridge tests: \(failures) failed, \(passed) passed")
exit(failures == 0 ? 0 : 1)
