//! C API for the macOS app. Every function catches panics so a bug surfaces
//! as an error message instead of taking the app down.
//!
//! Strings returned as `*const c_char` point into a per-document buffer that
//! stays valid until the next call that returns a string on that document.

#![allow(clippy::missing_safety_doc)]

pub(crate) use waffle_core::{drawings, ops, sheet, styles, view, workbook};
pub(crate) use waffle_io::{doc, xlsx};
pub(crate) use waffle_refs as refshift;

use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

use crate::doc::{Doc, Format};
use crate::ops::{ClearWhat, Clip, FilterRule, FindOpts, PasteWhat, SortKey, TextTransform};
use crate::refshift::Axis;
use crate::sheet::Rect;
use crate::styles::{Side, StyleChange};
use crate::view::WfCell;
use crate::workbook::Workbook;

pub struct WfDoc {
    doc: Doc,
    blob: Vec<u8>,
    cells: Vec<WfCell>,
    text: String,
    out: CString,
    error: CString,
    clip: Option<Clip>,
    filters: HashMap<usize, HashMap<u32, FilterRule>>,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct WfRect {
    pub r0: u32,
    pub c0: u32,
    pub r1: u32,
    pub c1: u32,
}

impl From<WfRect> for Rect {
    fn from(r: WfRect) -> Rect {
        Rect { r0: r.r0.min(r.r1), c0: r.c0.min(r.c1), r1: r.r0.max(r.r1), c1: r.c0.max(r.c1) }
    }
}
impl From<Rect> for WfRect {
    fn from(r: Rect) -> WfRect {
        WfRect { r0: r.r0, c0: r.c0, r1: r.r1, c1: r.c1 }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct WfStyle {
    pub font_size: f32,
    /// 0 = automatic, else 0xFF000000 | rgb
    pub text_color: u32,
    pub fill_color: u32,
    pub bold: u8,
    pub italic: u8,
    pub underline: u8,
    pub strike: u8,
    /// 0 general, 1 left, 2 center, 3 right, 4 fill, 5 justify, 6 centerContinuous, 7 distributed
    pub halign: u8,
    /// 0 bottom, 1 center, 2 top, 3 justify, 4 distributed
    pub valign: u8,
    pub wrap: u8,
    pub indent: u8,
    /// left, right, top, bottom; see styles::BORDER_STYLES
    pub border_style: [u8; 4],
    pub border_color: [u32; 4],
    pub rotation: i16,
    pub is_date: u8,
    pub is_percent: u8,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct WfStats {
    pub count: u64,
    pub numbers: u64,
    pub sum: f64,
    pub min: f64,
    pub max: f64,
}

thread_local! {
    static OPEN_ERROR: std::cell::RefCell<CString> = std::cell::RefCell::new(CString::default());
}

fn cstr<'a>(p: *const c_char) -> &'a str {
    if p.is_null() {
        return "";
    }
    unsafe { CStr::from_ptr(p) }.to_str().unwrap_or("")
}

fn to_c(s: &str) -> CString {
    CString::new(s.replace('\0', "")).unwrap_or_default()
}

unsafe fn rects(p: *const WfRect, n: u32) -> Vec<Rect> {
    if p.is_null() || n == 0 {
        return Vec::new();
    }
    unsafe { std::slice::from_raw_parts(p, n as usize) }.iter().map(|&r| r.into()).collect()
}

/// Run `f` with the locked workbook; on panic, record an error and return `dflt`.
fn with<T>(d: *mut WfDoc, dflt: T, f: impl FnOnce(&mut WfDoc) -> T) -> T {
    if d.is_null() {
        return dflt;
    }
    let d = unsafe { &mut *d };
    match catch_unwind(AssertUnwindSafe(|| f(d))) {
        Ok(v) => v,
        Err(e) => {
            let msg = e
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| e.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "internal error".into());
            d.error = to_c(&format!("Something went wrong: {msg}"));
            // A panic while the lock was held poisons it; clear that so the document stays usable.
            d.doc.wb.clear_poison();
            dflt
        }
    }
}

fn wb(d: &WfDoc) -> std::sync::MutexGuard<'_, Workbook> {
    d.doc.wb.lock().unwrap_or_else(|p| p.into_inner())
}

/// Run an edit returning OpResult; record the error message on failure.
fn edit<T: Default>(d: *mut WfDoc, f: impl FnOnce(&mut Workbook) -> Result<T, String>) -> bool {
    with(d, false, |x| {
        let r = f(&mut wb(x));
        match r {
            Ok(_) => true,
            Err(e) => {
                x.error = to_c(&e);
                false
            }
        }
    })
}

fn out(d: &mut WfDoc, s: &str) -> *const c_char {
    d.out = to_c(s);
    d.out.as_ptr()
}

// ---- lifecycle ------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn wf_open(path: *const c_char) -> *mut WfDoc {
    let path = cstr(path).to_string();
    let r = catch_unwind(|| Doc::open(Path::new(&path)));
    match r {
        Ok(Ok(doc)) => Box::into_raw(Box::new(WfDoc {
            doc,
            blob: Vec::new(),
            cells: Vec::new(),
            text: String::new(),
            out: CString::default(),
            error: CString::default(),
            clip: None,
            filters: HashMap::new(),
        })),
        Ok(Err(e)) => {
            OPEN_ERROR.with(|c| *c.borrow_mut() = to_c(&e.to_string()));
            std::ptr::null_mut()
        }
        Err(_) => {
            OPEN_ERROR.with(|c| *c.borrow_mut() = to_c("This file could not be read."));
            std::ptr::null_mut()
        }
    }
}

/// Error from the last failed `wf_open` on this thread.
#[unsafe(no_mangle)]
pub extern "C" fn wf_open_error() -> *const c_char {
    OPEN_ERROR.with(|c| c.borrow().as_ptr())
}

#[unsafe(no_mangle)]
pub extern "C" fn wf_new() -> *mut WfDoc {
    Box::into_raw(Box::new(WfDoc {
        doc: Doc::new_empty(),
        blob: Vec::new(),
        cells: Vec::new(),
        text: String::new(),
        out: CString::default(),
        error: CString::default(),
        clip: None,
        filters: HashMap::new(),
    }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn wf_close(d: *mut WfDoc) {
    if !d.is_null() {
        let _ = catch_unwind(AssertUnwindSafe(|| drop(unsafe { Box::from_raw(d) })));
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn wf_progress(d: *mut WfDoc) -> u32 {
    with(d, 1000, |x| x.doc.progress())
}

#[unsafe(no_mangle)]
pub extern "C" fn wf_loaded(d: *mut WfDoc) -> bool {
    with(d, true, |x| x.doc.is_loaded())
}

/// Load error (if any), consumed once.
#[unsafe(no_mangle)]
pub extern "C" fn wf_take_load_error(d: *mut WfDoc) -> *const c_char {
    with(d, std::ptr::null(), |x| match x.doc.take_error() {
        Some(e) => out(x, &e),
        None => std::ptr::null(),
    })
}

/// Last error from an edit or save.
#[unsafe(no_mangle)]
pub extern "C" fn wf_error(d: *mut WfDoc) -> *const c_char {
    with(d, std::ptr::null(), |x| x.error.as_ptr())
}

/// 0 xlsx, 1 xlsm, 2 csv, 3 import-only (xls/xlsb/ods), 4 new
#[unsafe(no_mangle)]
pub extern "C" fn wf_format(d: *mut WfDoc) -> u32 {
    with(d, 0, |x| {
        if matches!(wb(x).source, crate::workbook::Source::New) {
            return 4;
        }
        match x.doc.format {
            Format::Xlsx => 0,
            Format::Xlsm => 1,
            Format::Csv => 2,
            Format::Legacy => 3,
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn wf_save(d: *mut WfDoc, path: *const c_char, csv: bool) -> bool {
    let path = cstr(path).to_string();
    with(d, false, |x| match x.doc.save(Path::new(&path), csv) {
        Ok(()) => {
            wb(x).edited = false;
            true
        }
        Err(e) => {
            x.error = to_c(&e.to_string());
            false
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn wf_edited(d: *mut WfDoc) -> bool {
    with(d, false, |x| wb(x).edited)
}

#[unsafe(no_mangle)]
pub extern "C" fn wf_memory(d: *mut WfDoc) -> u64 {
    with(d, 0, |x| wb(x).heap_bytes() as u64)
}

// ---- sheets ------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn wf_sheet_count(d: *mut WfDoc) -> u32 {
    with(d, 0, |x| wb(x).sheets.len() as u32)
}

#[unsafe(no_mangle)]
pub extern "C" fn wf_sheet_name(d: *mut WfDoc, i: u32) -> *const c_char {
    with(d, std::ptr::null(), |x| {
        let name = wb(x).sheets.get(i as usize).map(|s| s.name.clone()).unwrap_or_default();
        out(x, &name)
    })
}

/// 0 visible, 1 hidden, 2 very hidden
#[unsafe(no_mangle)]
pub extern "C" fn wf_sheet_visibility(d: *mut WfDoc, i: u32) -> u32 {
    with(d, 0, |x| wb(x).sheets.get(i as usize).map_or(0, |s| s.visibility as u32))
}

#[unsafe(no_mangle)]
pub extern "C" fn wf_sheet_loaded(d: *mut WfDoc, i: u32) -> bool {
    with(d, true, |x| wb(x).sheets.get(i as usize).is_none_or(|s| s.loaded))
}

#[unsafe(no_mangle)]
pub extern "C" fn wf_active_sheet(d: *mut WfDoc) -> u32 {
    with(d, 0, |x| wb(x).active as u32)
}

#[unsafe(no_mangle)]
pub extern "C" fn wf_set_active_sheet(d: *mut WfDoc, i: u32) {
    with(d, (), |x| {
        let mut w = wb(x);
        if (i as usize) < w.sheets.len() {
            w.active = i as usize;
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn wf_add_sheet(d: *mut WfDoc, at: u32) -> i32 {
    let mut idx = -1;
    edit(d, |w| ops::add_sheet(w, at as usize).map(|i| idx = i as i32));
    idx
}

#[unsafe(no_mangle)]
pub extern "C" fn wf_rename_sheet(d: *mut WfDoc, i: u32, name: *const c_char) -> bool {
    let name = cstr(name).to_string();
    edit(d, |w| ops::rename_sheet(w, i as usize, &name))
}

#[unsafe(no_mangle)]
pub extern "C" fn wf_delete_sheet(d: *mut WfDoc, i: u32) -> bool {
    edit(d, |w| ops::delete_sheet(w, i as usize))
}

#[unsafe(no_mangle)]
pub extern "C" fn wf_move_sheet(d: *mut WfDoc, from: u32, to: u32) -> bool {
    edit(d, |w| ops::move_sheet(w, from as usize, to as usize))
}

#[unsafe(no_mangle)]
pub extern "C" fn wf_set_sheet_hidden(d: *mut WfDoc, i: u32, hidden: bool) -> bool {
    edit(d, |w| ops::set_sheet_hidden(w, i as usize, hidden))
}

// ---- geometry -----------------------------------------------------------------------

/// Run `f` on sheet `si` (with style metrics up to date), or return `dflt`.
fn with_sheet<T>(d: *mut WfDoc, si: u32, dflt: T, f: impl FnOnce(&mut sheet::Sheet) -> T) -> T {
    with(d, None, |x| {
        let mut w = wb(x);
        if w.sheets.get(si as usize).is_some_and(|s| s.loaded) {
            w.sync_metrics(si as usize);
        }
        w.sheets.get_mut(si as usize).map(f)
    })
    .unwrap_or(dflt)
}

/// Most rows the sheet may have (Excel's limit for workbooks, unbounded for CSV).
#[unsafe(no_mangle)]
pub extern "C" fn wf_max_rows(d: *mut WfDoc, si: u32) -> u32 {
    with_sheet(d, si, sheet::MAX_ROWS, |s| s.max_rows)
}

#[unsafe(no_mangle)]
pub extern "C" fn wf_rows(d: *mut WfDoc, si: u32) -> u32 {
    with_sheet(d, si, 0, |s| s.display_rows())
}

#[unsafe(no_mangle)]
pub extern "C" fn wf_cols(d: *mut WfDoc, si: u32) -> u32 {
    with_sheet(d, si, 0, |s| s.display_cols())
}

#[unsafe(no_mangle)]
pub extern "C" fn wf_data_rows(d: *mut WfDoc, si: u32) -> u32 {
    with_sheet(d, si, 0, |s| s.row_count())
}

#[unsafe(no_mangle)]
pub extern "C" fn wf_data_cols(d: *mut WfDoc, si: u32) -> u32 {
    with_sheet(d, si, 0, |s| s.col_count())
}

#[unsafe(no_mangle)]
pub extern "C" fn wf_row_y(d: *mut WfDoc, si: u32, r: u32) -> f64 {
    with_sheet(d, si, 0.0, |s| s.row_y(r))
}

#[unsafe(no_mangle)]
pub extern "C" fn wf_col_x(d: *mut WfDoc, si: u32, c: u32) -> f64 {
    with_sheet(d, si, 0.0, |s| s.col_x(c))
}

#[unsafe(no_mangle)]
pub extern "C" fn wf_row_at(d: *mut WfDoc, si: u32, y: f64) -> u32 {
    with_sheet(d, si, 0, |s| s.row_at(y))
}

#[unsafe(no_mangle)]
pub extern "C" fn wf_col_at(d: *mut WfDoc, si: u32, x: f64) -> u32 {
    with_sheet(d, si, 0, |s| s.col_at(x))
}

#[unsafe(no_mangle)]
pub extern "C" fn wf_total_height(d: *mut WfDoc, si: u32) -> f64 {
    with_sheet(d, si, 0.0, |s| s.total_height())
}

#[unsafe(no_mangle)]
pub extern "C" fn wf_total_width(d: *mut WfDoc, si: u32) -> f64 {
    with_sheet(d, si, 0.0, |s| s.total_width())
}

#[unsafe(no_mangle)]
pub extern "C" fn wf_freeze_rows(d: *mut WfDoc, si: u32) -> u32 {
    with_sheet(d, si, 0, |s| s.grid.freeze_rows)
}

#[unsafe(no_mangle)]
pub extern "C" fn wf_freeze_cols(d: *mut WfDoc, si: u32) -> u32 {
    with_sheet(d, si, 0, |s| s.grid.freeze_cols)
}

#[unsafe(no_mangle)]
pub extern "C" fn wf_row_hidden(d: *mut WfDoc, si: u32, r: u32) -> bool {
    with_sheet(d, si, false, |s| s.is_row_hidden(r))
}

#[unsafe(no_mangle)]
pub extern "C" fn wf_col_hidden(d: *mut WfDoc, si: u32, c: u32) -> bool {
    with_sheet(d, si, false, |s| s.is_col_hidden(c))
}

#[unsafe(no_mangle)]
pub extern "C" fn wf_row_filtered(d: *mut WfDoc, si: u32, r: u32) -> bool {
    with_sheet(d, si, false, |s| s.filter_hidden.as_ref().is_some_and(|f| f.get(r as usize).copied().unwrap_or(false)))
}

#[unsafe(no_mangle)]
pub extern "C" fn wf_has_formula(d: *mut WfDoc, si: u32, r: u32, c: u32) -> bool {
    with_sheet(d, si, false, |s| s.formula_id(r, c).is_some())
}

#[unsafe(no_mangle)]
pub extern "C" fn wf_is_blank(d: *mut WfDoc, si: u32, r: u32, c: u32) -> bool {
    with_sheet(d, si, true, |s| view::is_blank(s, r, c))
}

/// Merges intersecting a rectangle; returns how many were written (≤ max).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wf_merges(d: *mut WfDoc, si: u32, area: WfRect, outp: *mut WfRect, max: u32) -> u32 {
    with(d, 0, |x| {
        let w = wb(x);
        let Some(s) = w.sheets.get(si as usize) else { return 0 };
        let area: Rect = area.into();
        let mut n = 0;
        for m in s.grid.merges.iter().filter(|m| m.intersects(&area)) {
            if n >= max {
                break;
            }
            unsafe { *outp.add(n as usize) = (*m).into() };
            n += 1;
        }
        n
    })
}

/// Merge containing a cell, if any.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wf_merge_at(d: *mut WfDoc, si: u32, r: u32, c: u32, outp: *mut WfRect) -> bool {
    with(d, false, |x| {
        let w = wb(x);
        match w.sheets.get(si as usize).and_then(|s| s.merge_at(r, c)) {
            Some(m) => {
                unsafe { *outp = m.into() };
                true
            }
            None => false,
        }
    })
}

/// Fill the cell buffer for a rectangle. Pointers stay valid until the next fetch.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wf_fetch(d: *mut WfDoc, si: u32, area: WfRect, cells: *mut *const WfCell, text: *mut *const u8) -> u32 {
    with(d, 0, |x| {
        let a: Rect = area.into();
        let mut w = x.doc.wb.lock().unwrap_or_else(|p| p.into_inner());
        if si as usize >= w.sheets.len() {
            return 0;
        }
        view::fetch(&mut w, si as usize, a, &mut x.cells, &mut x.text);
        drop(w);
        unsafe {
            *cells = x.cells.as_ptr();
            *text = x.text.as_ptr();
        }
        x.cells.len() as u32
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn wf_edit_text(d: *mut WfDoc, si: u32, r: u32, c: u32) -> *const c_char {
    with(d, std::ptr::null(), |x| {
        let t = view::edit_text(&mut wb(x), si as usize, r, c);
        out(x, &t)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn wf_display_text(d: *mut WfDoc, si: u32, r: u32, c: u32) -> *const c_char {
    with(d, std::ptr::null(), |x| {
        let mut t = String::new();
        view::display(&mut wb(x), si as usize, r, c, &mut t);
        out(x, &t)
    })
}

/// Excel's Cmd+Arrow: jump to the edge of the current data region.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wf_jump(d: *mut WfDoc, si: u32, r: u32, c: u32, dr: i32, dc: i32, r_out: *mut u32, c_out: *mut u32) {
    with(d, (), |x| {
        let w = wb(x);
        let Some(s) = w.sheets.get(si as usize) else { return };
        let (max_r, max_c) = (s.display_rows().max(1) - 1, s.display_cols().max(1) - 1);
        let filled = |r: u32, c: u32| !view::is_blank(s, r, c);
        let (mut rr, mut cc) = (r, c);
        let step = |rr: u32, cc: u32| -> Option<(u32, u32)> {
            let nr = rr as i64 + dr as i64;
            let nc = cc as i64 + dc as i64;
            (nr >= 0 && nc >= 0 && nr as u32 <= max_r && nc as u32 <= max_c).then_some((nr as u32, nc as u32))
        };
        let limit = (s.row_count().max(s.col_count()) + 2) as usize;
        match step(rr, cc) {
            None => {}
            Some((nr, nc)) => {
                if filled(rr, cc) && filled(nr, nc) {
                    // Run to the last filled cell.
                    (rr, cc) = (nr, nc);
                    let mut i = 0;
                    while let Some((a, b)) = step(rr, cc) {
                        if !filled(a, b) || i > limit {
                            break;
                        }
                        (rr, cc) = (a, b);
                        i += 1;
                    }
                } else {
                    // Skip blanks to the next filled cell, or to the edge.
                    (rr, cc) = (nr, nc);
                    let mut i = 0;
                    while !filled(rr, cc) {
                        match step(rr, cc) {
                            Some((a, b)) if i <= limit => {
                                (rr, cc) = (a, b);
                                i += 1;
                            }
                            _ => {
                                if dr > 0 {
                                    rr = s.row_count().saturating_sub(1).max(r);
                                }
                                if dc > 0 {
                                    cc = s.col_count().saturating_sub(1).max(c);
                                }
                                break;
                            }
                        }
                    }
                }
            }
        }
        unsafe {
            *r_out = rr;
            *c_out = cc;
        }
    })
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct WfRun {
    /// UTF-8 byte range in the cell text.
    pub start: u32,
    pub end: u32,
    /// -1 inherit, 0 off, 1 on
    pub bold: i8,
    pub italic: i8,
    pub underline: i8,
    pub strike: i8,
    /// 0 = inherit
    pub size: f32,
    /// 0 = inherit, else 0xFF000000 | rgb
    pub color: u32,
}

/// Formatting runs of a rich-text cell; font names via `wf_run_font`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wf_runs(d: *mut WfDoc, si: u32, r: u32, c: u32, outp: *mut WfRun, max: u32) -> u32 {
    with(d, 0, |x| {
        let w = wb(x);
        let Some(id) = w.sheets.get(si as usize).and_then(|s| s.get(r, c).as_str_id()) else { return 0 };
        let Some(runs) = w.rich.get(&id) else { return 0 };
        let tri = |v: Option<bool>| v.map_or(-1, |b| b as i8);
        let n = runs.len().min(max as usize);
        for (i, run) in runs.iter().take(n).enumerate() {
            unsafe {
                *outp.add(i) = WfRun {
                    start: run.start,
                    end: run.end,
                    bold: tri(run.bold),
                    italic: tri(run.italic),
                    underline: tri(run.underline),
                    strike: tri(run.strike),
                    size: run.size.unwrap_or(0.0),
                    color: run.color.map_or(0, |c| 0xFF00_0000 | c),
                }
            };
        }
        n as u32
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn wf_run_font(d: *mut WfDoc, si: u32, r: u32, c: u32, i: u32) -> *const c_char {
    with(d, std::ptr::null(), |x| {
        let name = {
            let w = wb(x);
            w.sheets
                .get(si as usize)
                .and_then(|s| s.get(r, c).as_str_id())
                .and_then(|id| w.rich.get(&id))
                .and_then(|runs| runs.get(i as usize))
                .and_then(|r| r.font.clone())
        };
        match name {
            Some(n) => out(x, &n),
            None => std::ptr::null(),
        }
    })
}

// ---- drawings -------------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct WfDrawing {
    /// 0 image, 1 chart
    pub kind: u32,
    pub row0: u32,
    pub col0: u32,
    pub dx0: f64,
    pub dy0: f64,
    /// Whether (row1, col1, dx1, dy1) is set; otherwise use width/height.
    pub two_cell: bool,
    pub row1: u32,
    pub col1: u32,
    pub dx1: f64,
    pub dy1: f64,
    pub width: f64,
    pub height: f64,
}

#[unsafe(no_mangle)]
pub extern "C" fn wf_drawing_count(d: *mut WfDoc, si: u32) -> u32 {
    with_sheet(d, si, 0, |s| s.grid.drawings.len() as u32)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn wf_drawing(d: *mut WfDoc, si: u32, i: u32, outp: *mut WfDrawing) -> bool {
    with(d, false, |x| {
        let w = wb(x);
        let Some(dr) = w.sheets.get(si as usize).and_then(|s| s.grid.drawings.get(i as usize)) else { return false };
        let to = dr.to.unwrap_or_default();
        unsafe {
            *outp = WfDrawing {
                kind: matches!(dr.content, crate::drawings::Content::Chart(_)) as u32,
                row0: dr.from.row,
                col0: dr.from.col,
                dx0: dr.from.dx,
                dy0: dr.from.dy,
                two_cell: dr.to.is_some(),
                row1: to.row,
                col1: to.col,
                dx1: to.dx,
                dy1: to.dy,
                width: dr.size.0,
                height: dr.size.1,
            }
        };
        true
    })
}

/// Image path (for images) or title (for charts).
#[unsafe(no_mangle)]
pub extern "C" fn wf_drawing_ref(d: *mut WfDoc, si: u32, i: u32) -> *const c_char {
    with(d, std::ptr::null(), |x| {
        let s = {
            let w = wb(x);
            match w.sheets.get(si as usize).and_then(|s| s.grid.drawings.get(i as usize)).map(|d| &d.content) {
                Some(crate::drawings::Content::Image(p)) | Some(crate::drawings::Content::Chart(p)) => p.clone(),
                None => String::new(),
            }
        };
        out(x, &s)
    })
}

/// Bytes of a part inside the original package (e.g. an image). Valid until the next call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wf_part_bytes(d: *mut WfDoc, path: *const c_char, len: *mut usize) -> *const u8 {
    let p = cstr(path).to_string();
    with(d, std::ptr::null(), |x| {
        let bytes = {
            let w = wb(x);
            match &w.source {
                crate::workbook::Source::Xlsx(pkg) => {
                    crate::xlsx::open_zip(&pkg.file).ok().and_then(|mut z| crate::xlsx::read_entry(&mut z, &p).ok())
                }
                _ => None,
            }
        };
        match bytes {
            Some(b) => {
                x.blob = b;
                unsafe { *len = x.blob.len() };
                x.blob.as_ptr()
            }
            None => std::ptr::null(),
        }
    })
}

// ---- styles --------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn wf_style_generation(d: *mut WfDoc) -> u32 {
    with(d, 0, |x| wb(x).styles.generation)
}

#[unsafe(no_mangle)]
pub extern "C" fn wf_style_count(d: *mut WfDoc) -> u32 {
    with(d, 0, |x| wb(x).styles.xf_count() as u32)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn wf_style(d: *mut WfDoc, id: u32, outp: *mut WfStyle) {
    with(d, (), |x| {
        let mut w = wb(x);
        let st = &mut w.styles;
        let xf = *st.xf(id as u16);
        let font = st.font(xf.font).clone();
        let border = st.border(xf.border);
        let nf = st.numfmt(id as u16);
        let c = |v: Option<u32>| v.map_or(0, |c| 0xFF00_0000 | c);
        let s = WfStyle {
            font_size: font.size,
            text_color: c(font.color),
            fill_color: c(st.fill(xf.fill)),
            bold: font.bold as u8,
            italic: font.italic as u8,
            underline: font.underline as u8,
            strike: font.strike as u8,
            halign: xf.halign,
            valign: xf.valign,
            wrap: xf.wrap as u8,
            indent: xf.indent,
            border_style: [border[0].style, border[1].style, border[2].style, border[3].style],
            border_color: [c(border[0].color), c(border[1].color), c(border[2].color), c(border[3].color)],
            rotation: xf.rotation,
            is_date: nf.is_date() as u8,
            is_percent: nf.is_percent() as u8,
        };
        unsafe { *outp = s };
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn wf_style_font_name(d: *mut WfDoc, id: u32) -> *const c_char {
    with(d, std::ptr::null(), |x| {
        let name = {
            let w = wb(x);
            let xf = *w.styles.xf(id as u16);
            w.styles.font(xf.font).name.clone()
        };
        out(x, &name)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn wf_style_numfmt(d: *mut WfDoc, id: u32) -> *const c_char {
    with(d, std::ptr::null(), |x| {
        let code = {
            let w = wb(x);
            let xf = *w.styles.xf(id as u16);
            w.styles.numfmt_code(xf.numfmt).to_string()
        };
        out(x, &code)
    })
}

/// xf used by a cell (for toolbar state).
#[unsafe(no_mangle)]
pub extern "C" fn wf_cell_style(d: *mut WfDoc, si: u32, r: u32, c: u32) -> u32 {
    with(d, 0, |x| wb(x).sheets.get(si as usize).map_or(0, |s| s.style(r, c) as u32))
}

/// kind: 0 bold, 1 italic, 2 underline, 3 strike, 4 font size (num), 5 font name (text),
/// 6 text colour (num rgb, <0 = automatic), 7 fill (num rgb, <0 = none), 8 number format (text),
/// 9 horizontal align (num), 10 vertical align (num), 11 wrap (num 0/1),
/// 12 borders: num = style, text = "ltrb" sides (subset), color via `color` (<0 automatic).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wf_apply_style(
    d: *mut WfDoc,
    si: u32,
    rs: *const WfRect,
    n: u32,
    kind: u32,
    num: f64,
    text: *const c_char,
    color: f64,
) -> bool {
    let rects = unsafe { rects(rs, n) };
    let t = cstr(text).to_string();
    let rgb = |v: f64| (v >= 0.0).then_some(v as u32 & 0xFFFFFF);
    let change = match kind {
        0 => StyleChange::Bold(num != 0.0),
        1 => StyleChange::Italic(num != 0.0),
        2 => StyleChange::Underline(num != 0.0),
        3 => StyleChange::Strike(num != 0.0),
        4 => StyleChange::FontSize(num as f32),
        5 => StyleChange::FontName(t),
        6 => StyleChange::TextColor(rgb(num)),
        7 => StyleChange::Fill(rgb(num)),
        8 => StyleChange::NumFmt(if t.is_empty() { "General".into() } else { t }),
        9 => StyleChange::HAlign(num as u8),
        10 => StyleChange::VAlign(num as u8),
        11 => StyleChange::Wrap(num != 0.0),
        12 => {
            let side = Side { style: num as u8, color: rgb(color) };
            let mut sides = [None; 4];
            for (i, ch) in "lrtb".chars().enumerate() {
                if t.contains(ch) {
                    sides[i] = Some(side);
                }
            }
            StyleChange::Borders(sides)
        }
        _ => return false,
    };
    edit(d, |w| ops::apply_style(w, si as usize, &rects, &change))
}

// ---- edits ----------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn wf_set_input(d: *mut WfDoc, si: u32, r: u32, c: u32, text: *const c_char) -> bool {
    let t = cstr(text).to_string();
    edit(d, |w| ops::set_input(w, si as usize, r, c, &t))
}

#[unsafe(no_mangle)]
pub extern "C" fn wf_set_input_range(d: *mut WfDoc, si: u32, rect: WfRect, text: *const c_char) -> bool {
    let t = cstr(text).to_string();
    edit(d, |w| ops::set_input_range(w, si as usize, rect.into(), &t))
}

/// what: 0 contents, 1 formats, 2 all
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wf_clear(d: *mut WfDoc, si: u32, rs: *const WfRect, n: u32, what: u32) -> bool {
    let rects = unsafe { rects(rs, n) };
    let what = match what {
        1 => ClearWhat::Formats,
        2 => ClearWhat::All,
        _ => ClearWhat::Contents,
    };
    edit(d, |w| ops::clear(w, si as usize, &rects, what))
}

#[unsafe(no_mangle)]
pub extern "C" fn wf_insert_rows(d: *mut WfDoc, si: u32, at: u32, n: u32) -> bool {
    edit(d, |w| ops::insert_rows(w, si as usize, at, n))
}
#[unsafe(no_mangle)]
pub extern "C" fn wf_delete_rows(d: *mut WfDoc, si: u32, at: u32, n: u32) -> bool {
    edit(d, |w| ops::delete_rows(w, si as usize, at, n))
}
#[unsafe(no_mangle)]
pub extern "C" fn wf_insert_cols(d: *mut WfDoc, si: u32, at: u32, n: u32) -> bool {
    edit(d, |w| ops::insert_cols(w, si as usize, at, n))
}
#[unsafe(no_mangle)]
pub extern "C" fn wf_delete_cols(d: *mut WfDoc, si: u32, at: u32, n: u32) -> bool {
    edit(d, |w| ops::delete_cols(w, si as usize, at, n))
}
#[unsafe(no_mangle)]
pub extern "C" fn wf_set_col_width(d: *mut WfDoc, si: u32, c0: u32, c1: u32, px: f64) -> bool {
    edit(d, |w| ops::set_col_width(w, si as usize, c0, c1, px))
}
#[unsafe(no_mangle)]
pub extern "C" fn wf_set_row_height(d: *mut WfDoc, si: u32, r0: u32, r1: u32, px: f64) -> bool {
    edit(d, |w| ops::set_row_height(w, si as usize, r0, r1, px))
}
/// axis: 0 rows, 1 cols
#[unsafe(no_mangle)]
pub extern "C" fn wf_set_hidden(d: *mut WfDoc, si: u32, axis: u32, a0: u32, a1: u32, hidden: bool) -> bool {
    let axis = if axis == 0 { Axis::Rows } else { Axis::Cols };
    edit(d, |w| ops::set_hidden(w, si as usize, axis, a0, a1, hidden))
}
#[unsafe(no_mangle)]
pub extern "C" fn wf_set_freeze(d: *mut WfDoc, si: u32, rows: u32, cols: u32) -> bool {
    edit(d, |w| ops::set_freeze(w, si as usize, rows, cols))
}
#[unsafe(no_mangle)]
pub extern "C" fn wf_merge(d: *mut WfDoc, si: u32, rect: WfRect) -> bool {
    edit(d, |w| ops::merge(w, si as usize, rect.into()))
}
#[unsafe(no_mangle)]
pub extern "C" fn wf_unmerge(d: *mut WfDoc, si: u32, rect: WfRect) -> bool {
    edit(d, |w| ops::unmerge(w, si as usize, rect.into()))
}
#[unsafe(no_mangle)]
pub extern "C" fn wf_fill(d: *mut WfDoc, si: u32, rect: WfRect, down: bool) -> bool {
    edit(d, |w| ops::fill(w, si as usize, rect.into(), down))
}

#[unsafe(no_mangle)]
pub extern "C" fn wf_fill_from(d: *mut WfDoc, si: u32, src: WfRect, target: WfRect) -> bool {
    edit(d, |w| ops::fill_from(w, si as usize, src.into(), target.into()))
}

// ---- undo -------------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn wf_undo(d: *mut WfDoc, sheet_out: *mut u32) -> bool {
    with(d, false, |x| {
        let s = wb(x).undo();
        if let Some(s) = s {
            x.filters.clear();
            if !sheet_out.is_null() {
                unsafe { *sheet_out = s as u32 };
            }
        }
        s.is_some()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn wf_redo(d: *mut WfDoc, sheet_out: *mut u32) -> bool {
    with(d, false, |x| {
        let s = wb(x).redo();
        if let Some(s) = s {
            x.filters.clear();
            if !sheet_out.is_null() {
                unsafe { *sheet_out = s as u32 };
            }
        }
        s.is_some()
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn wf_undo_label(d: *mut WfDoc) -> *const c_char {
    with(d, std::ptr::null(), |x| {
        let l = wb(x).undo_label().map(str::to_string);
        match l {
            Some(l) => out(x, &l),
            None => std::ptr::null(),
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn wf_redo_label(d: *mut WfDoc) -> *const c_char {
    with(d, std::ptr::null(), |x| {
        let l = wb(x).redo_label().map(str::to_string);
        match l {
            Some(l) => out(x, &l),
            None => std::ptr::null(),
        }
    })
}

// ---- data tools --------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn wf_sort(d: *mut WfDoc, si: u32, r0: u32, r1: u32, cols: *const u32, asc: *const bool, n: u32) -> bool {
    let keys: Vec<SortKey> = (0..n as usize).map(|i| unsafe { SortKey { col: *cols.add(i), ascending: *asc.add(i) } }).collect();
    edit(d, |w| ops::sort_rows(w, si as usize, r0, r1, &keys))
}

/// Returns rows removed, or -1 on error.
#[unsafe(no_mangle)]
pub extern "C" fn wf_remove_empty_rows(d: *mut WfDoc, si: u32, r0: u32, r1: u32) -> i32 {
    let mut n = -1;
    edit(d, |w| ops::remove_empty_rows(w, si as usize, r0, r1).map(|v| n = v as i32));
    n
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn wf_remove_duplicates(d: *mut WfDoc, si: u32, r0: u32, r1: u32, cols: *const u32, ncols: u32) -> i32 {
    let cols: Vec<u32> = if cols.is_null() { Vec::new() } else { unsafe { std::slice::from_raw_parts(cols, ncols as usize) }.to_vec() };
    let mut n = -1;
    edit(d, |w| ops::remove_duplicates(w, si as usize, r0, r1, &cols).map(|v| n = v as i32));
    n
}

/// how: 0 trim, 1 UPPER, 2 lower, 3 Title
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wf_transform(d: *mut WfDoc, si: u32, rs: *const WfRect, n: u32, how: u32) -> i32 {
    let rects = unsafe { rects(rs, n) };
    let how = match how {
        1 => TextTransform::Upper,
        2 => TextTransform::Lower,
        3 => TextTransform::Title,
        _ => TextTransform::Trim,
    };
    let mut cnt = -1;
    edit(d, |w| ops::transform(w, si as usize, &rects, how).map(|v| cnt = v as i32));
    cnt
}

/// order: 0 D/M/Y, 1 M/D/Y, 2 Y/M/D
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wf_normalize_dates(d: *mut WfDoc, si: u32, rs: *const WfRect, n: u32, order: u32, fmt: *const c_char) -> i32 {
    let rects = unsafe { rects(rs, n) };
    let order = match order {
        1 => ops::DateOrder::Mdy,
        2 => ops::DateOrder::Ymd,
        _ => ops::DateOrder::Dmy,
    };
    let fmt = cstr(fmt).to_string();
    let mut cnt = -1;
    edit(d, |w| ops::normalize_dates(w, si as usize, &rects, order, &fmt).map(|v| cnt = v as i32));
    cnt
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn wf_normalize_amounts(
    d: *mut WfDoc,
    si: u32,
    rs: *const WfRect,
    n: u32,
    decimal_comma: bool,
    fmt: *const c_char,
) -> i32 {
    let rects = unsafe { rects(rs, n) };
    let fmt = cstr(fmt).to_string();
    let mut cnt = -1;
    edit(d, |w| ops::normalize_amounts(w, si as usize, &rects, decimal_comma, &fmt).map(|v| cnt = v as i32));
    cnt
}

#[unsafe(no_mangle)]
pub extern "C" fn wf_text_to_columns(d: *mut WfDoc, si: u32, col: u32, r0: u32, r1: u32, delim: *const c_char) -> i32 {
    let delim = cstr(delim).to_string();
    let mut cnt = -1;
    edit(d, |w| ops::text_to_columns(w, si as usize, col, r0, r1, &delim).map(|v| cnt = v as i32));
    cnt
}

fn find_opts(flags: u32) -> FindOpts {
    FindOpts { match_case: flags & 1 != 0, whole_cell: flags & 2 != 0, formulas: flags & 4 != 0 }
}

/// flags: 1 match case, 2 whole cell, 4 search formulas
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wf_find(
    d: *mut WfDoc,
    si: u32,
    r: u32,
    c: u32,
    query: *const c_char,
    flags: u32,
    forward: bool,
    r_out: *mut u32,
    c_out: *mut u32,
) -> bool {
    let q = cstr(query).to_string();
    with(d, false, |x| match ops::find_next(&mut wb(x), si as usize, r, c, &q, find_opts(flags), forward) {
        Some((rr, cc)) => {
            unsafe {
                *r_out = rr;
                *c_out = cc;
            }
            true
        }
        None => false,
    })
}

/// All matches as (row, col) pairs written to `out` (2 u32 per match); returns the count.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wf_find_all(d: *mut WfDoc, si: u32, query: *const c_char, flags: u32, outp: *mut u32, max: u32) -> u32 {
    let q = cstr(query).to_string();
    with(d, 0, |x| {
        let hits = ops::find_all(&mut wb(x), si as usize, &q, find_opts(flags), max as usize);
        for (i, (r, c)) in hits.iter().enumerate() {
            unsafe {
                *outp.add(i * 2) = *r;
                *outp.add(i * 2 + 1) = *c;
            }
        }
        hits.len() as u32
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn wf_count_matches(d: *mut WfDoc, si: u32, query: *const c_char, flags: u32) -> u32 {
    let q = cstr(query).to_string();
    with(d, 0, |x| ops::count_matches(&mut wb(x), si as usize, &q, find_opts(flags)))
}

#[unsafe(no_mangle)]
pub extern "C" fn wf_replace_one(d: *mut WfDoc, si: u32, r: u32, c: u32, query: *const c_char, with_: *const c_char, flags: u32) -> bool {
    let (q, w_) = (cstr(query).to_string(), cstr(with_).to_string());
    let mut hit = false;
    edit(d, |w| ops::replace_one(w, si as usize, r, c, &q, &w_, find_opts(flags)).map(|h| hit = h));
    hit
}

#[unsafe(no_mangle)]
pub extern "C" fn wf_replace_all(d: *mut WfDoc, si: u32, query: *const c_char, with_: *const c_char, flags: u32) -> i32 {
    let (q, w_) = (cstr(query).to_string(), cstr(with_).to_string());
    let mut n = -1;
    edit(d, |w| ops::replace_all(w, si as usize, &q, &w_, find_opts(flags)).map(|v| n = v as i32));
    n
}

/// Distinct values of a column as "value\tcount\n" lines.
#[unsafe(no_mangle)]
pub extern "C" fn wf_filter_values(d: *mut WfDoc, si: u32, header: u32, col: u32, limit: u32) -> *const c_char {
    with(d, std::ptr::null(), |x| {
        let vals = ops::distinct_values(&mut wb(x), si as usize, header, col, limit as usize);
        let mut s = String::new();
        for (v, n) in vals {
            s.push_str(&v.replace(['\t', '\n'], " "));
            s.push('\t');
            s.push_str(&n.to_string());
            s.push('\n');
        }
        out(x, &s)
    })
}

/// mode: 0 clear this column, 1 values (payload = "\n"-joined), 2 contains, 3 blanks only, 4 non-blanks.
/// Returns rows hidden.
#[unsafe(no_mangle)]
pub extern "C" fn wf_set_filter(d: *mut WfDoc, si: u32, header: u32, col: u32, mode: u32, payload: *const c_char) -> u32 {
    let p = cstr(payload).to_string();
    with(d, 0, |x| {
        let rule = match mode {
            1 => Some(FilterRule::Values(p.split('\n').map(|v| v.to_lowercase()).collect())),
            2 => Some(FilterRule::Contains(p.to_lowercase())),
            3 => Some(FilterRule::Blank(true)),
            4 => Some(FilterRule::Blank(false)),
            _ => None,
        };
        let mut filters = x.filters.remove(&(si as usize)).unwrap_or_default();
        let n = ops::set_filter(&mut wb(x), si as usize, header, col, rule, &mut filters);
        x.filters.insert(si as usize, filters);
        n
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn wf_clear_filters(d: *mut WfDoc, si: u32) {
    with(d, (), |x| {
        x.filters.remove(&(si as usize));
        let mut w = wb(x);
        if let Some(s) = w.sheets.get_mut(si as usize) {
            s.filter_hidden = None;
            s.invalidate_geometry();
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn wf_filter_active(d: *mut WfDoc, si: u32, col: u32) -> bool {
    with(d, false, |x| x.filters.get(&(si as usize)).is_some_and(|f| f.contains_key(&col)))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn wf_stats(d: *mut WfDoc, si: u32, rs: *const WfRect, n: u32, outp: *mut WfStats) {
    let rects = unsafe { rects(rs, n) };
    with(d, (), |x| {
        let st = ops::stats(&wb(x), si as usize, &rects);
        unsafe { *outp = WfStats { count: st.count, numbers: st.numbers, sum: st.sum, min: st.min, max: st.max } };
    })
}

// ---- clipboard ----------------------------------------------------------------------------

/// Copy a rectangle: returns tab-separated text and keeps an internal clip.
#[unsafe(no_mangle)]
pub extern "C" fn wf_copy(d: *mut WfDoc, si: u32, rect: WfRect) -> *const c_char {
    with(d, std::ptr::null(), |x| {
        let (tsv, clip) = ops::copy(&mut wb(x), si as usize, rect.into());
        x.clip = Some(clip);
        out(x, &tsv)
    })
}

/// Paste the internal clip copied from this document. what: 0 all, 1 values, 2 formats.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wf_paste_clip(d: *mut WfDoc, si: u32, target: WfRect, what: u32, out_rect: *mut WfRect) -> bool {
    with(d, false, |x| {
        let Some(clip) = x.clip.clone() else { return false };
        let what = match what {
            1 => PasteWhat::Values,
            2 => PasteWhat::Formats,
            _ => PasteWhat::All,
        };
        let r = ops::paste_clip(&mut wb(x), si as usize, target.into(), &clip, what);
        match r {
            Ok(rect) => {
                unsafe { *out_rect = rect.into() };
                true
            }
            Err(e) => {
                x.error = to_c(&e);
                false
            }
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn wf_paste_text(d: *mut WfDoc, si: u32, r: u32, c: u32, text: *const c_char, out_rect: *mut WfRect) -> bool {
    let t = cstr(text).to_string();
    with(d, false, |x| {
        let r = ops::paste_text(&mut wb(x), si as usize, r, c, &t);
        match r {
            Ok(rect) => {
                unsafe { *out_rect = rect.into() };
                true
            }
            Err(e) => {
                x.error = to_c(&e);
                false
            }
        }
    })
}
