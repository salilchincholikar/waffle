//! The open document: sheets, styles, where it came from, and undo history.

use std::sync::Arc;

use crate::sheet::{BN, Grid, Sheet};
use crate::source::CsvSource;
use crate::source::Package;
use crate::styles::Styles;

pub enum Source {
    /// Created in the app; Save must ask for a location.
    New,
    Xlsx(Box<Package>),
    Csv(Box<CsvSource>),
    /// xls / xlsb / ods: import-only formats, saved as a new xlsx.
    Legacy(String),
}

impl Source {
    pub fn is_csv(&self) -> bool {
        matches!(self, Source::Csv(_))
    }
}

/// Undo memory budget: oldest steps are dropped beyond this.
const UNDO_BUDGET: usize = 256 << 20;
const UNDO_MAX_STEPS: usize = 1000;

/// Changes that move or rename cells/sheets, recorded so that saving can fix up
/// references in parts of the file we don't model (charts, tables, names…).
#[derive(Clone, Debug)]
pub enum LogOp {
    Struct { sheet: String, op: crate::refshift::StructOp },
    Rename { old: String, new: String },
    Delete { name: String },
}

pub enum BookChange {
    Renamed { index: usize, old: String },
    Added { index: usize },
    Removed { index: usize, sheet: Box<Sheet> },
    Moved { from: usize, to: usize },
}

pub struct UndoEntry {
    pub label: String,
    /// Grids to restore, captured before the change (indices valid before `book` is reverted).
    grids: Vec<(usize, Grid, bool)>,
    book: Option<BookChange>,
    /// Grids are indexed by the sheet order *without* `book` applied, so the
    /// book change is reverted first on undo and re-applied last on redo.
    book_first: bool,
    log: std::sync::Arc<Vec<LogOp>>,
    bytes: usize,
    pub sheet: usize,
}

pub struct Workbook {
    pub sheets: Vec<Sheet>,
    pub styles: Styles,
    pub date1904: bool,
    pub source: Source,
    pub active: usize,
    undo: Vec<UndoEntry>,
    redo: Vec<UndoEntry>,
    pending: Option<UndoEntry>,
    undo_bytes: usize,
    /// Anything changed since the last save.
    pub edited: bool,
    /// Structural changes since the file was opened, in order.
    pub log: std::sync::Arc<Vec<LogOp>>,
    /// Run formatting of rich-text shared strings, by string id.
    pub rich: std::sync::Arc<crate::source::RichMap>,
    /// Defined names: (name, sheet scope, formula) — kept current for calculation.
    pub names: std::sync::Arc<Vec<(String, Option<usize>, String)>>,
    /// Names as they were in the file; `names` = these with the log replayed.
    pub names_orig: std::sync::Arc<Vec<(String, Option<usize>, String)>>,
    /// The file asks for a full recalculation once loaded (xlsx `fullCalcOnLoad`,
    /// written by tools that don't store formula results). Cleared by [`Workbook::finish_load`].
    pub calc_on_load: bool,
}

impl Workbook {
    pub fn new(sheets: Vec<Sheet>, styles: Styles, source: Source) -> Workbook {
        Workbook {
            sheets,
            styles,
            date1904: false,
            source,
            active: 0,
            undo: Vec::new(),
            redo: Vec::new(),
            pending: None,
            undo_bytes: 0,
            edited: false,
            log: std::sync::Arc::new(Vec::new()),
            rich: Default::default(),
            names: Default::default(),
            names_orig: Default::default(),
            calc_on_load: false,
        }
    }

    /// Every sheet is loaded and any recalculation the file asked for has run.
    pub fn loaded(&self) -> bool {
        !self.calc_on_load && self.sheets.iter().all(|s| s.loaded)
    }

    /// Call when a loader finishes: once every sheet is in, runs the recalculation the
    /// file asked for. Results live in memory only; the file isn't marked edited.
    pub fn finish_load(&mut self) {
        if self.calc_on_load && self.sheets.iter().all(|s| s.loaded) {
            crate::recalc::run(self, &[], true);
            self.calc_on_load = false;
        }
    }

    pub fn is_csv(&self) -> bool {
        self.source.is_csv()
    }

    pub fn sheet_index(&self, name: &str) -> Option<usize> {
        self.sheets.iter().position(|s| crate::refshift::sheet_names_eq(&s.name, name))
    }

    // ---- undo ---------------------------------------------------------------

    /// Start an undoable change touching `sheets`. Must be followed by `commit`.
    pub fn begin(&mut self, label: &str, sheets: &[usize]) {
        let n = self.sheets.len();
        let grids = sheets
            .iter()
            .filter(|&&i| i < n)
            .map(|&i| {
                self.sheets[i].cow_copies = 0;
                (i, self.sheets[i].grid.clone(), self.sheets[i].dirty)
            })
            .collect();
        for s in &mut self.sheets {
            s.track = true;
            s.touched.clear();
        }
        self.pending = Some(UndoEntry {
            label: label.to_string(),
            grids,
            book: None,
            book_first: true,
            log: self.log.clone(),
            bytes: 0,
            // Whole-workbook snapshots (structural edits) happen on the sheet being viewed.
            sheet: if sheets.len() == 1 { sheets[0] } else { self.active },
        });
    }

    pub fn begin_all(&mut self, label: &str) {
        let all: Vec<usize> = (0..self.sheets.len()).collect();
        self.begin(label, &all);
    }

    pub fn set_book_change(&mut self, change: BookChange) {
        if let Some(p) = &mut self.pending {
            p.book = Some(change);
        }
    }

    pub fn commit(&mut self) {
        // Recalculate formulas affected by this change, inside the same undo step.
        let structural = self.pending.as_ref().is_some_and(|p| !Arc::ptr_eq(&p.log, &self.log) || p.book.is_some());
        let mut touched = Vec::new();
        for (si, s) in self.sheets.iter_mut().enumerate() {
            s.track = false;
            touched.extend(s.take_touched().into_iter().map(|(r, c)| (si, r, c)));
        }
        if self.loaded() && (structural || !touched.is_empty()) {
            crate::recalc::run(self, &touched, structural || touched.len() > 200_000);
        }
        let Some(mut e) = self.pending.take() else { return };
        e.bytes = e
            .grids
            .iter()
            .map(|(i, g, _)| {
                let copies = self.sheets.get(*i).map_or(0, |s| s.cow_copies);
                copies * BN * 10 + (g.row_map.len() + g.col_map.len()) * 4 + g.blocks.len() * 64
            })
            .sum::<usize>()
            + 256;
        self.undo_bytes += e.bytes;
        self.undo.push(e);
        self.redo.clear();
        self.edited = true;
        while self.undo.len() > 1 && (self.undo_bytes > UNDO_BUDGET || self.undo.len() > UNDO_MAX_STEPS) {
            let old = self.undo.remove(0);
            self.undo_bytes -= old.bytes;
        }
    }

    /// Abandon a started change without recording it (nothing was modified).
    pub fn cancel(&mut self) {
        for s in &mut self.sheets {
            s.track = false;
            s.touched.clear();
        }
        self.pending = None;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|e| e.label.as_str())
    }
    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|e| e.label.as_str())
    }

    /// Returns the sheet index the change happened on.
    pub fn undo(&mut self) -> Option<usize> {
        let e = self.undo.pop()?;
        self.undo_bytes -= e.bytes;
        let inverse = self.apply(e);
        let sheet = inverse.sheet;
        self.redo.push(inverse);
        self.edited = true;
        Some(sheet)
    }

    pub fn redo(&mut self) -> Option<usize> {
        let e = self.redo.pop()?;
        let inverse = self.apply(e);
        let sheet = inverse.sheet;
        self.undo_bytes += inverse.bytes;
        self.undo.push(inverse);
        self.edited = true;
        Some(sheet)
    }

    /// Restore an entry's state and return the entry that reverses it.
    fn apply(&mut self, e: UndoEntry) -> UndoEntry {
        let mut inverse_book = None;
        let book_first = e.book_first;
        let inverse_log = std::mem::replace(&mut self.log, e.log);
        self.refresh_names();
        let mut book = e.book;
        if book_first {
            inverse_book = book.take().map(|c| self.apply_book(c));
        }
        let mut grids = Vec::with_capacity(e.grids.len());
        for (i, g, dirty) in e.grids {
            if let Some(s) = self.sheets.get_mut(i) {
                let cur = std::mem::replace(&mut s.grid, g);
                grids.push((i, cur, s.dirty));
                // Restored content differs from the file unless it was clean then.
                s.dirty = dirty || s.dirty;
                s.invalidate_geometry();
                s.filter_hidden = None;
            }
        }
        if !book_first {
            inverse_book = book.take().map(|c| self.apply_book(c));
        }
        if self.active >= self.sheets.len() {
            self.active = self.sheets.len().saturating_sub(1);
        }
        UndoEntry {
            label: e.label,
            grids,
            book: inverse_book,
            book_first: !book_first,
            log: inverse_log,
            bytes: e.bytes,
            sheet: e.sheet.min(self.sheets.len().saturating_sub(1)),
        }
    }

    fn apply_book(&mut self, change: BookChange) -> BookChange {
        self.mark_book_dirty();
        match change {
            BookChange::Renamed { index, old } => {
                let cur = std::mem::replace(&mut self.sheets[index].name, old);
                BookChange::Renamed { index, old: cur }
            }
            BookChange::Added { index } => {
                let s = self.sheets.remove(index);
                BookChange::Removed { index, sheet: Box::new(s) }
            }
            BookChange::Removed { index, sheet } => {
                self.sheets.insert(index, *sheet);
                BookChange::Added { index }
            }
            BookChange::Moved { from, to } => {
                let s = self.sheets.remove(to);
                self.sheets.insert(from, s);
                BookChange::Moved { from: to, to: from }
            }
        }
    }

    pub fn push_log(&mut self, op: LogOp) {
        std::sync::Arc::make_mut(&mut self.log).push(op);
        self.refresh_names();
    }

    /// Recompute defined names from the originals and the change log.
    pub fn refresh_names(&mut self) {
        if self.names_orig.is_empty() {
            return;
        }
        let mut names: Vec<(String, Option<usize>, String)> = (*self.names_orig).clone();
        for op in self.log.iter() {
            for n in &mut names {
                let new = match op {
                    LogOp::Struct { sheet, op } => crate::refshift::shift_formula(&n.2, "", sheet, *op),
                    LogOp::Rename { old, new } => crate::refshift::rename_sheet_in_formula(&n.2, old, new),
                    LogOp::Delete { name } => crate::refshift::delete_sheet_in_formula(&n.2, name),
                };
                if let Some(t) = new {
                    n.2 = t;
                }
            }
        }
        self.names = Arc::new(names);
    }

    pub fn mark_book_dirty(&mut self) {
        if let Source::Xlsx(p) = &mut self.source {
            p.book_dirty = true;
        }
    }

    /// Keep each sheet's style metrics (for automatic row heights) in step with the styles.
    pub fn sync_metrics(&mut self, si: usize) {
        let generation = self.styles.generation.wrapping_add(1);
        let Some(s) = self.sheets.get(si) else { return };
        if s.metrics.as_ref().is_some_and(|m| m.generation == generation && m.line_pt.len() == self.styles.xf_count()) {
            return;
        }
        let default_pt = self.styles.font(self.styles.xf(0).font).size;
        let n = self.styles.xf_count();
        let mut line_pt = Vec::with_capacity(n);
        let mut font_pt = Vec::with_capacity(n);
        let mut wrap = Vec::with_capacity(n);
        for i in 0..n {
            let xf = *self.styles.xf(i as u16);
            let size = self.styles.font(xf.font).size;
            font_pt.push(size);
            line_pt.push(if size > default_pt + 0.5 { (size * 1.36 * 4.0).round() / 4.0 } else { 0.0 });
            wrap.push(xf.wrap);
        }
        let m = std::sync::Arc::new(crate::sheet::StyleMetrics { generation, line_pt, font_pt, wrap });
        let s = &mut self.sheets[si];
        s.metrics = Some(m);
        s.invalidate_geometry();
    }

    /// Formula of a defined name visible from sheet `host` (sheet-scoped first).
    pub fn defined_name(&self, name: &str, host: usize) -> Option<String> {
        let eq = |a: &str| a.eq_ignore_ascii_case(name);
        self.names
            .iter()
            .find(|(n, scope, _)| eq(n) && *scope == Some(host))
            .or_else(|| self.names.iter().find(|(n, scope, _)| eq(n) && scope.is_none()))
            .map(|(_, _, f)| f.clone())
    }

    pub fn has_formulas(&self) -> bool {
        self.sheets.iter().any(|s| !s.formulas.is_empty())
    }

    pub fn heap_bytes(&self) -> usize {
        self.sheets.iter().map(|s| s.heap_bytes()).sum::<usize>()
            + self.sheets.first().map_or(0, |s| s.strings.shared.heap_bytes())
            + self.undo_bytes
    }
}
