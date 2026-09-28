//! Evaluation context, operators, reference handling and function dispatch.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use super::Grid;
use super::funcs::Func;
use super::parser::{self, BinOp, Expr, RefExpr, SheetSpec, UnOp};
use super::value::*;

/// A resolved rectangular reference, possibly spanning sheets `s0..=s1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Area {
    pub s0: usize,
    pub s1: usize,
    pub r0: u32,
    pub c0: u32,
    pub r1: u32,
    pub c1: u32,
}

impl Area {
    pub fn rows(&self) -> u32 {
        self.r1 - self.r0 + 1
    }
    pub fn cols(&self) -> u32 {
        self.c1 - self.c0 + 1
    }
    pub fn is_cell(&self) -> bool {
        self.r0 == self.r1 && self.c0 == self.c1 && self.s0 == self.s1
    }
}

/// Intermediate result: a value or a reference.
pub(crate) enum Val {
    V(Value),
    R(Area),
}

/// Range-like argument with random access.
pub(crate) enum Src {
    Area(Area),
    Arr(Rc<Vec<Vec<Value>>>),
    One(Value),
}

impl Src {
    pub fn rows(&self) -> u32 {
        match self {
            Src::Area(a) => a.rows(),
            Src::Arr(a) => a.len() as u32,
            Src::One(_) => 1,
        }
    }
    pub fn cols(&self) -> u32 {
        match self {
            Src::Area(a) => a.cols(),
            Src::Arr(a) => a.first().map_or(0, |r| r.len()) as u32,
            Src::One(_) => 1,
        }
    }
    #[inline]
    pub fn get(&self, cx: &Ctx, i: u32, j: u32) -> Value {
        match self {
            Src::Area(a) => cx.grid.value(a.s0, a.r0 + i, a.c0 + j),
            Src::Arr(a) => a.get(i as usize).and_then(|r| r.get(j as usize)).cloned().unwrap_or(Value::Empty),
            Src::One(v) => {
                if i == 0 && j == 0 {
                    v.clone()
                } else {
                    Value::Empty
                }
            }
        }
    }
    /// Is this a 1-D vector? Returns its length and orientation (true = vertical).
    pub fn vector(&self) -> Option<(u32, bool)> {
        let (r, c) = (self.rows(), self.cols());
        if c == 1 {
            Some((r, true))
        } else if r == 1 {
            Some((c, false))
        } else {
            None
        }
    }
    /// Element `k` of a vector.
    pub fn at(&self, cx: &Ctx, k: u32, vertical: bool) -> Value {
        if vertical { self.get(cx, k, 0) } else { self.get(cx, 0, k) }
    }
}

thread_local! {
    static NAME_CACHE: RefCell<HashMap<String, Rc<Expr>>> = RefCell::new(HashMap::new());
}

pub(crate) struct Ctx<'a> {
    pub grid: &'a dyn Grid,
    pub host: usize,
    pub row: u32,
    pub col: u32,
    pub d1904: bool,
    depth: Cell<u32>,
}

impl<'a> Ctx<'a> {
    pub fn new(grid: &'a dyn Grid, host: usize, row: u32, col: u32) -> Self {
        let d1904 = grid.date1904();
        set_date1904(d1904);
        Ctx { grid, host, row, col, d1904, depth: Cell::new(0) }
    }

    /// Final result of a formula: references dereferenced, blanks → 0.
    pub fn top(&self, e: &Expr) -> Value {
        fn fix(v: Value) -> Value {
            match v {
                Value::Empty => Value::Number(0.0),
                Value::Number(n) if !n.is_finite() => Value::Error(ErrorKind::Num),
                Value::Array(a) => {
                    if a.iter().flatten().any(|v| matches!(v, Value::Empty | Value::Array(_))) {
                        Value::array(a.iter().map(|r| r.iter().cloned().map(fix).collect()).collect())
                    } else {
                        Value::Array(a)
                    }
                }
                v => v,
            }
        }
        fix(self.value(e))
    }

    // ---------- references ----------

    fn sheet_idx(&self, name: &str) -> R<usize> {
        self.grid.sheet_index(name).ok_or(ErrorKind::Ref)
    }

    pub fn resolve(&self, r: &RefExpr) -> R<Area> {
        let (s0, s1) = match &r.sheet {
            SheetSpec::Host => (self.host, self.host),
            SheetSpec::One(n) => {
                let s = self.sheet_idx(n)?;
                (s, s)
            }
            SheetSpec::Span(a, b) => {
                let (x, y) = (self.sheet_idx(a)?, self.sheet_idx(b)?);
                (x.min(y), x.max(y))
            }
        };
        Ok(Area { s0, s1, r0: r.r0, c0: r.c0, r1: r.r1, c1: r.c1 })
    }

    /// Extent-clamped (rows, cols) counts of an area for iteration.
    pub fn clamped(&self, a: &Area) -> (u32, u32) {
        let (mut er, mut ec) = (0u32, 0u32);
        for s in a.s0..=a.s1 {
            let (r, c) = self.grid.extent(s);
            er = er.max(r);
            ec = ec.max(c);
        }
        let h = if a.r0 >= er { 0 } else { a.r1.min(er - 1) - a.r0 + 1 };
        let w = if a.c0 >= ec { 0 } else { a.c1.min(ec - 1) - a.c0 + 1 };
        (h, w)
    }

    /// Clamped rows/cols of a range-like source.
    pub fn src_eff(&self, s: &Src) -> (u32, u32) {
        match s {
            Src::Area(a) => self.clamped(a),
            _ => (s.rows(), s.cols()),
        }
    }

    /// Visits every (clamped) cell of an area, row-major.
    pub fn each_cell(&self, a: &Area, f: &mut dyn FnMut(&Value) -> R<()>) -> R<()> {
        for s in a.s0..=a.s1 {
            let (h, w) = self.clamped(&Area { s0: s, s1: s, ..*a });
            for r in a.r0..a.r0 + h {
                for c in a.c0..a.c0 + w {
                    f(&self.grid.value(s, r, c))?;
                }
            }
        }
        Ok(())
    }

    /// Materializes an area as a value (single cell → scalar).
    pub fn deref(&self, a: Area) -> Value {
        if a.s0 != a.s1 {
            return Value::Error(ErrorKind::Value);
        }
        if a.r0 == a.r1 && a.c0 == a.c1 {
            return self.grid.value(a.s0, a.r0, a.c0);
        }
        let (h, w) = self.clamped(&a);
        let (h, w) = (h.max(1), w.max(1));
        let rows = (0..h).map(|i| (0..w).map(|j| self.grid.value(a.s0, a.r0 + i, a.c0 + j)).collect()).collect();
        Value::array(rows)
    }

    // ---------- evaluation ----------

    pub fn eval(&self, e: &Expr) -> Val {
        match e {
            Expr::Num(n) => Val::V(Value::Number(*n)),
            Expr::Str(s) => Val::V(Value::Text(s.clone())),
            Expr::Bool(b) => Val::V(Value::Bool(*b)),
            Expr::Err(k) => Val::V(Value::Error(*k)),
            Expr::Array(a) => Val::V(Value::Array(a.clone())),
            Expr::Missing => Val::V(Value::Empty),
            Expr::Ref(r) => match self.resolve(r) {
                Ok(a) => Val::R(a),
                Err(k) => Val::V(Value::Error(k)),
            },
            Expr::Name(n) => self.eval_name(n),
            Expr::Unary(op, x) => Val::V(unary(*op, self.value(x))),
            Expr::Bin(BinOp::Range, lr) => {
                let (a, b) = (self.eval(&lr.0), self.eval(&lr.1));
                match (a, b) {
                    (Val::R(x), Val::R(y)) if x.s0 == y.s0 && x.s1 == y.s1 => {
                        Val::R(Area { s0: x.s0, s1: x.s1, r0: x.r0.min(y.r0), c0: x.c0.min(y.c0), r1: x.r1.max(y.r1), c1: x.c1.max(y.c1) })
                    }
                    (Val::V(Value::Error(k)), _) | (_, Val::V(Value::Error(k))) => Val::V(Value::Error(k)),
                    _ => Val::V(Value::Error(ErrorKind::Value)),
                }
            }
            Expr::Bin(op, lr) => {
                let a = self.value(&lr.0);
                let b = self.value(&lr.1);
                Val::V(binop(*op, &a, &b))
            }
            Expr::Call(f, args) => self.call(*f, args),
            Expr::Unknown(..) => Val::V(Value::Error(ErrorKind::Name)),
            Expr::BadRef => Val::V(Value::Error(ErrorKind::Ref)),
        }
    }

    fn eval_name(&self, n: &str) -> Val {
        if self.depth.get() > 16 {
            return Val::V(Value::Error(ErrorKind::Name));
        }
        let Some(text) = self.grid.defined_name(n, self.host) else {
            return Val::V(Value::Error(ErrorKind::Name));
        };
        let cached = NAME_CACHE.with(|c| c.borrow().get(&text).cloned());
        let expr = match cached {
            Some(e) => e,
            None => match parser::parse(&text) {
                Ok(e) => {
                    let e = Rc::new(e);
                    NAME_CACHE.with(|c| {
                        let mut c = c.borrow_mut();
                        if c.len() > 4096 {
                            c.clear();
                        }
                        c.insert(text, e.clone());
                    });
                    e
                }
                Err(_) => return Val::V(Value::Error(ErrorKind::Name)),
            },
        };
        self.depth.set(self.depth.get() + 1);
        let v = self.eval(&expr);
        self.depth.set(self.depth.get() - 1);
        v
    }

    /// Evaluates to a value; multi-cell references become arrays.
    pub fn value(&self, e: &Expr) -> Value {
        match self.eval(e) {
            Val::V(v) => v,
            Val::R(a) => self.deref(a),
        }
    }

    /// Evaluates to a single value (top-left of arrays/ranges).
    pub fn scalar(&self, e: &Expr) -> Value {
        match self.eval(e) {
            Val::V(v) => v.top_left(),
            Val::R(a) => {
                if a.s0 != a.s1 {
                    Value::Error(ErrorKind::Value)
                } else {
                    self.grid.value(a.s0, a.r0, a.c0)
                }
            }
        }
    }

    pub fn src(&self, e: &Expr) -> R<Src> {
        match self.eval(e) {
            Val::R(a) if a.s0 != a.s1 => Err(ErrorKind::Value),
            Val::R(a) => Ok(Src::Area(a)),
            Val::V(Value::Array(a)) => Ok(Src::Arr(a)),
            Val::V(Value::Error(k)) => Err(k),
            Val::V(v) => Ok(Src::One(v)),
        }
    }

    // ---------- argument helpers ----------

    pub fn num(&self, args: &[Expr], i: usize) -> R<f64> {
        match args.get(i) {
            Some(e) => to_num(&self.scalar(e)),
            None => Err(ErrorKind::Value),
        }
    }
    pub fn opt_num(&self, args: &[Expr], i: usize, def: f64) -> R<f64> {
        match args.get(i) {
            Some(e) => to_num(&self.scalar(e)),
            None => Ok(def),
        }
    }
    pub fn opt_num_m(&self, args: &[Expr], i: usize, def: f64) -> R<f64> {
        match args.get(i) {
            Some(Expr::Missing) | None => Ok(def),
            Some(e) => to_num(&self.scalar(e)),
        }
    }
    pub fn opt_bool(&self, args: &[Expr], i: usize, def: bool) -> R<bool> {
        match args.get(i) {
            Some(e) => to_bool(&self.scalar(e)),
            None => Ok(def),
        }
    }
    pub fn text(&self, args: &[Expr], i: usize) -> R<String> {
        match args.get(i) {
            Some(e) => to_text(&self.scalar(e)).map(|c| c.into_owned()),
            None => Err(ErrorKind::Value),
        }
    }

    /// Visits all values of aggregate-style arguments. The flag says whether
    /// the value came from a reference/array (true) or was a direct scalar.
    pub fn visit(&self, args: &[Expr], f: &mut dyn FnMut(&Value, bool) -> R<()>) -> R<()> {
        for a in args {
            match self.eval(a) {
                Val::R(area) => self.each_cell(&area, &mut |v| f(v, true))?,
                Val::V(Value::Array(arr)) => {
                    for v in arr.iter().flatten() {
                        f(v, true)?;
                    }
                }
                Val::V(v) => f(&v, false)?,
            }
        }
        Ok(())
    }

    // ---------- dispatch ----------

    pub fn call(&self, f: Func, args: &[Expr]) -> Val {
        use Func::*;
        match f {
            If => self.f_if(args),
            Choose => self.f_choose(args),
            Index => self.f_index(args),
            Offset => self.f_offset(args),
            Indirect => self.f_indirect(args),
            Single => match args.first() {
                Some(e) => self.eval(e),
                None => Val::V(Value::Error(ErrorKind::Value)),
            },
            Ifs => self.f_ifs(args),
            Switch => self.f_switch(args),
            _ if f.is_scalar() => Val::V(self.lifted(f, args)),
            _ => Val::V(self.special(f, args)),
        }
    }

    fn lifted(&self, f: Func, args: &[Expr]) -> Value {
        let vals: Vec<Value> = args.iter().map(|a| self.value(a)).collect();
        let mut dims: Option<(usize, usize)> = None;
        for v in &vals {
            if let Value::Array(_) = v {
                let (r, c) = v.dims();
                dims = Some(match dims {
                    None => (r, c),
                    Some((r0, c0)) => (bdim(r0, r), bdim(c0, c)),
                });
            }
        }
        match dims {
            None => self.scalar_fn(f, &vals),
            Some((rows, cols)) => {
                let mut buf = vals.clone();
                let mut out = Vec::with_capacity(rows);
                for i in 0..rows {
                    let mut row = Vec::with_capacity(cols);
                    for j in 0..cols {
                        for (k, v) in vals.iter().enumerate() {
                            if let Value::Array(a) = v {
                                buf[k] = pick(a, i, j);
                            }
                        }
                        row.push(self.scalar_fn(f, &buf).top_left());
                    }
                    out.push(row);
                }
                Value::array(out)
            }
        }
    }

    fn scalar_fn(&self, f: Func, a: &[Value]) -> Value {
        if let Some(v) = self.math_scalar(f, a) {
            return v;
        }
        if let Some(v) = self.text_scalar(f, a) {
            return v;
        }
        if let Some(v) = self.date_scalar(f, a) {
            return v;
        }
        if let Some(v) = self.info_scalar(f, a) {
            return v;
        }
        Value::Error(ErrorKind::Name)
    }

    pub(crate) fn special(&self, f: Func, args: &[Expr]) -> Value {
        if let Some(v) = self.math_special(f, args) {
            return v;
        }
        if let Some(v) = self.stats_special(f, args) {
            return v;
        }
        if let Some(v) = self.lookup_special(f, args) {
            return v;
        }
        if let Some(v) = self.text_special(f, args) {
            return v;
        }
        if let Some(v) = self.date_special(f, args) {
            return v;
        }
        Value::Error(ErrorKind::Name)
    }
}

pub(crate) fn bdim(a: usize, b: usize) -> usize {
    if a == 1 {
        b
    } else if b == 1 {
        a
    } else {
        a.max(b)
    }
}

/// Element (i, j) of an array with Excel broadcasting of single rows/cols.
pub(crate) fn pick(a: &[Vec<Value>], i: usize, j: usize) -> Value {
    let rows = a.len();
    let cols = a.first().map_or(0, |r| r.len());
    let ii = if rows == 1 { 0 } else { i };
    let jj = if cols == 1 { 0 } else { j };
    a.get(ii).and_then(|r| r.get(jj)).cloned().unwrap_or(Value::Error(ErrorKind::NA))
}

/// Applies `f` element-wise over two values with broadcasting.
pub(crate) fn broadcast2(a: &Value, b: &Value, f: &dyn Fn(&Value, &Value) -> Value) -> Value {
    match (a, b) {
        (Value::Array(_), _) | (_, Value::Array(_)) => {
            let (ra, ca) = a.dims();
            let (rb, cb) = b.dims();
            let (rows, cols) = (bdim(ra, rb), bdim(ca, cb));
            let get = |v: &Value, i: usize, j: usize| match v {
                Value::Array(x) => pick(x, i, j),
                s => s.clone(),
            };
            let out = (0..rows).map(|i| (0..cols).map(|j| f(&get(a, i, j), &get(b, i, j))).collect()).collect();
            Value::array(out)
        }
        _ => f(a, b),
    }
}

pub(crate) fn map1(v: Value, f: &dyn Fn(&Value) -> Value) -> Value {
    match &v {
        Value::Array(a) => Value::array(a.iter().map(|r| r.iter().map(f).collect()).collect()),
        _ => f(&v),
    }
}

fn unary(op: UnOp, v: Value) -> Value {
    map1(v, &|x| match op {
        UnOp::Plus => x.clone(),
        UnOp::Neg => match to_num(x) {
            Ok(n) => num(-n),
            Err(e) => Value::Error(e),
        },
        UnOp::Pct => match to_num(x) {
            Ok(n) => num(n / 100.0),
            Err(e) => Value::Error(e),
        },
    })
}

pub(crate) fn binop(op: BinOp, a: &Value, b: &Value) -> Value {
    broadcast2(a, b, &|x, y| scalar_binop(op, x, y))
}

pub(crate) fn scalar_binop(op: BinOp, a: &Value, b: &Value) -> Value {
    use std::cmp::Ordering::*;
    match op {
        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Pow => {
            let x = match to_num(a) {
                Ok(x) => x,
                Err(e) => return Value::Error(e),
            };
            let y = match to_num(b) {
                Ok(y) => y,
                Err(e) => return Value::Error(e),
            };
            match op {
                BinOp::Add => num(x + y),
                BinOp::Sub => num(x - y),
                BinOp::Mul => num(x * y),
                BinOp::Div => {
                    if y == 0.0 {
                        Value::Error(ErrorKind::Div0)
                    } else {
                        num(x / y)
                    }
                }
                _ => pow(x, y),
            }
        }
        BinOp::Concat => match (to_text(a), to_text(b)) {
            (Ok(x), Ok(y)) => {
                let mut s = String::with_capacity(x.len() + y.len());
                s.push_str(&x);
                s.push_str(&y);
                Value::from(s)
            }
            (Err(e), _) | (_, Err(e)) => Value::Error(e),
        },
        _ => {
            if let Value::Error(e) = a {
                return Value::Error(*e);
            }
            if let Value::Error(e) = b {
                return Value::Error(*e);
            }
            let o = cmp_values(a, b);
            Value::Bool(match op {
                BinOp::Eq => o == Equal,
                BinOp::Ne => o != Equal,
                BinOp::Lt => o == Less,
                BinOp::Le => o != Greater,
                BinOp::Gt => o == Greater,
                _ => o != Less,
            })
        }
    }
}

pub(crate) fn pow(x: f64, y: f64) -> Value {
    if x == 0.0 && y == 0.0 {
        return Value::Error(ErrorKind::Num);
    }
    if x == 0.0 && y < 0.0 {
        return Value::Error(ErrorKind::Div0);
    }
    if x < 0.0 && y.fract() != 0.0 {
        return Value::Error(ErrorKind::Num);
    }
    num(x.powf(y))
}
