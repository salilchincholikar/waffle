//! Integration tests against the corpus in ../testdata/files.

use std::io::Read;
use std::path::{Path, PathBuf};

use waffle_core::ops;
use waffle_core::sheet::Rect;
use waffle_core::view;
use waffle_io::doc::Doc;
use waffle_refs::Axis;

fn corpus(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/files").join(name)
}

fn tmp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("waffle-tests-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

fn open(path: &Path) -> Doc {
    let mut d = Doc::open(path).expect("open");
    d.wait();
    assert!(d.take_error().is_none());
    d
}

fn cell(d: &Doc, sheet: &str, a1: &str) -> String {
    let mut wb = d.wb.lock().unwrap();
    let si = wb.sheet_index(sheet).unwrap();
    let (r, c) = waffle_refs::parse_cell_ref(a1).unwrap();
    let mut s = String::new();
    view::display(&mut wb, si, r, c, &mut s);
    s
}

fn formula(d: &Doc, sheet: &str, a1: &str) -> Option<String> {
    let wb = d.wb.lock().unwrap();
    let si = wb.sheet_index(sheet).unwrap();
    let (r, c) = waffle_refs::parse_cell_ref(a1).unwrap();
    wb.sheets[si].formula_text(r, c)
}

fn zip_parts(p: &Path) -> Vec<(String, Vec<u8>)> {
    let mut z = zip::ZipArchive::new(std::fs::File::open(p).unwrap()).unwrap();
    (0..z.len())
        .map(|i| {
            let mut f = z.by_index(i).unwrap();
            let mut v = Vec::new();
            f.read_to_end(&mut v).unwrap();
            (f.name().to_string(), v)
        })
        .collect()
}

fn changed_parts(a: &Path, b: &Path) -> Vec<String> {
    let pa = zip_parts(a);
    let pb = zip_parts(b);
    let mut out: Vec<String> =
        pb.iter().filter(|(n, d)| pa.iter().find(|(m, _)| m == n).is_none_or(|(_, e)| e != d)).map(|(n, _)| n.clone()).collect();
    out.extend(pa.iter().filter(|(n, _)| !pb.iter().any(|(m, _)| m == n)).map(|(n, _)| format!("-{n}")));
    out
}

fn part(p: &Path, name: &str) -> String {
    String::from_utf8(zip_parts(p).into_iter().find(|(n, _)| n == name).unwrap().1).unwrap()
}

#[test]
fn styled_display_matches_expected_column() {
    let d = open(&corpus("styled.xlsx"));
    for row in 20..=36 {
        let expected = cell(&d, "Styled", &format!("C{row}"));
        let got = cell(&d, "Styled", &format!("B{row}"));
        if row == 31 {
            // Accounting format pads with repeat characters; compare the visible parts.
            assert!(got.contains("(1,234.50)") && got.contains('$'), "row {row}: {got:?}");
            continue;
        }
        let expected = expected.trim().trim_end_matches(" (red)");
        assert_eq!(got.trim(), expected, "row {row}");
    }
    let wb = d.wb.lock().unwrap();
    let s = &wb.sheets[0];
    assert_eq!((s.grid.freeze_rows, s.grid.freeze_cols), (1, 1));
    assert_eq!(s.grid.merges.len(), 3);
    assert!(s.is_row_hidden(36));
    assert!(s.is_col_hidden(6));
    let xf = s.style(1, 1);
    assert!(wb.styles.font(wb.styles.xf(xf).font).bold);
}

#[test]
fn shared_formulas_resolve() {
    let d = open(&corpus("formulas.xlsx"));
    assert_eq!(formula(&d, "Calc", "B2").as_deref(), Some("A2*2"));
    assert_eq!(formula(&d, "Calc", "B11").as_deref(), Some("A11*2"));
    assert_eq!(formula(&d, "Calc", "C11").as_deref(), Some("A11*$F$1"));
    assert_eq!(formula(&d, "Calc", "C13").as_deref(), Some("SUM(C2:C11)"));
    assert_eq!(cell(&d, "Calc", "B11"), "20");
    assert_eq!(cell(&d, "Calc", "D7"), "big");
    assert_eq!(cell(&d, "Calc", "H6"), "#DIV/0!");
    assert_eq!(cell(&d, "Calc", "H7"), "TRUE");
}

#[test]
fn unedited_saves_are_identical() {
    for f in ["styled.xlsx", "formulas.xlsx", "rich.xlsx", "macro.xlsm"] {
        let d = open(&corpus(f));
        let out = tmp(&format!("same-{f}"));
        d.save(&out, false).unwrap();
        assert!(changed_parts(&corpus(f), &out).is_empty(), "{f}: {:?}", changed_parts(&corpus(f), &out));
    }
    for f in [
        "comma.csv",
        "quoted.csv",
        "bom_crlf.csv",
        "latin1.csv",
        "leading_zeros.csv",
        "no_trailing_newline.csv",
        "semicolon.csv",
        "tabs.tsv",
    ] {
        let d = open(&corpus(f));
        let out = tmp(&format!("same-{f}"));
        d.save(&out, true).unwrap();
        assert_eq!(std::fs::read(corpus(f)).unwrap(), std::fs::read(&out).unwrap(), "{f}");
    }
}

#[test]
fn edit_one_cell_changes_only_that_sheet() {
    let d = open(&corpus("styled.xlsx"));
    {
        let mut wb = d.wb.lock().unwrap();
        ops::set_input(&mut wb, 0, 19, 1, "99").unwrap();
        ops::set_input(&mut wb, 0, 40, 0, "brand new text").unwrap();
    }
    let out = tmp("edit-styled.xlsx");
    d.save(&out, false).unwrap();
    let mut changed = changed_parts(&corpus("styled.xlsx"), &out);
    changed.sort();
    // styled.xlsx stores text inline (no shared-string table), so only the sheet changes.
    assert_eq!(changed, vec!["xl/worksheets/sheet1.xml"]);
    let d2 = open(&out);
    assert_eq!(cell(&d2, "Styled", "B20"), "99.00");
    assert_eq!(cell(&d2, "Styled", "A41"), "brand new text");
    assert_eq!(cell(&d2, "Styled", "B21"), "1,234,567.89");
    let wb = d2.wb.lock().unwrap();
    assert_eq!(wb.sheets[0].grid.merges.len(), 3);
    assert_eq!((wb.sheets[0].grid.freeze_rows, wb.sheets[0].grid.freeze_cols), (1, 1));
}

#[test]
fn insert_rows_shifts_formulas_and_names() {
    let d = open(&corpus("formulas.xlsx"));
    {
        let mut wb = d.wb.lock().unwrap();
        let si = wb.sheet_index("Calc").unwrap();
        ops::insert_rows(&mut wb, si, 4, 2).unwrap();
    }
    assert_eq!(formula(&d, "Calc", "B13").as_deref(), Some("A13*2"));
    assert_eq!(formula(&d, "Calc", "C15").as_deref(), Some("SUM(C2:C13)"));
    assert_eq!(formula(&d, "Calc", "H7").as_deref(), Some("SUM($A$2:$A$13)"));
    assert_eq!(formula(&d, "Calc", "H2").as_deref(), Some("SUM(A:A)"));
    let out = tmp("insert-formulas.xlsx");
    d.save(&out, false).unwrap();
    let d2 = open(&out);
    assert_eq!(formula(&d2, "Calc", "B13").as_deref(), Some("A13*2"));
    assert_eq!(cell(&d2, "Calc", "A13"), "10");
    let wbx = part(&out, "xl/workbook.xml");
    assert!(wbx.contains("fullCalcOnLoad=\"1\""));
    assert!(wbx.contains("'Other Sheet'!$B$1"));
    // Undo restores the original layout.
    let mut wb = d.wb.lock().unwrap();
    wb.undo().unwrap();
    assert_eq!(wb.sheets[0].formula_text(10, 1).as_deref(), Some("A11*2"));
    assert!(wb.log.is_empty());
}

#[test]
fn rich_file_structural_edits_fix_up_other_parts() {
    let d = open(&corpus("rich.xlsx"));
    {
        let mut wb = d.wb.lock().unwrap();
        let si = wb.sheet_index("Data").unwrap();
        ops::insert_rows(&mut wb, si, 0, 2).unwrap();
        ops::rename_sheet(&mut wb, si, "Sales Data").unwrap();
    }
    let out = tmp("rich-edit.xlsx");
    d.save(&out, false).unwrap();
    let parts = zip_parts(&out);
    let chart = parts.iter().find(|(n, _)| n.starts_with("xl/charts/chart")).map(|(_, v)| String::from_utf8_lossy(v).into_owned()).unwrap();
    assert!(chart.contains("'Sales Data'!$B$4:$B$13"), "{chart}");
    let table = parts.iter().find(|(n, _)| n.starts_with("xl/tables/")).map(|(_, v)| String::from_utf8_lossy(v).into_owned()).unwrap();
    assert!(table.contains("ref=\"A3:E23\""), "{table}");
    let wbx = part(&out, "xl/workbook.xml");
    assert!(wbx.contains("name=\"Sales Data\""), "{wbx}");
    // The untouched image survives byte-for-byte.
    let img_a = zip_parts(&corpus("rich.xlsx")).into_iter().find(|(n, _)| n.ends_with(".png")).unwrap();
    let img_b = parts.iter().find(|(n, _)| n.ends_with(".png")).unwrap();
    assert_eq!(img_a.1, img_b.1);
    let d2 = open(&out);
    assert_eq!(cell(&d2, "Sales Data", "A4"), "North");
    assert_eq!(cell(&d2, "Summary", "A1"), "Bold red plain italic blue");
}

#[test]
fn csv_edit_changes_one_line() {
    let d = open(&corpus("leading_zeros.csv"));
    assert_eq!(cell(&d, "leading_zeros", "A2"), "00123");
    {
        let mut wb = d.wb.lock().unwrap();
        ops::set_input(&mut wb, 0, 1, 1, "edited").unwrap();
    }
    let out = tmp("lz.csv");
    d.save(&out, true).unwrap();
    let a = std::fs::read_to_string(corpus("leading_zeros.csv")).unwrap();
    let b = std::fs::read_to_string(&out).unwrap();
    let diff: Vec<(usize, &str, &str)> =
        a.lines().zip(b.lines()).enumerate().filter(|(_, (x, y))| x != y).map(|(i, (x, y))| (i, x, y)).collect();
    assert_eq!(diff.len(), 1, "{diff:?}");
    assert_eq!(diff[0].0, 1);
    assert_eq!(a.lines().count(), b.lines().count());
}

#[test]
fn undo_redo_and_sort() {
    let d = open(&corpus("comma.csv"));
    let mut wb = d.wb.lock().unwrap();
    let before: Vec<String> = (0..wb.sheets[0].row_count())
        .map(|r| {
            let mut s = String::new();
            view::display(&mut wb, 0, r, 0, &mut s);
            s
        })
        .collect();
    let rows = wb.sheets[0].row_count();
    ops::sort_rows(&mut wb, 0, 1, rows - 1, &[ops::SortKey { col: 0, ascending: false }]).unwrap();
    wb.undo();
    let after: Vec<String> = (0..wb.sheets[0].row_count())
        .map(|r| {
            let mut s = String::new();
            view::display(&mut wb, 0, r, 0, &mut s);
            s
        })
        .collect();
    assert_eq!(before, after);
    wb.redo();
    ops::set_hidden(&mut wb, 0, Axis::Rows, 1, 1, true).unwrap();
    assert!(wb.sheets[0].is_row_hidden(1));
    wb.undo();
    assert!(!wb.sheets[0].is_row_hidden(1));
}

#[test]
fn cleanup_tools() {
    let d = open(&corpus("leading_zeros.csv"));
    let mut wb = d.wb.lock().unwrap();
    let rect = Rect { r0: 0, c0: 0, r1: 1000, c1: 100 };
    let n = ops::remove_empty_rows(&mut wb, 0, 0, 1000).unwrap();
    assert!(n > 0);
    let _ = ops::transform(&mut wb, 0, &[rect], ops::TextTransform::Trim).unwrap();
    assert_eq!(ops::parse_amount("₹1,23,456.78", false), Some(123456.78));
    assert_eq!(ops::parse_amount("(1,200)", false), Some(-1200.0));
    assert_eq!(ops::parse_amount("1.200,50", true), Some(1200.5));
    assert_eq!(ops::parse_amount("500 DR", false), Some(-500.0));
    assert!(ops::parse_any_date("05/03/2024", ops::DateOrder::Dmy, false).is_some());
}

#[test]
fn legacy_formats_open() {
    for f in ["legacy.xls", "sample.ods", "sample.xlsb", "sheet_states.xlsb"] {
        let d = open(&corpus(f));
        let wb = d.wb.lock().unwrap();
        assert!(!wb.sheets.is_empty(), "{f}");
        assert!(wb.sheets.iter().any(|s| s.row_count() > 0), "{f}");
        drop(wb);
        let out = tmp(&format!("{f}.xlsx"));
        d.save(&out, false).unwrap();
        let d2 = open(&out);
        assert!(d2.wb.lock().unwrap().sheets.iter().any(|s| s.row_count() > 0), "{f} resave");
    }
}

#[test]
fn fill_series() {
    let d = Doc::new_empty();
    let mut wb = d.wb.lock().unwrap();
    ops::set_input(&mut wb, 0, 0, 0, "1").unwrap();
    ops::set_input(&mut wb, 0, 1, 0, "3").unwrap();
    ops::set_input(&mut wb, 0, 0, 1, "Item 9").unwrap();
    ops::set_input(&mut wb, 0, 0, 2, "Mar").unwrap();
    ops::set_input(&mut wb, 0, 0, 3, "2024-01-31").unwrap();
    ops::set_input(&mut wb, 0, 0, 4, "7").unwrap();
    ops::fill_from(&mut wb, 0, Rect { r0: 0, c0: 0, r1: 1, c1: 0 }, Rect { r0: 2, c0: 0, r1: 4, c1: 0 }).unwrap();
    ops::fill_from(&mut wb, 0, Rect { r0: 0, c0: 1, r1: 0, c1: 4 }, Rect { r0: 1, c0: 1, r1: 3, c1: 4 }).unwrap();
    let t = |wb: &mut waffle_core::workbook::Workbook, r, c| {
        let mut s = String::new();
        view::display(wb, 0, r, c, &mut s);
        s
    };
    assert_eq!(t(&mut wb, 4, 0), "9");
    assert_eq!(t(&mut wb, 2, 1), "Item 11");
    assert_eq!(t(&mut wb, 1, 2), "Apr");
    assert_eq!(t(&mut wb, 3, 2), "Jun");
    assert_eq!(t(&mut wb, 1, 3), "2024-02-01");
    assert_eq!(t(&mut wb, 3, 4), "7", "single number copies");
    wb.undo();
    assert_eq!(t(&mut wb, 1, 2), "");
}

fn all_formula_cells(wb: &waffle_core::workbook::Workbook) -> Vec<(usize, u32, u32)> {
    let mut v = Vec::new();
    for (si, s) in wb.sheets.iter().enumerate() {
        for r in 0..s.row_count() {
            for c in 0..s.col_count() {
                if s.formula_id(r, c).is_some() {
                    v.push((si, r, c));
                }
            }
        }
    }
    v
}

#[test]
fn full_recalc_matches_excel_cached_values() {
    for f in ["formulas.xlsx", "rich.xlsx", "macro.xlsm"] {
        let d = open(&corpus(f));
        let mut wb = d.wb.lock().unwrap();
        let cells = all_formula_cells(&wb);
        let mut before = Vec::new();
        for &(si, r, c) in &cells {
            let mut s = String::new();
            view::display(&mut wb, si, r, c, &mut s);
            before.push(s);
        }
        waffle_core::recalc::run(&mut wb, &[], true);
        for (i, &(si, r, c)) in cells.iter().enumerate() {
            let mut s = String::new();
            view::display(&mut wb, si, r, c, &mut s);
            let ft = wb.sheets[si].formula_text(r, c).unwrap_or_default();
            // Formulas Excel left uncalculated have no cached value to compare.
            if before[i].is_empty() || before[i].starts_with('=') {
                continue;
            }
            assert_eq!(s, before[i], "{f} sheet {si} {} = {ft}", {
                let mut o = String::new();
                waffle_refs::cell_ref_string(r, c, &mut o);
                o
            });
        }
    }
}

#[test]
fn edits_recalculate_dependents() {
    let d = open(&corpus("formulas.xlsx"));
    let mut wb = d.wb.lock().unwrap();
    let calc = wb.sheet_index("Calc").unwrap();
    let other = wb.sheet_index("Other Sheet").unwrap();
    let show = |wb: &mut waffle_core::workbook::Workbook, si: usize, a1: &str| {
        let (r, c) = waffle_refs::parse_cell_ref(a1).unwrap();
        let mut s = String::new();
        view::display(wb, si, r, c, &mut s);
        s
    };
    ops::set_input(&mut wb, calc, 1, 0, "100").unwrap(); // A2
    assert_eq!(show(&mut wb, calc, "B2"), "200", "shared formula child");
    assert_eq!(show(&mut wb, calc, "B13"), (200 + 4 + 6 + 8 + 10 + 12 + 14 + 16 + 18 + 20).to_string(), "chain through SUM");
    assert_eq!(show(&mut wb, calc, "H2"), (100 + 54).to_string(), "whole column");
    ops::set_input(&mut wb, other, 0, 1, "0.5").unwrap(); // TaxRate
    assert_eq!(show(&mut wb, calc, "H4"), "50", "defined name on another sheet");
    ops::set_input(&mut wb, calc, 20, 0, "=SUM(A2:A11)*2").unwrap();
    assert_eq!(show(&mut wb, calc, "A21"), ((100 + 54) * 2).to_string(), "typed formula");
    ops::set_input(&mut wb, calc, 21, 0, "=A21+1").unwrap();
    ops::set_input(&mut wb, calc, 2, 0, "0").unwrap(); // A3: 2 -> 0
    assert_eq!(show(&mut wb, calc, "A22"), ((152) * 2 + 1).to_string(), "transitive via new formulas");
    wb.undo();
    assert_eq!(show(&mut wb, calc, "A22"), ((154) * 2 + 1).to_string(), "undo restores results");
}

#[test]
#[ignore]
fn bench_recalc_100k_formulas() {
    let d = Doc::new_empty();
    let mut wb = d.wb.lock().unwrap();
    let n = 100_000u32;
    let text: String = (0..n).map(|i| format!("{}\t=A{}*2\n", i, i + 1)).collect();
    let t = std::time::Instant::now();
    ops::paste_text(&mut wb, 0, 0, 0, &text).unwrap();
    eprintln!("paste {n} values+formulas (incl. first calc): {:?}", t.elapsed());
    ops::set_input(&mut wb, 0, 0, 2, "=SUM(B:B)").unwrap();
    let t = std::time::Instant::now();
    ops::set_input(&mut wb, 0, 5, 0, "1000").unwrap();
    eprintln!("edit one input (1 formula + total dirty): {:?}", t.elapsed());
    let mut s = String::new();
    view::display(&mut wb, 0, 0, 2, &mut s);
    eprintln!("total = {s}");
    let t = std::time::Instant::now();
    ops::set_input_range(&mut wb, 0, Rect { r0: 0, c0: 0, r1: n - 1, c1: 0 }, "1").unwrap();
    eprintln!("change all {n} inputs: {:?}", t.elapsed());
}

#[test]
fn undo_returns_to_the_edited_sheet() {
    let d = open(&corpus("formulas.xlsx"));
    let mut wb = d.wb.lock().unwrap();
    let other = wb.sheet_index("Other Sheet").unwrap();
    wb.active = other;
    ops::insert_rows(&mut wb, other, 0, 1).unwrap();
    ops::set_input(&mut wb, other, 5, 5, "x").unwrap();
    wb.active = 0;
    assert_eq!(wb.undo(), Some(other), "cell edit");
    assert_eq!(wb.undo(), Some(other), "structural edit snapshots every sheet but happened on this one");
    assert_eq!(wb.redo(), Some(other));
}

#[test]
fn csv_beyond_excel_row_limit() {
    let rows = 1_100_000u32;
    let src = tmp("tall.csv");
    let mut text = String::with_capacity(rows as usize * 10);
    for i in 0..rows {
        text.push_str(&format!("{i},x\n"));
    }
    std::fs::write(&src, &text).unwrap();
    let d = open(&src);
    {
        let mut wb = d.wb.lock().unwrap();
        let s = &wb.sheets[0];
        assert_eq!(s.row_count(), rows);
        assert!(s.display_rows() > waffle_core::sheet::MAX_ROWS, "grid extends past Excel's limit");
        ops::set_input(&mut wb, 0, 1_050_000, 1, "edited").unwrap();
        let mut v = String::new();
        view::display(&mut wb, 0, 1_050_000, 1, &mut v);
        assert_eq!(v, "edited");
    }
    let out = tmp("tall-out.csv");
    d.save(&out, true).unwrap();
    let saved = std::fs::read_to_string(&out).unwrap();
    assert_eq!(saved.lines().count(), rows as usize, "every row kept");
    let diff: Vec<_> = text.lines().zip(saved.lines()).filter(|(a, b)| a != b).collect();
    assert_eq!(diff, vec![("1050000,x", "1050000,edited")]);
    let err = d.save(&tmp("tall.xlsx"), false).unwrap_err().to_string();
    assert!(err.contains("1,100,000 rows") && err.contains("1,048,576"), "{err}");
    // Workbooks keep Excel's limit.
    let x = open(&corpus("styled.xlsx"));
    assert_eq!(x.wb.lock().unwrap().sheets[0].max_rows, waffle_core::sheet::MAX_ROWS);
}

/// A copy of a corpus file with one part's XML changed by `f`, written to a temp path.
fn patched(name: &str, part_name: &str, out: &str, f: impl Fn(String) -> String) -> PathBuf {
    let src = std::fs::read(corpus(name)).unwrap();
    let mut zin = zip::ZipArchive::new(std::io::Cursor::new(src)).unwrap();
    let mut zout = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for i in 0..zin.len() {
        let mut e = zin.by_index(i).unwrap();
        let name = e.name().to_string();
        let mut data = Vec::new();
        e.read_to_end(&mut data).unwrap();
        if name == part_name {
            data = f(String::from_utf8(data).unwrap()).into_bytes();
        }
        zout.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        std::io::Write::write_all(&mut zout, &data).unwrap();
    }
    let path = tmp(out);
    std::fs::write(&path, zout.finish().unwrap().into_inner()).unwrap();
    path
}

#[test]
fn hidden_gridlines_are_read() {
    // rich.xlsx keeps gridlines; hide them in one sheet's view and read it back.
    // Both forms: <sheetView showGridLines="0" …/> (no children) and one with a child element.
    let d = open(&corpus("rich.xlsx"));
    assert!(d.wb.lock().unwrap().sheets.iter().all(|s| !s.grid.hide_gridlines));
    let with_child = r#"<sheetView showGridLines="0" tabSelected="1" workbookViewId="0"><selection activeCell="A1"/></sheetView>"#;
    let forms = [("<sheetView ", r#"<sheetView showGridLines="0" "#), (r#"<sheetView tabSelected="1" workbookViewId="0"/>"#, with_child)];
    for (i, (from, to)) in forms.into_iter().enumerate() {
        let path = patched("rich.xlsx", "xl/worksheets/sheet1.xml", &format!("hidden-gridlines-{i}.xlsx"), |x| x.replacen(from, to, 1));
        assert!(open(&path).wb.lock().unwrap().sheets[0].grid.hide_gridlines);
    }
}

#[test]
fn saved_autofilter_can_be_cleared() {
    // An Excel filter on A1:J12 that hides rows 3 and 4 (as Excel saves it).
    let path = patched("formulas.xlsx", "xl/worksheets/sheet1.xml", "saved-filter.xlsx", |x| {
        x.replacen(r#"<row r="3" "#, r#"<row r="3" hidden="1" "#, 1)
            .replacen(r#"<row r="4" "#, r#"<row r="4" hidden="1" "#, 1)
            .replacen("</sheetData>", r#"</sheetData><autoFilter ref="A1:J12"><filterColumn colId="0"><filters><filter val="1"/></filters></filterColumn></autoFilter>"#, 1)
    });
    let d = open(&path);
    {
        let wb = d.wb.lock().unwrap();
        let f = wb.sheets[0].grid.auto_filter.clone().expect("autoFilter read");
        assert_eq!((f.range.r0, f.range.r1, f.cols.clone()), (0, 11, vec![0]));
        assert!(wb.sheets[0].is_row_hidden(2) && wb.sheets[0].is_row_hidden(3));
    }
    // The filter menu shows what the saved filter lets through (rows 3-4 hold 2 and 3).
    {
        let mut wb = d.wb.lock().unwrap();
        let vals = ops::distinct_values(&mut wb, 0, 0, 0, 100, None);
        let checked = |v: &str| vals.iter().find(|x| x.0 == v).map(|x| x.2);
        assert_eq!((checked("1"), checked("2"), checked("3"), checked("4")), (Some(true), Some(false), Some(false), Some(true)));
        // Unfiltered columns show everything checked; a column's own rule decides for it.
        assert!(ops::distinct_values(&mut wb, 0, 0, 1, 100, None).iter().all(|x| x.2));
        let rule = ops::FilterRule::Values(["4".to_string()].into_iter().collect());
        let vals = ops::distinct_values(&mut wb, 0, 0, 0, 100, Some(&rule));
        assert!(vals.iter().all(|x| x.2 == (x.0 == "4")));
    }
    // Clearing shows the rows and is one undo step.
    {
        let mut wb = d.wb.lock().unwrap();
        assert_eq!(ops::clear_saved_filter(&mut wb, 0).unwrap(), 2);
        assert!(!wb.sheets[0].is_row_hidden(2) && !wb.sheets[0].is_row_hidden(3));
        wb.undo().unwrap();
        assert!(wb.sheets[0].is_row_hidden(2));
        assert_eq!(wb.sheets[0].grid.auto_filter.as_ref().unwrap().cols, vec![0]);
        ops::clear_saved_filter(&mut wb, 0).unwrap();
    }
    // Saved: rows visible, the criteria gone, the filter range (buttons) kept.
    let out = tmp("saved-filter-cleared.xlsx");
    d.save(&out, false).unwrap();
    let xml = part(&out, "xl/worksheets/sheet1.xml");
    assert!(xml.contains(r#"<autoFilter ref="A1:J12">"#) && !xml.contains("filterColumn"));
    let d2 = open(&out);
    let wb = d2.wb.lock().unwrap();
    assert!(!wb.sheets[0].is_row_hidden(2) && !wb.sheets[0].is_row_hidden(3));
    assert!(wb.sheets[0].grid.auto_filter.as_ref().unwrap().cols.is_empty());
}
