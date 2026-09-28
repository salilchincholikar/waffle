//! Logical, lookup/reference, information and dynamic-array functions.

use std::cmp::Ordering;
use std::collections::HashMap;

use super::criteria::{Pattern, has_wildcards};
use super::eval::{Area, Ctx, Val, bdim, pick};
use super::funcs::Func;
use super::parser::{self, Expr};
use super::value::*;
use super::{MAX_COL, MAX_ROW};

fn same_kind(a: &Value, b: &Value) -> bool {
    matches!((a, b), (Value::Number(_), Value::Number(_)) | (Value::Text(_), Value::Text(_)) | (Value::Bool(_), Value::Bool(_)))
}

/// Exact search with optional wildcards; `rev` searches last-to-first.
fn exact_pos(needle: &Value, n: u32, get: &dyn Fn(u32) -> Value, wild: bool, rev: bool) -> Option<u32> {
    let pat = match needle {
        Value::Text(t) if wild && has_wildcards(t) => Some(Pattern::new(t)),
        _ => None,
    };
    let test = |i: u32| {
        let v = get(i);
        match (&pat, &v) {
            (Some(p), Value::Text(t)) => p.matches(t),
            (Some(_), _) => false,
            (None, v) => lookup_eq(needle, v),
        }
    };
    if rev { (0..n).rev().find(|&i| test(i)) } else { (0..n).find(|&i| test(i)) }
}

/// Approximate match in ascending data: last position whose value <= needle.
fn approx_asc(needle: &Value, n: u32, get: &dyn Fn(u32) -> Value) -> Option<u32> {
    // binary search while every probe has the needle's type
    let (mut lo, mut hi) = (0i64, n as i64 - 1);
    let mut ans: Option<u32> = None;
    let mut ok = true;
    while lo <= hi {
        let mid = (lo + hi) / 2;
        let v = get(mid as u32);
        if !same_kind(&v, needle) {
            ok = false;
            break;
        }
        if cmp_values(&v, needle) != Ordering::Greater {
            ans = Some(mid as u32);
            lo = mid + 1;
        } else {
            hi = mid - 1;
        }
    }
    if ok {
        return ans;
    }
    let mut ans = None;
    for i in 0..n {
        let v = get(i);
        if !same_kind(&v, needle) {
            continue;
        }
        if cmp_values(&v, needle) == Ordering::Greater {
            break;
        }
        ans = Some(i);
    }
    ans
}

/// Approximate match in descending data: last position whose value >= needle.
fn approx_desc(needle: &Value, n: u32, get: &dyn Fn(u32) -> Value) -> Option<u32> {
    let mut ans = None;
    for i in 0..n {
        let v = get(i);
        if !same_kind(&v, needle) {
            continue;
        }
        if cmp_values(&v, needle) == Ordering::Less {
            break;
        }
        ans = Some(i);
    }
    ans
}

fn to_rows(v: &Value) -> Vec<Vec<Value>> {
    match v {
        Value::Array(a) => (**a).clone(),
        v => vec![vec![v.clone()]],
    }
}

fn transpose(rows: &[Vec<Value>]) -> Vec<Vec<Value>> {
    let w = rows.first().map_or(0, |r| r.len());
    (0..w).map(|j| rows.iter().map(|r| r[j].clone()).collect()).collect()
}

fn sort_rank(v: &Value) -> u8 {
    match v {
        Value::Number(_) => 0,
        Value::Text(_) => 1,
        Value::Bool(_) => 2,
        Value::Error(_) => 3,
        _ => 4,
    }
}

fn sort_cmp(a: &Value, b: &Value) -> Ordering {
    let (ra, rb) = (sort_rank(a), sort_rank(b));
    if ra != rb {
        return ra.cmp(&rb);
    }
    match (a, b) {
        (Value::Error(_), Value::Error(_)) => Ordering::Equal,
        _ => cmp_values(a, b),
    }
}

fn unique_key(row: &[Value]) -> String {
    let mut k = String::new();
    for v in row {
        match v {
            Value::Number(n) => k.push_str(&format!("n{}", sig15(*n))),
            Value::Text(t) => {
                k.push('t');
                k.push_str(&t.to_lowercase());
            }
            Value::Bool(b) => k.push_str(if *b { "b1" } else { "b0" }),
            Value::Error(e) => k.push_str(e.as_str()),
            _ => k.push('e'),
        }
        k.push('\u{1}');
    }
    k
}

fn col_name(mut c: u32) -> String {
    let mut s = Vec::new();
    c += 1;
    while c > 0 {
        let r = (c - 1) % 26;
        s.push((b'A' + r as u8) as char);
        c = (c - 1) / 26;
    }
    s.iter().rev().collect()
}

impl Ctx<'_> {
    // ---------- lazy logical functions (return references too) ----------

    pub(crate) fn f_if(&self, args: &[Expr]) -> Val {
        let Some(c) = args.first() else {
            return Val::V(Value::Error(ErrorKind::Value));
        };
        let cv = self.value(c);
        if let Value::Array(_) = cv {
            let t = args.get(1).map_or(Value::Bool(true), |e| self.value(e));
            let f = args.get(2).map_or(Value::Bool(false), |e| self.value(e));
            let (mut rows, mut cols) = cv.dims();
            for v in [&t, &f] {
                let (r, c) = v.dims();
                rows = bdim(rows, r);
                cols = bdim(cols, c);
            }
            let get = |v: &Value, i: usize, j: usize| match v {
                Value::Array(a) => pick(a, i, j),
                s => s.clone(),
            };
            let out = (0..rows)
                .map(|i| {
                    (0..cols)
                        .map(|j| match to_bool(&get(&cv, i, j)) {
                            Ok(true) => get(&t, i, j),
                            Ok(false) => get(&f, i, j),
                            Err(e) => Value::Error(e),
                        })
                        .collect()
                })
                .collect();
            return Val::V(Value::array(out));
        }
        match to_bool(&cv) {
            Err(e) => Val::V(Value::Error(e)),
            Ok(true) => match args.get(1) {
                Some(e) => self.eval(e),
                None => Val::V(Value::Bool(true)),
            },
            Ok(false) => match args.get(2) {
                Some(e) => self.eval(e),
                None => Val::V(Value::Bool(false)),
            },
        }
    }

    pub(crate) fn f_ifs(&self, args: &[Expr]) -> Val {
        if args.len() < 2 || !args.len().is_multiple_of(2) {
            return Val::V(Value::Error(ErrorKind::Value));
        }
        for p in args.chunks(2) {
            match to_bool(&self.scalar(&p[0])) {
                Err(e) => return Val::V(Value::Error(e)),
                Ok(true) => return self.eval(&p[1]),
                Ok(false) => {}
            }
        }
        Val::V(Value::Error(ErrorKind::NA))
    }

    pub(crate) fn f_switch(&self, args: &[Expr]) -> Val {
        if args.len() < 3 {
            return Val::V(Value::Error(ErrorKind::Value));
        }
        let v = self.scalar(&args[0]);
        if let Value::Error(e) = v {
            return Val::V(Value::Error(e));
        }
        let rest = &args[1..];
        for p in rest.chunks(2) {
            if p.len() == 1 {
                return self.eval(&p[0]);
            }
            let c = self.scalar(&p[0]);
            if let Value::Error(e) = c {
                return Val::V(Value::Error(e));
            }
            if (same_kind(&v, &c) || matches!(v, Value::Empty) || matches!(c, Value::Empty)) && cmp_values(&v, &c) == Ordering::Equal {
                return self.eval(&p[1]);
            }
        }
        Val::V(Value::Error(ErrorKind::NA))
    }

    pub(crate) fn f_choose(&self, args: &[Expr]) -> Val {
        let idx = match self.num(args, 0) {
            Ok(n) => n.trunc(),
            Err(e) => return Val::V(Value::Error(e)),
        };
        if idx < 1.0 || idx as usize >= args.len() {
            return Val::V(Value::Error(ErrorKind::Value));
        }
        self.eval(&args[idx as usize])
    }

    // ---------- reference functions ----------

    pub(crate) fn f_index(&self, args: &[Expr]) -> Val {
        let r = (|| -> R<Val> {
            let first = args.first().ok_or(ErrorKind::Value)?;
            let row = self.opt_num_m(args, 1, 0.0)?.trunc();
            let col_given = matches!(args.get(2), Some(e) if !matches!(e, Expr::Missing));
            let col = self.opt_num_m(args, 2, 0.0)?.trunc();
            if row < 0.0 || col < 0.0 {
                return Err(ErrorKind::Value);
            }
            let (row, col) = (row as u32, col as u32);
            let v = self.eval(first);
            let (h, w) = match &v {
                Val::R(a) => {
                    if a.s0 != a.s1 {
                        return Err(ErrorKind::Ref);
                    }
                    (a.rows(), a.cols())
                }
                Val::V(Value::Error(e)) => return Err(*e),
                Val::V(x) => {
                    let (r, c) = x.dims();
                    (r as u32, c as u32)
                }
            };
            let (r, c) = if col_given {
                (row, col)
            } else if h == 1 {
                (1, row)
            } else if w == 1 {
                (row, 1)
            } else {
                (row, 0)
            };
            if r > h || c > w {
                return Err(ErrorKind::Ref);
            }
            let (r0, r1) = if r == 0 { (0, h - 1) } else { (r - 1, r - 1) };
            let (c0, c1) = if c == 0 { (0, w - 1) } else { (c - 1, c - 1) };
            Ok(match v {
                Val::R(a) => Val::R(Area { r0: a.r0 + r0, r1: a.r0 + r1, c0: a.c0 + c0, c1: a.c0 + c1, ..a }),
                Val::V(Value::Array(arr)) => {
                    if r0 == r1 && c0 == c1 {
                        Val::V(arr[r0 as usize][c0 as usize].clone())
                    } else {
                        Val::V(Value::array((r0..=r1).map(|i| arr[i as usize][c0 as usize..=c1 as usize].to_vec()).collect()))
                    }
                }
                x => x,
            })
        })();
        r.unwrap_or_else(|e| Val::V(Value::Error(e)))
    }

    pub(crate) fn f_offset(&self, args: &[Expr]) -> Val {
        let r = (|| -> R<Val> {
            let base = match self.eval(args.first().ok_or(ErrorKind::Value)?) {
                Val::R(a) => a,
                Val::V(Value::Error(e)) => return Err(e),
                _ => return Err(ErrorKind::Value),
            };
            let dr = self.num(args, 1)?.trunc() as i64;
            let dc = self.num(args, 2)?.trunc() as i64;
            let h = self.opt_num_m(args, 3, base.rows() as f64)?.trunc() as i64;
            let w = self.opt_num_m(args, 4, base.cols() as f64)?.trunc() as i64;
            if h == 0 || w == 0 {
                return Err(ErrorKind::Ref);
            }
            let r0 = base.r0 as i64 + dr;
            let c0 = base.c0 as i64 + dc;
            let (ra, rb) = if h > 0 { (r0, r0 + h - 1) } else { (r0 + h + 1, r0) };
            let (ca, cb) = if w > 0 { (c0, c0 + w - 1) } else { (c0 + w + 1, c0) };
            if ra < 0 || ca < 0 || rb > MAX_ROW as i64 || cb > MAX_COL as i64 {
                return Err(ErrorKind::Ref);
            }
            Ok(Val::R(Area { r0: ra as u32, r1: rb as u32, c0: ca as u32, c1: cb as u32, ..base }))
        })();
        r.unwrap_or_else(|e| Val::V(Value::Error(e)))
    }

    pub(crate) fn f_indirect(&self, args: &[Expr]) -> Val {
        let r = (|| -> R<Val> {
            let text = self.text(args, 0)?;
            if !self.opt_bool(args, 1, true)? {
                return Err(ErrorKind::Ref);
            }
            let e = parser::parse(&text).map_err(|_| ErrorKind::Ref)?;
            let ok = match &e {
                Expr::Ref(_) | Expr::Name(_) => true,
                Expr::Bin(parser::BinOp::Range, lr) => matches!(lr.0, Expr::Ref(_)) && matches!(lr.1, Expr::Ref(_)),
                _ => false,
            };
            if !ok {
                return Err(ErrorKind::Ref);
            }
            match self.eval(&e) {
                Val::R(a) => Ok(Val::R(a)),
                _ => Err(ErrorKind::Ref),
            }
        })();
        r.unwrap_or_else(|e| Val::V(Value::Error(e)))
    }

    // ---------- scalar info functions ----------

    pub(crate) fn info_scalar(&self, f: Func, a: &[Value]) -> Option<Value> {
        use Func::*;
        let v = a.first().cloned().unwrap_or(Value::Empty);
        let r = (|| -> R<Option<Value>> {
            Ok(Some(match f {
                IsBlank => Value::Bool(matches!(v, Value::Empty)),
                IsNumber => Value::Bool(matches!(v, Value::Number(_))),
                IsText => Value::Bool(matches!(v, Value::Text(_))),
                IsNonText => Value::Bool(!matches!(v, Value::Text(_))),
                IsLogical => Value::Bool(matches!(v, Value::Bool(_))),
                IsError => Value::Bool(v.is_error()),
                IsErr => Value::Bool(matches!(v, Value::Error(e) if e != ErrorKind::NA)),
                IsNa => Value::Bool(v == Value::Error(ErrorKind::NA)),
                IsEven | IsOdd => {
                    if matches!(v, Value::Bool(_)) {
                        return Err(ErrorKind::Value);
                    }
                    let n = to_num(&v)?.trunc();
                    let even = n % 2.0 == 0.0;
                    Value::Bool(even == (f == IsEven))
                }
                Na => Value::Error(ErrorKind::NA),
                ErrorType => match v {
                    Value::Error(e) => Value::Number(e.code()),
                    _ => Value::Error(ErrorKind::NA),
                },
                Not => Value::Bool(!to_bool(&v)?),
                True => Value::Bool(true),
                False => Value::Bool(false),
                Address => {
                    let r = an(a, 0)?.trunc();
                    let c = an(a, 1)?.trunc();
                    let abs = ao(a, 2, 1.0)?.trunc() as i64;
                    let a1 = ab(a, 3, true)?;
                    if r < 1.0 || c < 1.0 || r > MAX_ROW as f64 + 1.0 || c > MAX_COL as f64 + 1.0 || !(1..=4).contains(&abs) {
                        return Err(ErrorKind::Value);
                    }
                    let (ra, ca) = (abs == 1 || abs == 2, abs == 1 || abs == 3);
                    let body = if a1 {
                        format!("{}{}{}{}", if ca { "$" } else { "" }, col_name(c as u32 - 1), if ra { "$" } else { "" }, r as u32)
                    } else {
                        let rp = if ra { format!("R{}", r) } else { format!("R[{}]", r) };
                        let cp = if ca { format!("C{}", c) } else { format!("C[{}]", c) };
                        rp + &cp
                    };
                    match a.get(4) {
                        Some(Value::Empty) | None => Value::from(body),
                        Some(s) => {
                            let s = to_text(s)?;
                            let needs_q = !s.chars().all(|ch| ch.is_alphanumeric() || ch == '_' || ch == '.')
                                || s.chars().next().is_some_and(|c| c.is_ascii_digit());
                            if needs_q {
                                Value::from(format!("'{}'!{}", s.replace('\'', "''"), body))
                            } else {
                                Value::from(format!("{}!{}", s, body))
                            }
                        }
                    }
                }
                _ => return Ok(None),
            }))
        })();
        match r {
            Ok(v) => v,
            Err(e) => Some(Value::Error(e)),
        }
    }

    // ---------- eager special functions ----------

    pub(crate) fn lookup_special(&self, f: Func, args: &[Expr]) -> Option<Value> {
        use Func::*;
        let r = (|| -> R<Option<Value>> {
            Ok(Some(match f {
                IfError | IfNa => {
                    let v = self.value(args.first().ok_or(ErrorKind::Value)?);
                    let hit = |x: &Value| match x {
                        Value::Error(e) => f == IfError || *e == ErrorKind::NA,
                        _ => false,
                    };
                    let alt = || args.get(1).map_or(Value::Empty, |e| self.value(e));
                    match &v {
                        Value::Array(a) => {
                            if a.iter().flatten().any(hit) {
                                let alt = alt().top_left();
                                Value::array(
                                    a.iter().map(|r| r.iter().map(|x| if hit(x) { alt.clone() } else { x.clone() }).collect()).collect(),
                                )
                            } else {
                                v
                            }
                        }
                        x if hit(x) => alt(),
                        _ => v,
                    }
                }
                And | Or | Xor => {
                    let (mut t, mut n) = (0usize, 0usize);
                    self.visit(args, &mut |v, rng| {
                        let b = match v {
                            Value::Bool(b) => *b,
                            Value::Number(x) => *x != 0.0,
                            Value::Error(e) => return Err(*e),
                            Value::Text(_) if !rng => to_bool(v)?,
                            _ => return Ok(()),
                        };
                        n += 1;
                        if b {
                            t += 1;
                        }
                        Ok(())
                    })?;
                    if n == 0 {
                        return Err(ErrorKind::Value);
                    }
                    Value::Bool(match f {
                        And => t == n,
                        Or => t > 0,
                        _ => t % 2 == 1,
                    })
                }
                VLookup | HLookup => {
                    let needle = self.scalar(args.first().ok_or(ErrorKind::Value)?);
                    if let Value::Error(e) = needle {
                        return Err(e);
                    }
                    let table = self.src(args.get(1).ok_or(ErrorKind::Value)?)?;
                    let idx = self.num(args, 2)?.trunc();
                    let approx = self.opt_bool(args, 3, true)?;
                    let v = f == VLookup;
                    let span = if v { table.cols() } else { table.rows() };
                    if idx < 1.0 {
                        return Err(ErrorKind::Value);
                    }
                    if idx > span as f64 {
                        return Err(ErrorKind::Ref);
                    }
                    let (eh, ew) = self.src_eff(&table);
                    let n = if v { eh } else { ew };
                    let get = |i: u32| {
                        if v { table.get(self, i, 0) } else { table.get(self, 0, i) }
                    };
                    let pos = if approx { approx_asc(&needle, n, &get) } else { exact_pos(&needle, n, &get, true, false) };
                    let i = pos.ok_or(ErrorKind::NA)?;
                    let k = idx as u32 - 1;
                    if v { table.get(self, i, k) } else { table.get(self, k, i) }
                }
                Match => {
                    let needle = self.scalar(args.first().ok_or(ErrorKind::Value)?);
                    if let Value::Error(e) = needle {
                        return Err(e);
                    }
                    let arr = self.src(args.get(1).ok_or(ErrorKind::Value)?)?;
                    let mt = self.opt_num(args, 2, 1.0)?;
                    let (_, vert) = arr.vector().ok_or(ErrorKind::NA)?;
                    let (eh, ew) = self.src_eff(&arr);
                    let n = if vert { eh } else { ew };
                    let get = |i: u32| arr.at(self, i, vert);
                    let pos = if mt == 0.0 {
                        exact_pos(&needle, n, &get, true, false)
                    } else if mt > 0.0 {
                        approx_asc(&needle, n, &get)
                    } else {
                        approx_desc(&needle, n, &get)
                    };
                    num(pos.ok_or(ErrorKind::NA)? as f64 + 1.0)
                }
                XLookup | XMatch => {
                    let needle = self.scalar(args.first().ok_or(ErrorKind::Value)?);
                    if let Value::Error(e) = needle {
                        return Err(e);
                    }
                    let look = self.src(args.get(1).ok_or(ErrorKind::Value)?)?;
                    let (_, vert) = look.vector().ok_or(ErrorKind::Value)?;
                    let (mi, si) = if f == XLookup { (4, 5) } else { (2, 3) };
                    let mode = self.opt_num_m(args, mi, 0.0)?.trunc() as i64;
                    let search = self.opt_num_m(args, si, 1.0)?.trunc() as i64;
                    if !(-1..=2).contains(&mode) || !matches!(search, 1 | -1 | 2 | -2) {
                        return Err(ErrorKind::Value);
                    }
                    let (eh, ew) = self.src_eff(&look);
                    let n = if vert { eh } else { ew };
                    let get = |i: u32| look.at(self, i, vert);
                    let rev = search < 0;
                    let pos = match mode {
                        0 | 2 => exact_pos(&needle, n, &get, mode == 2, rev),
                        _ => {
                            let mut best: Option<(u32, Value)> = None;
                            let order: Box<dyn Iterator<Item = u32>> = if rev { Box::new((0..n).rev()) } else { Box::new(0..n) };
                            let mut exact = None;
                            for i in order {
                                let v = get(i);
                                if !same_kind(&v, &needle) {
                                    continue;
                                }
                                let o = cmp_values(&v, &needle);
                                if o == Ordering::Equal {
                                    exact = Some(i);
                                    break;
                                }
                                let wanted = if mode < 0 { Ordering::Less } else { Ordering::Greater };
                                if o == wanted {
                                    let better = match &best {
                                        None => true,
                                        Some((_, b)) => cmp_values(&v, b) == wanted.reverse(),
                                    };
                                    if better {
                                        best = Some((i, v));
                                    }
                                }
                            }
                            exact.or(best.map(|b| b.0))
                        }
                    };
                    if f == XMatch {
                        return Ok(Some(num(pos.ok_or(ErrorKind::NA)? as f64 + 1.0)));
                    }
                    let ret = self.src(args.get(2).ok_or(ErrorKind::Value)?)?;
                    if (vert && ret.rows() != look.rows()) || (!vert && ret.cols() != look.cols()) {
                        return Err(ErrorKind::Value);
                    }
                    match pos {
                        None => match args.get(3) {
                            Some(e) if !matches!(e, Expr::Missing) => self.value(e),
                            _ => return Err(ErrorKind::NA),
                        },
                        Some(i) => {
                            if vert {
                                if ret.cols() == 1 {
                                    ret.get(self, i, 0)
                                } else {
                                    Value::array(vec![(0..ret.cols()).map(|j| ret.get(self, i, j)).collect()])
                                }
                            } else if ret.rows() == 1 {
                                ret.get(self, 0, i)
                            } else {
                                Value::array((0..ret.rows()).map(|j| vec![ret.get(self, j, i)]).collect())
                            }
                        }
                    }
                }
                Lookup => {
                    let needle = self.scalar(args.first().ok_or(ErrorKind::Value)?);
                    if let Value::Error(e) = needle {
                        return Err(e);
                    }
                    let look = self.src(args.get(1).ok_or(ErrorKind::Value)?)?;
                    let (eh, ew) = self.src_eff(&look);
                    let vert = match look.vector() {
                        Some((_, v)) => v,
                        None => look.rows() >= look.cols(),
                    };
                    let n = if vert { eh } else { ew };
                    let get = |i: u32| {
                        if vert { look.get(self, i, 0) } else { look.get(self, 0, i) }
                    };
                    let i = approx_asc(&needle, n, &get).ok_or(ErrorKind::NA)?;
                    match args.get(2) {
                        Some(e) => {
                            let res = self.src(e)?;
                            let (_, rv) = res.vector().ok_or(ErrorKind::NA)?;
                            res.at(self, i, rv)
                        }
                        None => {
                            if vert {
                                look.get(self, i, look.cols() - 1)
                            } else {
                                look.get(self, look.rows() - 1, i)
                            }
                        }
                    }
                }
                Row | Column => {
                    let is_row = f == Row;
                    match args.first() {
                        None | Some(Expr::Missing) => num(if is_row { self.row as f64 + 1.0 } else { self.col as f64 + 1.0 }),
                        Some(e) => match self.eval(e) {
                            Val::R(a) => {
                                let (lo, hi) = if is_row { (a.r0, a.r1) } else { (a.c0, a.c1) };
                                if lo == hi || hi - lo > 65535 {
                                    num(lo as f64 + 1.0)
                                } else if is_row {
                                    Value::array((lo..=hi).map(|r| vec![Value::Number(r as f64 + 1.0)]).collect())
                                } else {
                                    Value::array(vec![(lo..=hi).map(|c| Value::Number(c as f64 + 1.0)).collect()])
                                }
                            }
                            Val::V(Value::Error(e)) => return Err(e),
                            _ => return Err(ErrorKind::Value),
                        },
                    }
                }
                Rows | Columns => {
                    let e = args.first().ok_or(ErrorKind::Value)?;
                    let (r, c) = match self.eval(e) {
                        Val::R(a) => (a.rows() as usize, a.cols() as usize),
                        Val::V(Value::Error(e)) => return Err(e),
                        Val::V(v) => v.dims(),
                    };
                    num(if f == Rows { r } else { c } as f64)
                }
                Transpose => {
                    let v = self.value(args.first().ok_or(ErrorKind::Value)?);
                    match &v {
                        Value::Array(a) => Value::array(transpose(a)),
                        _ => v,
                    }
                }
                IsRef => Value::Bool(matches!(args.first().map(|e| self.eval(e)), Some(Val::R(_)))),
                Type => match self.eval(args.first().ok_or(ErrorKind::Value)?) {
                    Val::R(a) if !a.is_cell() => num(64.0),
                    v => {
                        let v = match v {
                            Val::R(a) => self.deref(a),
                            Val::V(v) => v,
                        };
                        num(match v {
                            Value::Empty | Value::Number(_) => 1.0,
                            Value::Text(_) => 2.0,
                            Value::Bool(_) => 4.0,
                            Value::Error(_) => 16.0,
                            Value::Array(_) => 64.0,
                        })
                    }
                },
                Sequence => {
                    let r = self.num(args, 0)?.trunc();
                    let c = self.opt_num_m(args, 1, 1.0)?.trunc();
                    let start = self.opt_num_m(args, 2, 1.0)?;
                    let step = self.opt_num_m(args, 3, 1.0)?;
                    if r < 1.0 || c < 1.0 {
                        return Err(ErrorKind::Calc);
                    }
                    if r * c > 4_000_000.0 {
                        return Err(ErrorKind::Num);
                    }
                    let (r, c) = (r as usize, c as usize);
                    Value::array((0..r).map(|i| (0..c).map(|j| num(start + step * (i * c + j) as f64)).collect()).collect())
                }
                Unique => {
                    let v = self.value(args.first().ok_or(ErrorKind::Value)?);
                    if let Value::Error(e) = v {
                        return Err(e);
                    }
                    let by_col = self.opt_bool(args, 1, false)?;
                    let once = self.opt_bool(args, 2, false)?;
                    let mut rows = to_rows(&v);
                    if by_col {
                        rows = transpose(&rows);
                    }
                    let keys: Vec<String> = rows.iter().map(|r| unique_key(r)).collect();
                    let mut counts: HashMap<&str, usize> = HashMap::new();
                    for k in &keys {
                        *counts.entry(k.as_str()).or_insert(0) += 1;
                    }
                    let mut seen: HashMap<&str, ()> = HashMap::new();
                    let mut out = Vec::new();
                    for (k, r) in keys.iter().zip(&rows) {
                        if once && counts[k.as_str()] != 1 {
                            continue;
                        }
                        if seen.insert(k.as_str(), ()).is_none() {
                            out.push(r.clone());
                        }
                    }
                    if out.is_empty() {
                        return Err(ErrorKind::Calc);
                    }
                    if by_col {
                        out = transpose(&out);
                    }
                    Value::array(out)
                }
                Filter => {
                    let v = self.value(args.first().ok_or(ErrorKind::Value)?);
                    if let Value::Error(e) = v {
                        return Err(e);
                    }
                    let inc = self.value(args.get(1).ok_or(ErrorKind::Value)?);
                    let rows = to_rows(&v);
                    let (h, w) = (rows.len(), rows[0].len());
                    let (ih, iw) = inc.dims();
                    let flags: Vec<bool> = to_rows(&inc)
                        .into_iter()
                        .flatten()
                        .map(|x| match x {
                            Value::Error(e) => Err(e),
                            x => to_bool(&x),
                        })
                        .collect::<R<_>>()?;
                    let out: Vec<Vec<Value>> = if iw == 1 && ih == h {
                        rows.into_iter().zip(flags).filter(|(_, f)| *f).map(|(r, _)| r).collect()
                    } else if ih == 1 && iw == w {
                        let t: Vec<Vec<Value>> = transpose(&rows).into_iter().zip(flags).filter(|(_, f)| *f).map(|(r, _)| r).collect();
                        if t.is_empty() { t } else { transpose(&t) }
                    } else {
                        return Err(ErrorKind::Value);
                    };
                    if out.is_empty() {
                        return match args.get(2) {
                            Some(e) if !matches!(e, Expr::Missing) => Ok(Some(self.value(e))),
                            _ => Err(ErrorKind::Calc),
                        };
                    }
                    Value::array(out)
                }
                Sort => {
                    let v = self.value(args.first().ok_or(ErrorKind::Value)?);
                    if let Value::Error(e) = v {
                        return Err(e);
                    }
                    let idx = self.opt_num_m(args, 1, 1.0)?.trunc();
                    let ord = self.opt_num_m(args, 2, 1.0)?.trunc();
                    let by_col = self.opt_bool(args, 3, false)?;
                    let mut rows = to_rows(&v);
                    if by_col {
                        rows = transpose(&rows);
                    }
                    if idx < 1.0 || idx as usize > rows[0].len() || (ord != 1.0 && ord != -1.0) {
                        return Err(ErrorKind::Value);
                    }
                    let k = idx as usize - 1;
                    rows.sort_by(|a, b| {
                        let o = sort_cmp(&a[k], &b[k]);
                        if ord < 0.0 { o.reverse() } else { o }
                    });
                    if by_col {
                        rows = transpose(&rows);
                    }
                    Value::array(rows)
                }
                SortBy => {
                    let v = self.value(args.first().ok_or(ErrorKind::Value)?);
                    if let Value::Error(e) = v {
                        return Err(e);
                    }
                    let rows = to_rows(&v);
                    let (h, w) = (rows.len(), rows[0].len());
                    let mut keys: Vec<(Vec<Value>, bool)> = Vec::new();
                    let mut by_col = None;
                    for p in args[1..].chunks(2) {
                        let by = self.value(&p[0]);
                        if let Value::Error(e) = by {
                            return Err(e);
                        }
                        let desc = match p.get(1) {
                            Some(e) => to_num(&self.scalar(e))? < 0.0,
                            None => false,
                        };
                        let (bh, bw) = by.dims();
                        let col = if bw == 1 && bh == h {
                            false
                        } else if bh == 1 && bw == w {
                            true
                        } else {
                            return Err(ErrorKind::Value);
                        };
                        if *by_col.get_or_insert(col) != col {
                            return Err(ErrorKind::Value);
                        }
                        keys.push((to_rows(&by).into_iter().flatten().collect(), desc));
                    }
                    let by_col = by_col.unwrap_or(false);
                    let mut items = if by_col { transpose(&rows) } else { rows };
                    let mut order: Vec<usize> = (0..items.len()).collect();
                    order.sort_by(|&a, &b| {
                        for (k, desc) in &keys {
                            let o = sort_cmp(&k[a], &k[b]);
                            let o = if *desc { o.reverse() } else { o };
                            if o != Ordering::Equal {
                                return o;
                            }
                        }
                        Ordering::Equal
                    });
                    let mut out: Vec<Vec<Value>> = order.iter().map(|&i| std::mem::take(&mut items[i])).collect();
                    if by_col {
                        out = transpose(&out);
                    }
                    Value::array(out)
                }
                _ => return Ok(None),
            }))
        })();
        match r {
            Ok(v) => v,
            Err(e) => Some(Value::Error(e)),
        }
    }
}
