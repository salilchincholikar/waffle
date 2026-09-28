//! "Save as is" XLSX writer.
//!
//! Every package part we don't need to change is raw-copied (still
//! compressed). Changed sheets get a fresh `<sheetData>` between their
//! original prefix/suffix. Shared strings and styles are only appended to.
//! Structural edits recorded in the workbook log are replayed over the parts
//! we don't model (charts, tables, names, comments, drawings…).

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use quick_xml::Reader;
use quick_xml::events::Event;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use super::{Package, find_entry, open_zip, parse_rels, rel_is, rels_path, resolve};
use crate::cell::Kind;
use crate::refshift::{self, Axis, StructOp};
use crate::sheet::{ColMeta, FKind, Rect, Sheet, Visibility};
use crate::workbook::{LogOp, Source, Workbook};
use crate::xml::{self, attr, escape_attr, escape_text, local};
use crate::xmlrw::{self, Rules};

type Result<T> = std::io::Result<T>;

const NS_MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const NS_REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const REL_WS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet";
const REL_SST: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings";
const REL_STYLES: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles";
const CT_WS: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml";
const CT_SST: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml";
const CT_STYLES: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml";

fn opts(big: bool) -> SimpleFileOptions {
    SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .compression_level(Some(if big { 1 } else { 6 }))
        .large_file(big)
}

pub fn save(wb: &Workbook, path: &Path) -> Result<()> {
    // An xlsx sheet can't hold more rows than Excel's grid; refuse rather than write a broken file.
    if let Some(s) = wb.sheets.iter().find(|s| s.row_count() > crate::sheet::MAX_ROWS) {
        return Err(std::io::Error::other(format!(
            "“{}” has {} rows, but an Excel workbook can hold at most {} rows per sheet. Save it as CSV instead, or remove rows first.",
            s.name,
            group(s.row_count()),
            group(crate::sheet::MAX_ROWS)
        )));
    }
    match &wb.source {
        Source::Xlsx(pkg) => save_package(wb, pkg, path),
        _ => save_fresh(wb, path),
    }
}

// ---- shared strings ---------------------------------------------------------

/// Assigns shared-string indices to strings created in the app.
struct SstPlan {
    next: u32,
    maps: Vec<Vec<u32>>,
    order: Vec<(usize, u32)>,
    /// The package has no shared-string table: keep writing strings inline.
    inline: bool,
}

impl SstPlan {
    fn new(wb: &Workbook, base: u32, sheets: &[usize], include_shared: bool) -> SstPlan {
        let mut plan = SstPlan { next: base, maps: vec![Vec::new(); wb.sheets.len()], order: Vec::new(), inline: false };
        for &si in sheets {
            let s = &wb.sheets[si];
            let shared_len = s.strings.shared.len() as u32;
            plan.maps[si] = vec![u32::MAX; s.strings.local.len() + if include_shared { shared_len as usize } else { 0 }];
            for r in 0..s.row_count() {
                for c in 0..s.col_count() {
                    let v = s.get(r, c);
                    let Some(id) = v.as_str_id() else { continue };
                    if s.formula_id(r, c).is_some() {
                        continue;
                    }
                    let slot = if include_shared {
                        id
                    } else if id < shared_len {
                        continue;
                    } else {
                        id - shared_len
                    };
                    let m = &mut plan.maps[si][slot as usize];
                    if *m == u32::MAX {
                        *m = plan.next;
                        plan.next += 1;
                        plan.order.push((si, id));
                    }
                }
            }
        }
        plan
    }

    #[inline]
    fn inline() -> SstPlan {
        SstPlan { next: 0, maps: Vec::new(), order: Vec::new(), inline: true }
    }

    fn index(&self, s: &Sheet, si: usize, id: u32, include_shared: bool) -> u32 {
        let shared_len = s.strings.shared.len() as u32;
        if include_shared {
            self.maps[si][id as usize]
        } else if id < shared_len {
            id
        } else {
            self.maps[si][(id - shared_len) as usize]
        }
    }

    fn entries_xml(&self, wb: &Workbook, out: &mut String) {
        for &(si, id) in &self.order {
            let t = wb.sheets[si].strings.get(id);
            let needs_space = t.starts_with(char::is_whitespace) || t.ends_with(char::is_whitespace);
            out.push_str(if needs_space { "<si><t xml:space=\"preserve\">" } else { "<si><t>" });
            escape_text(t, out);
            out.push_str("</t></si>");
        }
    }
}

// ---- cell / sheet XML -----------------------------------------------------------

pub fn write_number(v: f64, out: &mut String) {
    let a = v.abs();
    if v.fract() == 0.0 && a < 1e15 {
        let _ = write!(out, "{}", v as i64);
    } else if a != 0.0 && !(1e-5..1e16).contains(&a) {
        let _ = write!(out, "{:e}", v);
    } else {
        let _ = write!(out, "{}", v);
    }
}

fn cell_ref(r: u32, c: u32, out: &mut String) {
    refshift::col_to_letters(c, out);
    let _ = write!(out, "{}", r + 1);
}

fn range_ref(rc: &Rect, out: &mut String) {
    cell_ref(rc.r0, rc.c0, out);
    if rc.r0 != rc.r1 || rc.c0 != rc.c1 {
        out.push(':');
        cell_ref(rc.r1, rc.c1, out);
    }
}

/// `<sheetData>…</sheetData>` for a sheet, streamed to `w`.
fn write_sheet_data<W: Write>(w: &mut W, wb: &Workbook, si: usize, sst: &SstPlan, include_shared: bool) -> Result<()> {
    let s = &wb.sheets[si];
    let mut buf = String::with_capacity(1 << 16);
    buf.push_str("<sheetData>");
    let cols = s.col_count();
    for r in 0..s.row_count() {
        let pr = s.grid.row_map[r as usize];
        let meta = s.grid.rows.get(pr as usize).copied().unwrap_or_default();
        let row_start = buf.len();
        let _ = write!(buf, "<row r=\"{}\"", r + 1);
        if let Some(st) = meta.style {
            let _ = write!(buf, " s=\"{st}\" customFormat=\"1\"");
        }
        if let Some(h) = meta.height {
            let _ = write!(buf, " ht=\"{h}\"");
        }
        if meta.hidden {
            buf.push_str(" hidden=\"1\"");
        }
        if meta.custom_height {
            buf.push_str(" customHeight=\"1\"");
        }
        buf.push_str(s.raw(meta.extra));
        buf.push('>');
        let header_len = buf.len();
        for c in 0..cols {
            let pc = s.grid.col_map[c as usize];
            let v = s.get_phys(pr, pc);
            let xf = s.own_style_phys(pr, pc);
            let fid = s.formula_id_phys(pr, pc);
            let extra = s.cell_extras.get(&(pr, pc));
            if v.is_empty() && xf == 0 && fid.is_none() && extra.is_none() {
                continue;
            }
            buf.push_str("<c r=\"");
            cell_ref(r, c, &mut buf);
            buf.push('"');
            if xf != 0 {
                let _ = write!(buf, " s=\"{xf}\"");
            }
            let inline_str = sst.inline && fid.is_none() && v.as_str_id().is_some_and(|id| !s.strings.is_shared(id));
            let t = match v.kind() {
                Kind::Str if fid.is_some() => " t=\"str\"",
                Kind::Str if inline_str => " t=\"inlineStr\"",
                Kind::Str => " t=\"s\"",
                Kind::Bool => " t=\"b\"",
                Kind::Error => " t=\"e\"",
                _ => "",
            };
            buf.push_str(t);
            if let Some(e) = extra {
                buf.push_str(e);
            }
            if v.is_empty() && fid.is_none() {
                buf.push_str("/>");
                continue;
            }
            buf.push('>');
            if let Some(fid) = fid {
                write_formula(s, r, c, fid, &mut buf);
            }
            match v.kind() {
                Kind::Empty => {}
                Kind::Number => {
                    buf.push_str("<v>");
                    write_number(v.as_number().unwrap(), &mut buf);
                    buf.push_str("</v>");
                }
                Kind::Str => {
                    let id = v.as_str_id().unwrap();
                    if fid.is_some() {
                        buf.push_str("<v>");
                        escape_text(s.strings.get(id), &mut buf);
                        buf.push_str("</v>");
                    } else if inline_str {
                        let t = s.strings.get(id);
                        let space = t.starts_with(char::is_whitespace) || t.ends_with(char::is_whitespace);
                        buf.push_str(if space { "<is><t xml:space=\"preserve\">" } else { "<is><t>" });
                        escape_text(t, &mut buf);
                        buf.push_str("</t></is>");
                    } else {
                        let _ = write!(buf, "<v>{}</v>", sst.index(s, si, id, include_shared));
                    }
                }
                Kind::Bool => buf.push_str(if v.as_bool().unwrap() { "<v>1</v>" } else { "<v>0</v>" }),
                Kind::Error => {
                    buf.push_str("<v>");
                    buf.push_str(v.as_error().unwrap());
                    buf.push_str("</v>");
                }
            }
            buf.push_str("</c>");
        }
        if buf.len() == header_len && meta == Default::default() {
            buf.truncate(row_start);
        } else {
            buf.push_str("</row>");
        }
        if buf.len() > (1 << 16) - 4096 {
            w.write_all(buf.as_bytes())?;
            buf.clear();
        }
    }
    buf.push_str("</sheetData>");
    w.write_all(buf.as_bytes())
}

fn write_formula(s: &Sheet, r: u32, c: u32, fid: u32, out: &mut String) {
    let f = &s.formulas[fid as usize];
    match &f.kind {
        FKind::Normal | FKind::Other => {
            out.push_str("<f");
            out.push_str(&f.attrs);
            out.push('>');
            escape_text(&f.text, out);
            out.push_str("</f>");
        }
        FKind::Array { ref_ } => {
            out.push_str("<f t=\"array\" ref=\"");
            escape_attr(ref_, out);
            out.push('"');
            out.push_str(&f.attrs);
            out.push('>');
            escape_text(&f.text, out);
            out.push_str("</f>");
        }
        FKind::Shared { si, ref_: Some(rf) } => {
            let _ = write!(out, "<f t=\"shared\" ref=\"{rf}\" si=\"{si}\"{}>", f.attrs);
            escape_text(&f.text, out);
            out.push_str("</f>");
        }
        FKind::Shared { si, ref_: None } => {
            // Child: valid only while its master is still in place.
            let master_ok = s.shared_masters.get(si).is_some_and(|&m| {
                let FKind::Shared { ref_: Some(rf), .. } = &s.formulas[m as usize].kind else { return false };
                refshift::parse_cell_ref(rf.split(':').next().unwrap_or("")).is_some_and(|(mr, mc)| s.formula_id(mr, mc) == Some(m))
            });
            if master_ok {
                let _ = write!(out, "<f t=\"shared\" si=\"{si}\"{}/>", f.attrs);
            } else if let Some(text) = s.formula_text(r, c) {
                out.push_str("<f>");
                escape_text(&text, out);
                out.push_str("</f>");
            }
        }
    }
}

/// Used range as an A1 ref ("A1" for an empty sheet).
fn dimension(s: &Sheet) -> String {
    let mut out = String::new();
    match s.used_rect() {
        Some(r) => range_ref(&r, &mut out),
        None => out.push_str("A1"),
    }
    out
}

fn cols_xml(s: &Sheet) -> String {
    let n = s.col_count();
    let mut out = String::new();
    let mut c = 0;
    while c < n {
        let m = s.col_meta(c).copied().unwrap_or_default();
        let mut end = c;
        while end + 1 < n && s.col_meta(end + 1).copied().unwrap_or_default() == m {
            end += 1;
        }
        if m != ColMeta::default() {
            let _ = write!(out, "<col min=\"{}\" max=\"{}\"", c + 1, end + 1);
            let _ = write!(out, " width=\"{}\"", m.width.unwrap_or(s.default_col_width));
            if let Some(st) = m.style {
                let _ = write!(out, " style=\"{st}\"");
            }
            if m.hidden {
                out.push_str(" hidden=\"1\"");
            }
            if m.custom_width || m.width.is_some() {
                out.push_str(" customWidth=\"1\"");
            }
            out.push_str(s.raw(m.extra));
            out.push_str("/>");
        }
        c = end + 1;
    }
    if out.is_empty() { out } else { format!("<cols>{out}</cols>") }
}

fn merges_xml(merges: &[Rect]) -> String {
    if merges.is_empty() {
        return String::new();
    }
    let mut out = format!("<mergeCells count=\"{}\">", merges.len());
    for m in merges {
        out.push_str("<mergeCell ref=\"");
        range_ref(m, &mut out);
        out.push_str("\"/>");
    }
    out.push_str("</mergeCells>");
    out
}

fn pane_xml(rows: u32, cols: u32) -> String {
    if rows == 0 && cols == 0 {
        return String::new();
    }
    let mut tl = String::new();
    cell_ref(rows, cols, &mut tl);
    let pane = match (rows > 0, cols > 0) {
        (true, true) => "bottomRight",
        (true, false) => "bottomLeft",
        _ => "topRight",
    };
    let mut s = String::from("<pane");
    if cols > 0 {
        let _ = write!(s, " xSplit=\"{cols}\"");
    }
    if rows > 0 {
        let _ = write!(s, " ySplit=\"{rows}\"");
    }
    let _ = write!(s, " topLeftCell=\"{tl}\" activePane=\"{pane}\" state=\"frozen\"/><selection pane=\"{pane}\"/>");
    s
}

/// Original freeze panes and merges declared in a sheet's raw XML.
fn original_freeze(prefix: &[u8]) -> (u32, u32) {
    let mut r = Reader::from_reader(prefix);
    loop {
        match r.read_event() {
            Ok(Event::Start(e) | Event::Empty(e)) if local(e.name().as_ref()) == b"pane" => {
                if matches!(attr(&e, b"state").as_deref(), Some("frozen") | Some("frozenSplit")) {
                    return (xml::attr_f32(&e, b"ySplit").unwrap_or(0.0) as u32, xml::attr_f32(&e, b"xSplit").unwrap_or(0.0) as u32);
                }
                return (0, 0);
            }
            Ok(Event::Eof) | Err(_) => return (0, 0),
            _ => {}
        }
    }
}

fn original_merges(suffix: &[u8]) -> Vec<Rect> {
    let mut out = Vec::new();
    let mut r = Reader::from_reader(suffix);
    loop {
        match r.read_event() {
            Ok(Event::Start(e) | Event::Empty(e)) if local(e.name().as_ref()) == b"mergeCell" => {
                if let Some((r0, c0, r1, c1)) = attr(&e, b"ref").and_then(|v| refshift::parse_range_ref(&v)) {
                    out.push(Rect { r0, c0, r1, c1 });
                }
            }
            Ok(Event::Eof) | Err(_) => return out,
            _ => {}
        }
    }
}

/// Replace `<dimension>`, `<cols>` and (if changed) the frozen pane in a sheet prefix.
fn rewrite_prefix(prefix: &[u8], s: &Sheet) -> Vec<u8> {
    let freeze_changed = original_freeze(prefix) != (s.grid.freeze_rows, s.grid.freeze_cols);
    let mut out = Vec::with_capacity(prefix.len() + 256);
    let mut r = Reader::from_reader(prefix);
    r.config_mut().trim_text(false);
    r.config_mut().check_end_names = false;
    let mut copied = 0;
    let mut skip_until: Option<Vec<u8>> = None; // skipping an element's content
    let mut in_first_view = false;
    let mut seen_view = false;
    let mut wrote_pane = false;
    loop {
        let before = r.buffer_position() as usize;
        let ev = match r.read_event() {
            Ok(Event::Eof) | Err(_) => break,
            Ok(e) => e,
        };
        let after = r.buffer_position() as usize;
        if let Some(name) = &skip_until {
            if let Event::End(e) = &ev
                && local(e.name().as_ref()) == name.as_slice()
            {
                skip_until = None;
                copied = after;
            }
            continue;
        }
        match &ev {
            Event::Empty(e) | Event::Start(e) => {
                let is_start = matches!(ev, Event::Start(_));
                match local(e.name().as_ref()) {
                    b"dimension" => {
                        out.extend_from_slice(&prefix[copied..before]);
                        out.extend_from_slice(format!("<dimension ref=\"{}\"/>", dimension(s)).as_bytes());
                        copied = after;
                        if is_start {
                            skip_until = Some(b"dimension".to_vec());
                        }
                    }
                    b"cols" => {
                        out.extend_from_slice(&prefix[copied..before]);
                        copied = after;
                        if is_start {
                            skip_until = Some(b"cols".to_vec());
                        }
                    }
                    b"sheetView" if freeze_changed && !seen_view => {
                        seen_view = true;
                        if is_start {
                            in_first_view = true;
                        } else {
                            // <sheetView …/> → open it to hold the pane.
                            out.extend_from_slice(&prefix[copied..before]);
                            let raw = &prefix[before..after];
                            out.extend_from_slice(&raw[..raw.len() - 2]);
                            out.push(b'>');
                            out.extend_from_slice(pane_xml(s.grid.freeze_rows, s.grid.freeze_cols).as_bytes());
                            out.extend_from_slice(b"</sheetView>");
                            copied = after;
                        }
                    }
                    b"pane" | b"selection" if in_first_view => {
                        out.extend_from_slice(&prefix[copied..before]);
                        if !wrote_pane {
                            out.extend_from_slice(pane_xml(s.grid.freeze_rows, s.grid.freeze_cols).as_bytes());
                            wrote_pane = true;
                        }
                        copied = after;
                        if is_start {
                            skip_until = Some(local(e.name().as_ref()).to_vec());
                        }
                    }
                    _ => {}
                }
            }
            Event::End(e) if in_first_view && local(e.name().as_ref()) == b"sheetView" => {
                in_first_view = false;
                if !wrote_pane {
                    out.extend_from_slice(&prefix[copied..before]);
                    out.extend_from_slice(pane_xml(s.grid.freeze_rows, s.grid.freeze_cols).as_bytes());
                    copied = before;
                    wrote_pane = true;
                }
            }
            _ => {}
        }
    }
    out.extend_from_slice(&prefix[copied..]);
    out.extend_from_slice(cols_xml(s).as_bytes());
    out
}

const AFTER_MERGES: [&[u8]; 22] = [
    b"phoneticPr",
    b"conditionalFormatting",
    b"dataValidations",
    b"hyperlinks",
    b"printOptions",
    b"pageMargins",
    b"pageSetup",
    b"headerFooter",
    b"rowBreaks",
    b"colBreaks",
    b"customProperties",
    b"cellWatches",
    b"ignoredErrors",
    b"smartTags",
    b"drawing",
    b"legacyDrawing",
    b"legacyDrawingHF",
    b"picture",
    b"oleObjects",
    b"controls",
    b"webPublishItems",
    b"tableParts",
];

/// Replace/insert `<mergeCells>` in a sheet suffix if the merges changed.
fn rewrite_suffix(suffix: &[u8], s: &Sheet) -> Vec<u8> {
    if original_merges(suffix) == *s.grid.merges {
        return suffix.to_vec();
    }
    let new = merges_xml(&s.grid.merges);
    let mut r = Reader::from_reader(suffix);
    r.config_mut().check_end_names = false;
    let mut insert_at = None;
    loop {
        let before = r.buffer_position() as usize;
        match r.read_event() {
            Ok(Event::Start(e) | Event::Empty(e)) => {
                let n = local(e.name().as_ref()).to_vec();
                if n == b"mergeCells" {
                    let end = if let Ok(Event::Start(_)) = Reader::from_reader(&suffix[before..]).read_event() {
                        let _ = r.read_to_end(e.name());
                        r.buffer_position() as usize
                    } else {
                        r.buffer_position() as usize
                    };
                    let mut out = suffix[..before].to_vec();
                    out.extend_from_slice(new.as_bytes());
                    out.extend_from_slice(&suffix[end..]);
                    return out;
                }
                if insert_at.is_none() && (AFTER_MERGES.contains(&n.as_slice()) || n == b"extLst") {
                    insert_at = Some(before);
                }
            }
            Ok(Event::End(e)) if local(e.name().as_ref()) == b"worksheet" => {
                insert_at.get_or_insert(before);
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    let at = insert_at.unwrap_or(suffix.len());
    let mut out = suffix[..at].to_vec();
    out.extend_from_slice(new.as_bytes());
    out.extend_from_slice(&suffix[at..]);
    out
}

fn fresh_prefix(s: &Sheet, selected: bool) -> Vec<u8> {
    let mut p = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<worksheet xmlns=\"{NS_MAIN}\" xmlns:r=\"{NS_REL}\"><dimension ref=\"A1\"/><sheetViews><sheetView workbookViewId=\"0\"{}/></sheetViews><sheetFormatPr defaultRowHeight=\"{}\"/>",
        if selected { " tabSelected=\"1\"" } else { "" },
        s.default_row_height
    );
    if s.default_col_width != 9.140625 {
        p = p.replace("<sheetFormatPr ", &format!("<sheetFormatPr defaultColWidth=\"{}\" ", s.default_col_width));
    }
    p.into_bytes()
}

fn write_sheet<W: Write>(
    w: &mut W,
    wb: &Workbook,
    si: usize,
    prefix: &[u8],
    suffix: &[u8],
    sst: &SstPlan,
    include_shared: bool,
) -> Result<()> {
    let s = &wb.sheets[si];
    w.write_all(&rewrite_prefix(prefix, s))?;
    write_sheet_data(w, wb, si, sst, include_shared)?;
    w.write_all(&rewrite_suffix(suffix, s))
}

// ---- structural log replay ------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum PartKind {
    Sheet,
    Workbook,
    Table,
    Comments,
    Chart,
    Drawing,
    Vml,
    Pivot,
}

struct LogRules<'a> {
    kind: PartKind,
    host: &'a str,
    op: &'a LogOp,
}

impl LogRules<'_> {
    fn formula(&self, f: &str) -> Option<String> {
        match self.op {
            LogOp::Struct { sheet, op } => {
                let host = if matches!(self.kind, PartKind::Workbook | PartKind::Chart | PartKind::Pivot) { "" } else { self.host };
                refshift::shift_formula(f, host, sheet, *op)
            }
            LogOp::Rename { old, new } => refshift::rename_sheet_in_formula(f, old, new),
            LogOp::Delete { name } => refshift::delete_sheet_in_formula(f, name),
        }
    }

    fn on_host(&self) -> Option<StructOp> {
        match self.op {
            LogOp::Struct { sheet, op } if refshift::sheet_names_eq(sheet, self.host) => Some(*op),
            _ => None,
        }
    }
}

impl Rules for LogRules<'_> {
    fn attr(&mut self, elem: &[u8], a: &[u8], v: &str) -> Option<String> {
        if let LogOp::Rename { old, new } = self.op {
            if (elem == b"worksheetSource" && a == b"sheet") && refshift::sheet_names_eq(v, old) {
                return Some(new.clone());
            }
            if elem == b"hyperlink" && a == b"location" {
                return self.formula(v);
            }
            return None;
        }
        let op = self.on_host()?;
        let sqref_attr = match self.kind {
            PartKind::Sheet => matches!(
                (elem, a),
                (b"conditionalFormatting", b"sqref")
                    | (b"dataValidation", b"sqref")
                    | (b"hyperlink", b"ref")
                    | (b"autoFilter", b"ref")
                    | (b"protectedRange", b"sqref")
                    | (b"ignoredError", b"sqref")
                    | (b"selection", b"sqref")
                    | (b"selection", b"activeCell")
                    | (b"pane", b"topLeftCell")
                    | (b"sortState", b"ref")
                    | (b"sortCondition", b"ref")
                    | (b"dimension", b"ref")
            ),
            PartKind::Table => matches!(a, b"ref") && matches!(elem, b"table" | b"autoFilter" | b"sortState" | b"sortCondition"),
            PartKind::Comments => elem == b"comment" && a == b"ref",
            _ => false,
        };
        if !sqref_attr {
            return None;
        }
        match refshift::shift_sqref(v, op) {
            Some(s) if s.is_empty() => Some(if matches!(a, b"activeCell" | b"topLeftCell") { "A1".into() } else { String::new() }),
            other => other,
        }
    }

    fn text(&mut self, elem: &[u8], t: &str) -> Option<String> {
        let is_formula = match self.kind {
            PartKind::Sheet => matches!(elem, b"formula" | b"formula1" | b"formula2" | b"f"),
            PartKind::Workbook => elem == b"definedName",
            PartKind::Table => matches!(elem, b"calculatedColumnFormula" | b"totalsRowFormula"),
            PartKind::Chart => elem == b"f",
            _ => false,
        };
        if is_formula {
            return self.formula(t);
        }
        let op = self.on_host()?;
        match (self.kind, elem) {
            (PartKind::Sheet, b"sqref") => refshift::shift_sqref(t, op),
            (PartKind::Drawing, b"row") | (PartKind::Vml, b"Row") if op.axis == Axis::Rows => shift_anchor(t, op),
            (PartKind::Drawing, b"col") | (PartKind::Vml, b"Column") if op.axis == Axis::Cols => shift_anchor(t, op),
            _ => None,
        }
    }
}

fn shift_anchor(t: &str, op: StructOp) -> Option<String> {
    let v: u32 = t.trim().parse().ok()?;
    let nv = if op.insert {
        if v >= op.at { v + op.count } else { v }
    } else if v >= op.at + op.count {
        v - op.count
    } else if v >= op.at {
        op.at
    } else {
        v
    };
    (nv != v).then(|| nv.to_string())
}

/// Replay the whole log over one part. `host` is the owning sheet's original name.
fn replay(xml: &[u8], kind: PartKind, host: Option<&str>, log: &[LogOp]) -> Option<Vec<u8>> {
    let mut cur: Option<Vec<u8>> = None;
    let mut host = host.unwrap_or("").to_string();
    for op in log {
        let src = cur.as_deref().unwrap_or(xml);
        let mut rules = LogRules { kind, host: &host, op };
        if let Some(n) = xmlrw::rewrite(src, &mut rules) {
            cur = Some(n);
        }
        if let LogOp::Rename { old, new } = op
            && refshift::sheet_names_eq(&host, old)
        {
            host = new.clone();
        }
    }
    cur
}

/// Name a sheet had when the file was opened, by walking renames backwards.
fn original_name(current: &str, log: &[LogOp]) -> String {
    let mut name = current.to_string();
    for op in log.iter().rev() {
        if let LogOp::Rename { old, new } = op
            && refshift::sheet_names_eq(&name, new)
        {
            name = old.clone();
        }
    }
    name
}

// ---- package save ---------------------------------------------------------------

fn save_package(wb: &Workbook, pkg: &Package, path: &Path) -> Result<()> {
    let mut zin = open_zip(&pkg.file)?;
    let log: &[LogOp] = &wb.log;
    let wb_rels = parse_rels(&pkg.wb_rels);

    // Original sheet order from workbook.xml (names + rel ids).
    let orig_sheets = sheet_elements(&pkg.wb_xml);
    let orig_name_by_rid: HashMap<&str, &str> = orig_sheets.iter().map(|e| (e.rid.as_str(), e.name.as_str())).collect();

    // Sheets we keep, by original part path.
    let mut kept_paths: HashSet<String> = HashSet::new();
    for s in &wb.sheets {
        if let Some(p) = &s.part {
            kept_paths.insert(p.path.clone());
        }
    }
    let deleted_paths: Vec<String> = wb_rels
        .iter()
        .filter(|r| rel_is(&r.kind, "worksheet"))
        .map(|r| resolve(&pkg.wb_path, &r.target))
        .filter(|p| !kept_paths.contains(p) && find_entry(&zin, p).is_some())
        .collect();

    // Replay the log on every loaded sheet's prefix/suffix.
    let mut sheet_xml: Vec<Option<(Vec<u8>, Vec<u8>)>> = vec![None; wb.sheets.len()];
    let mut write_sheet_flags = vec![false; wb.sheets.len()];
    for (i, s) in wb.sheets.iter().enumerate() {
        match &s.part {
            Some(p) => {
                let host = orig_name_by_rid.get(p.rel_id.as_str()).map(|s| s.to_string()).unwrap_or_else(|| original_name(&s.name, log));
                let pre = replay(&p.prefix, PartKind::Sheet, Some(&host), log);
                let suf = replay(&p.suffix, PartKind::Sheet, Some(&host), log);
                let changed = pre.is_some() || suf.is_some();
                write_sheet_flags[i] = s.dirty || changed;
                sheet_xml[i] = Some((pre.unwrap_or_else(|| p.prefix.clone()), suf.unwrap_or_else(|| p.suffix.clone())));
            }
            None => {
                write_sheet_flags[i] = true;
                sheet_xml[i] = Some((fresh_prefix(s, i == wb.active), b"</worksheet>".to_vec()));
            }
        }
    }
    let any_sheet_written = write_sheet_flags.iter().any(|&b| b);

    // Paths for new sheets.
    let mut used_paths: HashSet<String> = zin.file_names().map(str::to_string).collect();
    let mut max_rid = wb_rels.iter().filter_map(|r| r.id.strip_prefix("rId")?.parse::<u32>().ok()).max().unwrap_or(0);
    let mut max_sheet_id = orig_sheets.iter().map(|e| e.sheet_id).max().unwrap_or(0);
    let mut new_parts: Vec<(usize, String, String, u32)> = Vec::new(); // (sheet index, path, rid, sheetId)
    for (i, s) in wb.sheets.iter().enumerate() {
        if s.part.is_none() {
            let mut n = 1;
            let mut p = format!("xl/worksheets/sheet{n}.xml");
            while used_paths.contains(&p) {
                n += 1;
                p = format!("xl/worksheets/sheet{n}.xml");
            }
            used_paths.insert(p.clone());
            max_rid += 1;
            max_sheet_id += 1;
            new_parts.push((i, p, format!("rId{max_rid}"), max_sheet_id));
        }
    }

    // Shared strings.
    let written: Vec<usize> = (0..wb.sheets.len()).filter(|&i| write_sheet_flags[i]).collect();
    let sst = if pkg.sst_path.is_some() { SstPlan::new(wb, pkg.sst_file_count as u32, &written, false) } else { SstPlan::inline() };
    let need_new_sst_part = false;
    let sst_path = pkg.sst_path.clone().unwrap_or_else(|| "xl/sharedStrings.xml".into());
    let need_styles_part = wb.styles.is_modified() && pkg.styles_path.is_none();
    let styles_path = pkg.styles_path.clone().unwrap_or_else(|| "xl/styles.xml".into());

    // calcChain goes stale when cells or sheets change; Excel rebuilds it.
    let calc_chain: Option<String> = wb_rels
        .iter()
        .find(|r| rel_is(&r.kind, "calcChain"))
        .map(|r| resolve(&pkg.wb_path, &r.target))
        .filter(|_| any_sheet_written || !deleted_paths.is_empty());

    let recalc = wb.edited && wb.has_formulas() && any_sheet_written;
    let book_changed =
        pkg.book_dirty || !new_parts.is_empty() || !deleted_paths.is_empty() || log.iter().any(|o| !matches!(o, LogOp::Struct { .. }));

    // workbook.xml
    let wb_xml_new = if book_changed || recalc || !log.is_empty() {
        let mut x = if book_changed { rebuild_sheets(wb, pkg, &orig_sheets, &new_parts) } else { pkg.wb_xml.clone() };
        if let Some(n) = replay(&x, PartKind::Workbook, None, log) {
            x = n;
        }
        if recalc {
            x = set_full_calc(&x);
        }
        Some(x)
    } else {
        None
    };

    // workbook.xml.rels
    let rels_new = if !new_parts.is_empty() || !deleted_paths.is_empty() || calc_chain.is_some() || need_new_sst_part || need_styles_part {
        let mut dropped: Vec<&str> = deleted_paths.iter().map(String::as_str).collect();
        if let Some(c) = &calc_chain {
            dropped.push(c);
        }
        let mut add: Vec<(String, &str, String)> =
            new_parts.iter().map(|(_, p, rid, _)| (rid.clone(), REL_WS, rel_target(&pkg.wb_path, p))).collect();
        if need_new_sst_part {
            max_rid += 1;
            add.push((format!("rId{max_rid}"), REL_SST, rel_target(&pkg.wb_path, &sst_path)));
        }
        if need_styles_part {
            max_rid += 1;
            add.push((format!("rId{max_rid}"), REL_STYLES, rel_target(&pkg.wb_path, &styles_path)));
        }
        Some(edit_rels(&pkg.wb_rels, &pkg.wb_path, &dropped, &add))
    } else {
        None
    };

    // [Content_Types].xml
    let ct_new = if !new_parts.is_empty() || !deleted_paths.is_empty() || calc_chain.is_some() || need_new_sst_part || need_styles_part {
        let mut dropped: Vec<String> = deleted_paths.clone();
        if let Some(c) = &calc_chain {
            dropped.push(c.clone());
        }
        let mut add: Vec<(String, &str)> = new_parts.iter().map(|(_, p, _, _)| (p.clone(), CT_WS)).collect();
        if need_new_sst_part {
            add.push((sst_path.clone(), CT_SST));
        }
        if need_styles_part {
            add.push((styles_path.clone(), CT_STYLES));
        }
        Some(edit_content_types(&pkg.content_types, &dropped, &add))
    } else {
        None
    };

    // Parts owned by sheets (tables, comments, drawings), for log replay.
    let mut owner: HashMap<String, (PartKind, String)> = HashMap::new();
    if !log.is_empty() {
        for e in &orig_sheets {
            let Some(rel) = wb_rels.iter().find(|r| r.id == e.rid) else { continue };
            let sheet_path = resolve(&pkg.wb_path, &rel.target);
            let Ok(rels_xml) = super::read_entry(&mut zin, &rels_path(&sheet_path)) else { continue };
            for r in parse_rels(&rels_xml) {
                if r.external {
                    continue;
                }
                let kind = if rel_is(&r.kind, "table") {
                    PartKind::Table
                } else if rel_is(&r.kind, "comments") {
                    PartKind::Comments
                } else if rel_is(&r.kind, "drawing") {
                    PartKind::Drawing
                } else if rel_is(&r.kind, "vmlDrawing") {
                    PartKind::Vml
                } else {
                    continue;
                };
                owner.insert(resolve(&sheet_path, &r.target), (kind, e.name.clone()));
            }
        }
    }

    // ---- write ---------------------------------------------------------------
    let out = File::create(path)?;
    let mut zw = ZipWriter::new(BufWriter::with_capacity(1 << 20, out));
    let sheet_by_path: HashMap<&str, usize> =
        wb.sheets.iter().enumerate().filter_map(|(i, s)| s.part.as_ref().map(|p| (p.path.as_str(), i))).collect();
    let deleted_rels: HashSet<String> = deleted_paths.iter().map(|p| rels_path(p)).collect();

    let names: Vec<String> = zin.file_names().map(str::to_string).collect();
    // Keep the original entry order.
    let mut order: Vec<(usize, String)> = names.iter().map(|n| (zin.index_for_name(n).unwrap_or(usize::MAX), n.clone())).collect();
    order.sort();
    for (idx, name) in order {
        if deleted_paths.contains(&name) || deleted_rels.contains(&name) || calc_chain.as_deref() == Some(name.as_str()) {
            continue;
        }
        let replace: Option<Vec<u8>> = if name == pkg.wb_path {
            wb_xml_new.clone()
        } else if name == pkg.wb_rels_path {
            rels_new.clone()
        } else if name == "[Content_Types].xml" {
            ct_new.clone()
        } else if pkg.styles_path.as_deref() == Some(name.as_str()) && wb.styles.is_modified() {
            Some(wb.styles.to_xml())
        } else if pkg.sst_path.as_deref() == Some(name.as_str()) && !sst.order.is_empty() {
            Some(append_sst(&super::read_entry(&mut zin, &name)?, &sst, wb))
        } else if let Some(&si) = sheet_by_path.get(name.as_str()) {
            if write_sheet_flags[si] {
                let (pre, suf) = sheet_xml[si].as_ref().unwrap();
                let big = wb.sheets[si].row_count() as u64 * wb.sheets[si].col_count() as u64 > 2_000_000;
                zw.start_file(name.as_str(), opts(big)).map_err(std::io::Error::other)?;
                let mut w = BufWriter::with_capacity(1 << 18, &mut zw);
                write_sheet(&mut w, wb, si, pre, suf, &sst, false)?;
                w.flush()?;
                continue;
            }
            None
        } else if let Some((kind, host)) = owner.get(&name) {
            replay(&super::read_entry(&mut zin, &name)?, *kind, Some(host), log)
        } else if !log.is_empty()
            && (name.starts_with("xl/charts/chart") || name.contains("pivotCacheDefinition"))
            && name.ends_with(".xml")
        {
            let kind = if name.contains("pivot") { PartKind::Pivot } else { PartKind::Chart };
            replay(&super::read_entry(&mut zin, &name)?, kind, None, log)
        } else {
            None
        };
        match replace {
            Some(bytes) => {
                zw.start_file(name.as_str(), opts(false)).map_err(std::io::Error::other)?;
                zw.write_all(&bytes)?;
            }
            None => {
                let f = zin.by_index_raw(idx).map_err(std::io::Error::other)?;
                zw.raw_copy_file(f).map_err(std::io::Error::other)?;
            }
        }
    }
    for (si, p, _, _) in &new_parts {
        let (pre, suf) = sheet_xml[*si].as_ref().unwrap();
        zw.start_file(p.as_str(), opts(false)).map_err(std::io::Error::other)?;
        let mut w = BufWriter::with_capacity(1 << 18, &mut zw);
        write_sheet(&mut w, wb, *si, pre, suf, &sst, false)?;
        w.flush()?;
    }
    if need_new_sst_part {
        let mut x = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<sst xmlns=\"{NS_MAIN}\" count=\"{0}\" uniqueCount=\"{0}\">",
            sst.next
        );
        sst.entries_xml(wb, &mut x);
        x.push_str("</sst>");
        zw.start_file(sst_path.as_str(), opts(false)).map_err(std::io::Error::other)?;
        zw.write_all(x.as_bytes())?;
    }
    if need_styles_part {
        zw.start_file(styles_path.as_str(), opts(false)).map_err(std::io::Error::other)?;
        zw.write_all(&wb.styles.to_xml())?;
    }
    let mut inner = zw.finish().map_err(std::io::Error::other)?;
    inner.flush()?;
    Ok(())
}

fn rel_target(from_part: &str, target_path: &str) -> String {
    let dir = from_part.rsplit_once('/').map_or("", |(d, _)| d);
    if !dir.is_empty()
        && let Some(rest) = target_path.strip_prefix(&format!("{dir}/"))
    {
        return rest.to_string();
    }
    format!("/{target_path}")
}

struct SheetEl {
    name: String,
    rid: String,
    sheet_id: u32,
    raw: String,
    worksheet_slot: bool,
}

fn sheet_elements(wb_xml: &[u8]) -> Vec<SheetEl> {
    let mut out = Vec::new();
    let mut r = Reader::from_reader(wb_xml);
    loop {
        let before = r.buffer_position() as usize;
        match r.read_event() {
            Ok(Event::Start(e) | Event::Empty(e)) if local(e.name().as_ref()) == b"sheet" => {
                out.push(SheetEl {
                    name: attr(&e, b"name").unwrap_or_default(),
                    rid: attr(&e, b"id").unwrap_or_default(),
                    sheet_id: xml::attr_u32(&e, b"sheetId").unwrap_or(0),
                    raw: String::from_utf8_lossy(&wb_xml[before..r.buffer_position() as usize]).into_owned(),
                    worksheet_slot: false,
                });
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    out
}

/// Rebuild `<sheets>` for the current sheet list, and remap `localSheetId`.
fn rebuild_sheets(wb: &Workbook, pkg: &Package, orig: &[SheetEl], new_parts: &[(usize, String, String, u32)]) -> Vec<u8> {
    let src = &pkg.wb_xml;
    let ours: HashSet<&str> = wb.sheets.iter().filter_map(|s| s.part.as_ref().map(|p| p.rel_id.as_str())).collect();
    let rels = parse_rels(&pkg.wb_rels);
    let is_worksheet = |rid: &str| rels.iter().any(|r| r.id == rid && rel_is(&r.kind, "worksheet"));
    let orig: Vec<SheetEl> = orig
        .iter()
        .map(|e| SheetEl {
            worksheet_slot: is_worksheet(&e.rid),
            name: e.name.clone(),
            rid: e.rid.clone(),
            sheet_id: e.sheet_id,
            raw: e.raw.clone(),
        })
        .collect();
    let r_prefix = orig
        .first()
        .and_then(|e| e.raw.split_whitespace().find(|t| t.contains(":id=")).and_then(|t| t.split(':').next()))
        .unwrap_or("r")
        .to_string();

    let mut new_order_rids: Vec<String> = Vec::new();
    let mut xml_sheets = String::from("<sheets>");
    // Non-worksheet entries (chartsheets) keep their original index.
    let mut ws_iter = wb.sheets.iter().enumerate();
    for (i, e) in orig.iter().enumerate() {
        if !e.worksheet_slot {
            xml_sheets.push_str(&e.raw);
            new_order_rids.push(e.rid.clone());
            continue;
        }
        let _ = i;
        if let Some((si, s)) = ws_iter.next() {
            push_sheet_el(&mut xml_sheets, s, si, &orig, new_parts, &r_prefix, &mut new_order_rids);
        }
    }
    for (si, s) in ws_iter {
        push_sheet_el(&mut xml_sheets, s, si, &orig, new_parts, &r_prefix, &mut new_order_rids);
    }
    xml_sheets.push_str("</sheets>");
    let _ = ours;

    // Old index → new index for localSheetId.
    let remap: Vec<Option<usize>> = orig.iter().map(|e| new_order_rids.iter().position(|r| *r == e.rid)).collect();

    let sec = xml::find_section(src, b"sheets", b"sheet");
    let mut out = Vec::with_capacity(src.len() + 256);
    match sec {
        Some(sec) => {
            out.extend_from_slice(&src[..sec.open.start]);
            out.extend_from_slice(xml_sheets.as_bytes());
            out.extend_from_slice(&src[sec.close.end.max(sec.open.end)..]);
        }
        None => out.extend_from_slice(src),
    }
    // Drop names scoped to deleted sheets; renumber the rest; fix activeTab.
    struct Remap<'a>(&'a [Option<usize>], usize);
    impl Rules for Remap<'_> {
        fn attr(&mut self, e: &[u8], a: &[u8], v: &str) -> Option<String> {
            match (e, a) {
                (b"definedName", b"localSheetId") => {
                    let i: usize = v.parse().ok()?;
                    Some(self.0.get(i).copied().flatten().map_or_else(|| "4294967295".into(), |n| n.to_string()))
                }
                (b"workbookView", b"activeTab") => Some(self.1.to_string()),
                (b"workbookView", b"firstSheet") => Some("0".into()),
                _ => None,
            }
        }
    }
    let active_pos = wb.sheets.get(wb.active).and_then(|s| {
        let rid = s.part.as_ref().map(|p| p.rel_id.clone()).or_else(|| new_parts.iter().find(|n| n.0 == wb.active).map(|n| n.2.clone()))?;
        new_order_rids.iter().position(|r| *r == rid)
    });
    let out = xmlrw::rewrite(&out, &mut Remap(&remap, active_pos.unwrap_or(0))).unwrap_or(out);
    remove_orphan_names(&out)
}

fn push_sheet_el(
    out: &mut String,
    s: &Sheet,
    si: usize,
    orig: &[SheetEl],
    new_parts: &[(usize, String, String, u32)],
    r_prefix: &str,
    order: &mut Vec<String>,
) {
    let (rid, sheet_id) = match &s.part {
        Some(p) => (p.rel_id.clone(), orig.iter().find(|e| e.rid == p.rel_id).map_or(p.sheet_id, |e| e.sheet_id)),
        None => {
            let n = new_parts.iter().find(|n| n.0 == si).unwrap();
            (n.2.clone(), n.3)
        }
    };
    out.push_str("<sheet name=\"");
    escape_attr(&s.name, out);
    let _ = write!(out, "\" sheetId=\"{sheet_id}\"");
    match s.visibility {
        Visibility::Hidden => out.push_str(" state=\"hidden\""),
        Visibility::VeryHidden => out.push_str(" state=\"veryHidden\""),
        Visibility::Visible => {}
    }
    let _ = write!(out, " {r_prefix}:id=\"{rid}\"/>");
    order.push(rid);
}

/// Remove `<definedName localSheetId="4294967295">` markers left by `rebuild_sheets`.
fn remove_orphan_names(xml_bytes: &[u8]) -> Vec<u8> {
    let marker = b"localSheetId=\"4294967295\"";
    if memchr::memmem::find(xml_bytes, marker).is_none() {
        return xml_bytes.to_vec();
    }
    let mut out = Vec::with_capacity(xml_bytes.len());
    let mut r = Reader::from_reader(xml_bytes);
    let mut copied = 0;
    loop {
        let before = r.buffer_position() as usize;
        match r.read_event() {
            Ok(Event::Start(e))
                if local(e.name().as_ref()) == b"definedName"
                    && memchr::memmem::find(&xml_bytes[before..r.buffer_position() as usize], marker).is_some() =>
            {
                out.extend_from_slice(&xml_bytes[copied..before]);
                let _ = r.read_to_end(e.name());
                copied = r.buffer_position() as usize;
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    out.extend_from_slice(&xml_bytes[copied..]);
    out
}

fn set_full_calc(src: &[u8]) -> Vec<u8> {
    struct R(bool);
    impl Rules for R {
        fn attr(&mut self, e: &[u8], a: &[u8], _v: &str) -> Option<String> {
            if e == b"calcPr" && a == b"fullCalcOnLoad" {
                self.0 = true;
                return Some("1".into());
            }
            None
        }
    }
    let mut r = R(false);
    let out = xmlrw::rewrite(src, &mut r).unwrap_or_else(|| src.to_vec());
    if r.0 {
        return out;
    }
    // Add the attribute to an existing calcPr, or add a calcPr element.
    let mut rd = Reader::from_reader(out.as_slice());
    loop {
        let before = rd.buffer_position() as usize;
        match rd.read_event() {
            Ok(Event::Start(e) | Event::Empty(e)) if local(e.name().as_ref()) == b"calcPr" => {
                let name_end = before + 1 + e.name().as_ref().len();
                let mut o = out[..name_end].to_vec();
                o.extend_from_slice(b" fullCalcOnLoad=\"1\"");
                o.extend_from_slice(&out[name_end..]);
                return o;
            }
            Ok(Event::Start(e) | Event::Empty(e))
                if matches!(
                    local(e.name().as_ref()),
                    b"oleSize"
                        | b"customWorkbookViews"
                        | b"pivotCaches"
                        | b"smartTagPr"
                        | b"smartTagTypes"
                        | b"webPublishing"
                        | b"fileRecoveryPr"
                        | b"webPublishObjects"
                        | b"extLst"
                ) =>
            {
                let mut o = out[..before].to_vec();
                o.extend_from_slice(b"<calcPr fullCalcOnLoad=\"1\"/>");
                o.extend_from_slice(&out[before..]);
                return o;
            }
            Ok(Event::End(e)) if local(e.name().as_ref()) == b"workbook" => {
                let mut o = out[..before].to_vec();
                o.extend_from_slice(b"<calcPr fullCalcOnLoad=\"1\"/>");
                o.extend_from_slice(&out[before..]);
                return o;
            }
            Ok(Event::Eof) | Err(_) => return out,
            _ => {}
        }
    }
}

fn edit_rels(src: &[u8], base_part: &str, drop_targets: &[&str], add: &[(String, &str, String)]) -> Vec<u8> {
    let mut out = Vec::with_capacity(src.len() + 512);
    let mut r = Reader::from_reader(src);
    let mut copied = 0;
    loop {
        let before = r.buffer_position() as usize;
        match r.read_event() {
            Ok(Event::Empty(e)) if local(e.name().as_ref()) == b"Relationship" => {
                let target = attr(&e, b"Target").map(|t| resolve(base_part, &t)).unwrap_or_default();
                if drop_targets.contains(&target.as_str()) {
                    out.extend_from_slice(&src[copied..before]);
                    copied = r.buffer_position() as usize;
                }
            }
            Ok(Event::End(e)) if local(e.name().as_ref()) == b"Relationships" => {
                out.extend_from_slice(&src[copied..before]);
                for (id, kind, target) in add {
                    let mut s = format!("<Relationship Id=\"{id}\" Type=\"{kind}\" Target=\"");
                    escape_attr(target, &mut s);
                    s.push_str("\"/>");
                    out.extend_from_slice(s.as_bytes());
                }
                copied = before;
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    out.extend_from_slice(&src[copied..]);
    out
}

fn edit_content_types(src: &[u8], drop_parts: &[String], add: &[(String, &str)]) -> Vec<u8> {
    let mut out = Vec::with_capacity(src.len() + 512);
    let mut r = Reader::from_reader(src);
    let mut copied = 0;
    loop {
        let before = r.buffer_position() as usize;
        match r.read_event() {
            Ok(Event::Empty(e)) if local(e.name().as_ref()) == b"Override" => {
                let part = attr(&e, b"PartName").unwrap_or_default();
                if drop_parts.iter().any(|p| part.trim_start_matches('/').eq_ignore_ascii_case(p)) {
                    out.extend_from_slice(&src[copied..before]);
                    copied = r.buffer_position() as usize;
                }
            }
            Ok(Event::End(e)) if local(e.name().as_ref()) == b"Types" => {
                out.extend_from_slice(&src[copied..before]);
                for (p, ct) in add {
                    out.extend_from_slice(format!("<Override PartName=\"/{p}\" ContentType=\"{ct}\"/>").as_bytes());
                }
                copied = before;
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    out.extend_from_slice(&src[copied..]);
    out
}

/// Append new `<si>` entries before `</sst>` and update the counts.
fn append_sst(src: &[u8], plan: &SstPlan, wb: &Workbook) -> Vec<u8> {
    let Some(end) = memchr::memmem::rfind(src, b"</").filter(|&i| src[i..].windows(3).any(|w| w == b"sst")) else {
        return src.to_vec();
    };
    let mut entries = String::new();
    plan.entries_xml(wb, &mut entries);
    let mut out = Vec::with_capacity(src.len() + entries.len());
    out.extend_from_slice(&src[..end]);
    out.extend_from_slice(entries.as_bytes());
    out.extend_from_slice(&src[end..]);
    struct Counts(u32);
    impl Rules for Counts {
        fn attr(&mut self, e: &[u8], a: &[u8], v: &str) -> Option<String> {
            if e != b"sst" {
                return None;
            }
            match a {
                b"uniqueCount" => Some(self.0.to_string()),
                b"count" => Some(v.parse::<u32>().unwrap_or(0).max(self.0).to_string()),
                _ => None,
            }
        }
    }
    // Only the root element's attributes matter; the rewrite is cheap for sst sizes.
    xmlrw::rewrite(&out, &mut Counts(plan.next)).unwrap_or(out)
}

// ---- fresh package ------------------------------------------------------------------

fn save_fresh(wb: &Workbook, path: &Path) -> Result<()> {
    let all: Vec<usize> = (0..wb.sheets.len()).collect();
    let sst = SstPlan::new(wb, 0, &all, true);
    let out = File::create(path)?;
    let mut zw = ZipWriter::new(BufWriter::with_capacity(1 << 20, out));
    let o = opts(false);
    let e = std::io::Error::other;

    let mut ct = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/><Override PartName=\"/xl/styles.xml\" ContentType=\"{CT_STYLES}\"/><Override PartName=\"/xl/sharedStrings.xml\" ContentType=\"{CT_SST}\"/>"
    );
    for i in 0..wb.sheets.len() {
        let _ = write!(ct, "<Override PartName=\"/xl/worksheets/sheet{}.xml\" ContentType=\"{CT_WS}\"/>", i + 1);
    }
    ct.push_str("</Types>");
    zw.start_file("[Content_Types].xml", o).map_err(e)?;
    zw.write_all(ct.as_bytes())?;

    zw.start_file("_rels/.rels", o).map_err(e)?;
    zw.write_all(b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/></Relationships>")?;

    let mut wbx =
        format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<workbook xmlns=\"{NS_MAIN}\" xmlns:r=\"{NS_REL}\">");
    if wb.date1904 {
        wbx.push_str("<workbookPr date1904=\"1\"/>");
    }
    let _ = write!(wbx, "<bookViews><workbookView activeTab=\"{}\"/></bookViews><sheets>", wb.active);
    for (i, s) in wb.sheets.iter().enumerate() {
        wbx.push_str("<sheet name=\"");
        escape_attr(&s.name, &mut wbx);
        let _ = write!(wbx, "\" sheetId=\"{}\"", i + 1);
        match s.visibility {
            Visibility::Hidden => wbx.push_str(" state=\"hidden\""),
            Visibility::VeryHidden => wbx.push_str(" state=\"veryHidden\""),
            Visibility::Visible => {}
        }
        let _ = write!(wbx, " r:id=\"rId{}\"/>", i + 3);
    }
    wbx.push_str("</sheets>");
    if wb.has_formulas() {
        wbx.push_str("<calcPr fullCalcOnLoad=\"1\"/>");
    }
    wbx.push_str("</workbook>");
    zw.start_file("xl/workbook.xml", o).map_err(e)?;
    zw.write_all(wbx.as_bytes())?;

    let mut rels = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">",
    );
    let _ = write!(
        rels,
        "<Relationship Id=\"rId1\" Type=\"{REL_STYLES}\" Target=\"styles.xml\"/><Relationship Id=\"rId2\" Type=\"{REL_SST}\" Target=\"sharedStrings.xml\"/>"
    );
    for i in 0..wb.sheets.len() {
        let _ = write!(rels, "<Relationship Id=\"rId{}\" Type=\"{REL_WS}\" Target=\"worksheets/sheet{}.xml\"/>", i + 3, i + 1);
    }
    rels.push_str("</Relationships>");
    zw.start_file("xl/_rels/workbook.xml.rels", o).map_err(e)?;
    zw.write_all(rels.as_bytes())?;

    zw.start_file("xl/styles.xml", o).map_err(e)?;
    zw.write_all(&wb.styles.to_xml())?;

    for i in 0..wb.sheets.len() {
        let s = &wb.sheets[i];
        let big = s.row_count() as u64 * s.col_count() as u64 > 2_000_000;
        zw.start_file(format!("xl/worksheets/sheet{}.xml", i + 1), opts(big)).map_err(e)?;
        let mut w = BufWriter::with_capacity(1 << 18, &mut zw);
        write_sheet(&mut w, wb, i, &fresh_prefix(s, i == wb.active), b"</worksheet>", &sst, true)?;
        w.flush()?;
    }

    let mut x = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<sst xmlns=\"{NS_MAIN}\" count=\"{0}\" uniqueCount=\"{0}\">",
        sst.next
    );
    sst.entries_xml(wb, &mut x);
    x.push_str("</sst>");
    zw.start_file("xl/sharedStrings.xml", o).map_err(e)?;
    zw.write_all(x.as_bytes())?;

    let mut inner = zw.finish().map_err(e)?;
    inner.flush()?;
    Ok(())
}

fn group(n: u32) -> String {
    let d = n.to_string();
    let mut out = String::new();
    for (i, ch) in d.chars().enumerate() {
        if i > 0 && (d.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}
