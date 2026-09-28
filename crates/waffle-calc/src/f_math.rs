//! Math, trigonometry and financial functions.

use std::cell::Cell;

use super::eval::{Ctx, Src};
use super::format::round_half_away;
use super::funcs::Func;
use super::parser::Expr;
use super::value::*;

thread_local! {
    static RNG: Cell<u64> = Cell::new({
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E37_79B9_7F4A_7C15);
        t | 1
    });
}

/// Uniform random number in [0, 1) (xorshift64*).
pub(crate) fn rand01() -> f64 {
    RNG.with(|s| {
        let mut x = s.get();
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        s.set(x);
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
    })
}

fn round_dir(x: f64, d: f64, up: bool) -> f64 {
    let d = d.trunc().clamp(-308.0, 308.0) as i32;
    if x == 0.0 || !x.is_finite() {
        return x;
    }
    if d > 15 - x.abs().log10().floor() as i32 {
        return sig15(x); // no digits beyond 15 significant ones to cut
    }
    let p = 10f64.powi(d.abs());
    let x = sig15(x);
    let scaled = if d >= 0 { sig15(x * p) } else { sig15(x / p) };
    let r = if up { scaled.abs().ceil() * scaled.signum() } else { scaled.trunc() };
    sig15(if d >= 0 { r / p } else { r * p })
}

fn ceil_q(n: f64, s: f64) -> f64 {
    sig15(sig15(n / s).ceil() * s)
}
fn floor_q(n: f64, s: f64) -> f64 {
    sig15(sig15(n / s).floor() * s)
}

fn gcd(a: f64, b: f64) -> f64 {
    let (mut a, mut b) = (a, b);
    while b != 0.0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a
}

// ---- financial ----

pub(crate) fn fv(r: f64, n: f64, pmt: f64, pv: f64, t: f64) -> f64 {
    if r == 0.0 {
        -(pv + pmt * n)
    } else {
        let g = (1.0 + r).powf(n);
        -(pv * g + pmt * (1.0 + r * t) * (g - 1.0) / r)
    }
}

fn rate_solve(n: f64, pmt: f64, pv: f64, fvv: f64, t: f64, guess: f64) -> R<f64> {
    let f = |r: f64| {
        if r.abs() < 1e-12 {
            pv + pmt * n + fvv
        } else {
            let g = (1.0 + r).powf(n);
            pv * g + pmt * (1.0 + r * t) * (g - 1.0) / r + fvv
        }
    };
    let mut r = guess;
    for _ in 0..200 {
        let y = f(r);
        if y.abs() < 1e-10 {
            return Ok(r);
        }
        let h = 1e-7 * r.abs().max(1e-3);
        let d = (f(r + h) - f(r - h)) / (2.0 * h);
        if d == 0.0 || !d.is_finite() {
            break;
        }
        let nr = r - y / d;
        if !nr.is_finite() || nr <= -1.0 {
            break;
        }
        if (nr - r).abs() < 1e-12 {
            return Ok(nr);
        }
        r = nr;
    }
    Err(ErrorKind::Num)
}

pub(crate) fn npv(rate: f64, vals: &[f64]) -> f64 {
    vals.iter().enumerate().map(|(i, v)| v / (1.0 + rate).powi(i as i32 + 1)).sum()
}

fn irr(vals: &[f64], guess: f64) -> R<f64> {
    if !(vals.iter().any(|v| *v > 0.0) && vals.iter().any(|v| *v < 0.0)) {
        return Err(ErrorKind::Num);
    }
    let f = |r: f64| vals.iter().enumerate().map(|(i, v)| v / (1.0 + r).powi(i as i32)).sum::<f64>();
    let df = |r: f64| vals.iter().enumerate().map(|(i, v)| -(i as f64) * v / (1.0 + r).powi(i as i32 + 1)).sum::<f64>();
    let mut r = guess;
    for _ in 0..100 {
        let y = f(r);
        let d = df(r);
        if d == 0.0 || !d.is_finite() {
            break;
        }
        let nr = r - y / d;
        if !nr.is_finite() || nr <= -1.0 {
            break;
        }
        if (nr - r).abs() < 1e-12 {
            return Ok(nr);
        }
        r = nr;
    }
    Err(ErrorKind::Num)
}

impl Ctx<'_> {
    pub(crate) fn math_scalar(&self, f: Func, a: &[Value]) -> Option<Value> {
        use Func::*;
        let r = (|| -> R<Option<Value>> {
            let one = |g: fn(f64) -> f64| -> R<Option<Value>> { Ok(Some(num(g(an(a, 0)?)))) };
            Ok(Some(match f {
                Abs => return one(f64::abs),
                Int => return one(f64::floor),
                Exp => return one(f64::exp),
                Sin => return one(f64::sin),
                Cos => return one(f64::cos),
                Tan => return one(f64::tan),
                Atan => return one(f64::atan),
                Sinh => return one(f64::sinh),
                Cosh => return one(f64::cosh),
                Tanh => return one(f64::tanh),
                Degrees => return one(f64::to_degrees),
                Radians => return one(f64::to_radians),
                Asin | Acos => {
                    let x = an(a, 0)?;
                    if !(-1.0..=1.0).contains(&x) {
                        return Err(ErrorKind::Num);
                    }
                    num(if f == Asin { x.asin() } else { x.acos() })
                }
                Atan2 => {
                    let (x, y) = (an(a, 0)?, an(a, 1)?);
                    if x == 0.0 && y == 0.0 {
                        return Err(ErrorKind::Div0);
                    }
                    num(y.atan2(x))
                }
                Pi => num(std::f64::consts::PI),
                Sign => {
                    let x = an(a, 0)?;
                    num(if x > 0.0 {
                        1.0
                    } else if x < 0.0 {
                        -1.0
                    } else {
                        0.0
                    })
                }
                Round => num(round_half_away(an(a, 0)?, an(a, 1)?.trunc().clamp(-400.0, 400.0) as i32)),
                RoundUp => num(round_dir(an(a, 0)?, an(a, 1)?, true)),
                RoundDown => num(round_dir(an(a, 0)?, an(a, 1)?, false)),
                Trunc => num(round_dir(an(a, 0)?, ao(a, 1, 0.0)?, false)),
                MRound => {
                    let (n, m) = (an(a, 0)?, an(a, 1)?);
                    if m == 0.0 {
                        return Ok(Some(Value::Number(0.0)));
                    }
                    if n * m < 0.0 {
                        return Err(ErrorKind::Num);
                    }
                    num(sig15(round_half_away(sig15(n / m), 0) * m))
                }
                Ceiling => {
                    let (n, s) = (an(a, 0)?, ao(a, 1, 1.0)?);
                    if s == 0.0 || n == 0.0 {
                        return Ok(Some(Value::Number(0.0)));
                    }
                    if n > 0.0 && s < 0.0 {
                        return Err(ErrorKind::Num);
                    }
                    num(ceil_q(n, s))
                }
                Floor => {
                    let (n, s) = (an(a, 0)?, ao(a, 1, 1.0)?);
                    if s == 0.0 {
                        return if n == 0.0 { Ok(Some(Value::Number(0.0))) } else { Err(ErrorKind::Div0) };
                    }
                    if n > 0.0 && s < 0.0 {
                        return Err(ErrorKind::Num);
                    }
                    num(floor_q(n, s))
                }
                CeilingMath | CeilingPrecise => {
                    let n = an(a, 0)?;
                    let s = ao(a, 1, 1.0)?.abs();
                    let mode = if f == CeilingMath { ao(a, 2, 0.0)? } else { 0.0 };
                    if s == 0.0 {
                        return Ok(Some(Value::Number(0.0)));
                    }
                    num(if n < 0.0 && mode != 0.0 { -ceil_q(-n, s) } else { ceil_q(n, s) })
                }
                FloorMath | FloorPrecise => {
                    let n = an(a, 0)?;
                    let s = ao(a, 1, 1.0)?.abs();
                    let mode = if f == FloorMath { ao(a, 2, 0.0)? } else { 0.0 };
                    if s == 0.0 {
                        return Ok(Some(Value::Number(0.0)));
                    }
                    num(if n < 0.0 && mode != 0.0 { -floor_q(-n, s) } else { floor_q(n, s) })
                }
                Mod => {
                    let (n, d) = (an(a, 0)?, an(a, 1)?);
                    if d == 0.0 {
                        return Err(ErrorKind::Div0);
                    }
                    let q = sig15(n / d).floor();
                    let r = n - d * q;
                    num(if num_eq(r, d) || r.abs() < d.abs() * 1e-15 { 0.0 } else { r })
                }
                Power => super::eval::pow(an(a, 0)?, an(a, 1)?),
                Sqrt => {
                    let x = an(a, 0)?;
                    if x < 0.0 {
                        return Err(ErrorKind::Num);
                    }
                    num(x.sqrt())
                }
                Ln => {
                    let x = an(a, 0)?;
                    if x <= 0.0 {
                        return Err(ErrorKind::Num);
                    }
                    num(x.ln())
                }
                Log10 => {
                    let x = an(a, 0)?;
                    if x <= 0.0 {
                        return Err(ErrorKind::Num);
                    }
                    num(x.log10())
                }
                Log => {
                    let x = an(a, 0)?;
                    let b = ao(a, 1, 10.0)?;
                    if x <= 0.0 || b <= 0.0 {
                        return Err(ErrorKind::Num);
                    }
                    if b == 1.0 {
                        return Err(ErrorKind::Div0);
                    }
                    num(if b == 10.0 { x.log10() } else { x.ln() / b.ln() })
                }
                RandBetween => {
                    let lo = an(a, 0)?.ceil();
                    let hi = an(a, 1)?.floor();
                    if lo > hi {
                        return Err(ErrorKind::Num);
                    }
                    num(lo + (rand01() * (hi - lo + 1.0)).floor())
                }
                Quotient => {
                    let (n, d) = (an(a, 0)?, an(a, 1)?);
                    if d == 0.0 {
                        return Err(ErrorKind::Div0);
                    }
                    num(sig15(n / d).trunc())
                }
                Even => {
                    let x = an(a, 0)?;
                    num(x.signum() * (sig15(x.abs() / 2.0).ceil() * 2.0))
                }
                Odd => {
                    let x = an(a, 0)?;
                    let mut v = sig15(x.abs()).ceil();
                    if v % 2.0 == 0.0 {
                        v += 1.0;
                    }
                    num(if x < 0.0 { -v } else { v })
                }
                Fact => {
                    let n = an(a, 0)?;
                    if n < 0.0 {
                        return Err(ErrorKind::Num);
                    }
                    let n = n.trunc() as u64;
                    if n > 170 {
                        return Err(ErrorKind::Num);
                    }
                    num((1..=n).fold(1.0, |acc, k| acc * k as f64))
                }
                Combin => {
                    let (n, k) = (an(a, 0)?.trunc(), an(a, 1)?.trunc());
                    if n < 0.0 || k < 0.0 || k > n {
                        return Err(ErrorKind::Num);
                    }
                    let k = k.min(n - k) as u64;
                    let mut r = 1.0;
                    for i in 0..k {
                        r = r * (n - i as f64) / (i as f64 + 1.0);
                        if !r.is_finite() {
                            return Err(ErrorKind::Num);
                        }
                    }
                    num(r.round())
                }
                // financial
                Pmt => {
                    let (r, n, pv) = (an(a, 0)?, an(a, 1)?, an(a, 2)?);
                    let (fvv, t) = (ao(a, 3, 0.0)?, if ao(a, 4, 0.0)? != 0.0 { 1.0 } else { 0.0 });
                    if n == 0.0 {
                        return Err(ErrorKind::Num);
                    }
                    if r == 0.0 {
                        num(-(pv + fvv) / n)
                    } else {
                        let g = (1.0 + r).powf(n);
                        num(-(r * (fvv + pv * g)) / ((1.0 + r * t) * (g - 1.0)))
                    }
                }
                Fv => {
                    let (r, n, pmt) = (an(a, 0)?, an(a, 1)?, an(a, 2)?);
                    let (pv, t) = (ao(a, 3, 0.0)?, if ao(a, 4, 0.0)? != 0.0 { 1.0 } else { 0.0 });
                    num(fv(r, n, pmt, pv, t))
                }
                Pv => {
                    let (r, n, pmt) = (an(a, 0)?, an(a, 1)?, an(a, 2)?);
                    let (fvv, t) = (ao(a, 3, 0.0)?, if ao(a, 4, 0.0)? != 0.0 { 1.0 } else { 0.0 });
                    if r == 0.0 {
                        num(-(fvv + pmt * n))
                    } else {
                        let g = (1.0 + r).powf(n);
                        num(-(fvv + pmt * (1.0 + r * t) * (g - 1.0) / r) / g)
                    }
                }
                NPer => {
                    let (r, pmt, pv) = (an(a, 0)?, an(a, 1)?, an(a, 2)?);
                    let (fvv, t) = (ao(a, 3, 0.0)?, if ao(a, 4, 0.0)? != 0.0 { 1.0 } else { 0.0 });
                    if r == 0.0 {
                        if pmt == 0.0 {
                            return Err(ErrorKind::Num);
                        }
                        num(-(pv + fvv) / pmt)
                    } else {
                        let k = pmt * (1.0 + r * t);
                        let x = (k - fvv * r) / (k + pv * r);
                        if x <= 0.0 {
                            return Err(ErrorKind::Num);
                        }
                        num(x.ln() / (1.0 + r).ln())
                    }
                }
                Rate => {
                    let (n, pmt, pv) = (an(a, 0)?, an(a, 1)?, an(a, 2)?);
                    let (fvv, t) = (ao(a, 3, 0.0)?, if ao(a, 4, 0.0)? != 0.0 { 1.0 } else { 0.0 });
                    let guess = match a.get(5) {
                        Some(Value::Empty) | None => 0.1,
                        Some(v) => to_num(v)?,
                    };
                    num(rate_solve(n, pmt, pv, fvv, t, guess)?)
                }
                _ => return Ok(None),
            }))
        })();
        match r {
            Ok(v) => v,
            Err(e) => Some(Value::Error(e)),
        }
    }

    pub(crate) fn math_special(&self, f: Func, args: &[Expr]) -> Option<Value> {
        use Func::*;
        let r = (|| -> R<Option<Value>> {
            Ok(Some(match f {
                Sum | SumSq | Product => {
                    let mut acc = if f == Product { 1.0 } else { 0.0 };
                    let mut any = false;
                    self.visit(args, &mut |v, rng| {
                        let x = match v {
                            Value::Number(n) => *n,
                            Value::Error(e) => return Err(*e),
                            Value::Bool(b) if !rng => {
                                if *b {
                                    1.0
                                } else {
                                    0.0
                                }
                            }
                            Value::Text(t) if !rng => parse_num_text(t).ok_or(ErrorKind::Value)?,
                            _ => return Ok(()),
                        };
                        any = true;
                        match f {
                            Sum => acc += x,
                            SumSq => acc += x * x,
                            _ => acc *= x,
                        }
                        Ok(())
                    })?;
                    if f == Product && !any {
                        acc = 0.0;
                    }
                    num(acc)
                }
                SumProduct => self.sumproduct(args)?,
                Rand => num(rand01()),
                Gcd | Lcm => {
                    let mut acc: Option<f64> = None;
                    self.visit(args, &mut |v, rng| {
                        let x = match v {
                            Value::Number(n) => *n,
                            Value::Error(e) => return Err(*e),
                            Value::Empty => return Ok(()),
                            v if !rng => to_num(v)?,
                            _ => return Err(ErrorKind::Value),
                        };
                        if x < 0.0 {
                            return Err(ErrorKind::Num);
                        }
                        let x = x.trunc();
                        acc = Some(match acc {
                            None => x,
                            Some(a) if f == Gcd => gcd(a, x),
                            Some(a) => {
                                if a == 0.0 || x == 0.0 {
                                    0.0
                                } else {
                                    a / gcd(a, x) * x
                                }
                            }
                        });
                        Ok(())
                    })?;
                    num(acc.unwrap_or(0.0))
                }
                Subtotal => {
                    let code = self.num(args, 0)?.trunc() as i64;
                    let g = match code % 100 {
                        1 => Average,
                        2 => Count,
                        3 => CountA,
                        4 => Max,
                        5 => Min,
                        6 => Product,
                        7 => StdevS,
                        8 => StdevP,
                        9 => Sum,
                        10 => VarS,
                        11 => VarP,
                        _ => return Err(ErrorKind::Value),
                    };
                    if !(1..=11).contains(&code) && !(101..=111).contains(&code) {
                        return Err(ErrorKind::Value);
                    }
                    self.special(g, &args[1..])
                }
                Aggregate => self.aggregate(args)?,
                Npv => {
                    let rate = self.num(args, 0)?;
                    let vals = self.nums(&args[1..], false, false)?;
                    if rate == -1.0 {
                        return Err(ErrorKind::Div0);
                    }
                    num(npv(rate, &vals))
                }
                Irr => {
                    let vals = self.nums(&args[..1.min(args.len())], false, false)?;
                    let guess = self.opt_num_m(args, 1, 0.1)?;
                    num(irr(&vals, guess)?)
                }
                _ => return Ok(None),
            }))
        })();
        match r {
            Ok(v) => v,
            Err(e) => Some(Value::Error(e)),
        }
    }

    fn sumproduct(&self, args: &[Expr]) -> R<Value> {
        if args.is_empty() {
            return Err(ErrorKind::Value);
        }
        let srcs: Vec<Src> = args.iter().map(|a| self.src(a)).collect::<R<_>>()?;
        // Whole-column/row references are flexible: they match arrays that
        // were materialized clamped to the sheet extent.
        let shape = |s: &Src| match s {
            Src::Area(a) => (a.rows(), a.cols(), a.r0 == 0 && a.r1 == super::MAX_ROW, a.c0 == 0 && a.c1 == super::MAX_COL),
            s => (s.rows(), s.cols(), false, false),
        };
        let shapes: Vec<_> = srcs.iter().map(shape).collect();
        let h = shapes.iter().find(|s| !s.2).map_or(shapes[0].0, |s| s.0);
        let w = shapes.iter().find(|s| !s.3).map_or(shapes[0].1, |s| s.1);
        if shapes.iter().any(|s| (if s.2 { s.0 < h } else { s.0 != h }) || (if s.3 { s.1 < w } else { s.1 != w })) {
            return Err(ErrorKind::Value);
        }
        let (mut eh, mut ew) = (0, 0);
        for s in &srcs {
            let (a, b) = self.src_eff(s);
            eh = eh.max(a);
            ew = ew.max(b);
        }
        let (eh, ew) = (eh.min(h), ew.min(w));
        let mut total = 0.0;
        for i in 0..eh {
            for j in 0..ew {
                let mut p = 1.0;
                for s in &srcs {
                    match s.get(self, i, j) {
                        Value::Number(n) => p *= n,
                        Value::Error(e) => return Err(e),
                        _ => p = 0.0,
                    }
                }
                total += p;
            }
        }
        Ok(num(total))
    }

    fn aggregate(&self, args: &[Expr]) -> R<Value> {
        use Func::*;
        let code = self.num(args, 0)?.trunc() as i64;
        let opt = self.opt_num(args, 1, 0.0)?.trunc() as i64;
        if !(0..=7).contains(&opt) {
            return Err(ErrorKind::Value);
        }
        let skip = matches!(opt, 2 | 3 | 6 | 7);
        let rest = args.get(2..).unwrap_or(&[]);
        let g = match code {
            1 => Average,
            2 => Count,
            3 => CountA,
            4 => Max,
            5 => Min,
            6 => Product,
            7 => StdevS,
            8 => StdevP,
            9 => Sum,
            10 => VarS,
            11 => VarP,
            12 => Median,
            13 => Mode,
            14 => Large,
            15 => Small,
            16 => PercentileInc,
            17 => QuartileInc,
            18 => PercentileExc,
            19 => QuartileExc,
            _ => return Err(ErrorKind::Value),
        };
        if g == CountA {
            let mut n = 0.0;
            self.visit(rest, &mut |v, _| {
                if !(matches!(v, Value::Empty) || (skip && v.is_error())) {
                    n += 1.0;
                }
                Ok(())
            })?;
            return Ok(num(n));
        }
        if code >= 14 {
            let mut v = self.nums(&rest[..1.min(rest.len())], false, skip)?;
            let k = self.num(rest, 1)?;
            return Ok(num(super::f_stats::stat_k(g, &mut v, k)?));
        }
        let mut v = self.nums(rest, false, skip)?;
        Ok(num(super::f_stats::stat(g, &mut v)?))
    }
}
