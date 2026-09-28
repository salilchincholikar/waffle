//! Sheet storage.
//!
//! Cells live in fixed 256×16 blocks addressed by *physical* row/column.
//! Logical positions (what the user sees) go through `row_map`/`col_map`, so
//! inserting, deleting or sorting rows only rewrites a `Vec<u32>` and never
//! moves cell data. Blocks are `Arc`-shared with undo snapshots and copied
//! only when written (copy-on-write), so an undo step costs just the blocks
//! it touched.

use std::collections::HashMap;
use std::sync::Arc;

use crate::cell::Cell;
use crate::strings::SheetStrings;

pub const BR: usize = 256;
pub const BC: usize = 16;
pub const BN: usize = BR * BC;

/// Excel's hard limits.
pub const MAX_ROWS: u32 = 1_048_576;
/// Row limit for sheets whose format has none (CSV): effectively "as much as memory allows".
pub const UNBOUNDED_ROWS: u32 = i32::MAX as u32;
pub const MAX_COLS: u32 = 16_384;

#[derive(Clone)]
pub struct Block {
    pub vals: Box<[Cell]>,
    /// xf index per cell; `None` means every cell uses xf 0.
    pub styles: Option<Box<[u16]>>,
    /// formula id + 1 per cell; 0 = no formula.
    pub fids: Option<Box<[u32]>>,
}

impl Block {
    fn new() -> Self {
        Block { vals: vec![Cell::EMPTY; BN].into_boxed_slice(), styles: None, fids: None }
    }
    pub fn heap_bytes(&self) -> usize {
        BN * 8 + self.styles.as_ref().map_or(0, |_| BN * 2) + self.fids.as_ref().map_or(0, |_| BN * 4)
    }
}

#[inline]
fn slot(pr: u32, pc: u32) -> usize {
    (pr as usize % BR) * BC + pc as usize % BC
}

#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct RowMeta {
    /// Height in points, if set.
    pub height: Option<f32>,
    pub hidden: bool,
    pub custom_height: bool,
    /// Row-level default xf (`s` with `customFormat="1"`).
    pub style: Option<u16>,
    /// Index into `Sheet::raw_attrs` for attributes we don't model (1-based; 0 = none).
    pub extra: u32,
}

#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct ColMeta {
    /// Width in Excel character units, if set.
    pub width: Option<f32>,
    pub hidden: bool,
    pub custom_width: bool,
    pub style: Option<u16>,
    pub extra: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rect {
    pub r0: u32,
    pub c0: u32,
    pub r1: u32,
    pub c1: u32,
}

impl Rect {
    pub fn cell(r: u32, c: u32) -> Rect {
        Rect { r0: r, c0: c, r1: r, c1: c }
    }
    pub fn contains(&self, r: u32, c: u32) -> bool {
        r >= self.r0 && r <= self.r1 && c >= self.c0 && c <= self.c1
    }
    pub fn intersects(&self, o: &Rect) -> bool {
        self.r0 <= o.r1 && o.r0 <= self.r1 && self.c0 <= o.c1 && o.c0 <= self.c1
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum FKind {
    Normal,
    /// `ref_` is set on the master only.
    Shared {
        si: u32,
        ref_: Option<Box<str>>,
    },
    Array {
        ref_: Box<str>,
    },
    /// Anything else (data tables…) kept verbatim.
    Other,
}

#[derive(Clone, Debug)]
pub struct Formula {
    /// Formula text without the leading '='. Empty for shared-formula children.
    pub text: Box<str>,
    pub kind: FKind,
    /// Raw attributes we don't model (e.g. ` ca="1"`), written back verbatim.
    pub attrs: Box<str>,
}

/// Everything that an undo step needs to restore. Cloning is shallow.
#[derive(Clone)]
pub struct Grid {
    pub blocks: Vec<Vec<Option<Arc<Block>>>>,
    pub row_map: Arc<Vec<u32>>,
    pub col_map: Arc<Vec<u32>>,
    pub next_prow: u32,
    pub next_pcol: u32,
    pub rows: Arc<Vec<RowMeta>>,
    pub cols: Arc<Vec<ColMeta>>,
    pub merges: Arc<Vec<Rect>>,
    pub freeze_rows: u32,
    pub freeze_cols: u32,
    /// Conditional formatting (display only; the file's XML is what gets saved).
    pub cf: Arc<Vec<crate::cf::Block>>,
    /// Pictures and charts (display only).
    pub drawings: Arc<Vec<crate::drawings::Drawing>>,
    pub tables: Arc<Vec<crate::tables::Table>>,
}

impl Default for Grid {
    fn default() -> Self {
        Grid {
            blocks: Vec::new(),
            row_map: Arc::new(Vec::new()),
            col_map: Arc::new(Vec::new()),
            next_prow: 0,
            next_pcol: 0,
            rows: Arc::new(Vec::new()),
            cols: Arc::new(Vec::new()),
            merges: Arc::new(Vec::new()),
            freeze_rows: 0,
            freeze_cols: 0,
            cf: Arc::new(Vec::new()),
            drawings: Arc::new(Vec::new()),
            tables: Arc::new(Vec::new()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Visibility {
    Visible,
    Hidden,
    VeryHidden,
}

pub struct Sheet {
    pub name: String,
    pub grid: Grid,
    pub strings: SheetStrings,
    /// Append-only; blocks reference entries by id, so undo never needs to restore this.
    pub formulas: Vec<Formula>,
    /// Shared-formula group → formula id of its master.
    pub shared_masters: HashMap<u32, u32>,
    /// Raw XML for cells with attributes/children we don't model, keyed by physical position.
    pub cell_extras: HashMap<(u32, u32), Box<str>>,
    /// Interned raw attribute strings used by `RowMeta::extra` / `ColMeta::extra`.
    pub raw_attrs: Vec<Box<str>>,
    /// Rows this sheet may have: Excel's limit for workbook formats, unbounded for CSV.
    pub max_rows: u32,
    pub default_row_height: f32,
    /// Default column width in Excel character units.
    pub default_col_width: f32,
    pub visibility: Visibility,
    /// Transient filter state (logical rows hidden by a filter).
    pub filter_hidden: Option<Vec<bool>>,
    pub dirty: bool,
    pub loaded: bool,
    /// Where this sheet lives in the original xlsx package, if any.
    pub part: Option<Box<crate::source::SheetPart>>,
    /// Per-xf sizing facts used for automatic row heights (set by the workbook).
    pub metrics: Option<Arc<StyleMetrics>>,
    geom: Option<Geom>,
    /// Blocks copied by copy-on-write since the counter was last reset (undo accounting).
    pub cow_copies: usize,
    /// Bumped on every change; caches key off it.
    pub version: u64,
    pub cf_cache: crate::cf::Cache,
    /// While set, writes record their physical position (for recalculation).
    pub track: bool,
    pub touched: Vec<(u32, u32)>,
}

/// What each xf needs from a row: a taller line for a big font, or wrapping.
pub struct StyleMetrics {
    pub generation: u32,
    /// Row height in points needed by one line of this xf's font (0 = no more than default).
    pub line_pt: Vec<f32>,
    pub font_pt: Vec<f32>,
    pub wrap: Vec<bool>,
}

impl StyleMetrics {
    fn needs(&self, xf: u16) -> bool {
        let i = xf as usize;
        i < self.line_pt.len() && (self.line_pt[i] > 0.0 || self.wrap[i])
    }
}

struct Geom {
    row_pos: Vec<f64>,
    col_pos: Vec<f64>,
}

/// Points per Excel row-height point at 100% zoom (Excel renders 1pt as 4/3 px).
pub const PT_TO_PX: f64 = 4.0 / 3.0;
/// Maximum digit width of the default font (Calibri 11) in pixels.
pub const MDW: f64 = 7.0;

pub fn col_width_px(width: f32) -> f64 {
    ((width as f64 + 18.0 / 256.0) * MDW).trunc()
}

pub fn px_to_col_width(px: f64) -> f32 {
    let chars = ((px - 5.0) / MDW).max(0.0);
    (((chars * MDW + 5.0) / MDW * 256.0).trunc() / 256.0) as f32
}

impl Sheet {
    pub fn new(name: impl Into<String>, strings: SheetStrings) -> Sheet {
        Sheet {
            name: name.into(),
            grid: Grid::default(),
            strings,
            formulas: Vec::new(),
            shared_masters: HashMap::new(),
            cell_extras: HashMap::new(),
            raw_attrs: Vec::new(),
            max_rows: MAX_ROWS,
            default_row_height: 15.0,
            default_col_width: 9.140625,
            visibility: Visibility::Visible,
            filter_hidden: None,
            dirty: false,
            loaded: true,
            part: None,
            metrics: None,
            geom: None,
            cow_copies: 0,
            version: 0,
            cf_cache: Default::default(),
            track: false,
            touched: Vec::new(),
        }
    }

    // ---- extents -------------------------------------------------------

    /// Logical rows that may hold data (cells beyond are empty).
    #[inline]
    pub fn row_count(&self) -> u32 {
        self.grid.row_map.len() as u32
    }
    #[inline]
    pub fn col_count(&self) -> u32 {
        self.grid.col_map.len() as u32
    }

    /// Rows/columns shown in the grid: the data plus room to keep typing.
    pub fn display_rows(&self) -> u32 {
        (self.row_count() + 500).clamp(1000, self.max_rows)
    }
    pub fn display_cols(&self) -> u32 {
        (self.col_count() + 26).clamp(52, MAX_COLS)
    }

    /// Tight bounds of non-empty cells (logical), or None if the sheet is empty.
    pub fn used_rect(&self) -> Option<Rect> {
        let (mut r0, mut c0, mut r1, mut c1) = (u32::MAX, u32::MAX, 0u32, 0u32);
        for r in 0..self.row_count() {
            let mut any = false;
            for c in 0..self.col_count() {
                if !self.get(r, c).is_empty() || self.formula_id(r, c).is_some() {
                    any = true;
                    c0 = c0.min(c);
                    c1 = c1.max(c);
                }
            }
            if any {
                r0 = r0.min(r);
                r1 = r;
            }
        }
        (r0 != u32::MAX).then_some(Rect { r0, c0, r1, c1 })
    }

    // ---- reads -----------------------------------------------------------

    #[inline]
    pub fn phys(&self, r: u32, c: u32) -> Option<(u32, u32)> {
        Some((*self.grid.row_map.get(r as usize)?, *self.grid.col_map.get(c as usize)?))
    }

    #[inline]
    fn block(&self, pr: u32, pc: u32) -> Option<&Block> {
        self.grid.blocks.get(pr as usize / BR)?.get(pc as usize / BC)?.as_deref()
    }

    #[inline]
    pub fn get_phys(&self, pr: u32, pc: u32) -> Cell {
        self.block(pr, pc).map_or(Cell::EMPTY, |b| b.vals[slot(pr, pc)])
    }

    #[inline]
    pub fn get(&self, r: u32, c: u32) -> Cell {
        match self.phys(r, c) {
            Some((pr, pc)) => self.get_phys(pr, pc),
            None => Cell::EMPTY,
        }
    }

    /// xf index for a cell, falling back to the row, then column default.
    pub fn style(&self, r: u32, c: u32) -> u16 {
        let Some((pr, pc)) = self.phys(r, c) else {
            return self.fallback_style(r, c);
        };
        if let Some(b) = self.block(pr, pc) {
            if let Some(s) = &b.styles {
                let s = s[slot(pr, pc)];
                if s != 0 {
                    return s;
                }
            }
            if !b.vals[slot(pr, pc)].is_empty() {
                return 0;
            }
        }
        self.fallback_style(r, c)
    }

    fn fallback_style(&self, r: u32, c: u32) -> u16 {
        if let Some(&pr) = self.grid.row_map.get(r as usize)
            && let Some(s) = self.grid.rows.get(pr as usize).and_then(|m| m.style)
        {
            return s;
        }
        if let Some(&pc) = self.grid.col_map.get(c as usize)
            && let Some(s) = self.grid.cols.get(pc as usize).and_then(|m| m.style)
        {
            return s;
        }
        0
    }

    /// xf stored on the cell itself (no fallback).
    pub fn own_style_phys(&self, pr: u32, pc: u32) -> u16 {
        self.block(pr, pc).and_then(|b| b.styles.as_ref()).map_or(0, |s| s[slot(pr, pc)])
    }

    #[inline]
    pub fn formula_id_phys(&self, pr: u32, pc: u32) -> Option<u32> {
        let f = self.block(pr, pc)?.fids.as_ref()?[slot(pr, pc)];
        (f != 0).then(|| f - 1)
    }

    pub fn formula_id(&self, r: u32, c: u32) -> Option<u32> {
        let (pr, pc) = self.phys(r, c)?;
        self.formula_id_phys(pr, pc)
    }

    /// Formula text for a cell (without '='), resolving shared-formula children.
    pub fn formula_text(&self, r: u32, c: u32) -> Option<String> {
        let fid = self.formula_id(r, c)?;
        let f = &self.formulas[fid as usize];
        match &f.kind {
            FKind::Shared { si, ref_: None } => {
                let m = &self.formulas[*self.shared_masters.get(si)? as usize];
                let FKind::Shared { ref_: Some(rf), .. } = &m.kind else { return None };
                let (mr, mc) = crate::refshift::parse_cell_ref(rf.split(':').next()?)?;
                Some(crate::refshift::translate_formula(&m.text, r as i32 - mr as i32, c as i32 - mc as i32))
            }
            _ => Some(f.text.to_string()),
        }
    }

    pub fn is_row_hidden(&self, r: u32) -> bool {
        if let Some(f) = &self.filter_hidden
            && f.get(r as usize).copied().unwrap_or(false)
        {
            return true;
        }
        self.grid.row_map.get(r as usize).and_then(|&pr| self.grid.rows.get(pr as usize)).is_some_and(|m| m.hidden)
    }

    pub fn is_col_hidden(&self, c: u32) -> bool {
        self.col_meta(c).is_some_and(|m| m.hidden)
    }

    pub fn row_meta(&self, r: u32) -> Option<&RowMeta> {
        let pr = *self.grid.row_map.get(r as usize)?;
        self.grid.rows.get(pr as usize)
    }

    pub fn col_meta(&self, c: u32) -> Option<&ColMeta> {
        let pc = *self.grid.col_map.get(c as usize)?;
        self.grid.cols.get(pc as usize)
    }

    pub fn merge_at(&self, r: u32, c: u32) -> Option<Rect> {
        self.grid.merges.iter().copied().find(|m| m.contains(r, c))
    }

    // ---- geometry (pixels at 100% zoom) ---------------------------------

    pub fn invalidate_geometry(&mut self) {
        self.geom = None;
        self.version += 1;
    }

    fn geometry(&mut self) -> &Geom {
        if self.geom.is_none() {
            let rows = self.display_rows();
            let cols = self.display_cols();
            let default_h = self.default_row_height as f64 * PT_TO_PX;
            let default_w = col_width_px(self.default_col_width);
            let auto = self.auto_heights();
            let mut row_pos = Vec::with_capacity(rows as usize + 1);
            let mut y = 0.0;
            row_pos.push(0.0);
            for r in 0..rows {
                if !self.is_row_hidden(r) {
                    let explicit = self.row_meta(r).and_then(|m| m.height);
                    y += match explicit {
                        Some(h) => (h as f64 * PT_TO_PX).round(),
                        None => {
                            let pr = self.grid.row_map.get(r as usize).copied().unwrap_or(u32::MAX);
                            auto.get(&pr).map_or(default_h, |&pt| (pt as f64 * PT_TO_PX).round().max(default_h))
                        }
                    };
                }
                row_pos.push(y);
            }
            let mut col_pos = Vec::with_capacity(cols as usize + 1);
            let mut x = 0.0;
            col_pos.push(0.0);
            for c in 0..cols {
                match self.col_meta(c) {
                    Some(m) if m.hidden => {}
                    Some(ColMeta { width: Some(w), .. }) => x += col_width_px(*w),
                    _ => x += default_w,
                }
                col_pos.push(x);
            }
            self.geom = Some(Geom { row_pos, col_pos });
        }
        self.geom.as_ref().unwrap()
    }

    /// Rows (by physical index) without an explicit height that need to be taller
    /// than the default: big fonts or wrapped text. Values are points.
    fn auto_heights(&self) -> std::collections::HashMap<u32, f32> {
        let mut out = std::collections::HashMap::new();
        let Some(m) = self.metrics.as_deref() else { return out };
        if !m.line_pt.iter().any(|&h| h > 0.0) && !m.wrap.iter().any(|&w| w) {
            return out;
        }
        let default_w = col_width_px(self.default_col_width);
        // Excel leaves merged cells out of automatic row heights.
        let merged: std::collections::HashSet<(u32, u32)> = self.grid.merges.iter().filter_map(|m| self.phys(m.r0, m.c0)).collect();
        for (br, row) in self.grid.blocks.iter().enumerate() {
            for (bc, b) in row.iter().enumerate() {
                let Some(b) = b else { continue };
                let Some(styles) = &b.styles else { continue };
                for (i, &xf) in styles.iter().enumerate() {
                    if xf == 0 || !m.needs(xf) {
                        continue;
                    }
                    let pr = (br * BR + i / BC) as u32;
                    let pc = (bc * BC + i % BC) as u32;
                    if self.grid.rows.get(pr as usize).is_some_and(|r| r.height.is_some()) {
                        continue;
                    }
                    let v = b.vals[i];
                    if v.is_empty() || merged.contains(&(pr, pc)) {
                        continue;
                    }
                    let x = xf as usize;
                    let line = if m.line_pt[x] > 0.0 { m.line_pt[x] } else { self.default_row_height };
                    let mut h = line;
                    if m.wrap[x]
                        && let Some(id) = v.as_str_id()
                    {
                        let text = self.strings.get(id);
                        let width = self.grid.cols.get(pc as usize).and_then(|c| c.width).map_or(default_w, col_width_px) - 6.0;
                        // Average glyph ≈ 0.52 em; font size is in points (×4/3 → px).
                        let char_px = m.font_pt[x] as f64 * PT_TO_PX * 0.52;
                        let per_line = (width / char_px).max(1.0);
                        let lines: f64 = text.split('\n').map(|l| (l.chars().count() as f64 / per_line).ceil().max(1.0)).sum();
                        h = (lines as f32) * (m.font_pt[x] * 1.22) + 3.0;
                    }
                    let e = out.entry(pr).or_insert(0.0f32);
                    *e = e.max(h.min(409.0));
                }
            }
        }
        out
    }

    pub fn row_y(&mut self, r: u32) -> f64 {
        let g = self.geometry();
        g.row_pos[(r as usize).min(g.row_pos.len() - 1)]
    }
    pub fn col_x(&mut self, c: u32) -> f64 {
        let g = self.geometry();
        g.col_pos[(c as usize).min(g.col_pos.len() - 1)]
    }
    pub fn total_height(&mut self) -> f64 {
        *self.geometry().row_pos.last().unwrap()
    }
    pub fn total_width(&mut self) -> f64 {
        *self.geometry().col_pos.last().unwrap()
    }
    /// Row containing y (hidden rows are skipped).
    pub fn row_at(&mut self, y: f64) -> u32 {
        let p = &self.geometry().row_pos;
        (p.partition_point(|&v| v <= y).max(1) - 1).min(p.len() - 2) as u32
    }
    pub fn col_at(&mut self, x: f64) -> u32 {
        let p = &self.geometry().col_pos;
        (p.partition_point(|&v| v <= x).max(1) - 1).min(p.len() - 2) as u32
    }

    // ---- writes ------------------------------------------------------------

    /// Make sure logical (r, c) has a physical home, extending the maps with fresh rows/cols.
    fn ensure_logical(&mut self, r: u32, c: u32) -> (u32, u32) {
        if r as usize >= self.grid.row_map.len() {
            let n = r as usize + 1 - self.grid.row_map.len();
            let start = self.grid.next_prow;
            Arc::make_mut(&mut self.grid.row_map).extend(start..start + n as u32);
            self.grid.next_prow += n as u32;
            self.invalidate_geometry();
        }
        if c as usize >= self.grid.col_map.len() {
            let n = c as usize + 1 - self.grid.col_map.len();
            let start = self.grid.next_pcol;
            Arc::make_mut(&mut self.grid.col_map).extend(start..start + n as u32);
            self.grid.next_pcol += n as u32;
            self.invalidate_geometry();
        }
        (self.grid.row_map[r as usize], self.grid.col_map[c as usize])
    }

    /// Append `n` fresh physical rows at logical position `at`.
    pub fn alloc_rows(&mut self, at: u32, n: u32) {
        if at as usize > self.grid.row_map.len() {
            return;
        }
        let start = self.grid.next_prow;
        self.grid.next_prow += n;
        Arc::make_mut(&mut self.grid.row_map).splice(at as usize..at as usize, start..start + n);
        self.invalidate_geometry();
    }

    pub fn alloc_cols(&mut self, at: u32, n: u32) {
        if at as usize > self.grid.col_map.len() {
            return;
        }
        let start = self.grid.next_pcol;
        self.grid.next_pcol += n;
        Arc::make_mut(&mut self.grid.col_map).splice(at as usize..at as usize, start..start + n);
        self.invalidate_geometry();
    }

    fn block_mut(&mut self, pr: u32, pc: u32) -> &mut Block {
        let (br, bc) = (pr as usize / BR, pc as usize / BC);
        let blocks = &mut self.grid.blocks;
        if blocks.len() <= br {
            blocks.resize_with(br + 1, Vec::new);
        }
        let row = &mut blocks[br];
        if row.len() <= bc {
            row.resize(bc + 1, None);
        }
        self.version += 1;
        let arc = row[bc].get_or_insert_with(|| Arc::new(Block::new()));
        if Arc::strong_count(arc) > 1 {
            self.cow_copies += 1;
        }
        Arc::make_mut(arc)
    }

    pub fn set_phys(&mut self, pr: u32, pc: u32, v: Cell) {
        if self.track {
            self.touched.push((pr, pc));
        }
        self.block_mut(pr, pc).vals[slot(pr, pc)] = v;
    }

    pub fn set(&mut self, r: u32, c: u32, v: Cell) {
        if v.is_empty() && self.phys(r, c).is_none() {
            return;
        }
        let (pr, pc) = self.ensure_logical(r, c);
        self.set_phys(pr, pc, v);
        self.dirty = true;
    }

    pub fn set_style_phys(&mut self, pr: u32, pc: u32, xf: u16) {
        let b = self.block_mut(pr, pc);
        if xf == 0 && b.styles.is_none() {
            return;
        }
        b.styles.get_or_insert_with(|| vec![0; BN].into_boxed_slice())[slot(pr, pc)] = xf;
    }

    pub fn set_style(&mut self, r: u32, c: u32, xf: u16) {
        let (pr, pc) = self.ensure_logical(r, c);
        self.set_style_phys(pr, pc, xf);
        self.dirty = true;
    }

    pub fn set_formula_phys(&mut self, pr: u32, pc: u32, fid: Option<u32>) {
        if self.track {
            self.touched.push((pr, pc));
        }
        let b = self.block_mut(pr, pc);
        if fid.is_none() && b.fids.is_none() {
            return;
        }
        b.fids.get_or_insert_with(|| vec![0; BN].into_boxed_slice())[slot(pr, pc)] = fid.map_or(0, |f| f + 1);
    }

    pub fn set_formula(&mut self, r: u32, c: u32, fid: Option<u32>) {
        if fid.is_none() && self.phys(r, c).is_none() {
            return;
        }
        let (pr, pc) = self.ensure_logical(r, c);
        self.set_formula_phys(pr, pc, fid);
        self.dirty = true;
    }

    pub fn add_formula(&mut self, f: Formula) -> u32 {
        self.formulas.push(f);
        self.formulas.len() as u32 - 1
    }

    pub fn intern_raw(&mut self, raw: &str) -> u32 {
        if raw.is_empty() {
            return 0;
        }
        if let Some(i) = self.raw_attrs.iter().position(|a| &**a == raw) {
            return i as u32 + 1;
        }
        self.raw_attrs.push(raw.into());
        self.raw_attrs.len() as u32
    }

    pub fn raw(&self, id: u32) -> &str {
        if id == 0 { "" } else { &self.raw_attrs[id as usize - 1] }
    }

    pub fn row_meta_mut(&mut self, r: u32) -> &mut RowMeta {
        let (pr, _) = self.ensure_logical(r, 0);
        let rows = Arc::make_mut(&mut self.grid.rows);
        if rows.len() <= pr as usize {
            rows.resize(pr as usize + 1, RowMeta::default());
        }
        self.geom = None;
        self.dirty = true;
        &mut rows[pr as usize]
    }

    pub fn col_meta_mut(&mut self, c: u32) -> &mut ColMeta {
        let (_, pc) = self.ensure_logical(0, c);
        let cols = Arc::make_mut(&mut self.grid.cols);
        if cols.len() <= pc as usize {
            cols.resize(pc as usize + 1, ColMeta::default());
        }
        self.geom = None;
        self.dirty = true;
        &mut cols[pc as usize]
    }

    /// Logical positions of every cell holding a formula (only blocks with formulas are visited).
    pub fn formula_positions(&self) -> Vec<(u32, u32)> {
        let mut phys = Vec::new();
        for (br, row) in self.grid.blocks.iter().enumerate() {
            for (bc, b) in row.iter().enumerate() {
                let Some(fids) = b.as_ref().and_then(|b| b.fids.as_ref()) else { continue };
                for (i, &f) in fids.iter().enumerate() {
                    if f != 0 {
                        phys.push(((br * BR + i / BC) as u32, (bc * BC + i % BC) as u32));
                    }
                }
            }
        }
        if phys.is_empty() {
            return phys;
        }
        let rows: std::collections::HashMap<u32, u32> = self.grid.row_map.iter().enumerate().map(|(i, &p)| (p, i as u32)).collect();
        let cols: std::collections::HashMap<u32, u32> = self.grid.col_map.iter().enumerate().map(|(i, &p)| (p, i as u32)).collect();
        let mut out: Vec<(u32, u32)> = phys.into_iter().filter_map(|(pr, pc)| Some((*rows.get(&pr)?, *cols.get(&pc)?))).collect();
        out.sort_unstable();
        out
    }

    /// Logical positions of the cells written while tracking (clears the log).
    pub fn take_touched(&mut self) -> Vec<(u32, u32)> {
        if self.touched.is_empty() {
            return Vec::new();
        }
        let touched = std::mem::take(&mut self.touched);
        let rows: std::collections::HashMap<u32, u32> = self.grid.row_map.iter().enumerate().map(|(i, &p)| (p, i as u32)).collect();
        let cols: std::collections::HashMap<u32, u32> = self.grid.col_map.iter().enumerate().map(|(i, &p)| (p, i as u32)).collect();
        let mut out: Vec<(u32, u32)> = touched.into_iter().filter_map(|(pr, pc)| Some((*rows.get(&pr)?, *cols.get(&pc)?))).collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Approximate heap use, for the memory budget.
    pub fn heap_bytes(&self) -> usize {
        let blocks: usize = self.grid.blocks.iter().flatten().flatten().map(|b| b.heap_bytes()).sum();
        blocks
            + self.grid.row_map.capacity() * 4
            + self.grid.col_map.capacity() * 4
            + self.grid.rows.capacity() * std::mem::size_of::<RowMeta>()
            + self.strings.local.heap_bytes()
            + self.formulas.iter().map(|f| f.text.len() + 48).sum::<usize>()
    }
}

/// Builds a sheet row by row during a load, without per-cell bounds juggling.
pub struct SheetBuilder<'a> {
    pub sheet: &'a mut Sheet,
}

impl<'a> SheetBuilder<'a> {
    pub fn new(sheet: &'a mut Sheet) -> Self {
        SheetBuilder { sheet }
    }

    /// Loads place logical == physical, so extend the maps as identity.
    pub fn reserve(&mut self, rows: u32, cols: u32) {
        let g = &mut self.sheet.grid;
        if rows > g.next_prow {
            Arc::make_mut(&mut g.row_map).extend(g.next_prow..rows);
            g.next_prow = rows;
        }
        if cols > g.next_pcol {
            Arc::make_mut(&mut g.col_map).extend(g.next_pcol..cols);
            g.next_pcol = cols;
        }
    }

    #[inline]
    pub fn put(&mut self, r: u32, c: u32, v: Cell, xf: u16, fid: Option<u32>) {
        self.reserve(r + 1, c + 1);
        let s = &mut *self.sheet;
        if !v.is_empty() {
            s.set_phys(r, c, v);
        }
        if xf != 0 {
            s.set_style_phys(r, c, xf);
        }
        if fid.is_some() {
            s.set_formula_phys(r, c, fid);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strings::StringPool;

    fn sheet() -> Sheet {
        Sheet::new("S", SheetStrings::new(Arc::new(StringPool::new())))
    }

    #[test]
    fn set_get_and_cow() {
        let mut s = sheet();
        s.set(3, 20, Cell::number(1.0));
        assert_eq!(s.get(3, 20).as_number(), Some(1.0));
        assert!(s.get(1000, 1000).is_empty());
        let snap = s.grid.clone();
        s.cow_copies = 0;
        s.set(3, 20, Cell::number(2.0));
        assert_eq!(s.cow_copies, 1);
        s.set(3, 21, Cell::number(3.0));
        assert_eq!(s.cow_copies, 1, "same block is copied once");
        s.grid = snap;
        assert_eq!(s.get(3, 20).as_number(), Some(1.0));
    }

    #[test]
    fn insert_rows_moves_nothing() {
        let mut s = sheet();
        s.set(0, 0, Cell::number(1.0));
        s.set(1, 0, Cell::number(2.0));
        s.alloc_rows(1, 2);
        assert_eq!(s.get(0, 0).as_number(), Some(1.0));
        assert!(s.get(1, 0).is_empty());
        assert_eq!(s.get(3, 0).as_number(), Some(2.0));
    }

    #[test]
    fn geometry() {
        let mut s = sheet();
        assert_eq!(s.col_x(1), 64.0);
        assert_eq!(s.row_y(1), 20.0);
        s.row_meta_mut(1).hidden = true;
        assert_eq!(s.row_y(2), 20.0);
        assert_eq!(s.row_at(25.0), 2);
        assert_eq!(col_width_px(px_to_col_width(80.0)), 80.0);
    }
}
