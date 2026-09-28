//! Recalculation: after an edit, recompute formulas that depend on the touched
//! cells (transitively), in dependency order. Results are written into the
//! sheet like any other value, inside the same undo step.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::calc::{self, ErrorKind, Value};
use crate::cell::{Cell, Kind};
use crate::sheet::{Rect, Sheet};
use crate::workbook::Workbook;

thread_local! {
    /// Compiled formulas by text (compiling is the expensive part).
    static COMPILED: RefCell<HashMap<Box<str>, Option<Rc<calc::Formula>>>> = RefCell::new(HashMap::new());
}

pub fn compile(text: &str) -> Option<Rc<calc::Formula>> {
    COMPILED.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() > 200_000 {
            c.clear();
        }
        c.entry(text.into()).or_insert_with(|| calc::compile(text).ok().map(Rc::new)).clone()
    })
}

/// Read-only view of the workbook for the engine, with an overlay of fresh results.
pub struct View<'a> {
    pub wb: &'a Workbook,
    pub overlay: HashMap<(usize, u32, u32), Cell>,
    pub extra_text: HashMap<(usize, u32, u32), Rc<str>>,
}

pub fn cell_to_value(s: &Sheet, v: Cell) -> Value {
    match v.kind() {
        Kind::Empty => Value::Empty,
        Kind::Number => Value::Number(v.as_number().unwrap()),
        Kind::Str => Value::Text(Rc::from(s.strings.get(v.as_str_id().unwrap()))),
        Kind::Bool => Value::Bool(v.as_bool().unwrap()),
        Kind::Error => Value::Error(ErrorKind::from_str(v.as_error().unwrap()).unwrap_or(ErrorKind::Value)),
    }
}

impl calc::Grid for View<'_> {
    fn value(&self, sheet: usize, row: u32, col: u32) -> Value {
        if let Some(t) = self.extra_text.get(&(sheet, row, col)) {
            return Value::Text(t.clone());
        }
        let Some(s) = self.wb.sheets.get(sheet) else { return Value::Error(ErrorKind::Ref) };
        let v = self.overlay.get(&(sheet, row, col)).copied().unwrap_or_else(|| s.get(row, col));
        cell_to_value(s, v)
    }
    fn extent(&self, sheet: usize) -> (u32, u32) {
        self.wb.sheets.get(sheet).map_or((0, 0), |s| (s.row_count(), s.col_count()))
    }
    fn sheet_index(&self, name: &str) -> Option<usize> {
        self.wb.sheet_index(name)
    }
    fn sheet_count(&self) -> usize {
        self.wb.sheets.len()
    }
    fn defined_name(&self, name: &str, host: usize) -> Option<String> {
        self.wb.defined_name(name, host)
    }
    fn date1904(&self) -> bool {
        self.wb.date1904
    }
    fn now(&self) -> f64 {
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64());
        // Local time: Unix epoch is serial 25569 (1970-01-01).
        let offset = local_utc_offset_secs() as f64;
        25569.0 + (secs + offset) / 86400.0
    }
}

fn local_utc_offset_secs() -> i64 {
    // SAFETY: plain libc calls.
    unsafe extern "C" {
        fn time(t: *mut i64) -> i64;
        fn localtime_r(t: *const i64, out: *mut Tm) -> *mut Tm;
    }
    #[repr(C)]
    struct Tm {
        sec: i32,
        min: i32,
        hour: i32,
        mday: i32,
        mon: i32,
        year: i32,
        wday: i32,
        yday: i32,
        isdst: i32,
        gmtoff: i64,
        zone: *const i8,
    }
    unsafe {
        let now = time(std::ptr::null_mut());
        let mut tm: Tm = std::mem::zeroed();
        if localtime_r(&now, &mut tm).is_null() { 0 } else { tm.gmtoff }
    }
}

fn to_cell(v: &Value) -> Result<Cell, Rc<str>> {
    Ok(match v {
        Value::Empty => Cell::number(0.0),
        Value::Number(n) => {
            if n.is_finite() {
                Cell::number(*n)
            } else {
                Cell::error_from_str("#NUM!")
            }
        }
        Value::Bool(b) => Cell::boolean(*b),
        Value::Error(e) => Cell::error_from_str(e.as_str()),
        Value::Text(t) => return Err(t.clone()),
        Value::Array(a) => return to_cell(a.first().and_then(|r| r.first()).unwrap_or(&Value::Empty)),
    })
}

struct FCell {
    sheet: usize,
    r: u32,
    c: u32,
    text: String,
    f: Option<Rc<calc::Formula>>,
    /// Precedent rects per sheet index.
    prec: Vec<(usize, Rect)>,
}

fn collect(wb: &Workbook) -> Vec<FCell> {
    let mut out = Vec::new();
    for (si, s) in wb.sheets.iter().enumerate() {
        if s.formulas.is_empty() {
            continue;
        }
        for (r, c) in s.formula_positions() {
            {
                let Some(text) = s.formula_text(r, c) else { continue };
                let f = compile(&text);
                let prec = f.as_ref().map(|f| precedents_of(wb, f, si, 0)).unwrap_or_default();
                out.push(FCell { sheet: si, r, c, text, f, prec });
            }
        }
    }
    out
}

/// Precedent rects of a formula, following defined names (a few levels deep).
fn precedents_of(wb: &Workbook, f: &calc::Formula, si: usize, depth: u32) -> Vec<(usize, Rect)> {
    let mut out: Vec<(usize, Rect)> = Vec::new();
    for p in f.precedents() {
        let first = match &p.sheet {
            Some(n) => match wb.sheet_index(n) {
                Some(i) => i,
                None => continue,
            },
            None => si,
        };
        let last = p.sheet_last.as_ref().and_then(|n| wb.sheet_index(n)).unwrap_or(first);
        for s in first.min(last)..=first.max(last) {
            out.push((s, Rect { r0: p.r0, c0: p.c0, r1: p.r1, c1: p.c1 }));
        }
    }
    if depth < 4 {
        for name in f.names() {
            if let Some(text) = wb.defined_name(&name, si)
                && let Some(nf) = compile(&text)
            {
                out.extend(precedents_of(wb, &nf, si, depth + 1));
            }
        }
    }
    out
}

/// Recalculate formulas affected by `touched` (logical cells per sheet).
/// `all` recalculates everything (e.g. after structural edits).
pub fn run(wb: &mut Workbook, touched: &[(usize, u32, u32)], all: bool) -> usize {
    if !wb.has_formulas() {
        return 0;
    }
    let cells = collect(wb);
    if cells.is_empty() {
        return 0;
    }
    // Dirty set: touched formula cells, volatile ones, and anything reading a dirty cell.
    let mut dirty = vec![false; cells.len()];
    let mut frontier: Vec<(usize, u32, u32)> = touched.to_vec();
    let touched_set: HashSet<(usize, u32, u32)> = touched.iter().copied().collect();
    for (i, f) in cells.iter().enumerate() {
        let volatile = f.f.as_ref().is_some_and(|x| x.is_volatile());
        if (all || volatile || touched_set.contains(&(f.sheet, f.r, f.c))) && !dirty[i] {
            dirty[i] = true;
            frontier.push((f.sheet, f.r, f.c));
        }
    }
    // Propagate (bounded by the number of formulas).
    let mut rounds = 0;
    while !frontier.is_empty() && rounds <= cells.len() {
        rounds += 1;
        let changed: Vec<(usize, u32, u32)> = std::mem::take(&mut frontier);
        let set: HashSet<(usize, u32, u32)> = changed.iter().copied().collect();
        for (i, f) in cells.iter().enumerate() {
            if dirty[i] {
                continue;
            }
            // Check whichever is smaller: the changed cells or the cells of the precedent.
            let hit = f.prec.iter().any(|(ps, rect)| {
                let area = (rect.r1 as u64 - rect.r0 as u64 + 1) * (rect.c1 as u64 - rect.c0 as u64 + 1);
                if area <= changed.len() as u64 {
                    (rect.r0..=rect.r1).any(|r| (rect.c0..=rect.c1).any(|c| set.contains(&(*ps, r, c))))
                } else {
                    changed.iter().any(|&(s, r, c)| s == *ps && rect.contains(r, c))
                }
            });
            if hit {
                dirty[i] = true;
                frontier.push((f.sheet, f.r, f.c));
            }
        }
    }
    let order: Vec<usize> = topo_order(&cells, &dirty);

    // Evaluate against an overlay so later formulas see earlier results.
    let mut view = View { wb, overlay: HashMap::new(), extra_text: HashMap::new() };
    let mut results: Vec<Result_> = Vec::with_capacity(order.len());
    for &i in &order {
        let f = &cells[i];
        let Some(formula) = &f.f else { continue };
        if formula.uses_unsupported() {
            continue; // keep Excel's cached value
        }
        let v = formula.eval(&view, f.sheet, f.r, f.c);
        let res = to_cell(&v);
        match &res {
            Ok(cell) => {
                view.overlay.insert((f.sheet, f.r, f.c), *cell);
                view.extra_text.remove(&(f.sheet, f.r, f.c));
            }
            Err(t) => {
                view.extra_text.insert((f.sheet, f.r, f.c), t.clone());
            }
        }
        let _ = &f.text;
        results.push((f.sheet, f.r, f.c, res));
    }
    drop(view);
    let n = results.len();
    for (si, r, c, res) in results {
        let s = &mut wb.sheets[si];
        let cell = match res {
            Ok(c) => c,
            Err(t) => Cell::string(s.strings.add(&t)),
        };
        if s.get(r, c) != cell
            && let Some((pr, pc)) = s.phys(r, c)
        {
            s.set_phys(pr, pc, cell);
        }
    }
    n
}

/// A computed result: sheet, row, column, and the value (or text to intern).
type Result_ = (usize, u32, u32, Result<Cell, Rc<str>>);

/// Dirty formulas ordered so each comes after the dirty formulas it reads.
/// Cycles are evaluated in their original order (values then come from the previous pass).
fn topo_order(cells: &[FCell], dirty: &[bool]) -> Vec<usize> {
    let ids: Vec<usize> = (0..cells.len()).filter(|&i| dirty[i]).collect();
    if ids.len() > 20_000 {
        return ids; // large recalcs: reading order is usually right; a second pass would fix stragglers
    }
    let pos: HashMap<(usize, u32, u32), usize> = ids.iter().map(|&i| ((cells[i].sheet, cells[i].r, cells[i].c), i)).collect();
    // deps[i] = dirty formulas that i reads
    let mut indeg: HashMap<usize, usize> = ids.iter().map(|&i| (i, 0)).collect();
    let mut users: HashMap<usize, Vec<usize>> = HashMap::new();
    for &i in &ids {
        let mut seen = HashSet::new();
        for (ps, rect) in &cells[i].prec {
            let area = (rect.r1 as u64 - rect.r0 as u64 + 1) * (rect.c1 as u64 - rect.c0 as u64 + 1);
            if area as usize > pos.len() * 4 {
                for (&(s, r, c), &j) in &pos {
                    if s == *ps && rect.contains(r, c) && j != i && seen.insert(j) {
                        users.entry(j).or_default().push(i);
                        *indeg.get_mut(&i).unwrap() += 1;
                    }
                }
            } else {
                for r in rect.r0..=rect.r1 {
                    for c in rect.c0..=rect.c1 {
                        if let Some(&j) = pos.get(&(*ps, r, c))
                            && j != i
                            && seen.insert(j)
                        {
                            users.entry(j).or_default().push(i);
                            *indeg.get_mut(&i).unwrap() += 1;
                        }
                    }
                }
            }
        }
    }
    let mut queue: std::collections::VecDeque<usize> = ids.iter().copied().filter(|i| indeg[i] == 0).collect();
    let mut out = Vec::with_capacity(ids.len());
    while let Some(i) = queue.pop_front() {
        out.push(i);
        if let Some(us) = users.get(&i) {
            for &u in us {
                let d = indeg.get_mut(&u).unwrap();
                *d -= 1;
                if *d == 0 {
                    queue.push_back(u);
                }
            }
        }
    }
    // Cycles: append the rest in reading order.
    if out.len() < ids.len() {
        let done: HashSet<usize> = out.iter().copied().collect();
        out.extend(ids.iter().copied().filter(|i| !done.contains(i)));
    }
    out
}

/// Evaluate a formula (e.g. a conditional-format rule) at a cell without writing anything.
pub fn eval_at(wb: &Workbook, si: usize, text: &str, r: u32, c: u32) -> Option<Cell> {
    let f = compile(text)?;
    if f.uses_unsupported() {
        return None;
    }
    let view = View { wb, overlay: HashMap::new(), extra_text: HashMap::new() };
    let v = f.eval(&view, si, r, c);
    to_cell(&v).ok().or(Some(Cell::EMPTY))
}
