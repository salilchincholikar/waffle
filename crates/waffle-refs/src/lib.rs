//! Excel (xlsx, A1-style) reference rewriting.
//!
//! * [`shift_formula`] rewrites formula text after rows/columns are inserted or deleted.
//! * [`translate_formula`] moves relative references (copy/paste, fill, shared formulas).
//! * [`shift_sqref`] rewrites plain range-list attributes (mergeCell, sqref, autoFilter, ...).
//!
//! Std only. The tokenizer works on bytes; every split point is an ASCII byte, so UTF-8
//! sheet names / defined names pass through untouched. Nothing is allocated unless a
//! reference actually changes.

use std::borrow::Cow;

/// Largest 0-based row index (row 1048576).
pub const MAX_ROW: u32 = 1_048_575;
/// Largest 0-based column index (column XFD).
pub const MAX_COL: u32 = 16_383;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    Rows,
    Cols,
}

/// A structural edit on one sheet. Indices are 0-based. Delete removes [at, at+count).
#[derive(Clone, Copy, Debug)]
pub struct StructOp {
    pub axis: Axis,
    pub at: u32,
    pub count: u32,
    pub insert: bool,
}

impl StructOp {
    pub fn insert_rows(at: u32, count: u32) -> Self {
        StructOp { axis: Axis::Rows, at, count, insert: true }
    }
    pub fn delete_rows(at: u32, count: u32) -> Self {
        StructOp { axis: Axis::Rows, at, count, insert: false }
    }
    pub fn insert_cols(at: u32, count: u32) -> Self {
        StructOp { axis: Axis::Cols, at, count, insert: true }
    }
    pub fn delete_cols(at: u32, count: u32) -> Self {
        StructOp { axis: Axis::Cols, at, count, insert: false }
    }
    #[inline]
    fn max(&self) -> u32 {
        match self.axis {
            Axis::Rows => MAX_ROW,
            Axis::Cols => MAX_COL,
        }
    }
}

// ---------------------------------------------------------------------------
// Public helpers
// ---------------------------------------------------------------------------

/// Append column letters for a 0-based column index: 0 → "A", 16383 → "XFD".
pub fn col_to_letters(col: u32, out: &mut String) {
    let mut n = col as u64 + 1;
    let mut buf = [0u8; 8];
    let mut i = buf.len();
    while n > 0 {
        n -= 1;
        i -= 1;
        buf[i] = b'A' + (n % 26) as u8;
        n /= 26;
    }
    // SAFETY-free: all bytes are ASCII uppercase letters.
    for &b in &buf[i..] {
        out.push(b as char);
    }
}

/// "A" → 0, "xfd" → 16383. None for empty, non-letters, or beyond XFD.
pub fn letters_to_col(s: &str) -> Option<u32> {
    letters_value(s.as_bytes())
}

/// "B12" / "$B$12" → (row 11, col 1). None if not a single valid cell reference.
pub fn parse_cell_ref(s: &str) -> Option<(u32, u32)> {
    match parse_part(s.as_bytes())? {
        (Kind::Cell, p) => Some((p.row, p.col)),
        _ => None,
    }
}

/// Append an A1-style cell reference (no `$`) for 0-based (row, col).
pub fn cell_ref_string(row: u32, col: u32, out: &mut String) {
    col_to_letters(col, out);
    push_u32(out, row + 1);
}

/// Where does 0-based index `idx` (on `op.axis`) land after `op`? None if deleted
/// or pushed beyond the sheet edge.
pub fn shift_index(idx: u32, op: StructOp) -> Option<u32> {
    let (at, cnt) = (op.at as u64, op.count as u64);
    let v = idx as u64;
    let r = if op.insert {
        if v >= at { v + cnt } else { v }
    } else if v < at {
        v
    } else if v >= at + cnt {
        v - cnt
    } else {
        return None;
    };
    if r > op.max() as u64 { None } else { Some(r as u32) }
}

/// Where does cell (row, col) land after `op`? None if deleted / pushed off-sheet.
pub fn shift_cell(row: u32, col: u32, op: StructOp) -> Option<(u32, u32)> {
    match op.axis {
        Axis::Rows => shift_index(row, op).map(|r| (r, col)),
        Axis::Cols => shift_index(col, op).map(|c| (row, c)),
    }
}

/// Sheet-name equality as Excel does it (case-insensitive, Unicode aware).
pub fn sheet_names_eq(a: &str, b: &str) -> bool {
    if a.eq_ignore_ascii_case(b) {
        return true;
    }
    if a.is_ascii() && b.is_ascii() {
        return false;
    }
    a.chars().flat_map(char::to_lowercase).eq(b.chars().flat_map(char::to_lowercase))
}

// ---------------------------------------------------------------------------
// Main entry points
// ---------------------------------------------------------------------------

/// Rewrite a formula (text WITHOUT the leading '=') living on `host_sheet`, after `op`
/// was applied to `edited_sheet`. Returns None if nothing changed.
///
/// 3-D references (`Sheet1:Sheet3!A1`) are treated as affected when the edited sheet is
/// one of the two endpoints. Use [`shift_formula_3d`] to supply real sheet-order
/// knowledge.
pub fn shift_formula(formula: &str, host_sheet: &str, edited_sheet: &str, op: StructOp) -> Option<String> {
    shift_formula_3d(formula, host_sheet, edited_sheet, op, &|first, last| {
        sheet_names_eq(first, edited_sheet) || sheet_names_eq(last, edited_sheet)
    })
}

/// Like [`shift_formula`], but `in_span(first, last)` decides whether a 3-D reference
/// `first:last!...` covers the edited sheet (names are passed unescaped).
pub fn shift_formula_3d(
    formula: &str,
    host_sheet: &str,
    edited_sheet: &str,
    op: StructOp,
    in_span: &dyn Fn(&str, &str) -> bool,
) -> Option<String> {
    if op.count == 0 {
        return None;
    }
    let host_edited = sheet_names_eq(host_sheet, edited_sheet);
    if !host_edited && !formula.as_bytes().contains(&b'!') {
        return None;
    }
    scan(formula, |sheets, area| {
        let Some(area) = area else {
            return Action::Keep;
        };
        let affected = match sheets {
            None => host_edited,
            Some(s) if s.external => false,
            Some(s) => match s.last {
                None => sheet_names_eq(&s.unescape(s.first), edited_sheet),
                Some(last) => in_span(&s.unescape(s.first), &s.unescape(last)),
            },
        };
        if !affected {
            return Action::Keep;
        }
        match shift_area(area, op) {
            Shifted::Same => Action::Keep,
            Shifted::Moved(a) => Action::Set(a),
            Shifted::Deleted => Action::Ref,
        }
    })
}

/// Translate relative (non-`$`) parts of every reference by (drow, dcol). References
/// pushed off-sheet become `#REF!`. External-workbook refs are translated too (Excel
/// does the same when copying).
pub fn translate_formula(formula: &str, drow: i32, dcol: i32) -> String {
    if drow == 0 && dcol == 0 {
        return formula.to_owned();
    }
    scan(formula, |_, area| match area.map(|a| translate_area(a, drow, dcol)) {
        None | Some(Shifted::Same) => Action::Keep,
        Some(Shifted::Moved(a)) => Action::Set(a),
        Some(Shifted::Deleted) => Action::Ref,
    })
    .unwrap_or_else(|| formula.to_owned())
}

/// Does `sheet` need single quotes when used as a formula qualifier (`'My Sheet'!A1`)?
/// True for empty names, names starting with a non-letter/underscore, names containing
/// anything other than letters/digits/`_`/`.`, names that read as A1 cell refs ("A1",
/// "xfd1048576") or R1C1 refs ("R", "C", "R1C1", "RC2"), and TRUE/FALSE.
pub fn needs_quoting(sheet: &str) -> bool {
    let mut chars = sheet.chars();
    let Some(first) = chars.next() else {
        return true;
    };
    if !(first.is_alphabetic() || first == '_') {
        return true;
    }
    if !chars.all(|c| c.is_alphanumeric() || c == '_' || c == '.') {
        return true;
    }
    let b = sheet.as_bytes();
    if matches!(parse_part(b), Some((Kind::Cell, _))) {
        return true;
    }
    if looks_r1c1(b) {
        return true;
    }
    sheet.eq_ignore_ascii_case("TRUE") || sheet.eq_ignore_ascii_case("FALSE")
}

/// `R`, `C`, `R12`, `C3`, `R1C1`, `RC`, `R2C` (case-insensitive).
fn looks_r1c1(b: &[u8]) -> bool {
    let mut i = 0;
    let digits = |i: &mut usize| {
        while *i < b.len() && b[*i].is_ascii_digit() {
            *i += 1;
        }
    };
    if i < b.len() && b[i].eq_ignore_ascii_case(&b'R') {
        i += 1;
        digits(&mut i);
    }
    if i < b.len() && b[i].eq_ignore_ascii_case(&b'C') {
        i += 1;
        digits(&mut i);
    }
    i > 0 && i == b.len()
}

/// Append `sheet` as a qualifier (without '!'), quoting and doubling `'` if needed.
pub fn quote_sheet(sheet: &str, out: &mut String) {
    if needs_quoting(sheet) {
        push_quoted(out, &[sheet]);
    } else {
        out.push_str(sheet);
    }
}

fn push_quoted(out: &mut String, parts: &[&str]) {
    out.push('\'');
    for (k, p) in parts.iter().enumerate() {
        if k > 0 {
            out.push(':');
        }
        for ch in p.chars() {
            if ch == '\'' {
                out.push('\'');
            }
            out.push(ch);
        }
    }
    out.push('\'');
}

/// Rewrite every sheet qualifier naming `old` (case-insensitive; single or either end of
/// a 3-D span) to `new`, quoting only when needed. External refs are untouched.
/// Returns None if nothing changed.
pub fn rename_sheet_in_formula(formula: &str, old: &str, new: &str) -> Option<String> {
    if !formula.as_bytes().contains(&b'!') {
        return None;
    }
    scan(formula, |sheets, _| {
        let Some(s) = sheets else {
            return Action::Keep;
        };
        if s.external {
            return Action::Keep;
        }
        let first = s.unescape(s.first);
        let hit_first = sheet_names_eq(&first, old);
        let mut p = String::new();
        match s.last {
            None => {
                if !hit_first {
                    return Action::Keep;
                }
                quote_sheet(new, &mut p);
            }
            Some(last) => {
                let last = s.unescape(last);
                let hit_last = sheet_names_eq(&last, old);
                if !hit_first && !hit_last {
                    return Action::Keep;
                }
                let a: &str = if hit_first { new } else { &first };
                let b: &str = if hit_last { new } else { &last };
                if needs_quoting(a) || needs_quoting(b) {
                    push_quoted(&mut p, &[a, b]);
                } else {
                    p.push_str(a);
                    p.push(':');
                    p.push_str(b);
                }
            }
        }
        Action::Prefix(p)
    })
}

/// Replace every reference qualified with sheet `deleted` (case-insensitive) — cells,
/// ranges, sheet-scoped names — by `#REF!` (the whole token, qualifier included).
/// 3-D spans become `#REF!` only when both ends are the deleted sheet; when just one
/// end is deleted Excel narrows the span to the neighbouring sheet, which needs sheet
/// order: call [`rename_sheet_in_formula`]`(f, deleted, neighbour)` first for that.
/// External refs are untouched. Returns None if nothing changed.
pub fn delete_sheet_in_formula(formula: &str, deleted: &str) -> Option<String> {
    if !formula.as_bytes().contains(&b'!') {
        return None;
    }
    scan(formula, |sheets, _| {
        let Some(s) = sheets else {
            return Action::Keep;
        };
        if s.external {
            return Action::Keep;
        }
        let hit = sheet_names_eq(&s.unescape(s.first), deleted) && s.last.is_none_or(|l| sheet_names_eq(&s.unescape(l), deleted));
        if hit { Action::WholeRef } else { Action::Keep }
    })
}

/// Parse a range reference into inclusive 0-based, normalized (r0, c0, r1, c1).
/// "A1" → (0,0,0,0); "A:A" spans rows 0..=MAX_ROW; "3:5" spans cols 0..=MAX_COL.
pub fn parse_range_ref(s: &str) -> Option<(u32, u32, u32, u32)> {
    let b = s.as_bytes();
    let (a, end) = try_area(b, 0)?;
    if end != b.len() {
        return None;
    }
    let (r0, r1, c0, c1) = match a.kind {
        Kind::Cell => (a.a.row, a.b.row, a.a.col, a.b.col),
        Kind::Cols => (0, MAX_ROW, a.a.col, a.b.col),
        Kind::Rows => (a.a.row, a.b.row, 0, MAX_COL),
    };
    Some((r0.min(r1), c0.min(c1), r0.max(r1), c0.max(c1)))
}

/// Shift a plain space-separated range list ("A1:B2 C3", "$A$1:$B$10", "A:A", "3:5").
/// Returns None if nothing changed, Some("") if every range was deleted. Tokens that
/// are not parseable references are kept verbatim.
pub fn shift_sqref(sqref: &str, op: StructOp) -> Option<String> {
    if op.count == 0 {
        return None;
    }
    let mut changed = false;
    let mut results: Vec<Result<&str, Area>> = Vec::new();
    for tok in sqref.split_ascii_whitespace() {
        let tb = tok.as_bytes();
        let parsed = match try_area(tb, 0) {
            Some((a, end)) if end == tb.len() => Some(a),
            _ => None,
        };
        match parsed {
            None => results.push(Ok(tok)),
            Some(a) => match shift_area(&a, op) {
                Shifted::Same => results.push(Ok(tok)),
                Shifted::Moved(n) => {
                    changed = true;
                    results.push(Err(n));
                }
                Shifted::Deleted => changed = true,
            },
        }
    }
    if !changed {
        return None;
    }
    let mut out = String::with_capacity(sqref.len() + 8);
    for (i, r) in results.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        match r {
            Ok(s) => out.push_str(s),
            Err(a) => write_area(&mut out, a),
        }
    }
    Some(out)
}

// ---------------------------------------------------------------------------
// Reference model
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Cell,
    /// Whole columns (A:C) — rows are not meaningful.
    Cols,
    /// Whole rows (1:3) — columns are not meaningful.
    Rows,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Part {
    row: u32,
    col: u32,
    row_abs: bool,
    col_abs: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Area {
    kind: Kind,
    a: Part,
    b: Part,
    range: bool,
}

enum Shifted {
    Same,
    Moved(Area),
    Deleted,
}

#[inline]
fn coord(p: &Part, rows: bool) -> (u32, bool) {
    if rows { (p.row, p.row_abs) } else { (p.col, p.col_abs) }
}

#[inline]
fn set_coord(p: &mut Part, rows: bool, v: u32, abs: bool) {
    if rows {
        p.row = v;
        p.row_abs = abs;
    } else {
        p.col = v;
        p.col_abs = abs;
    }
}

fn shift_area(ar: &Area, op: StructOp) -> Shifted {
    let rows = op.axis == Axis::Rows;
    match (ar.kind, rows) {
        (Kind::Cols, true) | (Kind::Rows, false) => return Shifted::Same,
        _ => {}
    }
    let max = op.max() as u64;
    let (at, cnt) = (op.at as u64, op.count as u64);
    if !ar.range {
        let (v, _) = coord(&ar.a, rows);
        return match shift_index(v, op) {
            None => Shifted::Deleted,
            Some(n) if n == v => Shifted::Same,
            Some(n) => {
                let mut out = *ar;
                let abs = coord(&ar.a, rows).1;
                set_coord(&mut out.a, rows, n, abs);
                out.b = out.a;
                Shifted::Moved(out)
            }
        };
    }
    let (mut lo, mut lo_abs) = coord(&ar.a, rows);
    let (mut hi, mut hi_abs) = coord(&ar.b, rows);
    if lo > hi {
        std::mem::swap(&mut lo, &mut hi);
        std::mem::swap(&mut lo_abs, &mut hi_abs);
    }
    let (l, h) = (lo as u64, hi as u64);
    let (nl, nh) = if op.insert {
        if l >= at {
            let nl = l + cnt;
            if nl > max {
                return Shifted::Deleted;
            }
            (nl, (h + cnt).min(max))
        } else if h >= at {
            (l, (h + cnt).min(max))
        } else {
            return Shifted::Same;
        }
    } else {
        let e = at + cnt;
        if l >= at && h < e {
            return Shifted::Deleted;
        }
        let nl = if l < at {
            l
        } else if l >= e {
            l - cnt
        } else {
            at
        };
        let nh = if h < at {
            h
        } else if h >= e {
            h - cnt
        } else {
            at - 1 // l < at here, so at >= 1
        };
        (nl, nh)
    };
    let (nl, nh) = (nl as u32, nh as u32);
    let (ol, _) = coord(&ar.a, rows);
    let (oh, _) = coord(&ar.b, rows);
    if nl == ol && nh == oh {
        return Shifted::Same;
    }
    let mut out = *ar;
    set_coord(&mut out.a, rows, nl, lo_abs);
    set_coord(&mut out.b, rows, nh, hi_abs);
    Shifted::Moved(out)
}

fn translate_area(ar: &Area, drow: i32, dcol: i32) -> Shifted {
    fn mv(v: u32, abs: bool, d: i32, max: u32) -> Option<u32> {
        if abs || d == 0 {
            return Some(v);
        }
        let n = v as i64 + d as i64;
        if n < 0 || n > max as i64 { None } else { Some(n as u32) }
    }
    let mut out = *ar;
    for p in [&mut out.a, &mut out.b] {
        if ar.kind != Kind::Cols {
            match mv(p.row, p.row_abs, drow, MAX_ROW) {
                Some(r) => p.row = r,
                None => return Shifted::Deleted,
            }
        }
        if ar.kind != Kind::Rows {
            match mv(p.col, p.col_abs, dcol, MAX_COL) {
                Some(c) => p.col = c,
                None => return Shifted::Deleted,
            }
        }
    }
    if out == *ar { Shifted::Same } else { Shifted::Moved(out) }
}

fn push_u32(out: &mut String, mut n: u32) {
    let mut buf = [0u8; 10];
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    for &b in &buf[i..] {
        out.push(b as char);
    }
}

fn write_part(out: &mut String, kind: Kind, p: &Part) {
    match kind {
        Kind::Cell => {
            if p.col_abs {
                out.push('$');
            }
            col_to_letters(p.col, out);
            if p.row_abs {
                out.push('$');
            }
            push_u32(out, p.row + 1);
        }
        Kind::Cols => {
            if p.col_abs {
                out.push('$');
            }
            col_to_letters(p.col, out);
        }
        Kind::Rows => {
            if p.row_abs {
                out.push('$');
            }
            push_u32(out, p.row + 1);
        }
    }
}

fn write_area(out: &mut String, a: &Area) {
    write_part(out, a.kind, &a.a);
    if a.range {
        out.push(':');
        write_part(out, a.kind, &a.b);
    }
}

// ---------------------------------------------------------------------------
// Part / area parsing
// ---------------------------------------------------------------------------

fn letters_value(s: &[u8]) -> Option<u32> {
    if s.is_empty() || s.len() > 3 {
        return None;
    }
    let mut v: u32 = 0;
    for &c in s {
        if !c.is_ascii_alphabetic() {
            return None;
        }
        v = v * 26 + (c.to_ascii_uppercase() - b'A') as u32 + 1;
    }
    if v - 1 > MAX_COL { None } else { Some(v - 1) }
}

/// Parses 1-based row digits → 0-based row.
fn row_value(s: &[u8]) -> Option<u32> {
    if s.is_empty() || s.len() > 7 {
        return None;
    }
    let mut v: u32 = 0;
    for &c in s {
        if !c.is_ascii_digit() {
            return None;
        }
        v = v * 10 + (c - b'0') as u32;
    }
    if v == 0 || v > MAX_ROW + 1 { None } else { Some(v - 1) }
}

/// Parse one reference component: `$A$1` (Cell), `$A` (Cols), `$1` (Rows).
fn parse_part(s: &[u8]) -> Option<(Kind, Part)> {
    let n = s.len();
    let mut i = 0;
    let first_abs = n > 0 && s[0] == b'$';
    if first_abs {
        i = 1;
    }
    let ls = i;
    while i < n && s[i].is_ascii_alphabetic() {
        i += 1;
        if i - ls > 3 {
            return None;
        }
    }
    if i == ls {
        let row = row_value(&s[i..])?;
        return Some((Kind::Rows, Part { row, col: 0, row_abs: first_abs, col_abs: false }));
    }
    let col = letters_value(&s[ls..i])?;
    if i == n {
        return Some((Kind::Cols, Part { row: 0, col, row_abs: false, col_abs: first_abs }));
    }
    let row_abs = s[i] == b'$';
    if row_abs {
        i += 1;
    }
    let row = row_value(&s[i..])?;
    Some((Kind::Cell, Part { row, col, row_abs, col_abs: first_abs }))
}

#[inline]
fn is_word(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b'\\' | b'?' | b'$') || c >= 0x80
}

#[inline]
fn is_name_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c == b'\\' || c >= 0x80
}

#[inline]
fn word_end(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && is_word(b[i]) {
        i += 1;
    }
    i
}

#[inline]
fn bad_follow(b: &[u8], i: usize) -> bool {
    i < b.len() && matches!(b[i], b'(' | b'[' | b'!')
}

/// Try to parse an area (A1, A1:B2, A:C, 1:3) starting exactly at `pos`.
fn try_area(b: &[u8], pos: usize) -> Option<(Area, usize)> {
    let j = word_end(b, pos);
    if j == pos {
        return None;
    }
    let (k1, p1) = parse_part(&b[pos..j])?;
    if j < b.len() && b[j] == b':' {
        let m = word_end(b, j + 1);
        if m > j + 1
            && let Some((k2, p2)) = parse_part(&b[j + 1..m])
            && k1 == k2
            && !bad_follow(b, m)
        {
            return Some((Area { kind: k1, a: p1, b: p2, range: true }, m));
        }
    }
    if k1 == Kind::Cell && !bad_follow(b, j) {
        return Some((Area { kind: k1, a: p1, b: p1, range: false }, j));
    }
    None
}

// ---------------------------------------------------------------------------
// Tokenizer / scanner
// ---------------------------------------------------------------------------

/// Sheet qualifier of a reference, as written.
struct Sheets<'a> {
    first: &'a str,
    last: Option<&'a str>,
    quoted: bool,
    external: bool,
}

impl Sheets<'_> {
    fn unescape<'s>(&self, s: &'s str) -> Cow<'s, str> {
        if self.quoted && s.contains("''") { Cow::Owned(s.replace("''", "'")) } else { Cow::Borrowed(s) }
    }
}

enum Action {
    Keep,
    /// Replace the reference body (after any qualifier) with this area.
    Set(Area),
    /// Replace the reference body with `#REF!` (qualifier kept).
    Ref,
    /// Replace the whole token, qualifier included, with `#REF!`.
    WholeRef,
    /// Replace the qualifier (text before '!') with this text.
    Prefix(String),
}

struct Out<'a> {
    src: &'a str,
    buf: Option<String>,
    last: usize,
}

impl Out<'_> {
    /// Token layout: `[start .. body)` is the qualifier incl. '!', `[body .. end)` the body.
    fn apply(&mut self, start: usize, body: usize, end: usize, act: Action) {
        if matches!(act, Action::Keep) {
            return;
        }
        let src = self.src;
        let buf = self.buf.get_or_insert_with(|| String::with_capacity(src.len() + 16));
        match act {
            Action::Keep => {}
            Action::Set(a) => {
                buf.push_str(&src[self.last..body]);
                write_area(buf, &a);
                self.last = end;
            }
            Action::Ref => {
                buf.push_str(&src[self.last..body]);
                buf.push_str("#REF!");
                self.last = end;
            }
            Action::WholeRef => {
                buf.push_str(&src[self.last..start]);
                buf.push_str("#REF!");
                self.last = end;
            }
            Action::Prefix(p) => {
                buf.push_str(&src[self.last..start]);
                buf.push_str(&p);
                self.last = body - 1; // keep the '!' and the body
            }
        }
    }
    fn finish(self) -> Option<String> {
        let mut buf = self.buf?;
        buf.push_str(&self.src[self.last..]);
        Some(buf)
    }
}

const ERRORS: [&[u8]; 8] = [b"#GETTING_DATA", b"#DIV/0!", b"#VALUE!", b"#NAME?", b"#NULL!", b"#REF!", b"#NUM!", b"#N/A"];

fn skip_error(b: &[u8], i: usize) -> usize {
    for e in ERRORS {
        if b.len() - i >= e.len() && b[i..i + e.len()].eq_ignore_ascii_case(e) {
            return i + e.len();
        }
    }
    i + 1
}

/// `i` at opening '"'; returns index after the closing quote.
fn skip_string(b: &[u8], mut i: usize) -> usize {
    i += 1;
    while i < b.len() {
        if b[i] == b'"' {
            if i + 1 < b.len() && b[i + 1] == b'"' {
                i += 2;
                continue;
            }
            return i + 1;
        }
        i += 1;
    }
    b.len()
}

/// `i` at opening '\''; returns index of the closing quote (or len if unterminated).
fn find_quote_end(b: &[u8], mut i: usize) -> usize {
    i += 1;
    while i < b.len() {
        if b[i] == b'\'' {
            if i + 1 < b.len() && b[i + 1] == b'\'' {
                i += 2;
                continue;
            }
            return i;
        }
        i += 1;
    }
    b.len()
}

/// `i` at '['; returns index after the matching ']'. `'` escapes the next char
/// (structured-reference escaping).
fn skip_brackets(b: &[u8], mut i: usize) -> usize {
    let mut depth = 0usize;
    while i < b.len() {
        match b[i] {
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            b'\'' => i += 1,
            _ => {}
        }
        i += 1;
    }
    b.len()
}

/// `i` at '{'; returns index after the matching '}'.
fn skip_array(b: &[u8], mut i: usize) -> usize {
    i += 1;
    while i < b.len() {
        match b[i] {
            b'"' => {
                i = skip_string(b, i);
                continue;
            }
            b'}' => return i + 1,
            _ => {}
        }
        i += 1;
    }
    b.len()
}

fn skip_number(b: &[u8], i: usize) -> usize {
    let n = b.len();
    let mut j = i;
    while j < n && b[j].is_ascii_digit() {
        j += 1;
    }
    if j < n && b[j] == b'.' {
        j += 1;
        while j < n && b[j].is_ascii_digit() {
            j += 1;
        }
    }
    if j < n && (b[j] == b'e' || b[j] == b'E') {
        let mut k = j + 1;
        if k < n && (b[k] == b'+' || b[k] == b'-') {
            k += 1;
        }
        if k < n && b[k].is_ascii_digit() {
            while k < n && b[k].is_ascii_digit() {
                k += 1;
            }
            j = k;
        }
    }
    if j == i { i + 1 } else { j }
}

fn scan<F>(f: &str, mut cb: F) -> Option<String>
where
    F: FnMut(Option<&Sheets>, Option<&Area>) -> Action,
{
    let b = f.as_bytes();
    let n = b.len();
    let mut out = Out { src: f, buf: None, last: 0 };
    let mut i = 0;

    // Parse an area at `pos` (after an optional sheet qualifier) and apply the callback.
    // Returns the position to continue scanning from.
    // `start` is where the whole token (including any qualifier) begins; `pos` is where
    // the reference body begins. The callback also sees qualified non-area bodies
    // (`Sheet1!Name`, `Sheet1!#REF!`) with `area == None`.
    let mut handle = |start: usize, pos: usize, sheets: Option<&Sheets>, out: &mut Out| match try_area(b, pos) {
        Some((area, end)) => {
            let act = cb(sheets, Some(&area));
            out.apply(start, pos, end, act);
            end
        }
        None => {
            if sheets.is_none() {
                return word_end(b, pos);
            }
            let mut end = if pos < n && b[pos] == b'#' { skip_error(b, pos) } else { word_end(b, pos) };
            if end > pos && end < n && b[end] == b'[' {
                end = skip_brackets(b, end);
            }
            let act = cb(sheets, None);
            out.apply(start, pos, end, act);
            end.max(pos)
        }
    };

    while i < n {
        let c = b[i];
        match c {
            b'"' => i = skip_string(b, i),
            b'{' => i = skip_array(b, i),
            b'#' => i = skip_error(b, i),
            b'\'' => {
                let close = find_quote_end(b, i);
                if close + 1 < n && b[close + 1] == b'!' {
                    let inner = &f[i + 1..close];
                    let sheets = if inner.contains('[') {
                        Sheets { first: inner, last: None, quoted: true, external: true }
                    } else if let Some(p) = inner.find(':') {
                        Sheets { first: &inner[..p], last: Some(&inner[p + 1..]), quoted: true, external: false }
                    } else {
                        Sheets { first: inner, last: None, quoted: true, external: false }
                    };
                    i = handle(i, close + 2, Some(&sheets), &mut out);
                } else {
                    i = close + 1;
                }
            }
            b'[' => {
                // External workbook `[1]Sheet1!A1`, `[1]!Name`, or an implicit-table
                // structured reference `[@Col]` / `[[#This Row],[Col]]`.
                let e = skip_brackets(b, i);
                let j = word_end(b, e);
                if j > e && j < n && b[j] == b'!' {
                    let sheets = Sheets { first: &f[e..j], last: None, quoted: false, external: true };
                    i = handle(i, j + 1, Some(&sheets), &mut out);
                } else if j > e && j + 1 < n && b[j] == b':' {
                    let k = word_end(b, j + 1);
                    if k > j + 1 && k < n && b[k] == b'!' {
                        let sheets = Sheets { first: &f[e..j], last: Some(&f[j + 1..k]), quoted: false, external: true };
                        i = handle(i, k + 1, Some(&sheets), &mut out);
                    } else {
                        i = e;
                    }
                } else {
                    i = e;
                }
            }
            b'0'..=b'9' => {
                // Whole-row range (1:3) or a number literal.
                let j = word_end(b, i);
                if j < n && b[j] == b':' && try_area(b, i).is_some() {
                    i = handle(i, i, None, &mut out);
                    continue;
                }
                i = skip_number(b, i);
            }
            b'.' => i = skip_number(b, i),
            c if c == b'$' || is_name_start(c) => {
                let j = word_end(b, i);
                if j < n {
                    match b[j] {
                        b'(' => {
                            i = j; // function name
                            continue;
                        }
                        b'[' => {
                            i = skip_brackets(b, j); // Table1[...]
                            continue;
                        }
                        b'!' => {
                            let sheets = Sheets { first: &f[i..j], last: None, quoted: false, external: false };
                            i = handle(i, j + 1, Some(&sheets), &mut out);
                            continue;
                        }
                        b':' => {
                            // Possible unquoted 3-D prefix `Sheet1:Sheet3!`.
                            let k = word_end(b, j + 1);
                            if k > j + 1 && k < n && b[k] == b'!' && !matches!(parse_part(&b[i..j]), Some((Kind::Cell, _))) {
                                let sheets = Sheets { first: &f[i..j], last: Some(&f[j + 1..k]), quoted: false, external: false };
                                i = handle(i, k + 1, Some(&sheets), &mut out);
                                continue;
                            }
                        }
                        _ => {}
                    }
                }
                i = handle(i, i, None, &mut out);
            }
            _ => i += 1,
        }
    }
    out.finish()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(f: &str, op: StructOp) -> Option<String> {
        shift_formula(f, "Sheet1", "Sheet1", op)
    }
    fn s(f: &str, op: StructOp) -> String {
        sh(f, op).unwrap_or_else(|| f.to_string())
    }
    fn ir(at: u32, n: u32) -> StructOp {
        StructOp::insert_rows(at, n)
    }
    fn dr(at: u32, n: u32) -> StructOp {
        StructOp::delete_rows(at, n)
    }
    fn ic(at: u32, n: u32) -> StructOp {
        StructOp::insert_cols(at, n)
    }
    fn dc(at: u32, n: u32) -> StructOp {
        StructOp::delete_cols(at, n)
    }
    fn letters(c: u32) -> String {
        let mut s = String::new();
        col_to_letters(c, &mut s);
        s
    }

    #[test]
    fn helpers() {
        assert_eq!(letters(0), "A");
        assert_eq!(letters(25), "Z");
        assert_eq!(letters(26), "AA");
        assert_eq!(letters(701), "ZZ");
        assert_eq!(letters(702), "AAA");
        assert_eq!(letters(16383), "XFD");
        assert_eq!(letters_to_col("A"), Some(0));
        assert_eq!(letters_to_col("xfd"), Some(16383));
        assert_eq!(letters_to_col("XFE"), None);
        assert_eq!(letters_to_col("ABCD"), None);
        assert_eq!(letters_to_col(""), None);
        assert_eq!(letters_to_col("A1"), None);
        for c in [0, 1, 25, 26, 27, 51, 52, 701, 702, 703, 16383] {
            assert_eq!(letters_to_col(&letters(c)), Some(c));
        }
        assert_eq!(parse_cell_ref("B12"), Some((11, 1)));
        assert_eq!(parse_cell_ref("$B$12"), Some((11, 1)));
        assert_eq!(parse_cell_ref("XFD1048576"), Some((MAX_ROW, MAX_COL)));
        assert_eq!(parse_cell_ref("XFD1048577"), None);
        assert_eq!(parse_cell_ref("A0"), None);
        assert_eq!(parse_cell_ref("A"), None);
        assert_eq!(parse_cell_ref("1"), None);
        assert_eq!(parse_cell_ref("A1B"), None);
        let mut r = String::new();
        cell_ref_string(4, 27, &mut r);
        assert_eq!(r, "AB5");
        assert_eq!(shift_cell(4, 0, ir(4, 2)), Some((6, 0)));
        assert_eq!(shift_cell(4, 0, dr(4, 1)), None);
        assert_eq!(shift_cell(4, 3, dc(0, 2)), Some((4, 1)));
        assert_eq!(shift_index(MAX_ROW, ir(0, 1)), None);
    }

    #[test]
    fn insert_rows_basic() {
        assert_eq!(s("A1+A3+A5", ir(2, 1)), "A1+A4+A6");
        assert_eq!(s("SUM(A1:A10)", ir(4, 2)), "SUM(A1:A12)");
        // Insert at a range's first row pushes it down.
        assert_eq!(s("SUM(A3:A5)", ir(2, 1)), "SUM(A4:A6)");
        // Insert right after the end: unchanged.
        assert_eq!(sh("SUM(A3:A5)", ir(5, 1)), None);
        // Insert at the last row expands.
        assert_eq!(s("SUM(A3:A5)", ir(4, 1)), "SUM(A3:A6)");
        assert_eq!(s("$A$5", ir(0, 1)), "$A$6");
        assert_eq!(s("A$5:$B6", ir(0, 3)), "A$8:$B9");
        assert_eq!(sh("A1+B2", ir(10, 5)), None);
    }

    #[test]
    fn delete_rows_basic() {
        assert_eq!(s("A5", dr(4, 1)), "#REF!");
        assert_eq!(s("A5+1", dr(0, 2)), "A3+1");
        assert_eq!(s("SUM(A1:A10)", dr(2, 2)), "SUM(A1:A8)");
        assert_eq!(s("SUM(A3:A5)", dr(2, 3)), "SUM(#REF!)");
        assert_eq!(s("SUM(A3:A10)", dr(1, 3)), "SUM(A2:A7)");
        assert_eq!(s("SUM(A1:A10)", dr(4, 20)), "SUM(A1:A4)");
        assert_eq!(s("SUM($B$5:$B$10)", dr(0, 20)), "SUM(#REF!)");
        assert_eq!(s("SUM(A1:A1)", dr(0, 1)), "SUM(#REF!)");
        assert_eq!(sh("SUM(A1:A3)", dr(3, 1)), None);
    }

    #[test]
    fn columns() {
        assert_eq!(s("SUM(B1:D1)", ic(1, 1)), "SUM(C1:E1)");
        assert_eq!(s("SUM(A:C)", ic(1, 1)), "SUM(A:D)");
        assert_eq!(sh("SUM(A:C)", ir(0, 1)), None);
        assert_eq!(s("SUM($A:$A)", ic(0, 2)), "SUM($C:$C)");
        assert_eq!(s("SUM(B:D)", dc(2, 1)), "SUM(B:C)");
        assert_eq!(s("C1*2", dc(2, 1)), "#REF!*2");
        assert_eq!(s("XFD1", ic(0, 1)), "#REF!");
        assert_eq!(s("Z1+AA1", ic(0, 1)), "AA1+AB1");
        assert_eq!(s("SUM(A1:XFD1)", ic(3, 1)), "SUM(A1:XFD1)".to_string());
    }

    #[test]
    fn whole_rows_and_overflow() {
        assert_eq!(s("SUM(1:3)", ir(0, 1)), "SUM(2:4)");
        assert_eq!(s("SUM($5:$5)", dr(4, 1)), "SUM(#REF!)");
        assert_eq!(sh("SUM(1:3)", ic(0, 1)), None);
        assert_eq!(s("SUM(2:10)", dr(0, 3)), "SUM(1:7)");
        assert_eq!(s("A1048576", ir(0, 1)), "#REF!");
        assert_eq!(s("SUM(A1:A1048576)", ir(0, 1)), "SUM(A2:A1048576)");
        assert_eq!(sh("SUM(A1:A1048576)", ir(5, 1)), None);
    }

    #[test]
    fn sheets_and_qualifiers() {
        let op = ir(0, 1);
        assert_eq!(shift_formula("A5+Sheet1!A5", "Sheet2", "Sheet1", op).as_deref(), Some("A5+Sheet1!A6"));
        assert_eq!(shift_formula("A5+B5", "Sheet2", "Sheet1", op), None);
        assert_eq!(shift_formula("sheet1!A5", "Sheet2", "SHEET1", op).as_deref(), Some("sheet1!A6"));
        assert_eq!(shift_formula("'My Sheet'!A5*2", "X", "my sheet", op).as_deref(), Some("'My Sheet'!A6*2"));
        assert_eq!(shift_formula("'O''Brien'!A5", "X", "O'Brien", op).as_deref(), Some("'O''Brien'!A6"));
        assert_eq!(shift_formula("'Données 2024'!A5+Données!B1", "X", "DONNÉES", op).as_deref(), Some("'Données 2024'!A5+Données!B2"));
        assert_eq!(shift_formula("'Q1.2024'!A5+'2019'!A5", "X", "q1.2024", op).as_deref(), Some("'Q1.2024'!A6+'2019'!A5"));
        // Unqualified refs on host that is not the edited sheet are untouched.
        assert_eq!(shift_formula("A5+Other!A5", "Sheet1", "Other", dr(4, 1)).as_deref(), Some("A5+Other!#REF!"));
        assert_eq!(shift_formula("SUM('My Sheet'!A5:A6)", "X", "My Sheet", dr(0, 10)).as_deref(), Some("SUM('My Sheet'!#REF!)"));
        // Sheet-scoped defined name: untouched.
        assert_eq!(shift_formula("Sheet1!MyName+1", "X", "Sheet1", op), None);
    }

    #[test]
    fn three_d() {
        let op = ir(0, 1);
        assert_eq!(shift_formula("SUM(Sheet1:Sheet3!A5)", "X", "Sheet1", op).as_deref(), Some("SUM(Sheet1:Sheet3!A6)"));
        assert_eq!(shift_formula("SUM(Sheet1:Sheet3!A5)", "X", "sheet3", op).as_deref(), Some("SUM(Sheet1:Sheet3!A6)"));
        assert_eq!(shift_formula("SUM(Sheet1:Sheet3!A5)", "X", "Sheet9", op), None);
        assert_eq!(shift_formula("SUM('Jan 1:Mar 3'!A5:B6)", "X", "Mar 3", op).as_deref(), Some("SUM('Jan 1:Mar 3'!A6:B7)"));
        let span = |a: &str, b: &str| a == "Sheet1" && b == "Sheet3";
        assert_eq!(shift_formula_3d("SUM(Sheet1:Sheet3!A5)", "X", "Sheet2", op, &span).as_deref(), Some("SUM(Sheet1:Sheet3!A6)"));
    }

    #[test]
    fn strings_untouched() {
        let op = ir(0, 1);
        assert_eq!(s("\"A5\"&A5", op), "\"A5\"&A6");
        assert_eq!(s("\"a\"\"A5\"\"b\"&A5", op), "\"a\"\"A5\"\"b\"&A6");
        assert_eq!(sh("\"Sheet1!A5\"", op), None);
        assert_eq!(s("CONCAT(\"'\",A5,\"'\")", op), "CONCAT(\"'\",A6,\"'\")");
        assert_eq!(s("\"[1]x\"&A1", op), "\"[1]x\"&A2");
    }

    #[test]
    fn functions_names_literals() {
        let op = ir(0, 1);
        assert_eq!(s("LOG10(A5)", op), "LOG10(A6)");
        assert_eq!(s("ATAN2(A5,B5)", op), "ATAN2(A6,B6)");
        assert_eq!(s("DAYS360(A1,A5)", op), "DAYS360(A2,A6)");
        assert_eq!(s("_xlfn.F.DIST(A5,1,TRUE)", op), "_xlfn.F.DIST(A6,1,TRUE)");
        assert_eq!(s("_xlfn._xlws.SORT(A1:A5)", op), "_xlfn._xlws.SORT(A2:A6)");
        assert_eq!(s("MyName+Tax_Rate*A5", op), "MyName+Tax_Rate*A6");
        assert_eq!(sh("ABCD1+XFE1+A1048577+A0+TRUE+FALSE", op), None);
        assert_eq!(sh("_A1+A1_+A1.x+\\x", op), None);
        assert_eq!(
            s("IFERROR(A5,#N/A)+IF(#DIV/0!,#VALUE!,#NAME?)+#NULL!+#NUM!+#REF!+#GETTING_DATA", op),
            "IFERROR(A6,#N/A)+IF(#DIV/0!,#VALUE!,#NAME?)+#NULL!+#NUM!+#REF!+#GETTING_DATA"
        );
        assert_eq!(sh("Sheet1!#REF!+1", op), None);
    }

    #[test]
    fn numbers() {
        let op = ir(0, 1);
        assert_eq!(s("1E+5+A5", op), "1E+5+A6");
        assert_eq!(s("1.5E-3*E5", op), "1.5E-3*E6");
        assert_eq!(s("1E5+E5", op), "1E5+E6");
        assert_eq!(s("50%*A1+.5", op), "50%*A2+.5");
        assert_eq!(sh("1+2.5-3E2", op), None);
    }

    #[test]
    fn externals_structured_arrays() {
        let op = ir(0, 1);
        assert_eq!(sh("[1]Sheet1!A5", op), None);
        assert_eq!(sh("[1]Sheet1!$A$5:$B$9+[2]!Name", op), None);
        assert_eq!(shift_formula("'[Book.xlsx]Sheet 1'!A5", "Sheet 1", "Sheet 1", op), None);
        assert_eq!(s("[1]Sheet1!A5+A5", op), "[1]Sheet1!A5+A6");
        assert_eq!(s("SUM(Table1[Col])+A1", op), "SUM(Table1[Col])+A2");
        assert_eq!(s("Table1[[#This Row],[Amount]]*A1", op), "Table1[[#This Row],[Amount]]*A2");
        assert_eq!(s("[@Amount]*A1", op), "[@Amount]*A2");
        assert_eq!(s("Table1[[#Headers],[A1'[x']]]+B1", op), "Table1[[#Headers],[A1'[x']]]+B2");
        assert_eq!(s("SUM({1,2;3,4})*A5", op), "SUM({1,2;3,4})*A6");
        assert_eq!(sh("{\"A5\",1;2,3}", op), None);
    }

    #[test]
    fn operators() {
        let op = ir(0, 1);
        assert_eq!(s("SUM(A1:A5 A3:B3)", op), "SUM(A2:A6 A4:B4)");
        assert_eq!(s("SUM((A1,A5))", op), "SUM((A2,A6))");
        assert_eq!(s("SUM(A1:B2:C3)", op), "SUM(A2:B3:C4)");
        assert_eq!(s("A1:INDEX(B:B,3)", op), "A2:INDEX(B:B,3)");
        assert_eq!(s("Sheet1!A1:Sheet1!B2", op), "Sheet1!A2:Sheet1!B3");
        assert_eq!(s("A1:A3=\"x\"", op), "A2:A4=\"x\"");
    }

    #[test]
    fn translate() {
        assert_eq!(translate_formula("A1+$A$1+A$1+$A1", 4, 1), "B5+$A$1+B$1+$A5");
        assert_eq!(translate_formula("A1", -1, 0), "#REF!");
        assert_eq!(translate_formula("Sheet2!A1*2", 0, -1), "Sheet2!#REF!*2");
        assert_eq!(translate_formula("SUM(A:A)", 5, 1), "SUM(B:B)");
        assert_eq!(translate_formula("SUM(1:1)", 1, 5), "SUM(2:2)");
        assert_eq!(translate_formula("XFD1", 0, 1), "#REF!");
        assert_eq!(translate_formula("A1048576", 1, 0), "#REF!");
        assert_eq!(translate_formula("\"A1\"&A1", 1, 0), "\"A1\"&A2");
        assert_eq!(translate_formula("SUM($A$1:A1)", 4, 0), "SUM($A$1:A5)");
        assert_eq!(translate_formula("LOG10(A1)+Table1[Col]", 1, 0), "LOG10(A2)+Table1[Col]");
        assert_eq!(translate_formula("'My Sheet'!B2", 1, 1), "'My Sheet'!C3");
        assert_eq!(translate_formula("A1", 0, 0), "A1");
    }

    #[test]
    fn sqref() {
        assert_eq!(shift_sqref("A1:B2 C3:D4", ir(0, 1)).as_deref(), Some("A2:B3 C4:D5"));
        assert_eq!(shift_sqref("A1:B2 C3:D4", ir(10, 1)), None);
        assert_eq!(shift_sqref("A1:B2 C5", dr(4, 1)).as_deref(), Some("A1:B2"));
        assert_eq!(shift_sqref("C5 D5:E5", dr(4, 1)).as_deref(), Some(""));
        assert_eq!(shift_sqref("$A$1:$B$10", dr(0, 2)).as_deref(), Some("$A$1:$B$8"));
        assert_eq!(shift_sqref("A:A", ic(0, 1)).as_deref(), Some("B:B"));
        assert_eq!(shift_sqref("3:5", ir(3, 2)).as_deref(), Some("3:7"));
        assert_eq!(shift_sqref("A:A", ir(0, 1)), None);
        assert_eq!(shift_sqref("A1", ic(0, 1)).as_deref(), Some("B1"));
        assert_eq!(shift_sqref("A1:C3", dc(1, 1)).as_deref(), Some("A1:B3"));
    }

    #[test]
    fn quoting() {
        for n in ["Sheet1", "Data", "_x", "Données", "a.b", "ABCD1", "XFE1", "Rx", "Total_2024"] {
            assert!(!needs_quoting(n), "{n}");
        }
        for n in [
            "",
            "My Sheet",
            "2019",
            "A1",
            "xfd1048576",
            "R",
            "c",
            "R1C1",
            "RC2",
            "R12",
            "C3",
            "O'Brien",
            "a-b",
            "x!",
            ".x",
            "TRUE",
            "(1)",
            "Q1:Q2",
        ] {
            assert!(needs_quoting(n), "{n}");
        }
        let mut o = String::new();
        quote_sheet("O'Brien", &mut o);
        assert_eq!(o, "'O''Brien'");
        o.clear();
        quote_sheet("Plain", &mut o);
        assert_eq!(o, "Plain");
        o.clear();
        quote_sheet("A1", &mut o);
        assert_eq!(o, "'A1'");
    }

    #[test]
    fn rename_sheet() {
        let r = rename_sheet_in_formula;
        assert_eq!(r("Sheet1!A1+sheet1!B2", "Sheet1", "Data").as_deref(), Some("Data!A1+Data!B2"));
        assert_eq!(r("Sheet1!A1", "Sheet1", "My Data").as_deref(), Some("'My Data'!A1"));
        assert_eq!(r("'My Data'!A1:B2", "my data", "X").as_deref(), Some("X!A1:B2"));
        assert_eq!(r("'O''Brien'!A1", "O'Brien", "Smith's").as_deref(), Some("'Smith''s'!A1"));
        assert_eq!(r("Sheet1!A1", "Sheet1", "A1").as_deref(), Some("'A1'!A1"));
        assert_eq!(r("Sheet1!A1", "Sheet1", "R1C1").as_deref(), Some("'R1C1'!A1"));
        assert_eq!(r("Sheet1!A1", "Sheet1", "2024").as_deref(), Some("'2024'!A1"));
        assert_eq!(r("A1+Other!A1", "Sheet1", "X"), None);
        assert_eq!(r("A1+B2", "Sheet1", "X"), None);
        assert_eq!(r("SUM(Sheet1:Sheet3!A1)", "Sheet1", "Jan").as_deref(), Some("SUM(Jan:Sheet3!A1)"));
        assert_eq!(r("SUM(Sheet1:Sheet3!A1)", "sheet3", "Mar 1").as_deref(), Some("SUM('Sheet1:Mar 1'!A1)"));
        assert_eq!(r("SUM('Jan 1:Mar'!A1)", "Jan 1", "Jan").as_deref(), Some("SUM(Jan:Mar!A1)"));
        assert_eq!(r("[1]Sheet1!A1+'[B.xlsx]Sheet1'!A1", "Sheet1", "X"), None);
        assert_eq!(r("\"Sheet1!A1\"", "Sheet1", "X"), None);
        assert_eq!(r("Sheet1!MyName*2", "Sheet1", "X").as_deref(), Some("X!MyName*2"));
        assert_eq!(r("Sheet1!#REF!", "Sheet1", "X").as_deref(), Some("X!#REF!"));
        assert_eq!(r("SUM(Sheet1!A:A,Sheet1!1:2)+LOG10(Sheet1!C3)", "Sheet1", "S").as_deref(), Some("SUM(S!A:A,S!1:2)+LOG10(S!C3)"));
        assert_eq!(r("Données!A1", "DONNÉES", "Übersicht").as_deref(), Some("Übersicht!A1"));
    }

    #[test]
    fn delete_sheet() {
        let d = delete_sheet_in_formula;
        assert_eq!(d("Sheet1!A1+A1", "Sheet1").as_deref(), Some("#REF!+A1"));
        assert_eq!(d("SUM('My Sheet'!A1:B9)*2", "my sheet").as_deref(), Some("SUM(#REF!)*2"));
        assert_eq!(d("Other!A1", "Sheet1"), None);
        assert_eq!(d("Sheet1!Name+Sheet1!#REF!+Sheet1!T[C]", "Sheet1").as_deref(), Some("#REF!+#REF!+#REF!"));
        assert_eq!(d("SUM(Sheet1:Sheet1!A1)", "Sheet1").as_deref(), Some("SUM(#REF!)"));
        assert_eq!(d("SUM(Sheet1:Sheet3!A1)", "Sheet1"), None);
        assert_eq!(d("[1]Sheet1!A1", "Sheet1"), None);
        assert_eq!(d("\"Sheet1!A1\"&Sheet2!B1", "Sheet1"), None);
        assert_eq!(d("SUM(Sheet1!A:A)", "Sheet1").as_deref(), Some("SUM(#REF!)"));
    }

    #[test]
    fn range_ref() {
        assert_eq!(parse_range_ref("A1"), Some((0, 0, 0, 0)));
        assert_eq!(parse_range_ref("A1:B2"), Some((0, 0, 1, 1)));
        assert_eq!(parse_range_ref("$A$1:$B$2"), Some((0, 0, 1, 1)));
        assert_eq!(parse_range_ref("B2:A1"), Some((0, 0, 1, 1)));
        assert_eq!(parse_range_ref("A5:C1"), Some((0, 0, 4, 2)));
        assert_eq!(parse_range_ref("A:A"), Some((0, 0, MAX_ROW, 0)));
        assert_eq!(parse_range_ref("$C:A"), Some((0, 0, MAX_ROW, 2)));
        assert_eq!(parse_range_ref("3:5"), Some((2, 0, 4, MAX_COL)));
        assert_eq!(parse_range_ref("$5:$5"), Some((4, 0, 4, MAX_COL)));
        assert_eq!(parse_range_ref("A"), None);
        assert_eq!(parse_range_ref("3"), None);
        assert_eq!(parse_range_ref("A1:B"), None);
        assert_eq!(parse_range_ref("A1 B2"), None);
        assert_eq!(parse_range_ref("Sheet1!A1"), None);
        assert_eq!(parse_range_ref(""), None);
    }

    #[test]
    fn noop_count_zero() {
        assert_eq!(sh("A1", ir(0, 0)), None);
        assert_eq!(shift_sqref("A1", dr(0, 0)), None);
    }

    /// Pseudo-random formulas from fragments: never panics, and insert-then-delete of
    /// the same block is an identity.
    #[test]
    fn fuzz_no_panic_and_roundtrip() {
        let frags = [
            "A1",
            "$B$7",
            "C3:D9",
            "A:C",
            "$2:$4",
            "Sheet1!",
            "'É t''x'!",
            "[1]S!",
            "\"A1\"",
            "\"",
            "'",
            "[",
            "]",
            "{1,2}",
            "#REF!",
            "#",
            "LOG10(",
            ")",
            ",",
            " ",
            ":",
            "1E+5",
            "é",
            "Table1[[#All],[x]]",
            "!",
            "$",
            "XFD1048576",
            "ZZZ9",
            ".5",
            "%",
            "_xlfn.",
        ];
        let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
        for _ in 0..20_000 {
            let mut f = String::new();
            let len = (seed % 9) as usize + 1;
            for _ in 0..len {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                f.push_str(frags[(seed % frags.len() as u64) as usize]);
            }
            let _ = shift_formula(&f, "Sheet1", "Sheet1", dr(1, 2));
            let _ = shift_formula(&f, "X", "É t'x", ic(0, 1));
            let _ = translate_formula(&f, -3, 2);
            let _ = shift_sqref(&f, ir(0, 1));
            let _ = rename_sheet_in_formula(&f, "É t'x", "A 1");
            let _ = delete_sheet_in_formula(&f, "Sheet1");
            let _ = parse_range_ref(&f);
        }
        for f in ["A1+B5:C9*SUM(3:7)+Sheet1!$D$4", "SUM(A:A,B2:Z1000)", "E5:E5"] {
            for (at, c) in [(0, 1), (3, 2), (4, 10), (100, 1)] {
                let ins = s(f, ir(at, c));
                assert_eq!(s(&ins, dr(at, c)), f, "rows {f} {at} {c}");
                let ins = s(f, ic(at, c));
                assert_eq!(s(&ins, dc(at, c)), f, "cols {f} {at} {c}");
            }
        }
    }

    fn gen_formulas(n: usize) -> Vec<String> {
        let templates = [
            "SUM(A{r}:A{r2})*Sheet2!B{r}+IF(C{r}>0,VLOOKUP(D{r},'My Sheet'!$A$1:$C$500,2,FALSE),\"x\")",
            "A{r}+B{r}*C{r}",
            "IFERROR(INDEX(Data!$B:$B,MATCH($A{r},Data!$A:$A,0)),\"\")",
            "_xlfn.XLOOKUP(E{r},Table1[Key],Table1[Value],#N/A)",
            "ROUND(F{r}*1.5E-3+G{r}/H$2,2)",
            "SUMIFS($K:$K,$L:$L,\"A1\",$M:$M,\">\"&N{r})",
        ];
        (0..n)
            .map(|i| {
                let r = i % 50_000 + 1;
                templates[i % templates.len()].replace("{r2}", &(r + 20).to_string()).replace("{r}", &r.to_string())
            })
            .collect()
    }

    #[test]
    #[ignore]
    fn bench_100k() {
        use std::time::Instant;
        let fs = gen_formulas(100_000);
        let bytes: usize = fs.iter().map(|f| f.len()).sum();
        for (name, op) in [("insert", ir(1000, 3)), ("delete", dr(1000, 3)), ("cols", ic(3, 1))] {
            let t = Instant::now();
            let mut changed = 0;
            for f in &fs {
                if shift_formula(f, "Sheet1", "Sheet1", op).is_some() {
                    changed += 1;
                }
            }
            let el = t.elapsed();
            println!(
                "shift {name}: {} formulas ({:.1} MB) in {:?} -> {:.0} formulas/s, {} changed",
                fs.len(),
                bytes as f64 / 1e6,
                el,
                fs.len() as f64 / el.as_secs_f64(),
                changed
            );
        }
        let t = Instant::now();
        let mut changed = 0;
        for f in &fs {
            if shift_formula(f, "Sheet9", "Sheet1", ir(0, 1)).is_some() {
                changed += 1;
            }
        }
        println!("shift other-sheet: {:?} ({changed} changed)", t.elapsed());
        let t = Instant::now();
        let mut total = 0;
        for f in &fs {
            total += translate_formula(f, 4, 1).len();
        }
        println!("translate: {:?} ({total} bytes out)", t.elapsed());
    }
}
