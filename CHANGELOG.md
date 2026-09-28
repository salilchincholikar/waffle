# Changelog

## 0.1.0 — 2026-09-28

The first open-source release: a ground-up rewrite (Rust engine + native AppKit app) of the earlier web-based prototype.

- **Files:**
  - Opens xlsx, xlsm, csv, tsv and txt, and imports xls, xlsb and ods.
  - Big files load in the background.
  - Saves "as is": untouched xlsx parts are copied byte-for-byte, unchanged CSV rows are copied exactly, and structural edits update charts, tables and names.
- **Editing:**
  - Typing, the formula bar, copy/paste to and from Excel/Numbers/Sheets, fill handle with series, undo/redo.
  - Insert, delete, hide and resize rows and columns; merge cells; freeze panes; manage sheets.
- **Formulas:** 195 Excel functions, recalculated in dependency order.
- **Data:**
  - Sort, filter, find & replace across all open files.
  - Clean-up tools: trim, empty and duplicate rows, case, dates, amounts, text to columns.
- **Display:**
  - Number formats, fonts, fills, borders, rich text.
  - Conditional formatting (including colour scales and data bars), Excel table styles, images.
  - Automatic row heights.
- **Mac app:**
  - A thin, browser-style title bar: file tabs at the left, Find and the sheet tools at the right; the formula row carries the formatting controls.
  - Find covers every open file, with match options as toggles in the field, live match counts and a Replace bar.
  - Translucent chrome, dark mode, and an optional dark sheet (View ▸ Dark Sheet in Dark Mode).
  - Opens files saved by tools that leave formula results out (Python libraries, web exports) with their formulas calculated.
