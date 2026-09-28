//! Streaming XLSX reader.
//!
//! `open` reads the small workbook-level parts synchronously and returns a
//! workbook whose sheets are empty; `load_sheet` then streams one worksheet's
//! XML (never materialising it) and hands parsed rows to the shared workbook
//! in small batches so the UI can draw while the rest loads.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

use super::{Package, SheetPart, find_entry, open_zip, parse_rels, read_entry, rel_is, rels_path, resolve};
use crate::cell::Cell;
use crate::refshift;
use crate::sheet::{ColMeta, FKind, Formula, Rect, RowMeta, Sheet, Visibility};
use crate::strings::{SheetStrings, StringPool};
use crate::styles::Styles;
use crate::workbook::{Source, Workbook};
use crate::xml::{self, attr, attr_bool, attr_f32, attr_raw, attr_u32, local, parse_u32};

pub use waffle_core::source::{RichMap, Run};

pub type Error = std::io::Error;

fn err(msg: impl Into<String>) -> Error {
    Error::other(msg.into())
}

pub struct SheetJob {
    pub index: usize,
    pub path: String,
}

pub fn open(file: File, macro_enabled: bool) -> Result<(Workbook, Vec<SheetJob>), Error> {
    let mut zip = open_zip(&file)?;
    let entries: Vec<String> = zip.file_names().map(str::to_string).collect();
    let content_types = read_entry(&mut zip, "[Content_Types].xml").unwrap_or_default();

    let root_rels = parse_rels(&read_entry(&mut zip, "_rels/.rels")?);
    let wb_path = root_rels
        .iter()
        .find(|r| rel_is(&r.kind, "officeDocument"))
        .map(|r| resolve("", &r.target))
        .unwrap_or_else(|| "xl/workbook.xml".into());
    let wb_path = find_entry(&zip, &wb_path).ok_or_else(|| err("This file has no workbook part."))?;
    let wb_xml = read_entry(&mut zip, &wb_path)?;
    let wb_rels_path = rels_path(&wb_path);
    let wb_rels = read_entry(&mut zip, &wb_rels_path).unwrap_or_default();
    let rels = parse_rels(&wb_rels);
    let rel_target = |id: &str| rels.iter().find(|r| r.id == id).map(|r| resolve(&wb_path, &r.target));

    // Workbook properties and sheet list.
    let mut date1904 = false;
    let mut calc_on_load = false;
    let mut active = 0usize;
    let mut sheet_decls: Vec<(String, u32, String, Visibility)> = Vec::new();
    let mut names_raw: Vec<(String, Option<usize>, String)> = Vec::new();
    {
        let mut r = Reader::from_reader(wb_xml.as_slice());
        loop {
            match r.read_event() {
                Ok(Event::Start(e) | Event::Empty(e)) => match local(e.name().as_ref()) {
                    b"workbookPr" => date1904 = attr_bool(&e, b"date1904").unwrap_or(false),
                    b"calcPr" => calc_on_load = attr_bool(&e, b"fullCalcOnLoad").unwrap_or(false),
                    b"workbookView" => active = attr_u32(&e, b"activeTab").unwrap_or(0) as usize,
                    b"definedName" => {
                        let name = attr(&e, b"name").unwrap_or_default();
                        let local = attr_u32(&e, b"localSheetId").map(|v| v as usize);
                        if let Ok(t) = r.read_text(e.name()) {
                            names_raw.push((name, local, xml::unescape(&String::from_utf8_lossy(t.as_ref()))));
                        }
                    }
                    b"sheet" => {
                        let vis = match attr(&e, b"state").as_deref() {
                            Some("hidden") => Visibility::Hidden,
                            Some("veryHidden") => Visibility::VeryHidden,
                            _ => Visibility::Visible,
                        };
                        sheet_decls.push((
                            attr(&e, b"name").unwrap_or_default(),
                            attr_u32(&e, b"sheetId").unwrap_or(0),
                            attr(&e, b"id").unwrap_or_default(),
                            vis,
                        ));
                    }
                    _ => {}
                },
                Ok(Event::Eof) | Err(_) => break,
                _ => {}
            }
        }
    }

    let find_kind = |k: &str| rels.iter().find(|r| rel_is(&r.kind, k)).map(|r| resolve(&wb_path, &r.target));
    let sst_path = find_kind("sharedStrings").and_then(|p| find_entry(&zip, &p));
    let styles_path = find_kind("styles").and_then(|p| find_entry(&zip, &p));
    let theme = find_kind("theme").and_then(|p| read_entry(&mut zip, &p).ok());

    let styles = match &styles_path {
        Some(p) => Styles::parse(read_entry(&mut zip, p)?, theme.as_deref()),
        None => Styles::minimal(),
    };

    let (sst, sst_file_count, rich) = match &sst_path {
        Some(p) => {
            let real = find_entry(&zip, p).unwrap();
            let f = zip.by_name(&real).map_err(err_zip)?;
            read_sst(BufReader::with_capacity(1 << 16, f), &styles)?
        }
        None => (StringPool::new(), 0, RichMap::new()),
    };
    let sst = Arc::new(sst);

    let decl_rids: Vec<String> = sheet_decls.iter().map(|d| d.2.clone()).collect();
    let mut sheets = Vec::new();
    let mut jobs = Vec::new();
    for (name, sheet_id, rid, vis) in sheet_decls {
        let Some(target) = rel_target(&rid) else { continue };
        // Only worksheets get a grid; chartsheets/dialogsheets are preserved untouched.
        let kind_ok = rels.iter().any(|r| r.id == rid && rel_is(&r.kind, "worksheet"));
        if !kind_ok {
            continue;
        }
        let Some(path) = find_entry(&zip, &target) else { continue };
        let mut s = Sheet::new(name, SheetStrings::new(sst.clone()));
        s.visibility = vis;
        s.loaded = false;
        s.part = Some(Box::new(SheetPart { path: path.clone(), rel_id: rid, sheet_id, prefix: Vec::new(), suffix: Vec::new() }));
        jobs.push(SheetJob { index: sheets.len(), path });
        sheets.push(s);
    }
    if sheets.is_empty() {
        return Err(err("This workbook has no worksheets."));
    }

    let pkg = Package {
        file,
        entries,
        wb_path,
        wb_xml,
        wb_rels_path,
        wb_rels,
        content_types,
        sst_path,
        sst_file_count,
        styles_path,
        book_dirty: false,
        needs_recalc: false,
        macro_enabled,
    };
    let mut wb = Workbook::new(sheets, styles, Source::Xlsx(Box::new(pkg)));
    wb.rich = Arc::new(rich);
    // localSheetId counts all <sheet> entries; map it to our worksheet list.
    let names: Vec<(String, Option<usize>, String)> = names_raw
        .into_iter()
        .filter(|n| !n.0.starts_with("_xlnm."))
        .map(|(n, local, f)| {
            let scope = local
                .and_then(|i| decl_rids.get(i))
                .and_then(|rid| wb.sheets.iter().position(|s| s.part.as_ref().is_some_and(|p| &p.rel_id == rid)));
            (n, scope, f)
        })
        .collect();
    wb.names_orig = Arc::new(names.clone());
    wb.names = Arc::new(names);
    wb.date1904 = date1904;
    wb.calc_on_load = calc_on_load;
    wb.active = active.min(wb.sheets.len() - 1);
    // Load the visible sheet first.
    let first = wb.active;
    jobs.sort_by_key(|j| j.index != first);
    Ok((wb, jobs))
}

fn err_zip(e: zip::result::ZipError) -> Error {
    Error::other(e)
}

/// Shared strings: plain text of each `<si>` (rich runs concatenated, phonetic runs skipped),
/// plus the run formatting of rich entries.
pub fn read_sst<R: BufRead>(src: R, styles: &Styles) -> Result<(StringPool, usize, RichMap), Error> {
    let mut r = Reader::from_reader(src);
    r.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut pool = StringPool::new();
    let mut rich = RichMap::new();
    let mut runs: Vec<Run> = Vec::new();
    let mut run: Option<Run> = None;
    let mut in_rpr = false;
    let mut cur = String::new();
    let mut in_si = false;
    let mut in_t = false;
    let mut skip_depth = 0u32; // inside <rPh>
    loop {
        match r.read_event_into(&mut buf) {
            Ok(ev @ (Event::Start(_) | Event::Empty(_))) => {
                let is_empty = matches!(ev, Event::Empty(_));
                let (Event::Start(e) | Event::Empty(e)) = ev else { unreachable!() };
                let qn = e.name();
                let n = local(qn.as_ref());
                if is_empty && n == b"si" {
                    pool.push("");
                } else if in_rpr {
                    if let Some(rn) = &mut run {
                        let on = || attr(&e, b"val").as_deref().is_none_or(|v| v != "0" && v != "false");
                        match n {
                            b"b" => rn.bold = Some(on()),
                            b"i" => rn.italic = Some(on()),
                            b"strike" => rn.strike = Some(on()),
                            b"u" => rn.underline = Some(attr(&e, b"val").as_deref() != Some("none")),
                            b"sz" => rn.size = attr(&e, b"val").and_then(|v| v.parse().ok()),
                            b"color" => rn.color = styles.resolve_color(&e),
                            b"rFont" => rn.font = attr(&e, b"val"),
                            _ => {}
                        }
                    }
                } else if !is_empty {
                    match n {
                        b"si" => {
                            in_si = true;
                            cur.clear();
                            runs.clear();
                        }
                        b"r" if in_si && skip_depth == 0 => run = Some(Run { start: cur.len() as u32, ..Default::default() }),
                        b"rPr" if run.is_some() => in_rpr = true,
                        b"rPh" => skip_depth += 1,
                        b"t" if in_si && skip_depth == 0 => in_t = true,
                        _ => {}
                    }
                }
            }
            Ok(Event::Text(t)) if in_t => {
                let raw = String::from_utf8_lossy(t.as_ref());
                cur.push_str(&xml::unescape(&raw));
            }
            Ok(Event::GeneralRef(g)) if in_t => push_entity(&mut cur, g.as_ref()),
            Ok(Event::CData(t)) if in_t => cur.push_str(&String::from_utf8_lossy(&t)),
            Ok(Event::End(e)) => match local(e.name().as_ref()) {
                b"si" => {
                    in_si = false;
                    let decoded = xml::decode_ooxml_escapes(&cur);
                    // Keep runs only when offsets are still valid and something is formatted.
                    if runs.len() > 1 && decoded.len() == cur.len() {
                        rich.insert(pool.len() as u32, std::mem::take(&mut runs));
                    }
                    pool.push(&decoded);
                }
                b"rPr" => in_rpr = false,
                b"r" => {
                    if let Some(mut rn) = run.take() {
                        rn.end = cur.len() as u32;
                        runs.push(rn);
                    }
                }
                b"rPh" => skip_depth = skip_depth.saturating_sub(1),
                b"t" => in_t = false,
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(e) => return Err(err(format!("sharedStrings: {e}"))),
            _ => {}
        }
        buf.clear();
    }
    pool.shrink();
    let n = pool.len();
    Ok((pool, n, rich))
}

fn push_entity(out: &mut String, name: &[u8]) {
    match name {
        b"amp" => out.push('&'),
        b"lt" => out.push('<'),
        b"gt" => out.push('>'),
        b"quot" => out.push('"'),
        b"apos" => out.push('\''),
        n if n.first() == Some(&b'#') => {
            let s = std::str::from_utf8(&n[1..]).unwrap_or("");
            let code = if let Some(h) = s.strip_prefix('x').or_else(|| s.strip_prefix('X')) {
                u32::from_str_radix(h, 16).ok()
            } else {
                s.parse().ok()
            };
            if let Some(c) = code.and_then(char::from_u32) {
                out.push(c);
            }
        }
        n => {
            out.push('&');
            out.push_str(&String::from_utf8_lossy(n));
            out.push(';');
        }
    }
}

// ---- worksheet streaming ----------------------------------------------------

/// A BufRead wrapper that copies consumed bytes into `capture` while it is Some.
struct Tee<R> {
    inner: R,
    capture: Option<Vec<u8>>,
    consumed: u64,
}

impl<R: BufRead> Read for Tee<R> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        let n = {
            let buf = self.inner.fill_buf()?;
            let n = buf.len().min(out.len());
            out[..n].copy_from_slice(&buf[..n]);
            n
        };
        self.consume(n);
        Ok(n)
    }
}

impl<R: BufRead> BufRead for Tee<R> {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        self.inner.fill_buf()
    }
    fn consume(&mut self, n: usize) {
        if let Some(cap) = &mut self.capture
            && let Ok(buf) = self.inner.fill_buf()
        {
            cap.extend_from_slice(&buf[..n.min(buf.len())]);
        }
        self.consumed += n as u64;
        self.inner.consume(n);
    }
}

#[derive(Default)]
struct StagedFormula {
    text: String,
    kind: u8, // 0 normal, 1 shared, 2 array, 3 other
    si: Option<u32>,
    ref_: Option<String>,
    attrs: String,
}

enum Val {
    Empty,
    Num(f64),
    Sst(u32),
    Text(std::ops::Range<usize>),
    Bool(bool),
    Err(u8),
}

struct StagedCell {
    r: u32,
    c: u32,
    xf: u16,
    val: Val,
    formula: Option<StagedFormula>,
    extra: Option<Box<str>>,
}

#[derive(Default)]
struct Batch {
    cells: Vec<StagedCell>,
    text: String,
    rows: Vec<(u32, RowMeta, String)>,
    max_row: u32,
    max_col: u32,
}

/// Rows applied per lock acquisition.
const BATCH_CELLS: usize = 32 * 1024;

pub struct LoadCtl<'a> {
    pub wb: &'a Mutex<Workbook>,
    pub progress: &'a AtomicU32,
    pub cancel: &'a AtomicBool,
}

pub fn load_sheet(file: &File, job: &SheetJob, ctl: &LoadCtl<'_>) -> Result<(), Error> {
    let mut zip = open_zip(file)?;
    let entry = zip.by_name(&job.path).map_err(err_zip)?;
    let total = entry.size().max(1);
    let tee = Tee { inner: BufReader::with_capacity(1 << 18, entry), capture: Some(Vec::new()), consumed: 0 };
    let mut r = Reader::from_reader(tee);
    r.config_mut().trim_text(false);
    let mut buf = Vec::with_capacity(1024);

    let mut batch = Batch::default();
    let mut shared_child: HashMap<u32, u32> = HashMap::new();
    let mut prefix = Vec::new();
    let mut suffix_started = false;
    let mut cols: Vec<(u32, u32, ColMeta, String)> = Vec::new();
    let mut merges: Vec<Rect> = Vec::new();
    let mut freeze = (0u32, 0u32);
    let mut default_row_h = None;
    let mut default_col_w = None;
    let mut base_col_w = None;

    // Row/cell state.
    let mut row: u32 = 0;
    let mut next_row: u32 = 0;
    let mut next_col: u32 = 0;
    let mut cell: Option<StagedCell> = None;
    let mut cell_type: [u8; 9] = [0; 9];
    let mut cell_type_len = 0usize;
    let mut in_v = false;
    let mut in_f = false;
    let mut in_is = false;
    let mut in_t = false;
    let mut is_skip = 0u32;
    let mut text_buf = String::new();
    let mut in_pane_view = false;

    loop {
        let ev = r.read_event_into(&mut buf).map_err(|e| err(format!("{}: {e}", job.path)))?;
        match ev {
            Event::Start(e) => {
                let qn = e.name();
                let name = local(qn.as_ref());
                match name {
                    b"c" => {
                        let (c, xf, t, extra) = cell_attrs(&e, next_col);
                        cell_type_len = t.len().min(9);
                        cell_type[..cell_type_len].copy_from_slice(&t[..cell_type_len]);
                        next_col = c + 1;
                        cell = Some(StagedCell { r: row, c, xf, val: Val::Empty, formula: None, extra });
                    }
                    b"v" => {
                        in_v = true;
                        text_buf.clear();
                    }
                    b"f" => {
                        in_f = true;
                        text_buf.clear();
                        if let Some(cl) = &mut cell {
                            cl.formula = Some(staged_formula(&e));
                        }
                    }
                    b"is" => {
                        in_is = true;
                        text_buf.clear();
                    }
                    b"rPh" if in_is => is_skip += 1,
                    b"t" if in_is && is_skip == 0 => in_t = true,
                    b"row" => {
                        row = row_start(&e, next_row, &mut batch);
                        next_row = row + 1;
                        next_col = 0;
                    }
                    b"sheetData" => {
                        prefix = take_prefix(r.get_mut());
                    }
                    b"sheetView" => in_pane_view = true,
                    b"cols" | b"mergeCells" | b"sheetViews" | b"sheetFormatPr" => {}
                    _ => {}
                }
            }
            Event::Empty(e) => {
                let qn = e.name();
                match local(qn.as_ref()) {
                    b"c" => {
                        // Styled empty cell.
                        let (c, xf, _, extra) = cell_attrs(&e, next_col);
                        next_col = c + 1;
                        if xf != 0 || extra.is_some() {
                            batch.cells.push(StagedCell { r: row, c, xf, val: Val::Empty, formula: None, extra });
                            batch.max_col = batch.max_col.max(c + 1);
                        }
                    }
                    b"f" => {
                        if let Some(cl) = &mut cell {
                            cl.formula = Some(staged_formula(&e));
                        }
                    }
                    b"row" => {
                        row = row_start(&e, next_row, &mut batch);
                        next_row = row + 1;
                        next_col = 0;
                    }
                    b"sheetData" => {
                        // Empty sheet: the whole element is in the capture.
                        let cap = take_prefix(r.get_mut());
                        prefix = cap;
                        r.get_mut().capture = Some(Vec::new());
                        suffix_started = true;
                    }
                    b"pane" if in_pane_view => {
                        let frozen = matches!(attr(&e, b"state").as_deref(), Some("frozen") | Some("frozenSplit"));
                        if frozen {
                            freeze = (attr_f32(&e, b"ySplit").unwrap_or(0.0) as u32, attr_f32(&e, b"xSplit").unwrap_or(0.0) as u32);
                        }
                    }
                    b"sheetFormatPr" => {
                        default_row_h = attr_f32(&e, b"defaultRowHeight");
                        default_col_w = attr_f32(&e, b"defaultColWidth");
                        base_col_w = attr_f32(&e, b"baseColWidth");
                    }
                    b"col" => {
                        let min = attr_u32(&e, b"min").unwrap_or(1).max(1) - 1;
                        let max = attr_u32(&e, b"max").unwrap_or(min + 1).max(min + 1) - 1;
                        let meta = ColMeta {
                            width: attr_f32(&e, b"width"),
                            hidden: attr_bool(&e, b"hidden").unwrap_or(false),
                            custom_width: attr_bool(&e, b"customWidth").unwrap_or(false),
                            style: attr_u32(&e, b"style").map(|s| s.min(u16::MAX as u32) as u16),
                            extra: 0,
                        };
                        let extra = xml::other_attrs(&e, &[b"min", b"max", b"width", b"hidden", b"customWidth", b"style"]);
                        cols.push((min, max.min(crate::sheet::MAX_COLS - 1), meta, extra));
                    }
                    b"mergeCell" => {
                        if let Some(rf) = attr(&e, b"ref")
                            && let Some((r0, c0, r1, c1)) = refshift::parse_range_ref(&rf)
                        {
                            merges.push(Rect { r0, c0, r1, c1 });
                        }
                    }
                    _ => {}
                }
            }
            Event::Text(t) if (in_v || in_f || (in_t && is_skip == 0)) => {
                let raw = std::str::from_utf8(t.as_ref()).map_err(|_| err("invalid UTF-8 in sheet"))?;
                if raw.contains('&') {
                    text_buf.push_str(&xml::unescape(raw));
                } else {
                    text_buf.push_str(raw);
                }
            }
            Event::GeneralRef(g) if (in_v || in_f || (in_t && is_skip == 0)) => {
                push_entity(&mut text_buf, g.as_ref());
            }
            Event::CData(t) if (in_v || in_f || in_t) => {
                text_buf.push_str(&String::from_utf8_lossy(&t));
            }
            Event::End(e) => {
                let qn = e.name();
                match local(qn.as_ref()) {
                    b"v" => {
                        in_v = false;
                        if let Some(cl) = &mut cell {
                            cl.val = parse_value(&cell_type[..cell_type_len], &text_buf, &mut batch.text);
                        }
                    }
                    b"f" => {
                        in_f = false;
                        if let Some(StagedCell { formula: Some(f), .. }) = &mut cell {
                            f.text = std::mem::take(&mut text_buf);
                        }
                    }
                    b"t" => in_t = false,
                    b"rPh" if in_is => is_skip = is_skip.saturating_sub(1),
                    b"is" => {
                        in_is = false;
                        if let Some(cl) = &mut cell {
                            let s = batch.text.len();
                            batch.text.push_str(&xml::decode_ooxml_escapes(&text_buf));
                            cl.val = Val::Text(s..batch.text.len());
                        }
                    }
                    b"c" => {
                        if let Some(cl) = cell.take() {
                            batch.max_col = batch.max_col.max(cl.c + 1);
                            batch.cells.push(cl);
                        }
                    }
                    b"row" => {
                        batch.max_row = batch.max_row.max(row + 1);
                        if batch.cells.len() >= BATCH_CELLS {
                            flush(&mut batch, job.index, ctl, &mut shared_child)?;
                            ctl.progress.store(progress(r.get_ref().consumed, total), Ordering::Relaxed);
                            if ctl.cancel.load(Ordering::Relaxed) {
                                return Ok(());
                            }
                        }
                    }
                    b"sheetData" => {
                        r.get_mut().capture = Some(Vec::new());
                        suffix_started = true;
                    }
                    b"sheetView" => in_pane_view = false,
                    _ => {}
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    flush(&mut batch, job.index, ctl, &mut shared_child)?;

    let suffix = if suffix_started { r.get_mut().capture.take().unwrap_or_default() } else { Vec::new() };
    if prefix.is_empty() && !suffix_started {
        // No sheetData at all (unusual): keep everything as prefix, write sheetData before </worksheet>.
        prefix = r.get_mut().capture.take().unwrap_or_default();
        if let Some(i) = memchr::memmem::rfind(&prefix, b"</") {
            let tail = prefix.split_off(i);
            let mut wb = ctl.wb.lock().unwrap();
            finish(&mut wb.sheets[job.index], prefix, tail, cols, merges, freeze, (default_row_h, default_col_w, base_col_w));
            return Ok(());
        }
    }
    drop(r);
    let drawings = super::parts::load_drawings(&mut zip, &job.path, &suffix);
    let tables = super::parts::load_tables(&mut zip, &job.path);
    let mut guard = ctl.wb.lock().unwrap();
    let wb = &mut *guard;
    let cf = crate::cf::parse(&suffix, &wb.styles);
    let s = &mut wb.sheets[job.index];
    finish(s, prefix, suffix, cols, merges, freeze, (default_row_h, default_col_w, base_col_w));
    s.grid.cf = Arc::new(cf);
    s.grid.drawings = Arc::new(drawings);
    s.grid.tables = Arc::new(tables);
    ctl.progress.store(1000, Ordering::Relaxed);
    Ok(())
}

fn progress(done: u64, total: u64) -> u32 {
    ((done.saturating_mul(1000)) / total).min(999) as u32
}

/// Prefix = captured bytes up to (not including) the `<sheetData` tag.
fn take_prefix<R>(tee: &mut Tee<R>) -> Vec<u8> {
    let mut cap = tee.capture.take().unwrap_or_default();
    if let Some(i) = memchr::memmem::rfind(&cap, b"<").filter(|&i| {
        let rest = &cap[i + 1..];
        let name_end = rest.iter().position(|b| b.is_ascii_whitespace() || *b == b'>' || *b == b'/').unwrap_or(rest.len());
        local(&rest[..name_end]) == b"sheetData"
    }) {
        cap.truncate(i);
    }
    cap
}

fn finish(
    s: &mut Sheet,
    prefix: Vec<u8>,
    suffix: Vec<u8>,
    cols: Vec<(u32, u32, ColMeta, String)>,
    merges: Vec<Rect>,
    freeze: (u32, u32),
    (drh, dcw, bcw): (Option<f32>, Option<f32>, Option<f32>),
) {
    if let Some(h) = drh {
        s.default_row_height = h;
    }
    if let Some(w) = dcw {
        s.default_col_width = w;
    } else if let Some(b) = bcw {
        // baseColWidth is in characters; convert to the stored width unit (+5px padding, 8px granularity).
        let px = ((b as f64 * crate::sheet::MDW + 5.0) / 8.0).ceil() * 8.0;
        s.default_col_width = crate::sheet::px_to_col_width(px);
    }
    if let Some(&(_, max, _, _)) = cols.iter().max_by_key(|c| c.1) {
        // Only materialise columns that carry data or a real override; a trailing
        // <col min="30" max="16384" …/> must not create 16k columns.
        let data_cols = s.col_count();
        let needed = cols
            .iter()
            .filter(|c| c.2.width.is_some() || c.2.hidden || c.2.style.is_some())
            .map(|c| if c.1 >= data_cols.max(1) + 1024 { c.0 } else { c.1 })
            .max()
            .unwrap_or(0)
            .min(max);
        crate::sheet::SheetBuilder::new(s).reserve(0, needed + 1);
        let mut metas = vec![ColMeta::default(); s.grid.col_map.len()];
        for (min, max, meta, extra) in cols {
            let extra = s.intern_raw(&extra);
            for c in min..=max.min(metas.len() as u32 - 1) {
                metas[c as usize] = ColMeta { extra, ..meta };
            }
            if max as usize >= metas.len() {
                // Remember the open-ended tail as the sheet's default for the rest.
                if let Some(style) = meta.style {
                    let _ = style;
                }
            }
        }
        s.grid.cols = Arc::new(metas);
    }
    s.grid.merges = Arc::new(merges);
    s.grid.freeze_rows = freeze.0;
    s.grid.freeze_cols = freeze.1;
    if let Some(p) = &mut s.part {
        p.prefix = prefix;
        p.suffix = suffix;
    }
    s.loaded = true;
    s.dirty = false;
    s.invalidate_geometry();
}

fn row_start(e: &BytesStart<'_>, next_row: u32, batch: &mut Batch) -> u32 {
    let r = attr_raw(e, b"r").and_then(|v| parse_u32(&v)).map_or(next_row, |v| v.saturating_sub(1));
    let ht = attr_f32(e, b"ht");
    let hidden = attr_bool(e, b"hidden").unwrap_or(false);
    let custom_height = attr_bool(e, b"customHeight").unwrap_or(false);
    let custom_format = attr_bool(e, b"customFormat").unwrap_or(false);
    let style = if custom_format { attr_u32(e, b"s").map(|s| s.min(u16::MAX as u32) as u16) } else { None };
    let extra = xml::other_attrs(e, &[b"r", b"spans", b"ht", b"hidden", b"customHeight", b"customFormat", b"s"]);
    let meta = RowMeta { height: ht, hidden, custom_height, style, extra: 0 };
    if meta != RowMeta::default() || !extra.is_empty() {
        batch.rows.push((r, meta, extra));
    }
    r
}

#[inline]
fn cell_attrs(e: &BytesStart<'_>, next_col: u32) -> (u32, u16, Vec<u8>, Option<Box<str>>) {
    let mut c = next_col;
    let mut xf = 0u16;
    let mut t = Vec::new();
    let mut extra: Option<String> = None;
    for a in e.attributes().with_checks(false).flatten() {
        match local(a.key.as_ref()) {
            b"r" => {
                let v = &a.value;
                let n = v.iter().take_while(|b| b.is_ascii_alphabetic() || **b == b'$').count();
                let letters: Vec<u8> = v[..n].iter().copied().filter(|b| *b != b'$').collect();
                if let Some(col) = std::str::from_utf8(&letters).ok().and_then(refshift::letters_to_col) {
                    c = col;
                }
            }
            b"s" => xf = parse_u32(&a.value).unwrap_or(0).min(u16::MAX as u32) as u16,
            b"t" => t = a.value.to_vec(),
            _ => {
                let s = extra.get_or_insert_with(String::new);
                s.push(' ');
                s.push_str(&String::from_utf8_lossy(a.key.as_ref()));
                s.push_str("=\"");
                s.push_str(&String::from_utf8_lossy(&a.value));
                s.push('"');
            }
        }
    }
    (c, xf, t, extra.map(String::into_boxed_str))
}

fn staged_formula(e: &BytesStart<'_>) -> StagedFormula {
    let t = attr(e, b"t");
    let mut f = StagedFormula::default();
    match t.as_deref() {
        None | Some("normal") => {
            f.attrs = xml::other_attrs(e, &[b"t"]);
        }
        Some("shared") => {
            f.kind = 1;
            f.si = attr_u32(e, b"si");
            f.ref_ = attr(e, b"ref");
            f.attrs = xml::other_attrs(e, &[b"t", b"si", b"ref"]);
        }
        Some("array") => {
            f.kind = 2;
            f.ref_ = attr(e, b"ref");
            f.attrs = xml::other_attrs(e, &[b"t", b"ref"]);
        }
        Some(_) => {
            f.kind = 3;
            f.attrs = xml::other_attrs(e, &[]);
        }
    }
    f
}

#[inline]
fn parse_value(t: &[u8], v: &str, text: &mut String) -> Val {
    match t {
        b"" | b"n" => match v.trim().parse::<f64>() {
            Ok(n) => Val::Num(n),
            Err(_) => Val::Empty,
        },
        b"s" => match v.trim().parse::<u32>() {
            Ok(i) => Val::Sst(i),
            Err(_) => Val::Empty,
        },
        b"b" => Val::Bool(v.trim() == "1" || v.trim().eq_ignore_ascii_case("true")),
        b"e" => Val::Err(
            Cell::error_from_str(v.trim()).as_error().map_or(2, |e| crate::cell::ERRORS.iter().position(|x| *x == e).unwrap_or(2) as u8),
        ),
        b"d" => match parse_iso_datetime(v.trim()) {
            Some(n) => Val::Num(n),
            None => {
                let s = text.len();
                text.push_str(v);
                Val::Text(s..text.len())
            }
        },
        _ => {
            // "str" (formula string result) and anything unknown.
            let s = text.len();
            text.push_str(&xml::decode_ooxml_escapes(v));
            Val::Text(s..text.len())
        }
    }
}

fn parse_iso_datetime(s: &str) -> Option<f64> {
    let (date, time) = s.split_once('T').unwrap_or((s, ""));
    let mut d = date.split('-');
    let y: i32 = d.next()?.parse().ok()?;
    let m: u32 = d.next()?.parse().ok()?;
    let dd: u32 = d.next()?.parse().ok()?;
    let mut t = time.trim_end_matches('Z').split(':');
    let h: u32 = t.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    let mi: u32 = t.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    let sec: f64 = t.next().and_then(|v| v.parse().ok()).unwrap_or(0.0);
    let base = crate::numfmt::datetime_to_serial(y, m, dd, h, mi, sec.trunc() as u32, false)?;
    Some(base + sec.fract() / 86400.0)
}

/// Apply a batch of parsed rows to the shared sheet under one short lock.
fn flush(batch: &mut Batch, index: usize, ctl: &LoadCtl<'_>, shared_child: &mut HashMap<u32, u32>) -> Result<(), Error> {
    if batch.cells.is_empty() && batch.rows.is_empty() {
        return Ok(());
    }
    let mut wb = ctl.wb.lock().unwrap();
    let sst_len = wb.sheets[index].strings.shared.len() as u32;
    let s = &mut wb.sheets[index];
    let mut b = crate::sheet::SheetBuilder::new(s);
    b.reserve(batch.max_row, batch.max_col);
    for (r, mut meta, extra) in batch.rows.drain(..) {
        meta.extra = b.sheet.intern_raw(&extra);
        let rows = Arc::make_mut(&mut b.sheet.grid.rows);
        if rows.len() <= r as usize {
            rows.resize(r as usize + 1, RowMeta::default());
        }
        rows[r as usize] = meta;
    }
    for cl in batch.cells.drain(..) {
        let v = match cl.val {
            Val::Empty => Cell::EMPTY,
            Val::Num(n) => Cell::number(n),
            Val::Sst(i) if i < sst_len => Cell::string(i),
            Val::Sst(_) => Cell::EMPTY,
            Val::Text(range) => Cell::string(b.sheet.strings.add(&batch.text[range])),
            Val::Bool(v) => Cell::boolean(v),
            Val::Err(e) => Cell::error(e),
        };
        let fid = cl.formula.map(|f| match f.kind {
            1 => {
                let si = f.si.unwrap_or(0);
                if f.ref_.is_some() && !f.text.is_empty() {
                    let id = b.sheet.add_formula(Formula {
                        text: f.text.into(),
                        kind: FKind::Shared { si, ref_: f.ref_.map(Into::into) },
                        attrs: f.attrs.into(),
                    });
                    b.sheet.shared_masters.insert(si, id);
                    id
                } else {
                    *shared_child.entry(si).or_insert_with(|| {
                        b.sheet.add_formula(Formula { text: "".into(), kind: FKind::Shared { si, ref_: None }, attrs: f.attrs.into() })
                    })
                }
            }
            2 => b.sheet.add_formula(Formula {
                text: f.text.into(),
                kind: FKind::Array { ref_: f.ref_.unwrap_or_default().into() },
                attrs: f.attrs.into(),
            }),
            3 => b.sheet.add_formula(Formula { text: f.text.into(), kind: FKind::Other, attrs: f.attrs.into() }),
            _ => b.sheet.add_formula(Formula { text: f.text.into(), kind: FKind::Normal, attrs: f.attrs.into() }),
        });
        b.put(cl.r, cl.c, v, cl.xf, fid);
        if let Some(extra) = cl.extra {
            b.sheet.cell_extras.insert((cl.r, cl.c), extra);
        }
    }
    b.sheet.invalidate_geometry();
    batch.text.clear();
    Ok(())
}
