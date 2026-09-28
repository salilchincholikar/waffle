# What "save as is" means

Waffle's promise: **saving changes only what you changed.**

## xlsx / xlsm

| Part of the file | On save |
|---|---|
| Untouched parts: other sheets, charts, images, pivot tables, macros (`vbaProject.bin`), themes, comments… | Copied byte-for-byte, still compressed |
| An edited sheet | Its `<sheetData>` is rewritten from the model. Everything before and after it is kept: column widths, sheet views, merged cells, conditional formatting, data validation, hyperlinks, print setup, extensions |
| Shared strings | New strings are appended; existing indices never change. Files without a shared-string table keep writing text inline |
| Styles | New fonts/fills/borders/formats are appended; existing indices never change |
| Row/column insert or delete, sheet rename or delete | Replayed over formulas (all sheets), defined names, conditional formatting and data-validation ranges and formulas, hyperlinks, autofilters, tables, chart series references, comment anchors, image anchors |
| Formulas | Shared-formula groups stay shared unless a structural edit forces them apart. When cells change, `fullCalcOnLoad` is set so Excel recalculates on open, and a stale `calcChain.xml` is dropped (Excel rebuilds it) |

Checked by `crates/waffle-io/tests/roundtrip.rs`:

- Unedited saves of the test workbooks are part-for-part identical.
- Editing one cell changes only that sheet.
- Structural edits update the chart, table and names.

## CSV / TSV / TXT

- Rows you didn't change are copied byte-for-byte from the original file.
- Changed rows keep the original delimiter, quoting style, encoding (UTF-8 / UTF-16 / Windows-1252), BOM and line endings.
- Values are never reinterpreted: `00123`, `1.50`, `1e5` and long IDs stay text. A field becomes a number only when printing the number gives back the same text.
- A file without a trailing newline stays that way.
- There's no row limit. A CSV with more than 1,048,576 rows can't be saved as xlsx, because Excel can't hold it. Waffle says so instead of writing a broken file.

## xls / xlsb / ods

These are import-only. No writer can save them back faithfully, so they open as untitled workbooks and **Save** writes a new `.xlsx`. The original file is never modified. Formulas come in as their values.

## Known gaps

- **Pivot caches:** their source ranges aren't shifted by structural edits. Excel refreshes them.
- **Rich-text cells:** they keep their formatting until edited; an edited cell becomes plain text.
- **Charts:** they're preserved but drawn as a placeholder card in the app.
