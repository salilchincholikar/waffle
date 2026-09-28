//! Delimited-text tokenizing (CSV/TSV records and fields) and the rules for
//! turning field text into values without changing it.

use crate::cell::{Cell, Kind};
use crate::sheet::Sheet;

pub struct Field {
    pub start: usize,
    pub end: usize,
    pub quoted: bool,
    /// Contains doubled quotes that need unescaping.
    pub escaped: bool,
}

pub struct Record {
    pub fields: Vec<Field>,
}

/// Parse one record starting at `pos`. Returns the record and the offset of the next one.
pub fn next_record(buf: &[u8], pos: usize, delim: u8) -> Option<(Record, usize)> {
    let mut fields = Vec::new();
    next_record_into(buf, pos, delim, &mut fields).map(|next| (Record { fields }, next))
}

pub fn next_record_into(buf: &[u8], mut pos: usize, delim: u8, fields: &mut Vec<Field>) -> Option<usize> {
    fields.clear();
    if pos >= buf.len() {
        return None;
    }
    loop {
        if pos < buf.len() && buf[pos] == b'"' {
            // Quoted field.
            let start = pos + 1;
            let mut i = start;
            let mut escaped = false;
            loop {
                match memchr::memchr(b'"', &buf[i..]) {
                    Some(j) => {
                        let q = i + j;
                        if buf.get(q + 1) == Some(&b'"') {
                            escaped = true;
                            i = q + 2;
                            continue;
                        }
                        fields.push(Field { start, end: q, quoted: true, escaped });
                        // Skip to the delimiter or end of line (tolerate junk after the quote).
                        let mut k = q + 1;
                        while k < buf.len() && buf[k] != delim && buf[k] != b'\n' && buf[k] != b'\r' {
                            k += 1;
                        }
                        pos = k;
                        break;
                    }
                    None => {
                        // Unterminated quote: take the rest of the file.
                        fields.push(Field { start, end: buf.len(), quoted: true, escaped });
                        return Some(buf.len());
                    }
                }
            }
        } else {
            let rest = &buf[pos..];
            let end = memchr::memchr3(delim, b'\n', b'\r', rest).map_or(buf.len(), |j| pos + j);
            fields.push(Field { start: pos, end, quoted: false, escaped: false });
            pos = end;
        }
        match buf.get(pos) {
            Some(&c) if c == delim => pos += 1,
            Some(b'\r') => {
                pos += if buf.get(pos + 1) == Some(&b'\n') { 2 } else { 1 };
                return Some(pos);
            }
            Some(b'\n') => return Some(pos + 1),
            None => return Some(pos),
            Some(_) => unreachable!(),
        }
    }
}

/// A number only if printing it gives back exactly `s`.
#[inline]
pub fn canonical_number(s: &str) -> Option<f64> {
    let b = s.as_bytes();
    if b.is_empty() || b.len() > 24 {
        return None;
    }
    let mut i = 0;
    if b[0] == b'-' {
        i = 1;
    }
    let int_start = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    let int_len = i - int_start;
    if int_len == 0 || (int_len > 1 && b[int_start] == b'0') {
        return None;
    }
    let mut frac_len = 0;
    if i < b.len() {
        if b[i] != b'.' {
            return None;
        }
        i += 1;
        let fs = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        frac_len = i - fs;
        if i != b.len() || frac_len == 0 || b[i - 1] == b'0' {
            return None;
        }
    }
    // Significant digits: strip leading zeros, and trailing zeros of a bare integer.
    let digits: Vec<u8> = b[int_start..].iter().copied().filter(u8::is_ascii_digit).collect();
    let lead = digits.iter().take_while(|&&c| c == b'0').count();
    let trail = if frac_len == 0 { digits.iter().rev().take_while(|&&c| c == b'0').count() } else { 0 };
    let sig = digits.len().saturating_sub(lead + trail);
    if sig > 15 {
        return None;
    }
    let v: f64 = s.parse().ok()?;
    if v == 0.0 && b[0] == b'-' {
        return None;
    }
    Some(v)
}

/// Display/serialise text of a CSV cell.
pub fn cell_text(s: &Sheet, v: Cell, out: &mut String) {
    use std::fmt::Write as _;
    match v.kind() {
        Kind::Empty => {}
        Kind::Number => {
            let _ = write!(out, "{}", v.as_number().unwrap());
        }
        Kind::Str => out.push_str(s.strings.get(v.as_str_id().unwrap())),
        Kind::Bool => out.push_str(if v.as_bool().unwrap() { "TRUE" } else { "FALSE" }),
        Kind::Error => out.push_str(v.as_error().unwrap()),
    }
}
