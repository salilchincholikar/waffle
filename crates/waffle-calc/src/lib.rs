//! Excel formula calculation engine.
//!
//! * Formula text is in xlsx storage form (no leading `=` needed, `_xlfn.` /
//!   `_xlws.` prefixes are stripped).
//! * Rows and columns are **0-based** everywhere (`A1` = row 0, col 0);
//!   sheet indices are 0-based.
//! * [`compile`] parses once into a compact AST (references resolved to
//!   numeric coordinates, functions resolved to an enum); [`Formula::eval`]
//!   evaluates against any [`Grid`].
//! * Structured references (`Table1[Col]`) and external references
//!   (`[1]Sheet1!A1`) evaluate to `#REF!` and set
//!   [`Formula::uses_unsupported`], so the caller can keep Excel's cached value.
//! * Whole-column/row references (`A:A`, `3:5`) are clamped to
//!   [`Grid::extent`] when iterated.

mod criteria;
mod dates;
mod eval;
mod f_date;
mod f_lookup;
mod f_math;
mod f_stats;
mod f_text;
mod format;
mod funcs;
mod parser;
mod value;

#[cfg(test)]
mod tests;

pub use funcs::SUPPORTED_FUNCTIONS;
pub use value::{ErrorKind, Value, num_to_text};

use parser::{Expr, RefKind, SheetSpec};

/// Largest 0-based row index (row 1,048,576).
pub const MAX_ROW: u32 = 1_048_575;
/// Largest 0-based column index (XFD).
pub const MAX_COL: u32 = 16_383;
/// Sentinel used in [`Precedent`]: `r1 == WHOLE` for whole-column references
/// (`A:C`, with `r0 == 0`), `c1 == WHOLE` for whole-row references (`3:5`,
/// with `c0 == 0`).
pub const WHOLE: u32 = u32::MAX;

/// Cell data access supplied by the app. Sheet indices, rows and columns are 0-based.
pub trait Grid {
    fn value(&self, sheet: usize, row: u32, col: u32) -> Value;
    /// `(rows, cols)` such that every non-empty cell of `sheet` lies in
    /// `row < rows && col < cols`. Used to clamp whole-row/column refs and
    /// to bound range iteration.
    fn extent(&self, sheet: usize) -> (u32, u32);
    /// Case-insensitive sheet lookup.
    fn sheet_index(&self, name: &str) -> Option<usize>;
    fn sheet_count(&self) -> usize;
    /// Formula text of a defined name visible from `host` (sheet-scoped first, then global).
    fn defined_name(&self, name: &str, host: usize) -> Option<String>;
    fn date1904(&self) -> bool {
        false
    }
    /// Serial date-time "now" (lets tests pin TODAY/NOW).
    fn now(&self) -> f64;
}

/// A rectangular reference read by a formula (0-based, inclusive).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Precedent {
    /// Sheet name as written, or `None` for the host sheet. For 3-D
    /// references (`Sheet1:Sheet3!A1`) this is the first sheet.
    pub sheet: Option<String>,
    /// Last sheet of a 3-D reference, `None` otherwise.
    pub sheet_last: Option<String>,
    pub r0: u32,
    pub c0: u32,
    /// [`WHOLE`] for whole-column references.
    pub r1: u32,
    /// [`WHOLE`] for whole-row references.
    pub c1: u32,
}

/// A compiled formula.
#[derive(Clone, Debug)]
pub struct Formula {
    expr: Expr,
    volatile: bool,
    unsupported: bool,
}

/// Parses formula text once; evaluate many times with [`Formula::eval`].
pub fn compile(text: &str) -> Result<Formula, String> {
    let expr = parser::parse(text)?;
    let (mut volatile, mut unsupported) = (false, false);
    walk(&expr, &mut |e| match e {
        Expr::Call(f, _) if f.is_volatile() => volatile = true,
        Expr::Unknown(..) | Expr::BadRef => unsupported = true,
        _ => {}
    });
    Ok(Formula { expr, volatile, unsupported })
}

/// Compiles and evaluates in one step (convenience for one-off formulas).
pub fn eval_formula(text: &str, grid: &dyn Grid, host_sheet: usize, row: u32, col: u32) -> Value {
    match compile(text) {
        Ok(f) => f.eval(grid, host_sheet, row, col),
        Err(_) => Value::Error(ErrorKind::Name),
    }
}

fn walk<'a>(e: &'a Expr, f: &mut dyn FnMut(&'a Expr)) {
    f(e);
    match e {
        Expr::Unary(_, x) => walk(x, f),
        Expr::Bin(_, lr) => {
            walk(&lr.0, f);
            walk(&lr.1, f);
        }
        Expr::Call(_, args) | Expr::Unknown(_, args) => {
            for a in args.iter() {
                walk(a, f);
            }
        }
        _ => {}
    }
}

impl Formula {
    /// Evaluates the formula in cell (`row`, `col`) of `host_sheet`.
    /// Array results are returned as [`Value::Array`] (caller shows the top-left).
    /// Blank results become `0`, like Excel.
    pub fn eval(&self, grid: &dyn Grid, host_sheet: usize, row: u32, col: u32) -> Value {
        eval::Ctx::new(grid, host_sheet, row, col).top(&self.expr)
    }

    /// References read by this formula. Defined names are not expanded (see
    /// [`Formula::names`]); INDIRECT/OFFSET targets are dynamic (see
    /// [`Formula::is_volatile`]).
    pub fn precedents(&self) -> Vec<Precedent> {
        let mut out = Vec::new();
        walk(&self.expr, &mut |e| {
            if let Expr::Ref(r) = e {
                let (sheet, sheet_last) = match &r.sheet {
                    SheetSpec::Host => (None, None),
                    SheetSpec::One(s) => (Some(s.to_string()), None),
                    SheetSpec::Span(a, b) => (Some(a.to_string()), Some(b.to_string())),
                };
                let (r1, c1) = match r.kind {
                    RefKind::Cols => (WHOLE, r.c1),
                    RefKind::Rows => (r.r1, WHOLE),
                    _ => (r.r1, r.c1),
                };
                let p = Precedent { sheet, sheet_last, r0: r.r0, c0: r.c0, r1, c1 };
                if !out.contains(&p) {
                    out.push(p);
                }
            }
        });
        out
    }

    /// Defined names referenced by this formula (upper-case as written).
    pub fn names(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        walk(&self.expr, &mut |e| {
            if let Expr::Name(n) = e
                && !out.iter().any(|x| x.eq_ignore_ascii_case(n))
            {
                out.push(n.to_string());
            }
        });
        out
    }

    /// Names of functions used by this formula that the engine does not implement.
    pub fn unsupported_functions(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        walk(&self.expr, &mut |e| {
            if let Expr::Unknown(n, _) = e
                && !out.iter().any(|x| **x == **n)
            {
                out.push(n.to_string());
            }
        });
        out
    }

    /// Uses TODAY, NOW, RAND, RANDBETWEEN, OFFSET or INDIRECT.
    pub fn is_volatile(&self) -> bool {
        self.volatile
    }

    /// Uses a function the engine does not implement, or a structured /
    /// external reference. The caller should keep Excel's cached value.
    pub fn uses_unsupported(&self) -> bool {
        self.unsupported
    }
}

/// Serial number of a calendar date (`None` if out of range).
pub fn date_to_serial(y: i32, m: u32, d: u32, date1904: bool) -> Option<f64> {
    dates::date_serial(y as f64, m as f64, d as f64, date1904).ok()
}

/// Calendar date `(year, month, day)` of a serial number.
pub fn serial_to_date(serial: f64, date1904: bool) -> Option<(i32, u32, u32)> {
    dates::serial_to_ymd(serial, date1904).ok().map(|(y, m, d)| (y as i32, m, d))
}
