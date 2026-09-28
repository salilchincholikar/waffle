//! Date and time functions.

use std::collections::HashSet;

use super::dates::*;
use super::eval::Ctx;
use super::funcs::Func;
use super::parser::Expr;
use super::value::*;

/// Weekend mask indexed Monday = 0 .. Sunday = 6.
fn weekend_mask(v: &Value) -> R<[bool; 7]> {
    let mut m = [false; 7];
    match v {
        Value::Text(t) => {
            let b = t.as_bytes();
            if b.len() != 7 || !b.iter().all(|c| *c == b'0' || *c == b'1') || b.iter().all(|c| *c == b'1') {
                return Err(ErrorKind::Value);
            }
            for (i, c) in b.iter().enumerate() {
                m[i] = *c == b'1';
            }
        }
        Value::Empty => {
            m[5] = true;
            m[6] = true;
        }
        v => {
            let n = to_num(v)?.trunc() as i64;
            match n {
                1..=7 => {
                    // 1: Sat+Sun, 2: Sun+Mon, ... 7: Fri+Sat
                    let first = (n as usize + 4) % 7; // Sat index 5 for n = 1
                    m[first] = true;
                    m[(first + 1) % 7] = true;
                }
                11..=17 => m[(n as usize - 11 + 6) % 7] = true, // 11: Sun only, 12: Mon only
                _ => return Err(ErrorKind::Num),
            }
        }
    }
    Ok(m)
}

impl Ctx<'_> {
    fn ymd(&self, v: &Value) -> R<(i64, u32, u32)> {
        serial_to_ymd(to_num(v)?, self.d1904)
    }

    fn serial(&self, y: i64, m: u32, d: u32) -> R<f64> {
        let s = serial_from_ymd(y, m, d, self.d1904);
        if s < 0 || s > max_serial(self.d1904) {
            return Err(ErrorKind::Num);
        }
        Ok(s as f64)
    }

    fn add_months(&self, v: &Value, months: f64, eom: bool) -> R<f64> {
        let (y, m, d) = self.ymd(v)?;
        let total = y * 12 + (m as i64 - 1) + months.trunc() as i64;
        let (ny, nm) = (total.div_euclid(12), (total.rem_euclid(12) + 1) as u32);
        if !(1900..=9999).contains(&ny) {
            return Err(ErrorKind::Num);
        }
        let dim = days_in_month(ny, nm);
        self.serial(ny, nm, if eom { dim } else { d.clamp(1, dim) })
    }

    fn weeknum(&self, s: f64, ty: i64) -> R<f64> {
        let (y, _, _) = serial_to_ymd(s, self.d1904)?;
        if ty == 21 {
            return self.isoweek(s);
        }
        let start = match ty {
            1 | 17 => 0,
            2 | 11 => 1,
            12..=16 => ty - 10,
            _ => return Err(ErrorKind::Num),
        };
        let jan1 = serial_from_ymd(y, 1, 1, self.d1904) as f64;
        let off = (weekday_sun0(jan1, self.d1904) - start).rem_euclid(7);
        Ok(((s.floor() - jan1) as i64 + off) as f64 / 7.0).map(|w| w.floor() + 1.0)
    }

    fn isoweek(&self, s: f64) -> R<f64> {
        let (y, _, _) = serial_to_ymd(s, self.d1904)?;
        let s = s.floor();
        let wd = weekday_mon0(s, self.d1904) as i64 + 1;
        let doy = (s - serial_from_ymd(y, 1, 1, self.d1904) as f64) as i64 + 1;
        let weeks_in = |yy: i64| {
            let j = serial_from_ymd(yy, 1, 1, self.d1904) as f64;
            let w = weekday_mon0(j, self.d1904);
            if w == 3 || (w == 2 && is_leap(yy)) { 53 } else { 52 }
        };
        let w = (doy - wd + 10) / 7;
        Ok(if w < 1 {
            weeks_in(y - 1) as f64
        } else if w > weeks_in(y) {
            1.0
        } else {
            w as f64
        })
    }

    fn datedif(&self, s: f64, e: f64, unit: &str) -> R<f64> {
        let (s, e) = (s.floor(), e.floor());
        if s > e {
            return Err(ErrorKind::Num);
        }
        let (y1, m1, d1) = serial_to_ymd(s, self.d1904)?;
        let (y2, m2, d2) = serial_to_ymd(e, self.d1904)?;
        let months = (y2 - y1) * 12 + m2 as i64 - m1 as i64 - if d2 < d1 { 1 } else { 0 };
        Ok(match unit.to_ascii_uppercase().as_str() {
            "Y" => (months / 12) as f64,
            "M" => months as f64,
            "D" => e - s,
            "YM" => (months % 12) as f64,
            "YD" => {
                let yy = if (m1, d1) <= (m2, d2) { y2 } else { y2 - 1 };
                let d = d1.min(days_in_month(yy, m1));
                e - serial_from_ymd(yy, m1, d, self.d1904) as f64
            }
            "MD" => {
                if d2 >= d1 {
                    (d2 - d1) as f64
                } else {
                    let (py, pm) = if m2 == 1 { (y2 - 1, 12) } else { (y2, m2 - 1) };
                    (days_in_month(py, pm) as i64 - d1 as i64 + d2 as i64).max(0) as f64
                }
            }
            _ => return Err(ErrorKind::Num),
        })
    }

    fn yearfrac(&self, s: f64, e: f64, basis: i64) -> R<f64> {
        let (s, e) = if s > e { (e.floor(), s.floor()) } else { (s.floor(), e.floor()) };
        let (y1, m1, d1) = serial_to_ymd(s, self.d1904)?;
        let (y2, m2, d2) = serial_to_ymd(e, self.d1904)?;
        let days360 = |d1: i64, d2: i64| ((y2 - y1) * 360 + (m2 as i64 - m1 as i64) * 30 + (d2 - d1)) as f64 / 360.0;
        Ok(match basis {
            0 => {
                let (mut a, mut b) = (d1 as i64, d2 as i64);
                let lf1 = m1 == 2 && d1 == days_in_month(y1, 2);
                let lf2 = m2 == 2 && d2 == days_in_month(y2, 2);
                if lf1 && lf2 {
                    b = 30;
                }
                if lf1 {
                    a = 30;
                }
                if b == 31 && a >= 30 {
                    b = 30;
                }
                if a == 31 {
                    a = 30;
                }
                days360(a, b)
            }
            1 => {
                let den = if y1 == y2 {
                    if is_leap(y1) { 366.0 } else { 365.0 }
                } else if y2 == y1 + 1 && (m1 > m2 || (m1 == m2 && d1 >= d2)) {
                    if (is_leap(y1) && m1 < 3) || (is_leap(y2) && (m2 > 2 || (m2 == 2 && d2 == 29))) { 366.0 } else { 365.0 }
                } else {
                    let total: f64 = (y1..=y2).map(|y| if is_leap(y) { 366.0 } else { 365.0 }).sum();
                    total / (y2 - y1 + 1) as f64
                };
                (e - s) / den
            }
            2 => (e - s) / 360.0,
            3 => (e - s) / 365.0,
            4 => days360((d1 as i64).min(30), (d2 as i64).min(30)),
            _ => return Err(ErrorKind::Num),
        })
    }

    pub(crate) fn date_scalar(&self, f: Func, a: &[Value]) -> Option<Value> {
        use Func::*;
        let d1904 = self.d1904;
        let r = (|| -> R<Option<Value>> {
            Ok(Some(match f {
                Date => num(date_serial(an(a, 0)?, an(a, 1)?, an(a, 2)?, d1904)?),
                DateValue | TimeValue => {
                    let t = match a.first() {
                        Some(Value::Text(t)) => t.clone(),
                        Some(Value::Error(e)) => return Err(*e),
                        _ => return Err(ErrorKind::Value),
                    };
                    let v = parse_datetime(&t, d1904).ok_or(ErrorKind::Value)?;
                    num(if f == DateValue { v.floor() } else { v - v.floor() })
                }
                Time => {
                    let total = an(a, 0)?.trunc() * 3600.0 + an(a, 1)?.trunc() * 60.0 + an(a, 2)?.trunc();
                    if total < 0.0 {
                        return Err(ErrorKind::Num);
                    }
                    num((total % 86400.0) / 86400.0)
                }
                Today => num(self.grid.now().floor()),
                Now => num(self.grid.now()),
                Year => num(self.ymd(a.first().unwrap_or(&Value::Empty))?.0 as f64),
                Month => num(self.ymd(a.first().unwrap_or(&Value::Empty))?.1 as f64),
                Day => num(self.ymd(a.first().unwrap_or(&Value::Empty))?.2 as f64),
                Hour | Minute | Second => {
                    let (h, m, s) = time_parts(an(a, 0)?)?;
                    num(match f {
                        Hour => h,
                        Minute => m,
                        _ => s,
                    } as f64)
                }
                Weekday => {
                    let s = an(a, 0)?;
                    serial_to_ymd(s, d1904)?;
                    let ty = ao(a, 1, 1.0)?.trunc() as i64;
                    let sun0 = weekday_sun0(s, d1904);
                    num(match ty {
                        1 | 17 => sun0 + 1,
                        2 | 11 => (sun0 + 6) % 7 + 1,
                        3 => (sun0 + 6) % 7,
                        12..=16 => (sun0 - (ty - 10)).rem_euclid(7) + 1,
                        _ => return Err(ErrorKind::Num),
                    } as f64)
                }
                WeekNum => num(self.weeknum(an(a, 0)?, ao(a, 1, 1.0)?.trunc() as i64)?),
                IsoWeekNum => num(self.isoweek(an(a, 0)?)?),
                EDate => num(self.add_months(a.first().unwrap_or(&Value::Empty), an(a, 1)?, false)?),
                EOMonth => num(self.add_months(a.first().unwrap_or(&Value::Empty), an(a, 1)?, true)?),
                DateDif => num(self.datedif(an(a, 0)?, an(a, 1)?, &at(a, 2)?)?),
                Days => {
                    let (e, s) = (an(a, 0)?, an(a, 1)?);
                    serial_to_ymd(e, d1904)?;
                    serial_to_ymd(s, d1904)?;
                    num(e.floor() - s.floor())
                }
                YearFrac => num(self.yearfrac(an(a, 0)?, an(a, 1)?, ao(a, 2, 0.0)?.trunc() as i64)?),
                _ => return Ok(None),
            }))
        })();
        match r {
            Ok(v) => v,
            Err(e) => Some(Value::Error(e)),
        }
    }

    fn holidays(&self, e: Option<&Expr>) -> R<HashSet<i64>> {
        let mut set = HashSet::new();
        if let Some(e) = e {
            self.visit(std::slice::from_ref(e), &mut |v, _| {
                match v {
                    Value::Empty => {}
                    Value::Error(e) => return Err(*e),
                    v => {
                        set.insert(to_num(v)?.floor() as i64);
                    }
                }
                Ok(())
            })?;
        }
        Ok(set)
    }

    pub(crate) fn date_special(&self, f: Func, args: &[Expr]) -> Option<Value> {
        use Func::*;
        let d1904 = self.d1904;
        let r = (|| -> R<Option<Value>> {
            let (intl, hol_idx) = match f {
                NetworkDays | Workday => (false, 2),
                NetworkDaysIntl | WorkdayIntl => (true, 3),
                _ => return Ok(None),
            };
            let a = to_num(&self.scalar(args.first().ok_or(ErrorKind::Value)?))?.floor();
            let b = to_num(&self.scalar(args.get(1).ok_or(ErrorKind::Value)?))?;
            serial_to_ymd(a, d1904)?;
            let mask = if intl {
                match args.get(2) {
                    Some(e) => weekend_mask(&self.scalar(e))?,
                    None => weekend_mask(&Value::Empty)?,
                }
            } else {
                weekend_mask(&Value::Empty)?
            };
            let hol = self.holidays(args.get(hol_idx))?;
            let is_work = |s: i64| !mask[weekday_mon0(s as f64, d1904)] && !hol.contains(&s);
            Ok(Some(match f {
                NetworkDays | NetworkDaysIntl => {
                    let b = b.floor();
                    serial_to_ymd(b, d1904)?;
                    let (lo, hi, sign) = if a <= b { (a as i64, b as i64, 1.0) } else { (b as i64, a as i64, -1.0) };
                    let total = hi - lo + 1;
                    let per_week = mask.iter().filter(|w| !**w).count() as i64;
                    let mut n = (total / 7) * per_week;
                    for s in (lo + (total / 7) * 7)..=hi {
                        if !mask[weekday_mon0(s as f64, d1904)] {
                            n += 1;
                        }
                    }
                    for h in &hol {
                        if (lo..=hi).contains(h) && !mask[weekday_mon0(*h as f64, d1904)] {
                            n -= 1;
                        }
                    }
                    num(sign * n as f64)
                }
                _ => {
                    let days = b.trunc() as i64;
                    let step = if days >= 0 { 1 } else { -1 };
                    let mut s = a as i64;
                    let mut left = days.abs();
                    let max = max_serial(d1904);
                    while left > 0 {
                        s += step;
                        if s < 0 || s > max {
                            return Err(ErrorKind::Num);
                        }
                        if is_work(s) {
                            left -= 1;
                        }
                    }
                    num(s as f64)
                }
            }))
        })();
        match r {
            Ok(v) => v,
            Err(e) => Some(Value::Error(e)),
        }
    }
}
