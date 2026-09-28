# Waffle test corpus

Test fixtures for opening (csv/tsv/txt/xlsx/xlsm/xlsb/xls/ods) and for lossless
xlsx/csv save. Everything in `files/` is produced by `tools/corpus/generate.py` and is
**byte-for-byte deterministic** for the pinned library versions in
`tools/corpus/requirements.txt`: RNGs use fixed seeds, zip entries get fixed timestamps
(1980-01-01), and docProps dates are pinned to 2024-01-01T00:00:00Z. So you can
compare hashes of a no-op round trip against these files.

## Regenerate

```sh
VENV=.venv-corpus   # any path works
python3 -m venv "$VENV" && "$VENV/bin/pip" install -r tools/corpus/requirements.txt
"$VENV/bin/python" tools/corpus/generate.py        # all files (~100 s, ~0.8 GB peak RSS)
"$VENV/bin/python" tools/corpus/generate.py --skip-large   # small files only (<1 s)
"$VENV/bin/python" tools/corpus/validate.py        # re-opens every file and checks the facts below
```

The other files are small (<20 KB each) and can be committed.

`tools/corpus/vendor/` holds third-party fixtures copied verbatim from the
[calamine](https://github.com/tafia/calamine) test suite (MIT, commit
`0af05f4`, see `tools/corpus/vendor/SOURCE.txt`). They supply the `.xlsb` files and the
real `vbaProject.bin`.

Unless noted otherwise, cell addresses are A1-style and "display" means the
en-US formatted string.

## Files

| File | Size | Exercises |
|---|---|---|
| `styled.xlsx` | 6.9 KB | Fonts, colours, fills, borders, alignment, number formats, merges, freeze panes, sizes, hidden row and column (openpyxl) |
| `formulas.xlsx` | 6.6 KB | Cached formula values, cross-sheet and absolute references, whole-column ranges, a defined name, **real shared formulas**, array formulas, and str/bool/error results |
| `rich.xlsx` | 14.4 KB | The "must survive untouched" file: 4 sheets (1 hidden), chart, table, data validation, conditional formatting, hyperlinks, a comment (VML), rich text, an image, and an autofilter with filtered-out rows |
| `macro.xlsm` | 9.8 KB | xlsxwriter xlsm with a **real** `vbaProject.bin` (module `testVBA`) |
| `macro_calamine.xlsm` | 12.7 KB | Third-party xlsm, copied verbatim from calamine `tests/vba.xlsm` |
| `legacy.xls` | 5.6 KB | BIFF8 (xlwt): 3 sheets, numbers, text, dates, a formula |
| `sample.ods` | 1.8 KB | ODS (odfpy): 2 sheets, float, string, date, boolean, percentage, formula |
| `sample.xlsb` | 19.2 KB | Real xlsb (calamine `tests/issues.xlsb`) |
| `sheet_states.xlsb` | 14.8 KB | Real xlsb (calamine `tests/any_sheets.xlsb`): visible, hidden, veryHidden and chart sheets |
| `comma.csv` | 169 B | Basic UTF-8 file with LF line endings |
| `semicolon.csv` | 104 B | `;` delimiter with European decimals (`1,5`, `1.234,56`) |
| `tabs.tsv` / `tabs.txt` | 100 B | Tab-delimited (identical bytes). A field contains a comma |
| `quoted.csv` | 261 B | RFC-4180 quoting edge cases |
| `bom_crlf.csv` | 89 B | UTF-8 BOM and CRLF line endings |
| `latin1.csv` | 80 B | Windows-1252 bytes, CRLF |
| `leading_zeros.csv` | 370 B | Values that must stay text, mixed date formats, INR amounts, blank lines, ragged rows |
| `no_trailing_newline.csv` | 17 B | Last line has no line terminator |

All files above are small (<20 KB each) and committed. They regenerate in under a second.

## Large files (not in git, generate on demand)

The performance benchmarks use big files that are **not committed**, because they total about 220 MB. Generate them locally when you need them:

```sh
make corpus-large      # ~100 s, ~0.8 GB peak memory; writes testdata/files/large_*
```

| File | Size | What it's for |
|---|---|---|
| `large_10k_x_1k.xlsx` | 61.8 MB | 10,000 × 1,000 with a **sharedStrings** table (hand-streamed, the way Excel writes it) |
| `large_10k_x_1k_inline.xlsx` | 66.9 MB | Same data from xlsxwriter `constant_memory` (strings as `t="inlineStr"`) |
| `large_10k_x_1k.csv` | 62.0 MB | Same data as CSV |
| `large_200k_x_20.csv` | 28.8 MB | Tall file: 200,000 lines × 20 columns |

The tests never need these files. They're only for `make probe` and manual performance checks.

## Key facts

### styled.xlsx
- Sheets: `Styled`. Used range `A1:H38`. Freeze panes at `B2` (top row and column A frozen).
- Row 1 is the header (`Feature`, `Sample`, `Expected display`, `Merges`): bold white text on fill `FF305496`, height 30.
- Column A holds a label, column B the styled sample, and column C the **expected en-US display string** of B as text. Tests can loop over rows 20–36 and compare the rendered B to C.
- Fonts: B2 bold, B3 italic, B4 underline single, B5 strike. B6 is Georgia 18 and B7 Courier New 8. B8 colour rgb `FFFF0000`. B9 colour theme 4 with tint -0.25. B10 colour indexed 12.
- B11 solid fill `FFFFFF00`. B12/B13/B14 have thin/medium/thick borders on all sides. B15 has dashed borders coloured `FFFF0000`.
- B16 is right/top aligned (row height 40). B17 is center/center. B18 wraps text (row height 45). B19 is left-aligned with indent 2.
- Number formats (row: value → format → display):
  - 20: 1234.5 → `0.00` → `1234.50`
  - 21: 1234567.891 → `#,##0.00` → `1,234,567.89`
  - 22: 0.256 → `0%` → `26%`
  - 23: 0.256 → `0.00%` → `25.60%`
  - 24: 2024-03-05 (serial 45356) → `m/d/yyyy` (custom, not builtin 14) → `3/5/2024`
  - 25: 2024-03-05 → `dd-mmm-yyyy` → `05-Mar-2024`
  - 26: 2024-03-05 14:30 → `yyyy-mm-dd hh:mm` → `2024-03-05 14:30`
  - 27: 14:30:15 → `h:mm:ss AM/PM` → `2:30:15 PM`
  - 28: 09:05 → `hh:mm` → `09:05`
  - 29: 1234.5 → `"$"#,##0.00` → `$1,234.50`
  - 30: 123456.78 → `[$₹-4009] #,##0.00` → `₹ 123,456.78`
  - 31: -1234.5 → accounting `_("$"* #,##0.00_);_("$"* (#,##0.00);_("$"* "-"??_);_(@_)` → `$` padded left, then `(1,234.50)`
  - 32: -42.5 → `#,##0.00;[Red]-#,##0.00` → `-42.50` in red
  - 33: 0 → `#,##0.00;[Red](#,##0.00);"zero"` → `zero`
  - 34: text `"00123"` → `@` → `00123`
  - 35: 123456 → `0.000E+00` → `1.235E+05`
  - 36: 0.30000000000000004 → `General` → `0.3`
- Merges: `D2:F2` ("Merged across D2:F2"), `D4:D7` (multi-row), and `D9:F11` (block).
- Row 37 is hidden (A37=`hidden row`, B37=37). Column G is hidden (G1=`hidden column`, G2=7). H1=`after hidden column`.
- Column widths: A=22, B=26, C=24, D=18, E=10, F=10, H=20.

### formulas.xlsx
- Sheets: `Calc`, `Other Sheet`. Defined name `TaxRate` = `'Other Sheet'!$B$1` (0.18).
- `Other Sheet`: A1=100, B1=0.18, A2=`text on other sheet`, B2=7.
- `Calc`: A2:A11 = 1..10. F1=3.
- **Shared formula si=0**: B2 is the master `<f t="shared" ref="B2:B11" si="0">A2*2</f>`, and B3..B11 are children `<f t="shared" si="0"/>`. The logical formula of Bn is `=An*2`, with cached values 2,4,…,20.
- **Shared si=1 (with an absolute ref)**: C2:C11. The master is `A2*$F$1`, so C11 is `=A11*$F$1`. Cached values 3..30.
- **Shared si=2 (horizontal)**: B13:C13. The master is `SUM(B2:B11)`, so C13 is `=SUM(C2:C11)`. Cached values 110 and 165.
- D2:D11 `=IF(An>5,"big","small")` (t="str"): D2=`small` … D7=`big`.
- E2 `='Other Sheet'!A1` → 100. E3 `='Other Sheet'!B2*2` → 14. E4 `=Calc!A2+'Other Sheet'!$A$1` → 101.
- H2 `=SUM(A:A)` → 55. H3 `=SUM(B:B)` → 220. H4 `=TaxRate*100` → 18. H5 `=SUM($A$2:$A$11)` → 55. H6 `=A2/0` → `#DIV/0!` (t="e"). H7 `=A2>0` → TRUE (t="b").
- Array formulas: `J2:J4 {=A2:A4*B2:B4}` gives 2, 8, 18 (the formula is stored only on J2). `J6 {=SUM(A2:A11*B2:B11)}` gives 770.
- The sheet XML has exactly 22 `t="shared"` attributes.
- Note: xlsxwriter sets `fullCalcOnLoad="1"` in `calcPr`.

### rich.xlsx
- Sheets in order: `Summary`, `Data`, `Hidden Calc` (state=hidden), `Notes`.
- Defined name `RegionList` = `'Hidden Calc'!$A$1:$A$4` (North, South, East, West). `Hidden Calc`!C1=`secret`.
- `Summary`!A1 is rich text with 3 runs: `Bold red` (bold, #C00000), ` plain `, and `italic blue` (italic, #0070C0). Plain text: `Bold red plain italic blue`.
- `Summary`!A10 is rich text with 5 runs. Plain text: `Mixed: bold and blue italic end`.
- `Summary`!B3=`North` has list validation `=RegionList`. It also has a comment by `Xcell QA`: "Pick a region from the list". B4=`M` has list validation `"S,M,L,XL"`.
- Hyperlinks: A6 `External link` → `https://example.com/xcell`. A7 `Go to Data` → internal `'Data'!A1`. A8 `Mail QA` → `mailto:qa@example.com`.
- A column chart anchored at `Summary`!D2 ("Units by product"). It uses categories `Data!$B$2:$B$11` and values `Data!$C$2:$C$11`.
- `Data`: table `SalesTable` on `A1:E21` (style Medium 2). Headers: Region, Product, Units, Price, Revenue.
  - Row 2 = North, Widget, 42, 47.45, 1992.9. Row 3 = South, Gadget, 51, 32.9, 1677.9.
  - ΣUnits = 832. ΣRevenue ≈ 19098.27.
- Conditional formatting on `Data`: C2:C21 cell>50 (red fill), E2:E21 3-colour scale, D2:D21 data bar.
- `Notes`: a 24×16 PNG at A2 (`xl/media/image1.png`).
  - Autofilter `A10:C20` (headers Item, Qty, Status) filters Status == `open`.
  - Rows 12, 14, 16, 18, 20 (the `closed` items) are hidden.
- Parts include `xl/charts/chart1.xml`, `xl/drawings/drawing{1,2}.xml`, `xl/drawings/vmlDrawing1.vml`, `xl/comments1.xml`, `xl/tables/table1.xml`, and `xl/media/image1.png`.

### macro.xlsm / macro_calamine.xlsm
- Both have content type `application/vnd.ms-excel.sheet.macroEnabled.main+xml` and `xl/vbaProject.bin`.
- The VBA project has module `testVBA` with code `Public Sub test()  MsgBox "Hello from vba!"`, plus references `stdole` and `Office`.
- `macro.xlsm`: sheet `Macro Sheet`. A1 is text, A2=42, A3 `=A2*2` (cached 84).
- `macro_calamine.xlsm`: sheets `Sheet1`, `Sheet2`, `Sheet3` (from the calamine suite). The file is byte-identical to upstream.

### legacy.xls
- Sheets: `Numbers`, `Text`, `Dates`.
- `Numbers`: header row `n, half, money`. Rows 2–11 hold n=1..10, n/2, and n*1234.5 (format `#,##0.00`, so C11=12345 displays `12,345.00`). A12 `=SUM(A2:A11)`. xlwt stores no cached value, so readers see 0 or need to recalculate.
- `Text`: A1 `Greeting` (bold). A2..A6 = `Hello`, `Café`, `naïve`, `日本語`, `00123`.
- `Dates`: A2 2024-03-05, A3 1999-12-31 (`yyyy-mm-dd`). A4 2024-03-05 14:30 (`yyyy-mm-dd hh:mm`).

### sample.ods
- Sheets: `Sheet One`, `Second`.
- `Sheet One`: header `name, qty, price, date`. Rows: apple/3/1.25/2024-03-05, banana/12/0.5/2024-03-06, cherry/7/3.75/2023-12-31. A5=`total`, B5 = `of:=SUM([.B2:.B4])` (cached 22).
- `Second`: A1 `Unicode ✓ ₹ é`, B1 boolean true, C1 percentage 0.5 (`50%`).

### sample.xlsb (calamine issues.xlsb)
- Sheets: `datatypes`, `issue2`, `Sheet1`, `issue5`, `issue6`, `spc_chrs`.
- `datatypes`!A1:A6 = 1, 1.5, `ab`, FALSE, `test`, 42663 (a date serial, 2016-10-20).
- `issue2`!A1:B3 = [[1,`a`],[2,`b`],[3,`c`]].
- `Sheet1`!A1 has formula `B1+OneRange`.
- `spc_chrs`!A1:A8 = `&`, `<`, `>`, `aaa ' aaa`, `"`, `☺`, `֍`, `àâéêèçöïî«»`.
- Defined names: `MyBrokenRange`=`Sheet1!#REF!`, `MyDataTypes`=`datatypes!$A$1:$A$6`, `OneRange`=`Sheet1!$A$1`.

### sheet_states.xlsb (calamine any_sheets.xlsb)
- Sheets: `Visible`, `Hidden` (hidden), `VeryHidden` (veryHidden), `Chart` (chartsheet).
- `Visible`!A1:B3 = 1,2 / 3,4 / 5,6. A5 = `This workbook contains 4 sheets: Visible, Hidden, VeryHidden and Chart`.

### CSV variants (exact bytes are in the generator)
- `comma.csv`: 6 lines incl. header `id,name,amount,date,active`, LF.
  - Row 3 name is `Chloé`. Row 4 amount is `1e3`.
  - Row 5 amount is empty.
  - Last byte is `\n`.
- `semicolon.csv`: header `id;name;amount;ratio`.
  - Rows: `1;Anna;1.234,56;1,5` and `4;"Müller; Hans";3,14;0,5` (a quoted `;`).
- `tabs.tsv` = `tabs.txt`: header `id name city score` (tab-separated).
  - B2 = `Smith, John`.
  - Row 4 has empty B and D.
  - D5 = `75.25`, C5 = `北京`.
- `quoted.csv`: 8 records (header + 7). 3 columns everywhere.
  - Record 2 text `Hello, world`. Record 3 `She said "hi"`.
  - Record 4 `line one\nline two`.
  - Record 5 empty string. Record 6 `  spaces  `.
  - Record 8 contains an embedded `\r\n`.
- `bom_crlf.csv`: starts `EF BB BF`, CRLF line endings. Header `name,city,amount`.
  - Rows: `Zoë,Zürich,12.5`, `山田,東京,1000`, `Ravi,मुंबई,₹500`.
- `latin1.csv`: cp1252 (**not** valid UTF-8), CRLF. Decoded rows:
  - `Café,£3.50,résumé`
  - `Jürgen,€12,naïve` (`€` = byte 0x80, which ISO-8859-1 does not define)
  - `Ñoño,¥100,“smart quotes”` (0x93/0x94)
- `leading_zeros.csv`: header `id,phone,price_text,sci_text,date,amount_inr,zip`.
  - Values that must survive as text: `00123`, `00007`, `000`, `0001`, `+91 98765 43210`, `09876543210`, `1.50`, `1e5`, `1E-3`, `2e10`, `1.5e+3`, `01234`, `00501`, `07030`, and `₹1,23,456.78` (quoted, Indian grouping).
  - Dates: `2024-03-05`, `05/03/2024`, `3/5/2024`, `05-Mar-2024`, `"March 5, 2024"`, `20240305`.
  - Lines 5 and 9 are empty. Line 7 `short,row` has 2 fields. Line 8 has 9 fields. The file has 10 lines, LF.
- `no_trailing_newline.csv`: `a,b,c\n1,2,3\n4,5,6` (17 bytes, no final newline).

### Large files (seeded; the xlsx and csv files hold identical data)
- `large_10k_x_1k.{xlsx,csv}` and `large_10k_x_1k_inline.xlsx`: sheet `Data`, dimension `A1:ALL10000`.
  - That is 10,000 rows in total (1 header + 9,999 data rows) × 1,000 columns.
  - Header cells are `col_1` … `col_1000`. In both xlsx files, A1:E1 are styled bold white on `FF305496`. Row 1 is frozen.
  - Data is 70.03% numbers (half integers 0..99999, half 2-decimal floats in [-1000,1000]) and the rest one of 40 short words (`alpha` … `pending`).
  - SST version: 1,040 unique shared strings (1,000 headers + 40 words).
  - Samples: A2=944.08, B2=211.05, C2=`delta`, D2=191.87, E2=`west`. A10000=34796, B10000=-391.02, C10000=`pi`, ALL10000=`open`. CSV row 5001 col 500 (1-based) = `zeta`.
  - Numbers in the CSV are Python `repr` (shortest round-trip), LF endings, no quoting needed.
- `large_200k_x_20.csv`: 200,000 lines (header + 199,999). Header `id,date,region,product,qty,price,amount,flag,code,note,m1..m10`.
  - Line 2 = `1,2023-05-12,South,Gadget,207,381.06,78879.42,TRUE,99598,zeta,-0.711,…`.
  - Last line starts `199999,2023-12-13,West,Gizmo,311,610.2,189772.2,TRUE,91938,yes`.
  - `code` is a zero-padded 5-digit string.

## Could not produce / caveats
- The macro project in `macro.xlsm` was not authored here. It is calamine's real `vbaProject.bin`, re-embedded via xlsxwriter. Its codenames (ThisWorkbook/Sheet1..3) may not match the xlsxwriter sheet codename, but Excel tolerates this.
- There is no xlsb writer in Python, so the xlsb files are third-party fixtures and not generated.
- `legacy.xls` formulas have no cached values (an xlwt limitation).
