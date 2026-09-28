# Architecture

Waffle is two halves joined by a small C interface:

```
┌──────────────────────────── macos/ (Swift, AppKit) ────────────────────────────┐
│ App/  Document/  Window/  Grid/  Find/  Dialogs/                               │
│   WaffleBridge/Book.swift (library) ── calls ──┐                              │
└───────────────────────────────────────────────┼────────────────────────────────┘
                                                │  C ABI (wf_* functions, waffle.h)
┌──────────────────────────── crates/ (Rust) ───┼────────────────────────────────┐
│ waffle-ffi   ◀────────────────────────────────┘  handles, panics → errors       │
│   ├── waffle-io     xlsx (lossless) · csv · xls/xlsb/ods import · background load│
│   └── waffle-core   sheets · styles · ops · undo · view · recalc                 │
│         ├── waffle-calc    formula parser + evaluator                            │
│         ├── waffle-numfmt  Excel number formats                                  │
│         └── waffle-refs    A1 references: parse / shift / rename                 │
└──────────────────────────────────────────────────────────────────────────────────┘
```

The Rust side owns all data and logic. Swift draws, handles input, and follows macOS conventions (NSDocument, menus, dialogs, standard shortcuts). No data is copied across the boundary except what is on screen.

## Storage (`waffle-core::sheet`)

- **Cells are 8 bytes.** A cell is an `f64`; text, booleans, errors and "empty" live in the negative-NaN space (`cell.rs`).
- **Blocks of 256 rows × 16 columns**, shared through `Arc`. Undo keeps the old `Arc`s; writing to a shared block copies it (copy-on-write), so an undo step costs only the blocks it touched.
- **Logical → physical maps.** Rows and columns reach storage through `row_map`/`col_map`. Inserting, deleting or sorting rows rewrites a `Vec<u32>`; cell data never moves.
- **Per-cell extras are sparse.** Styles (an xf index, 2 bytes) and formulas exist only in blocks that need them. Strings are pooled: the file's shared-string table plus one pool per sheet.
- **Geometry** (row/column positions) is a lazily rebuilt prefix sum, so hit-testing and scrolling are binary searches.

## Editing and undo (`ops`, `workbook`)

Every public function in `ops` is one undo step: `workbook.begin(...)`, mutate, `workbook.commit()`.

- `commit()` recalculates the formulas affected by the cells written during the step. Writes are tracked while a step is open.
- Structural edits (row/column insert/delete, sheet rename/delete) are also appended to the workbook's **change log**. The model is updated immediately. The log is replayed at save time over the parts Waffle doesn't model (charts, tables, names…).
- Undo swaps grids and the log back, so it is instant.

## Recalculation (`recalc`)

1. Collect formula cells, visiting only blocks that contain formulas.
2. Mark dirty formulas: those touched, volatile ones, and anything reading a dirty cell (transitively, following defined names).
3. Evaluate in dependency order against an overlay, so later formulas see earlier results.
4. Write the results.

Formulas using features the engine lacks keep Excel's saved value.

## Loading (`waffle-io::doc`)

`Doc::open` sniffs the format and returns immediately. Sheets are parsed on background threads that push rows into the shared workbook in small batches. The UI polls progress and draws whatever has arrived, so the first rows appear in tens of milliseconds.

## Saving (`waffle-io::xlsx::write`, `csv`)

See [fidelity.md](fidelity.md).

## Drawing (`macos/Sources/Waffle/Grid`)

`GridView` owns an `NSScrollView` whose document view is an empty spacer sized to the content, which gives native scrolling physics. `GridCanvas` sits on top and draws only what is visible: headers, frozen panes, cells, conditional formats, images and the selection.

- **Fetching:** it asks Rust for a rectangle of display-ready cells (`wf_fetch`).
- **Text:** Core Text, with per-style fonts cached and lines cached by text and style.
- **Performance:** a full redraw of a 1200×660 pt window over a 10M-cell sheet takes about 3 ms.

## The window (`macos/Sources/Waffle/Window`, `Find`)

```
● ● ●  Roastery.xlsx  contacts.csv  +        [🔍 Find…  3 of 12 ⌃⌄ Aa ab ƒx ⇄]  ⇅ ⚲ 📌 ✨   ← title bar
A1 │ ƒx =D5*E5                              │ B I U  A▾ 🖍▾  ≡≡≡  ≣  #▾ ▦▾ ◫▾      ← formula row
┌────────────────────────────── sheet (GridView) ──────────────────────────────┐
└──────────────────────────────────────────────────────────────────────────────┘
 +  Sheet1  Sheet2                                               Sum 42   ← sheet tabs / status
```

- **Title bar.** There is no `NSToolbar` (it can't get thinner than about 38 pt). The window has a transparent title bar and Waffle lays its own row into it, next to the traffic lights: file tabs, Find and the sheet-wide tools (`SheetWindowController+TitleBar.swift`).
- **File tabs.** macOS window tabbing always adds a tab-bar row once a window has two tabs, so it is turned off and `WindowTabs` groups document windows itself. A group shows one window, the selected tab; the others stay ordered out at the same frame, and switching swaps them without animation. `DocumentWindow` routes every way a window can be ordered in (opening, Finder, the Window menu, windows macOS restores) through the group. `TitleTabs` draws the tabs.
- **Find & Replace.** `FindCenter` holds one search shared by every window and always covers every open file, in tab order. `FindBar` is the Find field (options as toggles inside it, the count and previous/next while finding) and the Replace bar that drops down under it. Matches update as you type and after every edit, undo or redo.
- **Colours.** Window chrome follows the system appearance. The sheet stays white by default; View ▸ Dark Sheet in Dark Mode switches it to `SheetPalette.dark`, which keeps file colours readable (dark text lightened, white fills dropped).
