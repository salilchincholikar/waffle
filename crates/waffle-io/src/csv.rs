//! CSV / TSV: dialect sniffing, streaming load, and "as is" saving.
//!
//! Values are kept exactly as written: a field becomes a number only when
//! printing that number reproduces the original text (`12`, `-3.5`), so
//! `00123`, `1.50`, `1e5` and long IDs stay text. On save, every record whose
//! fields are unchanged is copied byte-for-byte from the original file.

use std::fs::File;
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use encoding_rs::Encoding;

pub use waffle_core::delimited::*;
pub use waffle_core::source::{CsvSource, Dialect, Eol};

use crate::cell::Cell;
use crate::sheet::{Sheet, SheetBuilder};
use crate::strings::{SheetStrings, StringPool};
use crate::styles::Styles;
use crate::workbook::{Source, Workbook};

pub fn sniff(sample: &[u8], ext: &str) -> Dialect {
    let mut d = Dialect::default_for(ext);
    let mut body = sample;
    if sample.starts_with(b"\xEF\xBB\xBF") {
        d.bom = true;
        body = &sample[3..];
    } else if sample.starts_with(b"\xFF\xFE") {
        d.bom = true;
        d.encoding = encoding_rs::UTF_16LE;
        return d;
    } else if sample.starts_with(b"\xFE\xFF") {
        d.bom = true;
        d.encoding = encoding_rs::UTF_16BE;
        return d;
    }
    // Valid UTF-8 (allowing a truncated final character) or else Windows-1252.
    match std::str::from_utf8(body) {
        Ok(_) => {}
        Err(e) if e.error_len().is_none() => {}
        Err(_) => d.encoding = encoding_rs::WINDOWS_1252,
    }
    if let Some(i) = memchr::memchr(b'\n', body) {
        d.eol = if i > 0 && body[i - 1] == b'\r' { Eol::CrLf } else { Eol::Lf };
    } else if memchr::memchr(b'\r', body).is_some() {
        d.eol = Eol::Cr;
    }
    d.trailing_eol = true;

    if !ext.eq_ignore_ascii_case("tsv") {
        // Pick the delimiter whose per-line count is most consistent.
        let mut best = (0usize, b',');
        for &cand in b",;\t|" {
            let counts = count_per_line(body, cand, 50);
            if counts.is_empty() {
                continue;
            }
            let mut sorted = counts.clone();
            sorted.sort_unstable();
            let mode = sorted[sorted.len() / 2];
            if mode == 0 {
                continue;
            }
            let agree = counts.iter().filter(|&&c| c == mode).count();
            let score = agree * 1000 + mode.min(999);
            if score > best.0 {
                best = (score, cand);
            }
        }
        d.delim = best.1;
    }
    // Quote style from the first lines.
    let mut fields = 0;
    let mut quoted = 0;
    let mut pos = 0;
    for _ in 0..20 {
        let Some((rec, next)) = next_record(body, pos, d.delim) else { break };
        for f in rec.fields {
            fields += 1;
            if f.quoted {
                quoted += 1;
            }
        }
        pos = next;
    }
    d.quote_all = fields > 1 && quoted == fields;
    d
}

fn count_per_line(body: &[u8], delim: u8, max_lines: usize) -> Vec<usize> {
    let mut out = Vec::new();
    let mut pos = 0;
    while out.len() < max_lines {
        let Some((rec, next)) = next_record(body, pos, delim) else { break };
        // A final partial line in a sample is unreliable.
        if next >= body.len() && !body.ends_with(b"\n") && !out.is_empty() {
            break;
        }
        out.push(rec.fields.len() - 1);
        pos = next;
    }
    out
}

fn field_text<'a>(buf: &'a [u8], f: &Field, enc: &'static Encoding, scratch: &'a mut String) -> &'a str {
    let raw = &buf[f.start..f.end];
    let decoded: std::borrow::Cow<'_, str> =
        if enc == encoding_rs::UTF_8 { String::from_utf8_lossy(raw) } else { enc.decode_without_bom_handling(raw).0 };
    if f.escaped {
        *scratch = decoded.replace("\"\"", "\"");
        scratch.as_str()
    } else {
        match decoded {
            std::borrow::Cow::Borrowed(s) => s,
            std::borrow::Cow::Owned(s) => {
                *scratch = s;
                scratch.as_str()
            }
        }
    }
}

// ---- load ---------------------------------------------------------------

pub fn open(mut file: File, ext: &str, name: &str) -> std::io::Result<(Workbook, Vec<u8>)> {
    let mut sample = vec![0u8; 64 * 1024];
    let n = read_full(&mut file, &mut sample)?;
    sample.truncate(n);
    let dialect = sniff(&sample, ext);
    let sheet = Sheet::new(name.to_string(), SheetStrings::new(Arc::new(StringPool::new())));
    let mut sheet = sheet;
    sheet.loaded = false;
    // CSV has no row limit (unlike xlsx).
    sheet.max_rows = crate::sheet::UNBOUNDED_ROWS;
    sheet.default_col_width = 12.7109375; // ~94px: CSVs have no widths, give text a bit more room.
    let src = CsvSource { file, dialect, offsets: Vec::new(), transcoded: false };
    Ok((Workbook::new(vec![sheet], Styles::minimal(), Source::Csv(Box::new(src))), sample))
}

fn read_full(f: &mut File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match f.read(&mut buf[n..])? {
            0 => break,
            k => n += k,
        }
    }
    Ok(n)
}

pub struct LoadCtl<'a> {
    pub wb: &'a Mutex<Workbook>,
    pub progress: &'a AtomicU32,
    pub cancel: &'a AtomicBool,
}

const CHUNK: usize = 4 << 20;
const BATCH_CELLS: usize = 64 * 1024;

enum Staged {
    Num(f64),
    Text(u32, u32),
}

/// Stream the whole file into sheet 0.
pub fn load(ctl: &LoadCtl<'_>) -> std::io::Result<()> {
    let (mut file, dialect) = {
        let wb = ctl.wb.lock().unwrap();
        let Source::Csv(src) = &wb.source else { return Ok(()) };
        (src.file.try_clone()?, src.dialect)
    };
    let total = file.metadata()?.len().max(1);
    file.seek(SeekFrom::Start(0))?;

    // UTF-16: transcode to UTF-8 up front (rare; loses byte-exact copying).
    let transcoded = dialect.encoding == encoding_rs::UTF_16LE || dialect.encoding == encoding_rs::UTF_16BE;
    let mut data = Vec::new();
    let mut base: u64 = 0; // file offset of data[0]
    let mut eof = false;
    if transcoded {
        let mut raw = Vec::new();
        file.read_to_end(&mut raw)?;
        let (s, _, _) = dialect.encoding.decode(&raw);
        data = s.into_owned().into_bytes();
        eof = true;
    } else if dialect.bom {
        let mut bom = [0u8; 3];
        file.read_exact(&mut bom)?;
        base = 3;
    }
    let enc = if transcoded { encoding_rs::UTF_8 } else { dialect.encoding };

    let mut offsets: Vec<u64> = Vec::new();
    let mut fields = Vec::new();
    let mut row: u32 = 0;
    let mut scratch = String::new();
    let mut staged: Vec<(u32, u32, Staged)> = Vec::new();
    let mut text = String::new();
    let mut max_col = 0u32;
    let mut pos = 0usize;

    loop {
        if !eof {
            // Top up the buffer so it holds at least one complete record past `pos`.
            data.drain(..pos);
            base += pos as u64;
            pos = 0;
            let old = data.len();
            data.resize(old + CHUNK, 0);
            let n = read_full(&mut file, &mut data[old..])?;
            data.truncate(old + n);
            eof = n == 0 || n < CHUNK;
        }
        // Parse complete records; the last one may be cut by the chunk boundary.
        while let Some(next) = next_record_into(&data, pos, dialect.delim, &mut fields) {
            let complete = eof || (next < data.len() && ends_record(&data, next));
            if !complete {
                break;
            }
            offsets.push(base + pos as u64);
            let single_empty = fields.len() == 1 && fields[0].start == fields[0].end && !fields[0].quoted;
            if !single_empty {
                for (c, f) in fields.iter().enumerate() {
                    let t = field_text(&data, f, enc, &mut scratch);
                    if t.is_empty() {
                        continue;
                    }
                    let v = match canonical_number(t) {
                        Some(n) => Staged::Num(n),
                        None => {
                            let s = text.len() as u32;
                            text.push_str(t);
                            Staged::Text(s, text.len() as u32)
                        }
                    };
                    staged.push((row, c as u32, v));
                }
                max_col = max_col.max(fields.len() as u32);
            }
            row += 1;
            pos = next;
            if staged.len() >= BATCH_CELLS {
                flush(ctl, &mut staged, &mut text, row, max_col);
                ctl.progress.store(((base + pos as u64) * 1000 / total).min(999) as u32, Ordering::Relaxed);
                if ctl.cancel.load(Ordering::Relaxed) {
                    return Ok(());
                }
            }
        }
        if eof {
            break;
        }
    }
    offsets.push(base + pos as u64);
    flush(ctl, &mut staged, &mut text, row, max_col);
    let mut wb = ctl.wb.lock().unwrap();
    let ends_with_eol = total > 0 && {
        let mut f = file.try_clone()?;
        let mut last = [0u8; 1];
        f.seek(SeekFrom::End(-1)).is_ok() && f.read(&mut last).unwrap_or(0) == 1 && (last[0] == b'\n' || last[0] == b'\r')
    };
    if let Source::Csv(src) = &mut wb.source {
        src.offsets = offsets;
        src.transcoded = transcoded;
        src.dialect.trailing_eol = ends_with_eol;
    }
    let s = &mut wb.sheets[0];
    s.strings.local.drop_index();
    s.strings.local.shrink();
    s.loaded = true;
    s.dirty = false;
    s.invalidate_geometry();
    ctl.progress.store(1000, Ordering::Relaxed);
    Ok(())
}

/// Is `next` a real record boundary (not the middle of a line cut by the buffer)?
fn ends_record(data: &[u8], next: usize) -> bool {
    matches!(data[next - 1], b'\n' | b'\r') && !(data[next - 1] == b'\r' && next == data.len())
}

fn flush(ctl: &LoadCtl<'_>, staged: &mut Vec<(u32, u32, Staged)>, text: &mut String, rows: u32, cols: u32) {
    let mut wb = ctl.wb.lock().unwrap();
    let s = &mut wb.sheets[0];
    let mut b = SheetBuilder::new(s);
    b.reserve(rows, cols);
    for (r, c, v) in staged.drain(..) {
        let cell = match v {
            Staged::Num(n) => Cell::number(n),
            Staged::Text(a, z) => Cell::string(b.sheet.strings.add(&text[a as usize..z as usize])),
        };
        b.put(r, c, cell, 0, None);
    }
    b.sheet.invalidate_geometry();
    text.clear();
}

// ---- save -------------------------------------------------------------------

/// Write the active sheet as CSV. With a source dialect, unchanged records are
/// copied from the original bytes.
pub fn save(wb: &Workbook, sheet: usize, path: &std::path::Path, dialect: Option<Dialect>) -> std::io::Result<()> {
    let s = &wb.sheets[sheet];
    let src = match &wb.source {
        Source::Csv(src) if sheet == 0 && !src.transcoded => Some(src),
        _ => None,
    };
    let d = dialect.or(match &wb.source {
        Source::Csv(src) => Some(src.dialect),
        _ => None,
    });
    let d = d.unwrap_or_else(|| Dialect::default_for(path.extension().and_then(|e| e.to_str()).unwrap_or("csv")));
    let out = File::create(path)?;
    let mut w = BufWriter::with_capacity(1 << 20, out);
    if d.bom {
        if d.encoding == encoding_rs::UTF_16LE {
            w.write_all(b"\xFF\xFE")?;
        } else if d.encoding == encoding_rs::UTF_16BE {
            w.write_all(b"\xFE\xFF")?;
        } else {
            w.write_all(b"\xEF\xBB\xBF")?;
        }
    }

    // Original bytes, read in one pass per record through a small cache.
    let mut orig = src.map(|src| OrigReader { file: src.file.try_clone().ok(), offsets: &src.offsets, buf: Vec::new(), buf_start: 0 });

    let rows = s.row_count();
    let cols = s.col_count();
    let mut line = String::new();
    let mut cell = String::new();
    let mut fields = Vec::new();
    let mut scratch = String::new();
    // Drop trailing rows with no content.
    let mut last_row = rows;
    while last_row > 0 && (0..cols).all(|c| s.get(last_row - 1, c).is_empty()) {
        let pr = s.grid.row_map[last_row as usize - 1];
        // Keep originally-empty rows that were in the file.
        if src.is_some_and(|src| (pr as usize) < src.offsets.len().saturating_sub(1)) {
            break;
        }
        last_row -= 1;
    }
    for r in 0..last_row {
        let pr = s.grid.row_map[r as usize];
        let is_last = r + 1 == last_row;
        // Try the original record.
        if let (Some(o), Some(src)) = (orig.as_mut(), src)
            && let Some(bytes) = o.record(pr as usize)
        {
            let body = strip_eol(bytes);
            let _ = next_record_into(body, 0, d.delim, &mut fields);
            if body.is_empty() {
                fields.clear();
            }
            let n = fields.len() as u32;
            let same = (0..cols.max(n)).all(|c| {
                cell.clear();
                cell_text(s, s.get(r, c), &mut cell);
                let orig_text =
                    fields.get(c as usize).map(|f| field_text(body, f, src.dialect.encoding, &mut scratch).to_string()).unwrap_or_default();
                orig_text == cell
            });
            if same {
                w.write_all(body)?;
                if !is_last || d.trailing_eol {
                    w.write_all(d.eol.as_bytes())?;
                }
                continue;
            }
        }
        // Serialise.
        line.clear();
        let mut last = 0;
        for c in 0..cols {
            if !s.get(r, c).is_empty() {
                last = c + 1;
            }
        }
        let orig_n = orig.as_mut().and_then(|o| o.record(pr as usize)).map(|b| {
            let body = strip_eol(b);
            if body.is_empty() { 0 } else { next_record(body, 0, d.delim).map_or(0, |(rec, _)| rec.fields.len() as u32) }
        });
        let width = last.max(orig_n.unwrap_or(0));
        for c in 0..width {
            if c > 0 {
                line.push(d.delim as char);
            }
            cell.clear();
            cell_text(s, s.get(r, c), &mut cell);
            quote_into(&cell, d, &mut line);
        }
        let (bytes, _, _) = d.encoding.encode(&line);
        w.write_all(&bytes)?;
        if !is_last || d.trailing_eol {
            w.write_all(d.eol.as_bytes())?;
        }
    }
    w.flush()?;
    Ok(())
}

fn strip_eol(b: &[u8]) -> &[u8] {
    let mut e = b.len();
    while e > 0 && matches!(b[e - 1], b'\n' | b'\r') {
        e -= 1;
    }
    &b[..e]
}

fn quote_into(s: &str, d: Dialect, out: &mut String) {
    let needs =
        d.quote_all || s.bytes().any(|b| b == d.delim || b == b'"' || b == b'\n' || b == b'\r') || s.starts_with(' ') || s.ends_with(' ');
    if !needs || (s.is_empty() && !d.quote_all) {
        out.push_str(s);
        return;
    }
    out.push('"');
    for ch in s.chars() {
        if ch == '"' {
            out.push('"');
        }
        out.push(ch);
    }
    out.push('"');
}

struct OrigReader<'a> {
    file: Option<File>,
    offsets: &'a [u64],
    buf: Vec<u8>,
    buf_start: u64,
}

impl OrigReader<'_> {
    /// Raw bytes of original record `i` (including its line ending).
    fn record(&mut self, i: usize) -> Option<&[u8]> {
        if i + 1 >= self.offsets.len() {
            return None;
        }
        let (a, z) = (self.offsets[i], self.offsets[i + 1]);
        let in_buf = a >= self.buf_start && z <= self.buf_start + self.buf.len() as u64;
        if !in_buf {
            let f = self.file.as_mut()?;
            let len = ((z - a) as usize).max(1 << 20);
            f.seek(SeekFrom::Start(a)).ok()?;
            self.buf.resize(len, 0);
            let n = read_full(f, &mut self.buf).ok()?;
            self.buf.truncate(n);
            self.buf_start = a;
            if (z - a) as usize > n {
                return None;
            }
        }
        let s = (a - self.buf_start) as usize;
        Some(&self.buf[s..s + (z - a) as usize])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical() {
        for ok in ["0", "12", "-3.5", "0.1", "100", "1000000000000000000000", "123456789012345", "0.000123"] {
            assert!(canonical_number(ok).is_some(), "{ok}");
            assert_eq!(format!("{}", canonical_number(ok).unwrap()), ok);
        }
        for no in
            ["00123", "1.50", "1e5", "+1", ".5", "-0", "1,000", "1234567890123456", "12345678901234567890", "", "-", "1.", "0x1F", " 1"]
        {
            assert!(canonical_number(no).is_none(), "{no}");
        }
    }

    #[test]
    fn records() {
        let data = b"a,\"b,\"\"c\"\"\",d\r\n\"multi\nline\",x\nlast";
        let (r1, n1) = next_record(data, 0, b',').unwrap();
        assert_eq!(r1.fields.len(), 3);
        let mut s = String::new();
        assert_eq!(field_text(data, &r1.fields[1], encoding_rs::UTF_8, &mut s), "b,\"c\"");
        let (r2, n2) = next_record(data, n1, b',').unwrap();
        assert_eq!(&data[r2.fields[0].start..r2.fields[0].end], b"multi\nline");
        let (r3, n3) = next_record(data, n2, b',').unwrap();
        assert_eq!(r3.fields.len(), 1);
        assert_eq!(n3, data.len());
        assert!(next_record(data, n3, b',').is_none());
    }

    #[test]
    fn sniffing() {
        assert_eq!(sniff(b"a;b;c\n1,5;2;3\n", "csv").delim, b';');
        assert_eq!(sniff(b"a\tb\n1\t2\n", "txt").delim, b'\t');
        let d = sniff(b"\xEF\xBB\xBFa,b\r\n1,2\r\n", "csv");
        assert!(d.bom);
        assert_eq!(d.eol, Eol::CrLf);
        assert_eq!(sniff(b"caf\xE9,1\n", "csv").encoding, encoding_rs::WINDOWS_1252);
        assert!(sniff(b"\"a\",\"b\"\n\"1\",\"2\"\n", "csv").quote_all);
    }
}
