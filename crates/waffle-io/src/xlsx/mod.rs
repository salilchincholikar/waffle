//! XLSX / XLSM package handling.

pub mod parts;
pub mod read;
pub mod write;

pub use waffle_core::source::{Package, SheetPart};

use std::fs::File;
use std::io::Read;

use quick_xml::Reader;
use quick_xml::events::Event;

use crate::xml::{attr, local};

/// A reader with its own position over a shared file (positional reads), so
/// several threads can read one open file without disturbing each other.
pub struct At {
    file: std::sync::Arc<File>,
    pos: u64,
}

impl Read for At {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        use std::os::unix::fs::FileExt;
        let n = self.file.read_at(buf, self.pos)?;
        self.pos += n as u64;
        Ok(n)
    }
}

impl std::io::Seek for At {
    fn seek(&mut self, to: std::io::SeekFrom) -> std::io::Result<u64> {
        let len = self.file.metadata()?.len();
        let new = match to {
            std::io::SeekFrom::Start(p) => p as i64,
            std::io::SeekFrom::End(d) => len as i64 + d,
            std::io::SeekFrom::Current(d) => self.pos as i64 + d,
        };
        if new < 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "seek before start"));
        }
        self.pos = new as u64;
        Ok(self.pos)
    }
}

pub type ZipArchive = zip::ZipArchive<At>;

pub fn open_zip(file: &File) -> std::io::Result<ZipArchive> {
    let at = At { file: std::sync::Arc::new(file.try_clone()?), pos: 0 };
    zip::ZipArchive::new(at).map_err(std::io::Error::other)
}

pub fn read_entry(zip: &mut ZipArchive, name: &str) -> std::io::Result<Vec<u8>> {
    let real = find_entry(zip, name).ok_or_else(|| std::io::Error::other(format!("missing part {name}")))?;
    let mut f = zip.by_name(&real).map_err(std::io::Error::other)?;
    let mut out = Vec::with_capacity(f.size() as usize);
    f.read_to_end(&mut out)?;
    Ok(out)
}

/// Exact name of an entry, tolerating case differences and a leading slash.
pub fn find_entry(zip: &ZipArchive, name: &str) -> Option<String> {
    let name = name.trim_start_matches('/');
    if zip.index_for_name(name).is_some() {
        return Some(name.to_string());
    }
    zip.file_names().find(|n| n.eq_ignore_ascii_case(name) || n.replace('\\', "/").eq_ignore_ascii_case(name)).map(str::to_string)
}

/// Resolve a relationship target against the directory of its source part.
pub fn resolve(base_part: &str, target: &str) -> String {
    if let Some(abs) = target.strip_prefix('/') {
        return abs.to_string();
    }
    let mut parts: Vec<&str> = base_part.split('/').collect();
    parts.pop();
    for seg in target.split('/') {
        match seg {
            "." | "" => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

/// `xl/workbook.xml` → `xl/_rels/workbook.xml.rels`
pub fn rels_path(part: &str) -> String {
    match part.rsplit_once('/') {
        Some((dir, file)) => format!("{dir}/_rels/{file}.rels"),
        None => format!("_rels/{part}.rels"),
    }
}

pub struct Rel {
    pub id: String,
    pub kind: String,
    pub target: String,
    pub external: bool,
}

pub fn parse_rels(xml: &[u8]) -> Vec<Rel> {
    let mut out = Vec::new();
    let mut r = Reader::from_reader(xml);
    loop {
        match r.read_event() {
            Ok(Event::Start(e) | Event::Empty(e)) if local(e.name().as_ref()) == b"Relationship" => {
                out.push(Rel {
                    id: attr(&e, b"Id").unwrap_or_default(),
                    kind: attr(&e, b"Type").unwrap_or_default(),
                    target: attr(&e, b"Target").unwrap_or_default(),
                    external: attr(&e, b"TargetMode").as_deref() == Some("External"),
                });
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    out
}

pub fn rel_is(kind: &str, suffix: &str) -> bool {
    kind.rsplit('/').next() == Some(suffix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths() {
        assert_eq!(resolve("xl/workbook.xml", "worksheets/sheet1.xml"), "xl/worksheets/sheet1.xml");
        assert_eq!(resolve("xl/workbook.xml", "/xl/styles.xml"), "xl/styles.xml");
        assert_eq!(resolve("xl/worksheets/sheet1.xml", "../drawings/d1.xml"), "xl/drawings/d1.xml");
        assert_eq!(rels_path("xl/workbook.xml"), "xl/_rels/workbook.xml.rels");
    }
}
