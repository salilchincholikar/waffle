//! Statistical and conditional-aggregate functions.

use super::criteria::Crit;
use super::eval::{Ctx, Src, pick};
use super::funcs::Func;
use super::parser::Expr;
use super::value::*;

fn mean(v: &[f64]) -> f64 {
    v.iter().sum::<f64>() / v.len() as f64
}

fn var(v: &[f64], sample: bool) -> R<f64> {
    let n = v.len();
    if n == 0 || (sample && n < 2) {
        return Err(ErrorKind::Div0);
    }
    let m = mean(v);
    let ss: f64 = v.iter().map(|x| (x - m) * (x - m)).sum();
    Ok(ss / if sample { (n - 1) as f64 } else { n as f64 })
}

fn sort(v: &mut [f64]) {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
}

fn percentile_inc(v: &mut [f64], k: f64) -> R<f64> {
    if v.is_empty() || !(0.0..=1.0).contains(&k) {
        return Err(ErrorKind::Num);
    }
    sort(v);
    let r = k * (v.len() - 1) as f64;
    let lo = r.floor() as usize;
    let fr = r - lo as f64;
    Ok(if lo + 1 < v.len() { v[lo] + fr * (v[lo + 1] - v[lo]) } else { v[lo] })
}

fn percentile_exc(v: &mut [f64], k: f64) -> R<f64> {
    let n = v.len() as f64;
    if v.is_empty() || k <= 0.0 || k >= 1.0 {
        return Err(ErrorKind::Num);
    }
    let r = k * (n + 1.0) - 1.0;
    if r < 0.0 || r > n - 1.0 {
        return Err(ErrorKind::Num);
    }
    sort(v);
    let lo = r.floor() as usize;
    let fr = r - lo as f64;
    Ok(if lo + 1 < v.len() { v[lo] + fr * (v[lo + 1] - v[lo]) } else { v[lo] })
}

/// Aggregate over collected numbers.
pub(crate) fn stat(f: Func, v: &mut [f64]) -> R<f64> {
    use Func::*;
    Ok(match f {
        Sum => v.iter().sum(),
        Count => v.len() as f64,
        Average | AverageA => {
            if v.is_empty() {
                return Err(ErrorKind::Div0);
            }
            mean(v)
        }
        Max | MaxA => v.iter().copied().fold(None, |m: Option<f64>, x| Some(m.map_or(x, |m| m.max(x)))).unwrap_or(0.0),
        Min | MinA => v.iter().copied().fold(None, |m: Option<f64>, x| Some(m.map_or(x, |m| m.min(x)))).unwrap_or(0.0),
        Product => {
            if v.is_empty() {
                0.0
            } else {
                v.iter().product()
            }
        }
        StdevS => var(v, true)?.sqrt(),
        StdevP => var(v, false)?.sqrt(),
        VarS => var(v, true)?,
        VarP => var(v, false)?,
        Median => {
            if v.is_empty() {
                return Err(ErrorKind::Num);
            }
            percentile_inc(v, 0.5)?
        }
        Mode => {
            let mut best: Option<(f64, usize)> = None;
            for (i, x) in v.iter().enumerate() {
                if v[..i].iter().any(|y| num_eq(*x, *y)) {
                    continue;
                }
                let c = v[i..].iter().filter(|y| num_eq(*x, **y)).count();
                if c > 1 && best.is_none_or(|(_, bc)| c > bc) {
                    best = Some((*x, c));
                }
            }
            best.ok_or(ErrorKind::NA)?.0
        }
        _ => return Err(ErrorKind::Value),
    })
}

/// Aggregates taking a `k` parameter.
pub(crate) fn stat_k(f: Func, v: &mut [f64], k: f64) -> R<f64> {
    use Func::*;
    match f {
        Large | Small => {
            let k = k.ceil();
            if k < 1.0 || k > v.len() as f64 {
                return Err(ErrorKind::Num);
            }
            sort(v);
            let k = k as usize;
            Ok(if f == Small { v[k - 1] } else { v[v.len() - k] })
        }
        PercentileInc => percentile_inc(v, k),
        PercentileExc => percentile_exc(v, k),
        QuartileInc => {
            let q = k.trunc();
            if !(0.0..=4.0).contains(&q) {
                return Err(ErrorKind::Num);
            }
            percentile_inc(v, q / 4.0)
        }
        QuartileExc => {
            let q = k.trunc();
            if !(1.0..=3.0).contains(&q) {
                return Err(ErrorKind::Num);
            }
            percentile_exc(v, q / 4.0)
        }
        _ => Err(ErrorKind::Value),
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Agg {
    Sum,
    Count,
    Avg,
    Max,
    Min,
}

impl Ctx<'_> {
    /// Collects numbers from aggregate-style arguments.
    /// `a_mode`: text in ranges counts as 0 and booleans as 1/0 (…A functions).
    pub(crate) fn nums(&self, args: &[Expr], a_mode: bool, skip_err: bool) -> R<Vec<f64>> {
        let mut out = Vec::new();
        self.visit(args, &mut |v, rng| {
            match v {
                Value::Number(n) => out.push(*n),
                Value::Error(e) => {
                    if !skip_err {
                        return Err(*e);
                    }
                }
                Value::Bool(b) => {
                    if a_mode || !rng {
                        out.push(if *b { 1.0 } else { 0.0 })
                    }
                }
                Value::Text(t) => {
                    if rng {
                        if a_mode {
                            out.push(0.0)
                        }
                    } else {
                        match parse_num_text(t) {
                            Some(n) => out.push(n),
                            None => {
                                if !skip_err {
                                    return Err(ErrorKind::Value);
                                }
                            }
                        }
                    }
                }
                Value::Empty => {
                    if !rng {
                        out.push(0.0)
                    }
                }
                Value::Array(_) => {}
            }
            Ok(())
        })?;
        Ok(out)
    }

    pub(crate) fn stats_special(&self, f: Func, args: &[Expr]) -> Option<Value> {
        use Func::*;
        let r = (|| -> R<Option<Value>> {
            Ok(Some(match f {
                Average | Max | Min | StdevS | StdevP | VarS | VarP | Median | Mode => num(stat(f, &mut self.nums(args, false, false)?)?),
                AverageA | MaxA | MinA => num(stat(f, &mut self.nums(args, true, false)?)?),
                Count => {
                    let mut n = 0.0;
                    self.visit(args, &mut |v, rng| {
                        match v {
                            Value::Number(_) => n += 1.0,
                            Value::Bool(_) if !rng => n += 1.0,
                            Value::Text(t) if !rng && parse_num_text(t).is_some() => n += 1.0,
                            _ => {}
                        }
                        Ok(())
                    })?;
                    num(n)
                }
                CountA => {
                    let mut n = 0.0;
                    self.visit(args, &mut |v, rng| {
                        if !rng || !matches!(v, Value::Empty) {
                            n += 1.0;
                        }
                        Ok(())
                    })?;
                    num(n)
                }
                CountBlank => {
                    let s = self.src(args.first().ok_or(ErrorKind::Value)?)?;
                    let total = s.rows() as f64 * s.cols() as f64;
                    let mut nonblank = 0.0;
                    let mut chk = |v: &Value| {
                        match v {
                            Value::Empty => {}
                            Value::Text(t) if t.is_empty() => {}
                            _ => nonblank += 1.0,
                        }
                        Ok(())
                    };
                    match &s {
                        Src::Area(a) => self.each_cell(a, &mut chk)?,
                        Src::Arr(a) => {
                            for v in a.iter().flatten() {
                                chk(v)?;
                            }
                        }
                        Src::One(v) => chk(v)?,
                    }
                    num(total - nonblank)
                }
                Large | Small | PercentileInc | PercentileExc | QuartileInc | QuartileExc => {
                    let v = self.nums(&args[..1.min(args.len())], false, false)?;
                    let k = match args.get(1) {
                        Some(e) => self.value(e),
                        None => return Err(ErrorKind::Value),
                    };
                    let one = |k: &Value| -> Value {
                        match to_num(k) {
                            Ok(k) => res(stat_k(f, &mut v.clone(), k).map(num)),
                            Err(e) => Value::Error(e),
                        }
                    };
                    match &k {
                        Value::Array(a) => Value::array(a.iter().map(|r| r.iter().map(one).collect()).collect()),
                        k => one(k),
                    }
                }
                Rank => {
                    let x = self.num(args, 0)?;
                    let v = self.nums(args.get(1..2).ok_or(ErrorKind::Value)?, false, false)?;
                    let asc = self.opt_num(args, 2, 0.0)? != 0.0;
                    if !v.iter().any(|y| num_eq(*y, x)) {
                        return Err(ErrorKind::NA);
                    }
                    let better = v.iter().filter(|y| if asc { **y < x && !num_eq(**y, x) } else { **y > x && !num_eq(**y, x) }).count();
                    num(better as f64 + 1.0)
                }
                CountIf => self.ifs(Agg::Count, None, args.first(), &[(args.first(), args.get(1))])?,
                SumIf => {
                    let target = args.get(2).filter(|e| !matches!(e, Expr::Missing)).or(args.first());
                    self.ifs(Agg::Sum, Some(target), args.first(), &[(args.first(), args.get(1))])?
                }
                AverageIf => {
                    let target = args.get(2).filter(|e| !matches!(e, Expr::Missing)).or(args.first());
                    self.ifs(Agg::Avg, Some(target), args.first(), &[(args.first(), args.get(1))])?
                }
                CountIfs => {
                    if args.is_empty() || !args.len().is_multiple_of(2) {
                        return Err(ErrorKind::Value);
                    }
                    let pairs: Vec<_> = args.chunks(2).map(|c| (c.first(), c.get(1))).collect();
                    self.ifs(Agg::Count, None, None, &pairs)?
                }
                SumIfs | AverageIfs | MaxIfs | MinIfs => {
                    if args.len() < 3 || args.len() % 2 != 1 {
                        return Err(ErrorKind::Value);
                    }
                    let pairs: Vec<_> = args[1..].chunks(2).map(|c| (c.first(), c.get(1))).collect();
                    let agg = match f {
                        SumIfs => Agg::Sum,
                        AverageIfs => Agg::Avg,
                        MaxIfs => Agg::Max,
                        _ => Agg::Min,
                    };
                    self.ifs(agg, Some(args.first()), None, &pairs)?
                }
                _ => return Ok(None),
            }))
        })();
        match r {
            Ok(v) => v,
            Err(e) => Some(Value::Error(e)),
        }
    }

    /// Shared engine for the *IF / *IFS family.
    /// `resize_like`: SUMIF/AVERAGEIF resize the target to the criteria range.
    fn ifs(
        &self,
        agg: Agg,
        target: Option<Option<&Expr>>,
        resize_like: Option<&Expr>,
        pairs: &[(Option<&Expr>, Option<&Expr>)],
    ) -> R<Value> {
        let mut ranges = Vec::with_capacity(pairs.len());
        let mut crit_vals = Vec::with_capacity(pairs.len());
        for (r, c) in pairs {
            let (r, c) = (r.ok_or(ErrorKind::Value)?, c.ok_or(ErrorKind::Value)?);
            ranges.push(self.src(r)?);
            crit_vals.push(self.value(c));
        }
        let mut tsrc = match target {
            Some(t) => Some(self.src(t.ok_or(ErrorKind::Value)?)?),
            None => None,
        };
        let (h, w) = (ranges[0].rows(), ranges[0].cols());
        if ranges.iter().any(|r| r.rows() != h || r.cols() != w) {
            return Err(ErrorKind::Value);
        }
        if let Some(t) = &mut tsrc {
            if resize_like.is_some() {
                if let Src::Area(a) = t {
                    a.r1 = (a.r0 + h - 1).min(super::MAX_ROW);
                    a.c1 = (a.c0 + w - 1).min(super::MAX_COL);
                }
            } else if t.rows() != h || t.cols() != w {
                return Err(ErrorKind::Value);
            }
        }
        // criteria arrays → array result
        let mut dims: Option<(usize, usize)> = None;
        for v in &crit_vals {
            if let Value::Array(_) = v {
                let (r, c) = v.dims();
                dims = Some(dims.map_or((r, c), |(a, b)| (a.max(r), b.max(c))));
            }
        }
        match dims {
            None => {
                let crits: Vec<Crit> = crit_vals.iter().map(Crit::parse).collect();
                Ok(self.ifs_core(agg, tsrc.as_ref(), &ranges, &crits))
            }
            Some((rr, cc)) => {
                let out = (0..rr)
                    .map(|i| {
                        (0..cc)
                            .map(|j| {
                                let crits: Vec<Crit> = crit_vals
                                    .iter()
                                    .map(|v| match v {
                                        Value::Array(a) => Crit::parse(&pick(a, i, j)),
                                        v => Crit::parse(v),
                                    })
                                    .collect();
                                self.ifs_core(agg, tsrc.as_ref(), &ranges, &crits)
                            })
                            .collect()
                    })
                    .collect();
                Ok(Value::array(out))
            }
        }
    }

    fn ifs_core(&self, agg: Agg, target: Option<&Src>, ranges: &[Src], crits: &[Crit]) -> Value {
        let (h, w) = (ranges[0].rows(), ranges[0].cols());
        let (mut eh, mut ew) = (0u32, 0u32);
        for s in ranges.iter().chain(target) {
            let (a, b) = self.src_eff(s);
            eh = eh.max(a);
            ew = ew.max(b);
        }
        let (eh, ew) = (eh.min(h), ew.min(w));
        let (mut sum, mut count) = (0.0f64, 0.0f64);
        let mut best: Option<f64> = None;
        for i in 0..eh {
            'cell: for j in 0..ew {
                for (r, c) in ranges.iter().zip(crits) {
                    if !c.matches(&r.get(self, i, j)) {
                        continue 'cell;
                    }
                }
                match (agg, target) {
                    (Agg::Count, _) => count += 1.0,
                    (_, Some(t)) => match t.get(self, i, j) {
                        Value::Number(n) => {
                            sum += n;
                            count += 1.0;
                            best = Some(match (agg, best) {
                                (_, None) => n,
                                (Agg::Max, Some(b)) => b.max(n),
                                (_, Some(b)) => b.min(n),
                            });
                        }
                        Value::Error(e) => return Value::Error(e),
                        _ => {}
                    },
                    _ => {}
                }
            }
        }
        if agg == Agg::Count {
            let extra = h as f64 * w as f64 - eh as f64 * ew as f64;
            if extra > 0.0 && crits.iter().all(|c| c.matches(&Value::Empty)) {
                count += extra;
            }
        }
        match agg {
            Agg::Sum => num(sum),
            Agg::Count => num(count),
            Agg::Avg => {
                if count == 0.0 {
                    Value::Error(ErrorKind::Div0)
                } else {
                    num(sum / count)
                }
            }
            Agg::Max | Agg::Min => num(best.unwrap_or(0.0)),
        }
    }
}
