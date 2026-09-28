//! Editing operations. Every public function here is one undo step.

use std::collections::HashMap;
use std::sync::Arc;

use crate::cell::{Cell, Kind};
use crate::refshift::{self, Axis, StructOp};
use crate::sheet::{FKind, Formula, MAX_COLS, MAX_ROWS, Rect, RowMeta, Sheet};
use crate::styles::StyleChange;
use crate::view::{self, Input};
use crate::workbook::{BookChange, LogOp, Workbook};

pub type OpResult<T = ()> = Result<T, String>;

fn clamp_rect(s: &Sheet, r: Rect) -> Option<Rect> {
    let rows = s.row_count();
    let cols = s.col_count();
    if rows == 0 || cols == 0 || r.r0 >= rows || r.c0 >= cols {
        return None;
    }
    Some(Rect { r0: r.r0, c0: r.c0, r1: r.r1.min(rows - 1), c1: r.c1.min(cols - 1) })
}

/// Whole-column / whole-row selections reach the sheet limits; for operations that
/// visit every cell, stop at the data (but cover at least `min_rows` × `min_cols`).
fn bound_to_data(s: &Sheet, r: Rect, min_rows: u32, min_cols: u32) -> Rect {
    let mut r = r;
    if r.r1 >= MAX_ROWS - 1 {
        r.r1 = r.r1.min(s.row_count().max(r.r0 + min_rows).saturating_sub(1)).max(r.r0);
    }
    if r.c1 >= MAX_COLS - 1 {
        r.c1 = r.c1.min(s.col_count().max(r.c0 + min_cols).saturating_sub(1)).max(r.c0);
    }
    r
}

fn check_loaded(wb: &Workbook) -> OpResult {
    if wb.loaded() { Ok(()) } else { Err("The file is still loading.".into()) }
}

// ---- cell values ------------------------------------------------------------

/// Put typed text into one cell.
pub fn set_input(wb: &mut Workbook, si: usize, r: u32, c: u32, text: &str) -> OpResult {
    check_loaded(wb)?;
    wb.begin("Typing", &[si]);
    apply_input(wb, si, r, c, text);
    wb.sheets[si].invalidate_geometry();
    wb.commit();
    Ok(())
}

/// Put the same typed text into every cell of a range (Cmd+Enter), translating formulas.
pub fn set_input_range(wb: &mut Workbook, si: usize, rect: Rect, text: &str) -> OpResult {
    check_loaded(wb)?;
    let rect = bound_to_data(&wb.sheets[si], rect, 1, 1);
    wb.begin("Typing", &[si]);
    let formula = text.strip_prefix('=').filter(|_| !wb.is_csv());
    for r in rect.r0..=rect.r1 {
        for c in rect.c0..=rect.c1 {
            match formula {
                Some(f) => {
                    let t = refshift::translate_formula(f, (r - rect.r0) as i32, (c - rect.c0) as i32);
                    apply_input(wb, si, r, c, &format!("={t}"));
                }
                None => apply_input(wb, si, r, c, text),
            }
        }
    }
    wb.commit();
    Ok(())
}

fn apply_input(wb: &mut Workbook, si: usize, r: u32, c: u32, text: &str) {
    let input = view::parse_input(wb, text);
    match input {
        Input::Clear => {
            let s = &mut wb.sheets[si];
            s.set(r, c, Cell::EMPTY);
            s.set_formula(r, c, None);
        }
        Input::Formula(f) => {
            let s = &mut wb.sheets[si];
            let fid = s.add_formula(Formula { text: f.into(), kind: FKind::Normal, attrs: "".into() });
            s.set_formula(r, c, Some(fid));
            // No calculation engine yet: the old cached value would be wrong.
            s.set(r, c, Cell::EMPTY);
        }
        Input::Value(v, fmt) => {
            if let Some(code) = fmt {
                let cur = wb.sheets[si].style(r, c);
                let keep = {
                    let f = wb.styles.numfmt(cur);
                    // Keep an existing specific format of the same family.
                    !f.is_general() && (f.is_date() == code.contains('y') && f.is_percent() == code.contains('%'))
                };
                if !keep {
                    let xf = wb.styles.derive(cur, &StyleChange::NumFmt(code));
                    wb.sheets[si].set_style(r, c, xf);
                }
            }
            let s = &mut wb.sheets[si];
            s.set_formula(r, c, None);
            s.set(r, c, v);
        }
        Input::Text(t) => {
            let s = &mut wb.sheets[si];
            let id = s.strings.add(&t);
            s.set_formula(r, c, None);
            s.set(r, c, Cell::string(id));
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ClearWhat {
    Contents,
    Formats,
    All,
}

pub fn clear(wb: &mut Workbook, si: usize, rects: &[Rect], what: ClearWhat) -> OpResult {
    check_loaded(wb)?;
    wb.begin(if what == ClearWhat::Formats { "Clear Formats" } else { "Clear" }, &[si]);
    let s = &mut wb.sheets[si];
    for &rect in rects {
        let Some(rect) = clamp_rect(s, rect) else { continue };
        for r in rect.r0..=rect.r1 {
            for c in rect.c0..=rect.c1 {
                let Some((pr, pc)) = s.phys(r, c) else { continue };
                if what != ClearWhat::Formats {
                    if !s.get_phys(pr, pc).is_empty() {
                        s.set_phys(pr, pc, Cell::EMPTY);
                    }
                    if s.formula_id_phys(pr, pc).is_some() {
                        s.set_formula_phys(pr, pc, None);
                    }
                }
                if what != ClearWhat::Contents && s.own_style_phys(pr, pc) != 0 {
                    s.set_style_phys(pr, pc, 0);
                }
            }
        }
        s.dirty = true;
    }
    wb.commit();
    Ok(())
}

// ---- formatting -----------------------------------------------------------------

pub fn apply_style(wb: &mut Workbook, si: usize, rects: &[Rect], change: &StyleChange) -> OpResult {
    check_loaded(wb)?;
    if wb.is_csv() {
        return Err("CSV files can't store formatting. Save as .xlsx to keep formatting.".into());
    }
    wb.begin("Format", &[si]);
    let mut cache: HashMap<u16, u16> = HashMap::new();
    for &rect in rects {
        // Whole columns / rows: set the column/row default too, then touch existing cells.
        let full_cols = rect.r0 == 0 && rect.r1 >= MAX_ROWS - 1;
        let full_rows = rect.c0 == 0 && rect.c1 >= MAX_COLS - 1;
        if full_cols && !full_rows {
            for c in rect.c0..=rect.c1.min(wb.sheets[si].display_cols()) {
                let base = wb.sheets[si].col_meta(c).and_then(|m| m.style).unwrap_or(0);
                let xf = *cache.entry(base).or_insert_with(|| wb.styles.derive(base, change));
                wb.sheets[si].col_meta_mut(c).style = Some(xf);
            }
        } else if full_rows && !full_cols {
            for r in rect.r0..=rect.r1.min(wb.sheets[si].display_rows()) {
                let base = wb.sheets[si].row_meta(r).and_then(|m| m.style).unwrap_or(0);
                let xf = *cache.entry(base).or_insert_with(|| wb.styles.derive(base, change));
                wb.sheets[si].row_meta_mut(r).style = Some(xf);
            }
        }
        let bounded = if full_cols || full_rows {
            let s = &wb.sheets[si];
            match clamp_rect(s, rect) {
                Some(r) => r,
                None => continue,
            }
        } else {
            rect
        };
        for r in bounded.r0..=bounded.r1 {
            for c in bounded.c0..=bounded.c1 {
                let base = wb.sheets[si].style(r, c);
                let xf = *cache.entry(base).or_insert_with(|| wb.styles.derive(base, change));
                if xf != base || (full_cols || full_rows) {
                    wb.sheets[si].set_style(r, c, xf);
                }
            }
        }
    }
    wb.sheets[si].invalidate_geometry();
    wb.commit();
    Ok(())
}

// ---- structure --------------------------------------------------------------------

/// Replace shared-formula groups with plain formulas (needed before moving cells).
fn unshare(s: &mut Sheet) {
    if s.shared_masters.is_empty() {
        return;
    }
    let mut todo = Vec::new();
    for (pr_idx, r) in s.grid.row_map.clone().iter().enumerate() {
        let _ = r;
        let r = pr_idx as u32;
        for c in 0..s.col_count() {
            if let Some(fid) = s.formula_id(r, c)
                && matches!(s.formulas[fid as usize].kind, FKind::Shared { .. })
            {
                todo.push((r, c));
            }
        }
    }
    for (r, c) in todo {
        if let Some(text) = s.formula_text(r, c) {
            let fid = s.add_formula(Formula { text: text.into(), kind: FKind::Normal, attrs: "".into() });
            s.set_formula(r, c, Some(fid));
        }
    }
}

/// Every (row, col, fid) holding a formula, by logical position.
fn formula_cells(s: &Sheet) -> Vec<(u32, u32, u32)> {
    let mut out = Vec::new();
    if s.formulas.is_empty() {
        return out;
    }
    for r in 0..s.row_count() {
        for c in 0..s.col_count() {
            if let Some(f) = s.formula_id(r, c) {
                out.push((r, c, f));
            }
        }
    }
    out
}

/// Rewrite every formula in the workbook after `op` on sheet `edited`.
fn shift_all_formulas(wb: &mut Workbook, edited: &str, op: StructOp) {
    for s in &mut wb.sheets {
        if s.formulas.is_empty() {
            continue;
        }
        unshare(s);
        let host = s.name.clone();
        for (r, c, fid) in formula_cells(s) {
            let f = &s.formulas[fid as usize];
            let new_text = refshift::shift_formula(&f.text, &host, edited, op);
            let new_kind = match &f.kind {
                FKind::Array { ref_ } if refshift::sheet_names_eq(&host, edited) => {
                    refshift::shift_sqref(ref_, op).map(|r| FKind::Array { ref_: r.into() })
                }
                _ => None,
            };
            if new_text.is_some() || new_kind.is_some() {
                let nf = Formula {
                    text: new_text.map_or_else(|| f.text.clone(), Into::into),
                    kind: new_kind.unwrap_or_else(|| f.kind.clone()),
                    attrs: f.attrs.clone(),
                };
                let id = s.add_formula(nf);
                s.set_formula(r, c, Some(id));
            }
        }
    }
}

fn shift_merges(merges: &[Rect], op: StructOp) -> Vec<Rect> {
    merges
        .iter()
        .filter_map(|m| {
            let (a0, a1) = match op.axis {
                Axis::Rows => (m.r0, m.r1),
                Axis::Cols => (m.c0, m.c1),
            };
            let (n0, n1) = if op.insert {
                let s = |v: u32| if v >= op.at { v + op.count } else { v };
                (s(a0), if a1 >= op.at && a0 < op.at { a1 + op.count } else { s(a1) })
            } else {
                let end = op.at + op.count;
                if a0 >= op.at && a1 < end {
                    return None;
                }
                let s = |v: u32| {
                    if v >= end {
                        v - op.count
                    } else if v >= op.at {
                        op.at
                    } else {
                        v
                    }
                };
                let n0 = s(a0);
                let n1 = if a1 >= end {
                    a1 - op.count
                } else if a1 >= op.at {
                    op.at.saturating_sub(1).max(n0)
                } else {
                    a1
                };
                (n0, n1)
            };
            let r = match op.axis {
                Axis::Rows => Rect { r0: n0, r1: n1, ..*m },
                Axis::Cols => Rect { c0: n0, c1: n1, ..*m },
            };
            (r.r0 != r.r1 || r.c0 != r.c1).then_some(r)
        })
        .collect()
}

pub fn insert_rows(wb: &mut Workbook, si: usize, at: u32, count: u32) -> OpResult {
    structural(wb, si, StructOp { axis: Axis::Rows, at, count, insert: true }, "Insert Rows")
}
pub fn delete_rows(wb: &mut Workbook, si: usize, at: u32, count: u32) -> OpResult {
    structural(wb, si, StructOp { axis: Axis::Rows, at, count, insert: false }, "Delete Rows")
}
pub fn insert_cols(wb: &mut Workbook, si: usize, at: u32, count: u32) -> OpResult {
    structural(wb, si, StructOp { axis: Axis::Cols, at, count, insert: true }, "Insert Columns")
}
pub fn delete_cols(wb: &mut Workbook, si: usize, at: u32, count: u32) -> OpResult {
    structural(wb, si, StructOp { axis: Axis::Cols, at, count, insert: false }, "Delete Columns")
}

fn structural(wb: &mut Workbook, si: usize, op: StructOp, label: &str) -> OpResult {
    check_loaded(wb)?;
    if op.count == 0 {
        return Ok(());
    }
    wb.begin_all(label);
    structural_inner(wb, si, op);
    wb.commit();
    Ok(())
}

fn structural_inner(wb: &mut Workbook, si: usize, op: StructOp) {
    // Shared formulas are resolved relative to their current positions, so
    // expand them before anything moves.
    for s in &mut wb.sheets {
        unshare(s);
    }
    {
        let s = &mut wb.sheets[si];
        match (op.axis, op.insert) {
            (Axis::Rows, true) => {
                if op.at < s.row_count() {
                    s.alloc_rows(op.at, op.count);
                }
            }
            (Axis::Rows, false) => {
                let n = s.row_count();
                if op.at < n {
                    let end = (op.at + op.count).min(n);
                    Arc::make_mut(&mut s.grid.row_map).drain(op.at as usize..end as usize);
                }
            }
            (Axis::Cols, true) => {
                if op.at < s.col_count() {
                    s.alloc_cols(op.at, op.count);
                }
            }
            (Axis::Cols, false) => {
                let n = s.col_count();
                if op.at < n {
                    let end = (op.at + op.count).min(n);
                    Arc::make_mut(&mut s.grid.col_map).drain(op.at as usize..end as usize);
                }
            }
        }
        let merges = shift_merges(&s.grid.merges, op);
        s.grid.merges = Arc::new(merges);
        if !s.grid.cf.is_empty() {
            s.grid.cf = Arc::new(crate::cf::shift(&s.grid.cf, op));
        }
        if !s.grid.tables.is_empty() {
            s.grid.tables = Arc::new(crate::tables::shift(&s.grid.tables, op));
        }
        if !s.grid.drawings.is_empty() {
            s.grid.drawings = Arc::new(crate::drawings::shift(&s.grid.drawings, op));
        }
        s.filter_hidden = None;
        s.dirty = true;
        s.invalidate_geometry();
    }
    let name = wb.sheets[si].name.clone();
    shift_all_formulas(wb, &name, op);
    wb.push_log(LogOp::Struct { sheet: name, op });
}

pub fn set_col_width(wb: &mut Workbook, si: usize, c0: u32, c1: u32, px: f64) -> OpResult {
    check_loaded(wb)?;
    wb.begin("Column Width", &[si]);
    let w = crate::sheet::px_to_col_width(px.max(0.0));
    for c in c0..=c1 {
        let m = wb.sheets[si].col_meta_mut(c);
        m.width = Some(w);
        m.custom_width = true;
        m.hidden = px <= 0.0;
    }
    wb.commit();
    Ok(())
}

pub fn set_row_height(wb: &mut Workbook, si: usize, r0: u32, r1: u32, px: f64) -> OpResult {
    check_loaded(wb)?;
    wb.begin("Row Height", &[si]);
    let pt = ((px / crate::sheet::PT_TO_PX) * 4.0).round() / 4.0;
    for r in r0..=r1 {
        let m = wb.sheets[si].row_meta_mut(r);
        m.height = Some(pt as f32);
        m.custom_height = true;
        m.hidden = px <= 0.0;
    }
    wb.commit();
    Ok(())
}

pub fn set_hidden(wb: &mut Workbook, si: usize, axis: Axis, a0: u32, a1: u32, hidden: bool) -> OpResult {
    check_loaded(wb)?;
    wb.begin(if hidden { "Hide" } else { "Unhide" }, &[si]);
    for i in a0..=a1 {
        match axis {
            Axis::Rows => wb.sheets[si].row_meta_mut(i).hidden = hidden,
            Axis::Cols => wb.sheets[si].col_meta_mut(i).hidden = hidden,
        }
    }
    wb.commit();
    Ok(())
}

pub fn set_freeze(wb: &mut Workbook, si: usize, rows: u32, cols: u32) -> OpResult {
    wb.begin("Freeze Panes", &[si]);
    let s = &mut wb.sheets[si];
    s.grid.freeze_rows = rows;
    s.grid.freeze_cols = cols;
    s.dirty = true;
    wb.commit();
    Ok(())
}

pub fn merge(wb: &mut Workbook, si: usize, rect: Rect) -> OpResult {
    check_loaded(wb)?;
    if rect.r0 == rect.r1 && rect.c0 == rect.c1 {
        return Ok(());
    }
    wb.begin("Merge Cells", &[si]);
    let s = &mut wb.sheets[si];
    let mut merges: Vec<Rect> = s.grid.merges.iter().copied().filter(|m| !m.intersects(&rect)).collect();
    merges.push(rect);
    s.grid.merges = Arc::new(merges);
    // Excel keeps only the top-left value.
    for r in rect.r0..=rect.r1 {
        for c in rect.c0..=rect.c1 {
            if (r, c) != (rect.r0, rect.c0) {
                s.set(r, c, Cell::EMPTY);
                s.set_formula(r, c, None);
            }
        }
    }
    s.dirty = true;
    wb.commit();
    Ok(())
}

pub fn unmerge(wb: &mut Workbook, si: usize, rect: Rect) -> OpResult {
    wb.begin("Unmerge Cells", &[si]);
    let s = &mut wb.sheets[si];
    let merges: Vec<Rect> = s.grid.merges.iter().copied().filter(|m| !m.intersects(&rect)).collect();
    s.grid.merges = Arc::new(merges);
    s.dirty = true;
    wb.commit();
    Ok(())
}

// ---- sheets ------------------------------------------------------------------------

fn valid_sheet_name(wb: &Workbook, name: &str, except: Option<usize>) -> OpResult {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 31 {
        return Err("Sheet names must be 1–31 characters.".into());
    }
    if name.contains(['\\', '/', '?', '*', '[', ']', ':']) || name.starts_with('\'') || name.ends_with('\'') {
        return Err("Sheet names can't contain \\ / ? * [ ] : or start/end with an apostrophe.".into());
    }
    if wb.sheets.iter().enumerate().any(|(i, s)| Some(i) != except && refshift::sheet_names_eq(&s.name, name)) {
        return Err(format!("A sheet named “{name}” already exists."));
    }
    Ok(())
}

pub fn unique_sheet_name(wb: &Workbook, base: &str) -> String {
    let mut n = 1;
    loop {
        let name = if n == 1 && !base.ends_with(char::is_numeric) { base.to_string() } else { format!("{base}{n}") };
        if wb.sheet_index(&name).is_none() {
            return name;
        }
        n += 1;
    }
}

pub fn add_sheet(wb: &mut Workbook, at: usize) -> OpResult<usize> {
    check_loaded(wb)?;
    let name = unique_sheet_name(wb, "Sheet");
    let shared = wb.sheets[0].strings.shared.clone();
    let mut s = Sheet::new(name, crate::strings::SheetStrings::new(shared));
    s.dirty = true;
    if wb.is_csv() {
        return Err("CSV files hold a single sheet. Save as .xlsx to add sheets.".into());
    }
    let at = at.min(wb.sheets.len());
    wb.begin("Add Sheet", &[]);
    wb.sheets.insert(at, s);
    wb.set_book_change(BookChange::Added { index: at });
    wb.mark_book_dirty();
    wb.active = at;
    wb.commit();
    Ok(at)
}

pub fn rename_sheet(wb: &mut Workbook, si: usize, name: &str) -> OpResult {
    check_loaded(wb)?;
    let name = name.trim().to_string();
    if wb.sheets[si].name == name {
        return Ok(());
    }
    valid_sheet_name(wb, &name, Some(si))?;
    let old = wb.sheets[si].name.clone();
    wb.begin_all("Rename Sheet");
    for s in &mut wb.sheets {
        for (r, c, fid) in formula_cells(s) {
            if let Some(t) = refshift::rename_sheet_in_formula(&s.formulas[fid as usize].text, &old, &name) {
                let f = s.formulas[fid as usize].clone();
                let id = s.add_formula(Formula { text: t.into(), ..f });
                s.set_formula(r, c, Some(id));
            }
        }
    }
    wb.sheets[si].name = name.clone();
    wb.set_book_change(BookChange::Renamed { index: si, old: old.clone() });
    wb.push_log(LogOp::Rename { old, new: name });
    wb.mark_book_dirty();
    wb.commit();
    Ok(())
}

pub fn delete_sheet(wb: &mut Workbook, si: usize) -> OpResult {
    check_loaded(wb)?;
    if wb.sheets.iter().filter(|s| s.visibility == crate::sheet::Visibility::Visible).count() <= 1
        && wb.sheets[si].visibility == crate::sheet::Visibility::Visible
    {
        return Err("A workbook must keep at least one visible sheet.".into());
    }
    let name = wb.sheets[si].name.clone();
    wb.begin_all("Delete Sheet");
    for (i, s) in wb.sheets.iter_mut().enumerate() {
        if i == si {
            continue;
        }
        for (r, c, fid) in formula_cells(s) {
            if let Some(t) = refshift::delete_sheet_in_formula(&s.formulas[fid as usize].text, &name) {
                let f = s.formulas[fid as usize].clone();
                let id = s.add_formula(Formula { text: t.into(), ..f });
                s.set_formula(r, c, Some(id));
            }
        }
    }
    let removed = wb.sheets.remove(si);
    // Grid snapshots were captured before removal; drop the removed sheet's entry
    // since the sheet object itself goes into the book change.
    wb.set_book_change(BookChange::Removed { index: si, sheet: Box::new(removed) });
    wb.push_log(LogOp::Delete { name });
    wb.mark_book_dirty();
    if wb.active >= wb.sheets.len() {
        wb.active = wb.sheets.len() - 1;
    }
    wb.commit();
    Ok(())
}

pub fn move_sheet(wb: &mut Workbook, from: usize, to: usize) -> OpResult {
    if from == to || to >= wb.sheets.len() {
        return Ok(());
    }
    wb.begin("Move Sheet", &[]);
    let s = wb.sheets.remove(from);
    wb.sheets.insert(to, s);
    wb.set_book_change(BookChange::Moved { from, to });
    wb.mark_book_dirty();
    wb.active = to;
    wb.commit();
    Ok(())
}

pub fn set_sheet_hidden(wb: &mut Workbook, si: usize, hidden: bool) -> OpResult {
    let visible = wb.sheets.iter().filter(|s| s.visibility == crate::sheet::Visibility::Visible).count();
    if hidden && visible <= 1 {
        return Err("A workbook must keep at least one visible sheet.".into());
    }
    wb.sheets[si].visibility = if hidden { crate::sheet::Visibility::Hidden } else { crate::sheet::Visibility::Visible };
    wb.mark_book_dirty();
    wb.edited = true;
    Ok(())
}

// ---- data tools ---------------------------------------------------------------------

#[derive(Clone, Copy)]
pub struct SortKey {
    pub col: u32,
    pub ascending: bool,
}

/// Sort rows r0..=r1 (whole rows move) by the given columns.
pub fn sort_rows(wb: &mut Workbook, si: usize, r0: u32, r1: u32, keys: &[SortKey]) -> OpResult {
    check_loaded(wb)?;
    let s = &wb.sheets[si];
    let r1 = r1.min(s.row_count().saturating_sub(1));
    if r0 >= r1 || keys.is_empty() {
        return Ok(());
    }
    // Precompute sort keys (numbers < text < bools < errors < blanks, like Excel).
    #[derive(PartialEq, PartialOrd)]
    enum K {
        Num(f64),
        Text(String),
        Bool(bool),
        Err(u8),
        Blank,
    }
    let key = |v: Cell, s: &Sheet| match v.kind() {
        Kind::Number => K::Num(v.as_number().unwrap()),
        Kind::Str => {
            let t = s.strings.get(v.as_str_id().unwrap());
            match wb.is_csv().then(|| view::parse_number(t)).flatten() {
                Some((n, _)) => K::Num(n),
                None => K::Text(t.to_lowercase()),
            }
        }
        Kind::Bool => K::Bool(v.as_bool().unwrap()),
        Kind::Error => K::Err(0),
        Kind::Empty => K::Blank,
    };
    let rank = |k: &K| match k {
        K::Num(_) => 0,
        K::Text(_) => 1,
        K::Bool(_) => 2,
        K::Err(_) => 3,
        K::Blank => 4,
    };
    let mut rows: Vec<(u32, Vec<K>)> = (r0..=r1).map(|r| (r, keys.iter().map(|k| key(s.get(r, k.col), s)).collect())).collect();
    rows.sort_by(|a, b| {
        for (i, k) in keys.iter().enumerate() {
            let (x, y) = (&a.1[i], &b.1[i]);
            let (rx, ry) = (rank(x), rank(y));
            // Blanks always last, regardless of direction.
            let ord = if rx == 4 || ry == 4 {
                rx.cmp(&ry)
            } else if rx != ry {
                if k.ascending { rx.cmp(&ry) } else { ry.cmp(&rx) }
            } else {
                let o = x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal);
                if k.ascending { o } else { o.reverse() }
            };
            if ord != std::cmp::Ordering::Equal {
                return ord;
            }
        }
        a.0.cmp(&b.0)
    });
    let order: Vec<u32> = rows.into_iter().map(|(r, _)| r).collect();
    wb.begin("Sort", &[si]);
    permute_rows(wb, si, r0, &order);
    wb.commit();
    Ok(())
}

/// Reorder rows starting at `r0` so that new row r0+i is old row order[i].
/// Formulas move like a cut/paste: relative refs are translated by the row delta.
fn permute_rows(wb: &mut Workbook, si: usize, r0: u32, order: &[u32]) {
    let s = &mut wb.sheets[si];
    unshare(s);
    let old_map = s.grid.row_map.clone();
    {
        let map = Arc::make_mut(&mut s.grid.row_map);
        for (i, &old_r) in order.iter().enumerate() {
            map[r0 as usize + i] = old_map[old_r as usize];
        }
    }
    if !s.formulas.is_empty() {
        for (i, &old_r) in order.iter().enumerate() {
            let new_r = r0 + i as u32;
            if new_r == old_r {
                continue;
            }
            for c in 0..s.col_count() {
                if let Some(fid) = s.formula_id(new_r, c) {
                    let f = s.formulas[fid as usize].clone();
                    let t = refshift::translate_formula(&f.text, new_r as i32 - old_r as i32, 0);
                    let id = s.add_formula(Formula { text: t.into(), ..f });
                    s.set_formula(new_r, c, Some(id));
                }
            }
        }
    }
    s.filter_hidden = None;
    s.dirty = true;
    s.invalidate_geometry();
}

/// Keep rows for which `keep` is true, packed at the top of r0..=r1; the rest become empty.
fn compact_rows(wb: &mut Workbook, si: usize, r0: u32, r1: u32, keep: &[bool]) -> u32 {
    let removed = keep.iter().filter(|k| !**k).count() as u32;
    if removed == 0 {
        return 0;
    }
    let kept: Vec<u32> = (r0..=r1).filter(|r| keep[(r - r0) as usize]).collect();
    let n = kept.len();
    permute_rows(wb, si, r0, &kept);
    // Fresh empty rows for the tail.
    let s2 = &mut wb.sheets[si];
    let start = s2.grid.next_prow;
    s2.grid.next_prow += removed;
    let map = Arc::make_mut(&mut s2.grid.row_map);
    for i in 0..removed as usize {
        map[r0 as usize + n + i] = start + i as u32;
    }
    removed
}

pub fn remove_empty_rows(wb: &mut Workbook, si: usize, r0: u32, r1: u32) -> OpResult<u32> {
    check_loaded(wb)?;
    let s = &wb.sheets[si];
    let r1 = r1.min(s.row_count().saturating_sub(1));
    if r0 > r1 {
        return Ok(0);
    }
    let cols = s.col_count();
    let keep: Vec<bool> = (r0..=r1).map(|r| (0..cols).any(|c| !view::is_blank(s, r, c))).collect();
    wb.begin("Remove Empty Rows", &[si]);
    let n = compact_rows(wb, si, r0, r1, &keep);
    if n == 0 {
        wb.cancel()
    } else {
        wb.commit()
    }
    Ok(n)
}

/// Remove rows whose values in `cols` repeat an earlier row. Returns rows removed.
pub fn remove_duplicates(wb: &mut Workbook, si: usize, r0: u32, r1: u32, cols: &[u32]) -> OpResult<u32> {
    check_loaded(wb)?;
    let s = &wb.sheets[si];
    let r1 = r1.min(s.row_count().saturating_sub(1));
    if r0 > r1 {
        return Ok(0);
    }
    let all: Vec<u32> = if cols.is_empty() { (0..s.col_count()).collect() } else { cols.to_vec() };
    let mut seen: std::collections::HashSet<Vec<u64>> = std::collections::HashSet::new();
    let keep: Vec<bool> = (r0..=r1)
        .map(|r| {
            let k: Vec<u64> = all
                .iter()
                .map(|&c| {
                    let v = s.get(r, c);
                    // Equal text in different pools must compare equal: hash the text.
                    match v.as_str_id() {
                        Some(id) => {
                            use std::hash::{Hash, Hasher};
                            let mut h = std::collections::hash_map::DefaultHasher::new();
                            s.strings.get(id).to_lowercase().hash(&mut h);
                            h.finish() | 1 << 63
                        }
                        None => v.bits(),
                    }
                })
                .collect();
            seen.insert(k)
        })
        .collect();
    wb.begin("Remove Duplicates", &[si]);
    let n = compact_rows(wb, si, r0, r1, &keep);
    if n == 0 {
        wb.cancel()
    } else {
        wb.commit()
    }
    Ok(n)
}

#[derive(Clone, Copy)]
pub enum TextTransform {
    Trim,
    Upper,
    Lower,
    Title,
}

fn transform_text(t: &str, how: TextTransform) -> String {
    match how {
        TextTransform::Trim => t.split_whitespace().collect::<Vec<_>>().join(" "),
        TextTransform::Upper => t.to_uppercase(),
        TextTransform::Lower => t.to_lowercase(),
        TextTransform::Title => {
            let mut out = String::with_capacity(t.len());
            let mut start = true;
            for ch in t.chars() {
                if ch.is_alphanumeric() {
                    if start {
                        out.extend(ch.to_uppercase());
                    } else {
                        out.extend(ch.to_lowercase());
                    }
                    start = false;
                } else {
                    out.push(ch);
                    start = ch.is_whitespace() || ch == '-' || ch == '(' || ch == '/';
                }
            }
            out
        }
    }
}

/// Apply a text transform to text cells in the rects. Returns cells changed.
pub fn transform(wb: &mut Workbook, si: usize, rects: &[Rect], how: TextTransform) -> OpResult<u32> {
    check_loaded(wb)?;
    wb.begin(
        match how {
            TextTransform::Trim => "Trim Spaces",
            _ => "Change Case",
        },
        &[si],
    );
    let mut n = 0;
    let s = &mut wb.sheets[si];
    for &rect in rects {
        let Some(rect) = clamp_rect(s, rect) else { continue };
        for r in rect.r0..=rect.r1 {
            for c in rect.c0..=rect.c1 {
                if s.formula_id(r, c).is_some() {
                    continue;
                }
                let Some(id) = s.get(r, c).as_str_id() else { continue };
                let old = s.strings.get(id);
                let new = transform_text(old, how);
                if new != old {
                    let nid = if new.is_empty() { None } else { Some(s.strings.add(&new)) };
                    s.set(r, c, nid.map_or(Cell::EMPTY, Cell::string));
                    n += 1;
                }
            }
        }
    }
    if n == 0 {
        wb.cancel()
    } else {
        wb.commit()
    }
    Ok(n)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DateOrder {
    Dmy,
    Mdy,
    Ymd,
}

/// Turn date-like text (and existing dates) in the rects into real dates shown with `fmt`.
/// CSV files keep text, rewritten in `fmt`.
pub fn normalize_dates(wb: &mut Workbook, si: usize, rects: &[Rect], order: DateOrder, fmt: &str) -> OpResult<u32> {
    check_loaded(wb)?;
    let csv = wb.is_csv();
    let date1904 = wb.date1904;
    let nf = crate::numfmt::NumFmt::parse(fmt);
    wb.begin("Format Dates", &[si]);
    let xf_cache: &mut HashMap<u16, u16> = &mut HashMap::new();
    let mut n = 0;
    let rects: Vec<Rect> = rects.iter().filter_map(|&r| clamp_rect(&wb.sheets[si], r)).collect();
    for rect in rects {
        for r in rect.r0..=rect.r1 {
            for c in rect.c0..=rect.c1 {
                let s = &wb.sheets[si];
                if s.formula_id(r, c).is_some() {
                    continue;
                }
                let v = s.get(r, c);
                let serial = match v.kind() {
                    Kind::Str => parse_any_date(s.strings.get(v.as_str_id().unwrap()), order, date1904),
                    Kind::Number if !csv => {
                        let cur = s.style(r, c);
                        if wb.styles.is_date_xf(cur) { v.as_number() } else { None }
                    }
                    _ => None,
                };
                let Some(serial) = serial else { continue };
                n += 1;
                if csv {
                    let mut t = String::new();
                    nf.format_number(serial, false, &mut t);
                    let s = &mut wb.sheets[si];
                    let id = s.strings.add(&t);
                    s.set(r, c, Cell::string(id));
                } else {
                    let cur = wb.sheets[si].style(r, c);
                    let xf = *xf_cache.entry(cur).or_insert_with(|| wb.styles.derive(cur, &StyleChange::NumFmt(fmt.to_string())));
                    let s = &mut wb.sheets[si];
                    s.set(r, c, Cell::number(serial));
                    s.set_style(r, c, xf);
                }
            }
        }
    }
    if n == 0 {
        wb.cancel()
    } else {
        wb.commit()
    }
    Ok(n)
}

/// Parse dates in many shapes: 05/03/2024, 5.3.24, 2024-03-05, 5 Mar 2024, 20240305, with optional time.
pub fn parse_any_date(t: &str, order: DateOrder, date1904: bool) -> Option<f64> {
    let t = t.trim();
    if let Some((n, _)) = view::parse_date(t, date1904) {
        return Some(n);
    }
    let (date, time) = match t.find(' ').filter(|&i| t[i + 1..].contains(':')) {
        Some(i) => (&t[..i], Some(&t[i + 1..])),
        None => (t, None),
    };
    let parts: Vec<&str> = date.split(['/', '-', '.']).collect();
    let (y, m, d) = if parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit())) {
        let p: Vec<u32> = parts.iter().map(|p| p.parse().unwrap_or(0)).collect();
        if parts[0].len() == 4 {
            (p[0], p[1], p[2])
        } else {
            match order {
                DateOrder::Dmy => (p[2], p[1], p[0]),
                DateOrder::Mdy => (p[2], p[0], p[1]),
                DateOrder::Ymd => (p[0], p[1], p[2]),
            }
        }
    } else if date.len() == 8 && date.chars().all(|c| c.is_ascii_digit()) {
        let v: u32 = date.parse().ok()?;
        (v / 10000, v / 100 % 100, v % 100)
    } else {
        return None;
    };
    let y = if y < 100 { if y < 50 { 2000 + y } else { 1900 + y } } else { y };
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let (mut h, mut mi, mut s) = (0, 0, 0);
    if let Some(tm) = time {
        let tp: Vec<&str> = tm.trim().split(':').collect();
        h = tp.first()?.trim().parse().ok()?;
        mi = tp.get(1)?.trim().parse().ok()?;
        s = tp.get(2).and_then(|v| v.trim().parse().ok()).unwrap_or(0);
    }
    crate::numfmt::datetime_to_serial(y as i32, m, d, h, mi, s, date1904)
}

/// Turn amount-like text ("₹1,23,456.78", "(1,200)", "1.200,50", "500 CR") into numbers shown with `fmt`.
pub fn normalize_amounts(wb: &mut Workbook, si: usize, rects: &[Rect], decimal_comma: bool, fmt: &str) -> OpResult<u32> {
    check_loaded(wb)?;
    let csv = wb.is_csv();
    let nf = crate::numfmt::NumFmt::parse(fmt);
    wb.begin("Format Amounts", &[si]);
    let mut xf_cache: HashMap<u16, u16> = HashMap::new();
    let mut n = 0;
    let rects: Vec<Rect> = rects.iter().filter_map(|&r| clamp_rect(&wb.sheets[si], r)).collect();
    for rect in rects {
        for r in rect.r0..=rect.r1 {
            for c in rect.c0..=rect.c1 {
                let s = &wb.sheets[si];
                if s.formula_id(r, c).is_some() {
                    continue;
                }
                let v = s.get(r, c);
                let amount = match v.kind() {
                    Kind::Number => v.as_number(),
                    Kind::Str => parse_amount(s.strings.get(v.as_str_id().unwrap()), decimal_comma),
                    _ => None,
                };
                let Some(a) = amount else { continue };
                n += 1;
                if csv {
                    let mut t = String::new();
                    nf.format_number(a, false, &mut t);
                    let s = &mut wb.sheets[si];
                    let id = s.strings.add(&t);
                    s.set(r, c, crate::delimited::canonical_number(&t).map_or(Cell::string(id), Cell::number));
                } else {
                    let cur = wb.sheets[si].style(r, c);
                    let xf = *xf_cache.entry(cur).or_insert_with(|| wb.styles.derive(cur, &StyleChange::NumFmt(fmt.to_string())));
                    let s = &mut wb.sheets[si];
                    s.set(r, c, Cell::number(a));
                    s.set_style(r, c, xf);
                }
            }
        }
    }
    if n == 0 {
        wb.cancel()
    } else {
        wb.commit()
    }
    Ok(n)
}

pub fn parse_amount(t: &str, decimal_comma: bool) -> Option<f64> {
    let mut s = t.trim().to_string();
    if s.is_empty() {
        return None;
    }
    let mut neg = false;
    let upper = s.to_uppercase();
    for (suffix, is_neg) in [(" DR", true), (" CR", false), ("DR", true), ("CR", false)] {
        if upper.ends_with(suffix) {
            neg = is_neg;
            s.truncate(s.len() - suffix.len());
            break;
        }
    }
    let mut s = s.trim().to_string();
    if s.starts_with('(') && s.ends_with(')') {
        neg = !neg;
        s = s[1..s.len() - 1].to_string();
    }
    if s.ends_with('-') {
        neg = !neg;
        s.pop();
    }
    let mut body = String::new();
    for ch in s.chars() {
        match ch {
            '0'..='9' => body.push(ch),
            '-' if body.is_empty() => neg = !neg,
            '.' if decimal_comma => {}
            ',' if !decimal_comma => {}
            '.' => body.push('.'),
            ',' => body.push('.'),
            ' ' | '\u{a0}' | '\'' | '_' => {}
            c if c.is_alphabetic() || "₹$€£¥".contains(c) => {
                if !body.is_empty() && c.is_alphabetic() {
                    return None;
                }
            }
            _ => return None,
        }
    }
    if body.is_empty() || body.matches('.').count() > 1 {
        return None;
    }
    let v: f64 = body.parse().ok()?;
    Some(if neg { -v } else { v })
}

/// Split one column by a delimiter into new columns inserted to its right.
pub fn text_to_columns(wb: &mut Workbook, si: usize, col: u32, r0: u32, r1: u32, delim: &str) -> OpResult<u32> {
    check_loaded(wb)?;
    if delim.is_empty() {
        return Err("Choose a delimiter.".into());
    }
    let s = &wb.sheets[si];
    let r1 = r1.min(s.row_count().saturating_sub(1));
    let mut parts: Vec<(u32, Vec<String>)> = Vec::new();
    let mut width = 1;
    for r in r0..=r1 {
        if let Some(id) = s.get(r, col).as_str_id() {
            let p: Vec<String> = s.strings.get(id).split(delim).map(|x| x.trim().to_string()).collect();
            width = width.max(p.len());
            parts.push((r, p));
        }
    }
    if width <= 1 {
        return Ok(0);
    }
    let extra = width as u32 - 1;
    // One undo step: insert + fill.
    wb.begin_all("Text to Columns");
    structural_inner(wb, si, StructOp { axis: Axis::Cols, at: col + 1, count: extra, insert: true });
    for (r, p) in parts {
        for (i, t) in p.iter().enumerate() {
            let c = col + i as u32;
            if t.is_empty() {
                wb.sheets[si].set(r, c, Cell::EMPTY);
            } else {
                apply_input(wb, si, r, c, t);
            }
        }
    }
    wb.commit();
    Ok(extra + 1)
}

// ---- find / replace ---------------------------------------------------------------------

#[derive(Clone, Copy, Default)]
pub struct FindOpts {
    pub match_case: bool,
    pub whole_cell: bool,
    pub formulas: bool,
}

fn matches(hay: &str, needle: &str, o: FindOpts) -> bool {
    if o.whole_cell {
        if o.match_case { hay == needle } else { hay.to_lowercase() == needle.to_lowercase() }
    } else if o.match_case {
        hay.contains(needle)
    } else {
        hay.to_lowercase().contains(&needle.to_lowercase())
    }
}

fn cell_search_text(wb: &mut Workbook, si: usize, r: u32, c: u32, o: FindOpts, buf: &mut String) -> bool {
    buf.clear();
    let s = &wb.sheets[si];
    if o.formulas
        && let Some(t) = s.formula_text(r, c)
    {
        buf.push('=');
        buf.push_str(&t);
        return true;
    }
    let v = s.get(r, c);
    if v.is_empty() {
        return false;
    }
    view::display(wb, si, r, c, buf);
    true
}

/// Next match after (r, c) in reading order (wrapping). Hidden rows are skipped.
pub fn find_next(wb: &mut Workbook, si: usize, r: u32, c: u32, query: &str, o: FindOpts, forward: bool) -> Option<(u32, u32)> {
    if query.is_empty() {
        return None;
    }
    let rows = wb.sheets[si].row_count();
    let cols = wb.sheets[si].col_count();
    if rows == 0 || cols == 0 {
        return None;
    }
    let total = rows as u64 * cols as u64;
    let start = (r.min(rows - 1) as u64) * cols as u64 + c.min(cols - 1) as u64;
    let mut buf = String::new();
    for step in 1..=total {
        let i = if forward { (start + step) % total } else { (start + total - step % total) % total };
        let (rr, cc) = ((i / cols as u64) as u32, (i % cols as u64) as u32);
        if wb.sheets[si].is_row_hidden(rr) {
            continue;
        }
        if cell_search_text(wb, si, rr, cc, o, &mut buf) && matches(&buf, query, o) {
            return Some((rr, cc));
        }
    }
    None
}

/// Every match in reading order (hidden rows skipped), up to `limit`.
pub fn find_all(wb: &mut Workbook, si: usize, query: &str, o: FindOpts, limit: usize) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    if query.is_empty() || si >= wb.sheets.len() {
        return out;
    }
    let (rows, cols) = (wb.sheets[si].row_count(), wb.sheets[si].col_count());
    let mut buf = String::new();
    'outer: for r in 0..rows {
        if wb.sheets[si].is_row_hidden(r) {
            continue;
        }
        for c in 0..cols {
            if cell_search_text(wb, si, r, c, o, &mut buf) && matches(&buf, query, o) {
                out.push((r, c));
                if out.len() >= limit {
                    break 'outer;
                }
            }
        }
    }
    out
}

pub fn count_matches(wb: &mut Workbook, si: usize, query: &str, o: FindOpts) -> u32 {
    if query.is_empty() {
        return 0;
    }
    let (rows, cols) = (wb.sheets[si].row_count(), wb.sheets[si].col_count());
    let mut buf = String::new();
    let mut n = 0;
    for r in 0..rows {
        if wb.sheets[si].is_row_hidden(r) {
            continue;
        }
        for c in 0..cols {
            if cell_search_text(wb, si, r, c, o, &mut buf) && matches(&buf, query, o) {
                n += 1;
            }
        }
    }
    n
}

fn replace_in(hay: &str, needle: &str, with: &str, o: FindOpts) -> Option<String> {
    if o.whole_cell {
        return matches(hay, needle, o).then(|| with.to_string());
    }
    if o.match_case {
        return hay.contains(needle).then(|| hay.replace(needle, with));
    }
    let lower = hay.to_lowercase();
    let ln = needle.to_lowercase();
    if !lower.contains(&ln) || lower.len() != hay.len() {
        // Case-folding changed byte lengths; fall back to char-wise matching.
        if !lower.contains(&ln) {
            return None;
        }
    }
    let mut out = String::new();
    let mut i = 0;
    let hb = hay;
    while i < hb.len() {
        if hb[i..].to_lowercase().starts_with(&ln) {
            // Advance by the number of chars in the needle.
            let mut j = i;
            for _ in 0..needle.chars().count() {
                j += hb[j..].chars().next().map_or(0, |c| c.len_utf8());
            }
            out.push_str(with);
            i = j;
        } else {
            let ch = hb[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    Some(out)
}

/// Replace in the given cell only.
pub fn replace_one(wb: &mut Workbook, si: usize, r: u32, c: u32, query: &str, with: &str, o: FindOpts) -> OpResult<bool> {
    check_loaded(wb)?;
    let mut buf = String::new();
    if !cell_search_text(wb, si, r, c, o, &mut buf) {
        return Ok(false);
    }
    let Some(new) = replace_in(&buf, query, with, o) else { return Ok(false) };
    wb.begin("Replace", &[si]);
    apply_input(wb, si, r, c, &if wb.is_csv() || new.starts_with('=') { new } else { protect_text(&new, &buf) });
    wb.commit();
    Ok(true)
}

/// Replacing inside text must not turn it into a number/date/formula by accident.
fn protect_text(new: &str, old: &str) -> String {
    let was_text = view::parse_number(old).is_none();
    if was_text && (view::parse_number(new).is_some() || new.starts_with('=')) { format!("'{new}") } else { new.to_string() }
}

pub fn replace_all(wb: &mut Workbook, si: usize, query: &str, with: &str, o: FindOpts) -> OpResult<u32> {
    check_loaded(wb)?;
    if query.is_empty() {
        return Ok(0);
    }
    let (rows, cols) = (wb.sheets[si].row_count(), wb.sheets[si].col_count());
    let mut buf = String::new();
    let mut hits = Vec::new();
    for r in 0..rows {
        if wb.sheets[si].is_row_hidden(r) {
            continue;
        }
        for c in 0..cols {
            if cell_search_text(wb, si, r, c, o, &mut buf)
                && let Some(new) = replace_in(&buf, query, with, o)
            {
                let text = if wb.is_csv() || new.starts_with('=') { new } else { protect_text(&new, &buf) };
                hits.push((r, c, text));
            }
        }
    }
    if hits.is_empty() {
        return Ok(0);
    }
    wb.begin("Replace All", &[si]);
    for (r, c, t) in &hits {
        apply_input(wb, si, *r, *c, t);
    }
    wb.commit();
    Ok(hits.len() as u32)
}

// ---- filter ------------------------------------------------------------------------------

#[derive(Clone)]
pub enum FilterRule {
    /// Show rows whose cell's display text is one of these (lowercased).
    Values(std::collections::HashSet<String>),
    Contains(String),
    Blank(bool),
}

/// Hide rows (below `header`) whose cell in `col` fails the rule. Multiple columns combine (AND).
pub fn set_filter(
    wb: &mut Workbook,
    si: usize,
    header: u32,
    col: u32,
    rule: Option<FilterRule>,
    filters: &mut HashMap<u32, FilterRule>,
) -> u32 {
    match rule {
        Some(r) => {
            filters.insert(col, r);
        }
        None => {
            filters.remove(&col);
        }
    }
    let rows = wb.sheets[si].row_count();
    if filters.is_empty() {
        wb.sheets[si].filter_hidden = None;
        wb.sheets[si].invalidate_geometry();
        return 0;
    }
    let mut hidden = vec![false; rows as usize];
    let mut buf = String::new();
    let mut n = 0;
    for r in header + 1..rows {
        let mut show = true;
        for (&c, rule) in filters.iter() {
            buf.clear();
            view::display(wb, si, r, c, &mut buf);
            let ok = match rule {
                FilterRule::Values(set) => set.contains(&buf.to_lowercase()),
                FilterRule::Contains(q) => buf.to_lowercase().contains(q.as_str()),
                FilterRule::Blank(b) => buf.is_empty() == *b,
            };
            if !ok {
                show = false;
                break;
            }
        }
        if !show {
            hidden[r as usize] = true;
            n += 1;
        }
    }
    let s = &mut wb.sheets[si];
    s.filter_hidden = Some(hidden);
    s.invalidate_geometry();
    n
}

/// Distinct display values in a column (below `header`) with counts, for the filter menu.
pub fn distinct_values(wb: &mut Workbook, si: usize, header: u32, col: u32, limit: usize) -> Vec<(String, u32)> {
    let rows = wb.sheets[si].row_count();
    let mut counts: HashMap<String, u32> = HashMap::new();
    let mut buf = String::new();
    for r in header + 1..rows {
        buf.clear();
        view::display(wb, si, r, col, &mut buf);
        *counts.entry(buf.clone()).or_insert(0) += 1;
        if counts.len() > limit * 4 {
            break;
        }
    }
    let mut v: Vec<(String, u32)> = counts.into_iter().collect();
    v.sort_by(|a, b| natural_cmp(&a.0, &b.0));
    v.truncate(limit);
    v
}

fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    match (view::parse_number(a), view::parse_number(b)) {
        (Some((x, _)), Some((y, _))) => x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        _ => a.to_lowercase().cmp(&b.to_lowercase()),
    }
}

// ---- stats / clipboard ------------------------------------------------------------------

#[derive(Default, Clone, Copy)]
pub struct Stats {
    pub count: u64,
    pub numbers: u64,
    pub sum: f64,
    pub min: f64,
    pub max: f64,
}

pub fn stats(wb: &Workbook, si: usize, rects: &[Rect]) -> Stats {
    let s = &wb.sheets[si];
    let mut st = Stats { min: f64::INFINITY, max: f64::NEG_INFINITY, ..Default::default() };
    for &rect in rects {
        let Some(rect) = clamp_rect(s, rect) else { continue };
        for r in rect.r0..=rect.r1 {
            if s.is_row_hidden(r) {
                continue;
            }
            for c in rect.c0..=rect.c1 {
                let v = s.get(r, c);
                if v.is_empty() {
                    continue;
                }
                st.count += 1;
                let n = match v.kind() {
                    Kind::Number => v.as_number(),
                    Kind::Str if wb.is_csv() => view::parse_number(s.strings.get(v.as_str_id().unwrap())).map(|x| x.0),
                    _ => None,
                };
                if let Some(n) = n {
                    st.numbers += 1;
                    st.sum += n;
                    st.min = st.min.min(n);
                    st.max = st.max.max(n);
                }
            }
        }
    }
    st
}

/// Internal clipboard: raw values, styles and formulas of a copied block.
#[derive(Clone)]
pub struct Clip {
    pub rows: u32,
    pub cols: u32,
    pub origin: (u32, u32),
    cells: Vec<(Cell, Option<String>, u16, Option<String>)>, // value, string text, xf, formula
    pub csv_source: bool,
}

/// Copy a rectangle: returns tab-separated text for other apps plus an internal clip.
pub fn copy(wb: &mut Workbook, si: usize, rect: Rect) -> (String, Clip) {
    let s = &wb.sheets[si];
    let r1 = rect.r1.min(s.row_count().saturating_sub(1).max(rect.r0));
    let c1 = rect.c1.min(s.col_count().saturating_sub(1).max(rect.c0));
    let mut tsv = String::new();
    let mut cells = Vec::new();
    let mut buf = String::new();
    for r in rect.r0..=r1 {
        if wb.sheets[si].is_row_hidden(r) {
            continue;
        }
        for c in rect.c0..=c1 {
            if c > rect.c0 {
                tsv.push('\t');
            }
            buf.clear();
            view::display(wb, si, r, c, &mut buf);
            if buf.contains(['\t', '\n', '"']) {
                tsv.push('"');
                tsv.push_str(&buf.replace('"', "\"\""));
                tsv.push('"');
            } else {
                tsv.push_str(&buf);
            }
            let s = &wb.sheets[si];
            let v = s.get(r, c);
            let text = v.as_str_id().map(|id| s.strings.get(id).to_string());
            cells.push((v, text, s.style(r, c), s.formula_text(r, c)));
        }
        tsv.push('\n');
    }
    let rows = cells.len() as u32 / (c1 - rect.c0 + 1).max(1);
    let clip = Clip { rows, cols: c1 - rect.c0 + 1, origin: (rect.r0, rect.c0), cells, csv_source: wb.is_csv() };
    (tsv, clip)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PasteWhat {
    All,
    Values,
    Formats,
}

/// Paste an internal clip at (r, c), tiling it over `target` if that is a multiple.
pub fn paste_clip(wb: &mut Workbook, si: usize, target: Rect, clip: &Clip, what: PasteWhat) -> OpResult<Rect> {
    check_loaded(wb)?;
    let target = bound_to_data(&wb.sheets[si], target, clip.rows, clip.cols);
    wb.begin("Paste", &[si]);
    let r = paste_clip_inner(wb, si, target, clip, what);
    wb.sheets[si].invalidate_geometry();
    wb.commit();
    Ok(r)
}

fn paste_clip_inner(wb: &mut Workbook, si: usize, target: Rect, clip: &Clip, what: PasteWhat) -> Rect {
    let tile_r = ((target.r1 - target.r0 + 1) / clip.rows.max(1)).max(1);
    let tile_c = ((target.c1 - target.c0 + 1) / clip.cols.max(1)).max(1);
    let csv = wb.is_csv();
    let same_styles = !clip.csv_source && !csv;
    for tr in 0..tile_r {
        for tc in 0..tile_c {
            for i in 0..clip.rows {
                for j in 0..clip.cols {
                    let (v, text, xf, formula) = &clip.cells[(i * clip.cols + j) as usize];
                    let r = target.r0 + tr * clip.rows + i;
                    let c = target.c0 + tc * clip.cols + j;
                    if r >= wb.sheets[si].max_rows || c >= MAX_COLS {
                        continue;
                    }
                    if what != PasteWhat::Formats {
                        match (formula, what) {
                            (Some(f), PasteWhat::All) if !csv => {
                                let t = refshift::translate_formula(
                                    f,
                                    r as i32 - (clip.origin.0 + i) as i32,
                                    c as i32 - (clip.origin.1 + j) as i32,
                                );
                                let s = &mut wb.sheets[si];
                                let fid = s.add_formula(Formula { text: t.into(), kind: FKind::Normal, attrs: "".into() });
                                s.set_formula(r, c, Some(fid));
                                s.set(r, c, Cell::EMPTY);
                            }
                            _ => {
                                let s = &mut wb.sheets[si];
                                s.set_formula(r, c, None);
                                let nv = match text {
                                    Some(t) => Cell::string(s.strings.add(t)),
                                    None => *v,
                                };
                                s.set(r, c, nv);
                            }
                        }
                    }
                    if what != PasteWhat::Values && same_styles {
                        wb.sheets[si].set_style(r, c, *xf);
                    }
                }
            }
        }
    }
    Rect { r0: target.r0, c0: target.c0, r1: target.r0 + tile_r * clip.rows - 1, c1: target.c0 + tile_c * clip.cols - 1 }
}

/// Parse tab-separated text as Excel/Numbers put it on the clipboard.
pub fn parse_tsv(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let b = text.as_bytes();
    let mut pos = 0;
    let mut fields = Vec::new();
    let mut scratch = String::new();
    while let Some(next) = crate::delimited::next_record_into(b, pos, b'\t', &mut fields) {
        let mut row = Vec::with_capacity(fields.len());
        for f in &fields {
            scratch.clear();
            let raw = &text[f.start..f.end];
            row.push(if f.escaped { raw.replace("\"\"", "\"") } else { raw.to_string() });
        }
        rows.push(row);
        pos = next;
    }
    while rows.last().is_some_and(|r: &Vec<String>| r.len() == 1 && r[0].is_empty()) {
        rows.pop();
    }
    rows
}

/// Paste plain text (from another app) starting at (r, c). Typed-input rules apply.
pub fn paste_text(wb: &mut Workbook, si: usize, r: u32, c: u32, text: &str) -> OpResult<Rect> {
    check_loaded(wb)?;
    let rows = parse_tsv(text);
    if rows.is_empty() {
        return Ok(Rect::cell(r, c));
    }
    wb.begin("Paste", &[si]);
    let mut maxc = 0;
    for (i, row) in rows.iter().enumerate() {
        for (j, t) in row.iter().enumerate() {
            let (rr, cc) = (r + i as u32, c + j as u32);
            if rr >= wb.sheets[si].max_rows || cc >= MAX_COLS {
                continue;
            }
            // Pasted text is data, not a formula to evaluate.
            apply_input(wb, si, rr, cc, t);
            maxc = maxc.max(j as u32);
        }
    }
    wb.sheets[si].invalidate_geometry();
    wb.commit();
    Ok(Rect { r0: r, c0: c, r1: r + rows.len() as u32 - 1, c1: c + maxc })
}

/// Fill the first row (down) or column (right) of `rect` across the rest.
pub fn fill(wb: &mut Workbook, si: usize, rect: Rect, down: bool) -> OpResult {
    check_loaded(wb)?;
    let rect = bound_to_data(&wb.sheets[si], rect, 1, 1);
    let src = if down { Rect { r1: rect.r0, ..rect } } else { Rect { c1: rect.c0, ..rect } };
    let (_, clip) = copy(wb, si, src);
    let target = if down { Rect { r0: rect.r0 + 1, ..rect } } else { Rect { c0: rect.c0 + 1, ..rect } };
    if target.r0 > target.r1 || target.c0 > target.c1 {
        return Ok(());
    }
    paste_clip(wb, si, target, &clip, PasteWhat::All)?;
    Ok(())
}

/// Fill handle: extend `src` over `target` (below or right of it). Numbers, dates,
/// "Item 1"-style text and month/day names continue as series; everything else
/// is copied (formulas adjusted). Doesn't touch the clipboard.
pub fn fill_from(wb: &mut Workbook, si: usize, src: Rect, target: Rect) -> OpResult {
    check_loaded(wb)?;
    let target = bound_to_data(&wb.sheets[si], target, 1, 1);
    let down = target.r0 > src.r1;
    let (_, clip) = copy(wb, si, src);
    wb.begin("Fill", &[si]);
    paste_clip_inner(wb, si, target, &clip, PasteWhat::All);
    let lanes = if down { src.c0..=src.c1 } else { src.r0..=src.r1 };
    for lane in lanes {
        let cells: Vec<(u32, u32)> =
            if down { (src.r0..=src.r1).map(|r| (r, lane)).collect() } else { (src.c0..=src.c1).map(|c| (lane, c)).collect() };
        let outs: Vec<(u32, u32)> =
            if down { (target.r0..=target.r1).map(|r| (r, lane)).collect() } else { (target.c0..=target.c1).map(|c| (lane, c)).collect() };
        let series = fill_series_values(wb, si, &cells, outs.len());
        if let Some(vals) = series {
            for ((r, c), v) in outs.into_iter().zip(vals) {
                let s = &mut wb.sheets[si];
                s.set_formula(r, c, None);
                let cell = match v {
                    SeriesVal::Num(n) => Cell::number(n),
                    SeriesVal::Text(t) => Cell::string(s.strings.add(&t)),
                };
                s.set(r, c, cell);
            }
        }
    }
    wb.sheets[si].invalidate_geometry();
    wb.commit();
    Ok(())
}

enum SeriesVal {
    Num(f64),
    Text(String),
}

const MONTHS_SHORT: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const MONTHS_LONG: [&str; 12] =
    ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const DAYS_SHORT: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const DAYS_LONG: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];

/// Values for `n` cells continuing the series in `cells`, or None to keep the copy.
fn fill_series_values(wb: &mut Workbook, si: usize, cells: &[(u32, u32)], n: usize) -> Option<Vec<SeriesVal>> {
    let s = &wb.sheets[si];
    if cells.iter().any(|&(r, c)| s.formula_id(r, c).is_some()) {
        return None;
    }
    let vals: Vec<Cell> = cells.iter().map(|&(r, c)| s.get(r, c)).collect();
    // Numbers (in CSV, canonical numbers are numbers too).
    if vals.iter().all(|v| v.kind() == Kind::Number) {
        let nums: Vec<f64> = vals.iter().map(|v| v.as_number().unwrap()).collect();
        let is_date = !wb.is_csv() && wb.styles.is_date_xf(s.style(cells[0].0, cells[0].1));
        let step = if nums.len() >= 2 {
            (nums[nums.len() - 1] - nums[0]) / (nums.len() - 1) as f64
        } else if is_date {
            1.0
        } else {
            return None; // a single number is copied, like Excel
        };
        let last = *nums.last().unwrap();
        return Some((1..=n).map(|i| SeriesVal::Num(round12(last + step * i as f64))).collect());
    }
    if !vals.iter().all(|v| v.kind() == Kind::Str) {
        return None;
    }
    let texts: Vec<String> = vals.iter().map(|v| s.strings.get(v.as_str_id().unwrap()).to_string()).collect();
    // Month / weekday names.
    for (list, cycle) in [(&MONTHS_LONG[..], 12), (&MONTHS_SHORT[..], 12), (&DAYS_LONG[..], 7), (&DAYS_SHORT[..], 7)] {
        let idx: Option<Vec<usize>> = texts.iter().map(|t| list.iter().position(|m| m.eq_ignore_ascii_case(t))).collect();
        if let Some(idx) = idx {
            let step = if idx.len() >= 2 { (idx[1] + cycle - idx[0]) % cycle } else { 1 };
            let step = if step == 0 { 1 } else { step };
            let case = |w: &str| {
                let t = &texts[0];
                if t.chars().all(|c| !c.is_lowercase()) {
                    w.to_uppercase()
                } else if t.chars().all(|c| !c.is_uppercase()) {
                    w.to_lowercase()
                } else {
                    w.to_string()
                }
            };
            let last = *idx.last().unwrap();
            return Some((1..=n).map(|i| SeriesVal::Text(case(list[(last + step * i) % cycle]))).collect());
        }
    }
    // Text with a trailing number: "Item 7", "Q1", "INV-0042".
    let split = |t: &str| -> Option<(String, String)> {
        let digits = t.chars().rev().take_while(|c| c.is_ascii_digit()).count();
        (digits > 0 && digits < t.len()).then(|| (t[..t.len() - digits].to_string(), t[t.len() - digits..].to_string()))
    };
    let parts: Option<Vec<(String, String)>> = texts.iter().map(|t| split(t)).collect();
    if let Some(parts) = parts
        && parts.iter().all(|p| p.0 == parts[0].0)
    {
        let nums: Vec<i64> = parts.iter().map(|p| p.1.parse().unwrap_or(0)).collect();
        let width = parts.last().unwrap().1.len();
        let step = if nums.len() >= 2 { (nums[nums.len() - 1] - nums[0]) / (nums.len() as i64 - 1) } else { 1 };
        let step = if step == 0 { 1 } else { step };
        let last = *nums.last().unwrap();
        let prefix = parts[0].0.clone();
        return Some(
            (1..=n as i64).map(|i| SeriesVal::Text(format!("{prefix}{:0width$}", (last + step * i).max(0), width = width))).collect(),
        );
    }
    None
}

fn round12(v: f64) -> f64 {
    let s = format!("{:.12e}", v);
    s.parse().unwrap_or(v)
}

/// Default row meta for tests.
pub fn row_meta(s: &Sheet, r: u32) -> RowMeta {
    s.row_meta(r).copied().unwrap_or_default()
}
