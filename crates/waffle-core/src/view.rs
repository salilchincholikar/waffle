//! What the grid shows: display text, alignment and colours for a block of
//! cells, plus the text used when editing a cell and parsing typed input.

use crate::cell::{Cell, Kind};
use crate::numfmt;
use crate::sheet::Sheet;
use crate::workbook::Workbook;

pub const KIND_EMPTY: u8 = 0;
pub const KIND_NUMBER: u8 = 1;
pub const KIND_TEXT: u8 = 2;
pub const KIND_BOOL: u8 = 3;
pub const KIND_ERROR: u8 = 4;

pub const ALIGN_LEFT: u8 = 0;
pub const ALIGN_CENTER: u8 = 1;
pub const ALIGN_RIGHT: u8 = 2;

pub const FLAG_FORMULA: u8 = 1;
/// Formula without a cached value: text shows the formula itself.
pub const FLAG_UNCALCULATED: u8 = 2;
/// Text has several formatted runs (fetch them with `runs`).
pub const FLAG_RICH: u8 = 4;

/// One visible cell. `text_off/len` index into the fetch text buffer.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct WfCell {
    pub text_off: u32,
    pub text_len: u32,
    pub style: u32,
    /// 0 = use the style's colour, else 0xFF000000 | rgb from the number format.
    pub color: u32,
    pub kind: u8,
    pub align: u8,
    pub flags: u8,
    /// Conditional formatting: 1 bold, 2 italic, 4 strikethrough, 8 underline.
    pub cf_flags: u8,
    /// Conditional fill / font colour (0 = none, else 0xFF000000 | rgb).
    pub cf_fill: u32,
    pub cf_font: u32,
    /// Data bar colour and length (0 = none, else 1…1000 of the cell width).
    pub bar_color: u32,
    pub bar: u32,
}

/// Display text of a cell (number formats applied).
pub fn display(wb: &mut Workbook, si: usize, r: u32, c: u32, out: &mut String) -> (u8, Option<u32>) {
    let csv = wb.is_csv();
    let date1904 = wb.date1904;
    let s = &wb.sheets[si];
    let v = s.get(r, c);
    let xf = s.style(r, c);
    let f = s.formula_id(r, c).is_some();
    match v.kind() {
        Kind::Empty => {
            if f {
                out.push('=');
                if let Some(t) = s.formula_text(r, c) {
                    out.push_str(&t);
                }
                return (KIND_TEXT, None);
            }
            (KIND_EMPTY, None)
        }
        Kind::Number => {
            let n = v.as_number().unwrap();
            if csv {
                crate::delimited::cell_text(s, v, out);
                return (KIND_NUMBER, None);
            }
            let fmt = wb.styles.numfmt(xf);
            (KIND_NUMBER, fmt.format_number(n, date1904, out))
        }
        Kind::Str => {
            let text = s.strings.get(v.as_str_id().unwrap());
            if csv {
                out.push_str(text);
                return (KIND_TEXT, None);
            }
            let fmt = wb.styles.numfmt(xf);
            let text = wb.sheets[si].strings.get(v.as_str_id().unwrap());
            (KIND_TEXT, fmt.format_text(text, out))
        }
        Kind::Bool => {
            out.push_str(if v.as_bool().unwrap() { "TRUE" } else { "FALSE" });
            (KIND_BOOL, None)
        }
        Kind::Error => {
            out.push_str(v.as_error().unwrap());
            (KIND_ERROR, None)
        }
    }
}

/// Fill `cells` (row-major) and `text` for a rectangle of cells.
pub fn fetch(wb: &mut Workbook, si: usize, area: crate::sheet::Rect, cells: &mut Vec<WfCell>, text: &mut String) {
    let (r0, r1, c0, c1) = (area.r0, area.r1, area.c0, area.c1);
    cells.clear();
    text.clear();
    for r in r0..=r1 {
        for c in c0..=c1 {
            let start = text.len();
            let (kind, color) = display(wb, si, r, c, text);
            let s = &wb.sheets[si];
            let xf = s.style(r, c);
            let x = wb.styles.xf(xf);
            let align = match x.halign {
                1 | 4 | 5 | 7 => ALIGN_LEFT,
                2 | 6 => ALIGN_CENTER,
                3 => ALIGN_RIGHT,
                _ => match kind {
                    KIND_NUMBER => ALIGN_RIGHT,
                    KIND_BOOL | KIND_ERROR => ALIGN_CENTER,
                    _ => ALIGN_LEFT,
                },
            };
            let mut flags = 0;
            if !wb.rich.is_empty()
                && let Some(id) = s.get(r, c).as_str_id()
                && wb.rich.contains_key(&id)
                && s.formula_id(r, c).is_none()
            {
                flags |= FLAG_RICH;
            }
            if s.formula_id(r, c).is_some() {
                flags |= FLAG_FORMULA;
                if s.get(r, c).is_empty() {
                    flags |= FLAG_UNCALCULATED;
                }
            }
            let mut cell = WfCell {
                text_off: start as u32,
                text_len: (text.len() - start) as u32,
                style: xf as u32,
                color: color.map_or(0, |c| 0xFF00_0000 | c),
                kind,
                align,
                flags,
                ..Default::default()
            };
            if !wb.sheets[si].grid.tables.is_empty() {
                let s = &wb.sheets[si];
                // Explicit cell fills win over the table style.
                let has_fill = wb.styles.fill(wb.styles.xf(xf).fill).is_some();
                if let Some((fill, font, bold)) = s.grid.tables.iter().find_map(|t| crate::tables::look(t, &wb.styles, r, c)) {
                    if !has_fill {
                        cell.cf_fill = fill.map_or(0, |c| 0xFF00_0000 | c);
                    }
                    // The workbook's default font colour doesn't count as a deliberate choice.
                    let own = wb.styles.font(wb.styles.xf(xf).font).color;
                    if own.is_none() || own == wb.styles.font(wb.styles.xf(0).font).color {
                        cell.cf_font = font.map_or(0, |c| 0xFF00_0000 | c);
                    }
                    if bold {
                        cell.cf_flags |= 1;
                    }
                }
            }
            if !wb.sheets[si].grid.cf.is_empty() {
                let mut cache = std::mem::take(&mut wb.sheets[si].cf_cache);
                let wbr: &Workbook = wb;
                let block_origin = |r: u32, c: u32| {
                    wbr.sheets[si]
                        .grid
                        .cf
                        .iter()
                        .find(|b| b.contains(r, c))
                        .and_then(|b| b.rects.first())
                        .map(|x| (x.r0, x.c0))
                        .unwrap_or((r, c))
                };
                // Rule formulas are written for the block's top-left cell; shift them to this cell.
                let eval = |f: &str, r: u32, c: u32| {
                    let (r0, c0) = block_origin(r, c);
                    let t = crate::refshift::translate_formula(f, r as i32 - r0 as i32, c as i32 - c0 as i32);
                    crate::recalc::eval_at(wbr, si, &t, r, c)
                };
                let looked = crate::cf::look(&wbr.sheets[si], &mut cache, &wbr.styles, r, c, Some(&eval));
                let s = &mut wb.sheets[si];
                if let Some(l) = looked {
                    if let Some(f) = l.fill {
                        cell.cf_fill = 0xFF00_0000 | f;
                    }
                    if let Some(f) = l.font {
                        cell.cf_font = 0xFF00_0000 | f;
                    }
                    cell.cf_flags |= l.bold as u8 | (l.italic as u8) << 1 | (l.strike as u8) << 2 | (l.underline as u8) << 3;
                    if let Some((len, col)) = l.bar {
                        cell.bar = ((len * 1000.0) as u32).max(1);
                        cell.bar_color = 0xFF00_0000 | col;
                    }
                }
                s.cf_cache = cache;
            }
            cells.push(cell);
        }
    }
}

/// Text shown in the formula bar / cell editor.
pub fn edit_text(wb: &mut Workbook, si: usize, r: u32, c: u32) -> String {
    let date1904 = wb.date1904;
    let csv = wb.is_csv();
    let s = &wb.sheets[si];
    if let Some(t) = s.formula_text(r, c) {
        return format!("={t}");
    }
    let v = s.get(r, c);
    let xf = s.style(r, c);
    match v.kind() {
        Kind::Empty => String::new(),
        Kind::Str => s.strings.get(v.as_str_id().unwrap()).to_string(),
        Kind::Bool => (if v.as_bool().unwrap() { "TRUE" } else { "FALSE" }).into(),
        Kind::Error => v.as_error().unwrap().into(),
        Kind::Number => {
            let n = v.as_number().unwrap();
            if csv {
                let mut o = String::new();
                crate::delimited::cell_text(s, v, &mut o);
                return o;
            }
            let fmt = wb.styles.numfmt(xf);
            if fmt.is_date()
                && let Some((y, m, d, h, mi, sec, _)) = numfmt::serial_to_datetime(n, date1904)
            {
                return if n.fract().abs() < 1e-9 {
                    format!("{y:04}-{m:02}-{d:02}")
                } else if n.abs() < 1.0 {
                    format!("{h:02}:{mi:02}:{sec:02}")
                } else {
                    format!("{y:04}-{m:02}-{d:02} {h:02}:{mi:02}:{sec:02}")
                };
            }
            if fmt.is_percent() {
                return format!("{}%", round15(n * 100.0));
            }
            round15(n)
        }
    }
}

/// Up to 15 significant digits, like Excel's editor.
pub fn round15(n: f64) -> String {
    if n == 0.0 || !n.is_finite() {
        return "0".into();
    }
    let s = format!("{:.14e}", n);
    let v: f64 = s.parse().unwrap_or(n);
    let mut out = format!("{}", v);
    if out.contains('e') {
        out = format!("{}", v);
    }
    out
}

/// What typed text means.
pub enum Input {
    Clear,
    Formula(String),
    Value(Cell, Option<String>),
    Text(String),
}

/// Parse typed text. `Value(cell, fmt)` may suggest a number format (dates, %).
pub fn parse_input(wb: &Workbook, text: &str) -> Input {
    if text.is_empty() {
        return Input::Clear;
    }
    if wb.is_csv() {
        return match crate::delimited::canonical_number(text) {
            Some(n) => Input::Value(Cell::number(n), None),
            None => Input::Text(text.to_string()),
        };
    }
    if let Some(f) = text.strip_prefix('=')
        && !f.trim().is_empty()
    {
        return Input::Formula(f.to_string());
    }
    if let Some(t) = text.strip_prefix('\'') {
        return Input::Text(t.to_string());
    }
    let t = text.trim();
    if t.eq_ignore_ascii_case("true") {
        return Input::Value(Cell::boolean(true), None);
    }
    if t.eq_ignore_ascii_case("false") {
        return Input::Value(Cell::boolean(false), None);
    }
    if t.starts_with('#') && crate::cell::ERRORS.iter().any(|e| e.eq_ignore_ascii_case(t)) {
        return Input::Value(Cell::error_from_str(t), None);
    }
    if let Some((n, fmt)) = parse_number(t) {
        return Input::Value(Cell::number(n), fmt);
    }
    if let Some((n, fmt)) = parse_date(t, wb.date1904) {
        return Input::Value(Cell::number(n), Some(fmt.into()));
    }
    Input::Text(text.to_string())
}

/// Numbers as people type them: 1,234.5  -3  (12)  ₹1,00,000  45%  1e3
pub fn parse_number(t: &str) -> Option<(f64, Option<String>)> {
    let mut s = t.trim();
    let mut neg = false;
    if s.starts_with('(') && s.ends_with(')') {
        neg = true;
        s = &s[1..s.len() - 1];
    }
    if let Some(r) = s.strip_prefix('-') {
        neg = !neg;
        s = r;
    } else if let Some(r) = s.strip_prefix('+') {
        s = r;
    }
    let mut pct = false;
    if let Some(r) = s.strip_suffix('%') {
        pct = true;
        s = r.trim_end();
    }
    let mut currency = None;
    for sym in ["₹", "$", "€", "£", "¥", "Rs.", "Rs", "INR "] {
        if let Some(r) = s.strip_prefix(sym) {
            currency = Some(sym);
            s = r.trim_start();
            break;
        }
    }
    if s.is_empty() || !s.as_bytes()[0].is_ascii_digit() && !s.starts_with('.') {
        return None;
    }
    let has_sep = s.contains(',');
    if has_sep {
        // Grouping commas only before the decimal point.
        let int_part = s.split('.').next().unwrap();
        if int_part.starts_with(',') || int_part.ends_with(',') || int_part.contains(",,") {
            return None;
        }
    }
    let clean: String = s.chars().filter(|&c| c != ',').collect();
    if !clean.bytes().all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-')) {
        return None;
    }
    let mut n: f64 = clean.parse().ok()?;
    if neg {
        n = -n;
    }
    let decimals = clean.split('.').nth(1).map_or(0, |d| d.chars().take_while(|c| c.is_ascii_digit()).count());
    let fmt = if pct {
        n /= 100.0;
        Some(if decimals > 0 { format!("0.{}%", "0".repeat(decimals)) } else { "0%".into() })
    } else if let Some(sym) = currency {
        let sym = if sym.starts_with("Rs") || sym.starts_with("INR") { "₹" } else { sym };
        let code = if sym == "₹" { "[$₹-4009] " } else { "" };
        let body = if decimals > 0 { "#,##0.00" } else { "#,##0" };
        Some(if code.is_empty() { format!("\"{sym}\"{body}") } else { format!("{code}{body}") })
    } else if has_sep {
        Some(if decimals > 0 { "#,##0.00".into() } else { "#,##0".into() })
    } else {
        None
    };
    Some((n, fmt))
}

const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];

/// Unambiguous dates: 2024-03-05, 2024/03/05, 5-Mar-2024, 5 Mar 2024, Mar 5, 2024 (+ optional hh:mm[:ss]).
pub fn parse_date(t: &str, date1904: bool) -> Option<(f64, &'static str)> {
    let (date, time) = match t.find([' ', 'T']).filter(|&i| t[i + 1..].contains(':')) {
        Some(i) => (&t[..i], Some(t[i + 1..].trim())),
        None => (t, None),
    };
    let month_of = |s: &str| MONTHS.iter().position(|m| s.len() >= 3 && s[..3].eq_ignore_ascii_case(m)).map(|i| i as u32 + 1);
    let parts: Vec<&str> = date.split(['-', '/', ' ', ',']).filter(|p| !p.is_empty()).collect();
    let (y, m, d, fmt) = match parts.as_slice() {
        [a, b, c] if a.len() == 4 && a.chars().all(|x| x.is_ascii_digit()) => {
            (a.parse().ok()?, b.parse().ok()?, c.parse().ok()?, "yyyy-mm-dd")
        }
        [a, b, c] if month_of(b).is_some() && a.chars().all(|x| x.is_ascii_digit()) => {
            (c.parse().ok()?, month_of(b)?, a.parse().ok()?, "d-mmm-yyyy")
        }
        [a, b, c] if month_of(a).is_some() && b.chars().all(|x| x.is_ascii_digit()) => {
            (c.parse().ok()?, month_of(a)?, b.parse().ok()?, "mmm d, yyyy")
        }
        _ => return None,
    };
    let y: i32 = if y < 100 { 2000 + y } else { y };
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let (mut h, mut mi, mut sec) = (0, 0, 0);
    let mut fmt = fmt;
    if let Some(tm) = time {
        let tp: Vec<&str> = tm.split(':').collect();
        h = tp.first()?.parse().ok()?;
        mi = tp.get(1)?.parse().ok()?;
        sec = tp.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
        fmt = "yyyy-mm-dd hh:mm:ss";
    }
    let n = numfmt::datetime_to_serial(y, m, d, h, mi, sec, date1904)?;
    Some((n, fmt))
}

/// Plain text of a cell for copy / search (display text).
pub fn cell_plain(wb: &mut Workbook, si: usize, r: u32, c: u32, out: &mut String) {
    display(wb, si, r, c, out);
}

pub fn is_blank(s: &Sheet, r: u32, c: u32) -> bool {
    s.get(r, c).is_empty() && s.formula_id(r, c).is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers() {
        assert_eq!(parse_number("1,234.50").unwrap(), (1234.5, Some("#,##0.00".into())));
        assert_eq!(parse_number("(12)").unwrap().0, -12.0);
        assert_eq!(parse_number("45%").unwrap(), (0.45, Some("0%".into())));
        assert_eq!(parse_number("₹1,00,000").unwrap().0, 100000.0);
        assert!(parse_number("12abc").is_none());
        assert!(parse_number("1,,2").is_none());
        assert_eq!(parse_number("1e3").unwrap().0, 1000.0);
        assert_eq!(round15(0.1 + 0.2), "0.3");
    }

    #[test]
    fn dates() {
        assert!(parse_date("2024-03-05", false).is_some());
        assert_eq!(parse_date("5-Mar-2024", false).unwrap().0, parse_date("2024-03-05", false).unwrap().0);
        assert_eq!(parse_date("Mar 5, 2024", false).unwrap().0, parse_date("2024-03-05", false).unwrap().0);
        assert!(parse_date("2024-03-05 14:30", false).unwrap().0.fract() > 0.6);
        assert!(parse_date("hello", false).is_none());
        assert!(parse_date("5/3/2024", false).is_none(), "ambiguous d/m vs m/d");
    }
}
