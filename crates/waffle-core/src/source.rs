//! Where a workbook came from: the facts about the original file that saving
//! needs (so unchanged parts can be copied as they were). Plain data; the
//! reading and writing live in `waffle-io`.

use std::fs::File;

use encoding_rs::Encoding;

/// What we remember about the original package so that saving can copy
/// every part we didn't change byte-for-byte.
pub struct Package {
    /// The original file, kept open: even if it is replaced on disk, we keep
    /// reading the version we loaded.
    pub file: File,
    pub entries: Vec<String>,
    pub wb_path: String,
    pub wb_xml: Vec<u8>,
    pub wb_rels_path: String,
    pub wb_rels: Vec<u8>,
    pub content_types: Vec<u8>,
    pub sst_path: Option<String>,
    /// Number of `<si>` entries currently in the file's shared-string table.
    pub sst_file_count: usize,
    pub styles_path: Option<String>,
    /// Set when a sheet was added, removed, renamed or reordered.
    pub book_dirty: bool,
    /// Set when cells that formulas may depend on were edited.
    pub needs_recalc: bool,
    pub macro_enabled: bool,
}

/// Per-sheet info about its part in the original package.
#[derive(Clone)]
pub struct SheetPart {
    pub path: String,
    pub rel_id: String,
    pub sheet_id: u32,
    /// Raw XML before `<sheetData>` and after `</sheetData>`.
    pub prefix: Vec<u8>,
    pub suffix: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Eol {
    Lf,
    CrLf,
    Cr,
}

impl Eol {
    pub fn as_bytes(self) -> &'static [u8] {
        match self {
            Eol::Lf => b"\n",
            Eol::CrLf => b"\r\n",
            Eol::Cr => b"\r",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Dialect {
    pub delim: u8,
    pub encoding: &'static Encoding,
    pub bom: bool,
    pub eol: Eol,
    /// Every field in the sample was quoted.
    pub quote_all: bool,
    pub trailing_eol: bool,
}

impl Dialect {
    pub fn default_for(ext: &str) -> Dialect {
        Dialect {
            delim: if ext.eq_ignore_ascii_case("tsv") { b'\t' } else { b',' },
            encoding: encoding_rs::UTF_8,
            bom: false,
            eol: Eol::Lf,
            quote_all: false,
            trailing_eol: true,
        }
    }
}

pub struct CsvSource {
    pub file: File,
    pub dialect: Dialect,
    /// Byte offset where each original record starts (+ one final end offset).
    pub offsets: Vec<u64>,
    /// The file was transcoded (UTF-16) so byte copying isn't possible.
    pub transcoded: bool,
}

/// Shared strings: plain text of each `<si>` (rich runs concatenated, phonetic runs skipped).
/// One formatted run inside a rich-text shared string (byte offsets into the text).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Run {
    pub start: u32,
    pub end: u32,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub strike: Option<bool>,
    pub size: Option<f32>,
    pub color: Option<u32>,
    pub font: Option<String>,
}

pub type RichMap = std::collections::HashMap<u32, Vec<Run>>;
