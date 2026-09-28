//! Import-only formats (xls, xlsb, ods) via calamine. Values only: dates get a
//! date format; formulas keep their cached results. Saving produces a new xlsx.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use calamine::{Data, DataRef, Reader, SheetType, SheetVisible, Sheets, open_workbook_auto};

use crate::cell::Cell;
use crate::sheet::{Sheet, SheetBuilder, Visibility};
use crate::strings::{SheetStrings, StringPool};
use crate::styles::{StyleChange, Styles};
use crate::workbook::{Source, Workbook};
use crate::xlsx::read::LoadCtl;

fn err(e: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::other(e.to_string())
}

pub fn open(path: &std::path::Path, ext: &str) -> std::io::Result<Workbook> {
    let book = open_workbook_auto(path).map_err(err)?;
    let shared = Arc::new(StringPool::new());
    let sheets: Vec<Sheet> = book
        .sheets_metadata()
        .iter()
        .filter(|m| m.typ == SheetType::WorkSheet)
        .map(|m| {
            let mut s = Sheet::new(m.name.clone(), SheetStrings::new(shared.clone()));
            s.visibility = match m.visible {
                SheetVisible::Visible => Visibility::Visible,
                SheetVisible::Hidden => Visibility::Hidden,
                SheetVisible::VeryHidden => Visibility::VeryHidden,
            };
            s.loaded = false;
            s
        })
        .collect();
    if sheets.is_empty() {
        return Err(err("This file has no worksheets."));
    }
    let mut wb = Workbook::new(sheets, Styles::minimal(), Source::Legacy(ext.to_string()));
    wb.active = wb.sheets.iter().position(|s| s.visibility == Visibility::Visible).unwrap_or(0);
    Ok(wb)
}

enum V<'a> {
    Num(f64),
    Date(f64),
    Str(&'a str),
    Bool(bool),
    Err(String),
}

pub fn load(path: &std::path::Path, ctl: &LoadCtl<'_>) -> std::io::Result<()> {
    let mut book = open_workbook_auto(path).map_err(err)?;
    let names: Vec<String> = ctl.wb.lock().unwrap().sheets.iter().map(|s| s.name.clone()).collect();
    let (date_xf, datetime_xf) = {
        let mut wb = ctl.wb.lock().unwrap();
        let d = wb.styles.derive(0, &StyleChange::NumFmt("yyyy-mm-dd".into()));
        let dt = wb.styles.derive(0, &StyleChange::NumFmt("yyyy-mm-dd hh:mm:ss".into()));
        (d, dt)
    };
    let n = names.len();
    for (i, name) in names.iter().enumerate() {
        if ctl.cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        let mut staged: Vec<(u32, u32, Cell, u16)> = Vec::new();
        let apply = |staged: &mut Vec<(u32, u32, Cell, u16)>, texts: &mut Vec<String>| {
            let mut wb = ctl.wb.lock().unwrap();
            let s = &mut wb.sheets[i];
            let mut b = SheetBuilder::new(s);
            for (r, c, mut v, xf) in staged.drain(..) {
                if let Some(t) = v.as_str_id() {
                    v = Cell::string(b.sheet.strings.add(&texts[t as usize]));
                }
                b.put(r, c, v, xf, None);
            }
            texts.clear();
            b.sheet.invalidate_geometry();
        };
        let mut texts: Vec<String> = Vec::new();
        let push = |r: u32, c: u32, v: V<'_>, staged: &mut Vec<(u32, u32, Cell, u16)>, texts: &mut Vec<String>| {
            let (cell, xf) = match v {
                V::Num(n) => (Cell::number(n), 0),
                V::Date(n) => (Cell::number(n), if n.fract() != 0.0 { datetime_xf } else { date_xf }),
                V::Str("") => return,
                V::Str(s) => {
                    texts.push(s.to_string());
                    (Cell::string(texts.len() as u32 - 1), 0)
                }
                V::Bool(b) => (Cell::boolean(b), 0),
                V::Err(e) => (Cell::error_from_str(&e), 0),
            };
            staged.push((r, c, cell, xf));
        };

        if let Sheets::Xlsb(x) = &mut book {
            // Stream: xlsb sheets can be large.
            let mut reader = x.worksheet_cells_reader(name).map_err(err)?;
            while let Some(cell) = reader.next_cell().map_err(err)? {
                let (r, c) = cell.get_position();
                let v = match cell.get_value() {
                    DataRef::Int(n) => V::Num(*n as f64),
                    DataRef::Float(f) => V::Num(*f),
                    DataRef::String(s) => V::Str(s),
                    DataRef::SharedString(s) => V::Str(s),
                    DataRef::Bool(b) => V::Bool(*b),
                    DataRef::DateTime(d) => V::Date(d.as_f64()),
                    DataRef::DateTimeIso(s) | DataRef::DurationIso(s) => V::Str(s),
                    DataRef::Error(e) => V::Err(e.to_string()),
                    DataRef::Empty => continue,
                };
                push(r, c, v, &mut staged, &mut texts);
                if staged.len() >= 64 * 1024 {
                    apply(&mut staged, &mut texts);
                }
            }
        } else {
            let range = book.worksheet_range(name).map_err(err)?;
            let (r0, c0) = range.start().unwrap_or((0, 0));
            for (r, c, d) in range.used_cells() {
                let (r, c) = (r0 + r as u32, c0 + c as u32);
                let v = match d {
                    Data::Int(n) => V::Num(*n as f64),
                    Data::Float(f) => V::Num(*f),
                    Data::String(s) => V::Str(s),
                    Data::Bool(b) => V::Bool(*b),
                    Data::DateTime(d) => V::Date(d.as_f64()),
                    Data::DateTimeIso(s) | Data::DurationIso(s) => V::Str(s),
                    Data::Error(e) => V::Err(e.to_string()),
                    Data::Empty => continue,
                };
                push(r, c, v, &mut staged, &mut texts);
                if staged.len() >= 64 * 1024 {
                    apply(&mut staged, &mut texts);
                }
            }
        }
        apply(&mut staged, &mut texts);
        let mut wb = ctl.wb.lock().unwrap();
        let s = &mut wb.sheets[i];
        s.strings.local.drop_index();
        s.strings.local.shrink();
        s.loaded = true;
        s.dirty = false;
        drop(wb);
        ctl.progress.store(((i + 1) * 1000 / n).min(999) as u32, Ordering::Relaxed);
    }
    ctl.progress.store(1000, Ordering::Relaxed);
    Ok(())
}
