//! Decimal rounding and a compact number-format engine for `TEXT`.

use super::dates;
use super::value::*;

/// Decimal digits of |x| rounded half-away-from-zero at `dec` decimals,
/// based on the 15-significant-digit representation (as Excel does).
/// Returns (digits, point) meaning `0.DIGITS × 10^point`; empty = zero.
fn round_digits(x: f64, dec: i32) -> (Vec<u8>, i32) {
    if x == 0.0 || !x.is_finite() {
        return (Vec::new(), 0);
    }
    let s = format!("{:.14e}", x.abs());
    let (mant, exp) = s.split_once('e').unwrap_or((&s, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let mut d: Vec<u8> = mant.bytes().filter(u8::is_ascii_digit).map(|b| b - b'0').collect();
    let mut point = exp + 1;
    let keep = point.saturating_add(dec);
    if keep < 0 {
        return (Vec::new(), 0);
    }
    let keep = keep as usize;
    if keep < d.len() {
        let up = d[keep] >= 5;
        d.truncate(keep);
        if up {
            let mut k = keep as isize - 1;
            loop {
                if k < 0 {
                    d.insert(0, 1);
                    point += 1;
                    break;
                }
                if d[k as usize] == 9 {
                    d[k as usize] = 0;
                    k -= 1;
                } else {
                    d[k as usize] += 1;
                    break;
                }
            }
        }
    }
    while d.last() == Some(&0) {
        d.pop();
    }
    if d.is_empty() {
        return (d, 0);
    }
    (d, point)
}

/// Excel `ROUND`: half away from zero on the decimal representation.
pub(crate) fn round_half_away(x: f64, dec: i32) -> f64 {
    let (d, point) = round_digits(x, dec);
    if d.is_empty() {
        return 0.0;
    }
    let s: String = d.iter().map(|b| (b + b'0') as char).collect();
    let v: f64 = format!("0.{s}e{point}").parse().unwrap_or(0.0);
    if x < 0.0 { -v } else { v }
}

/// Integer and fraction digit strings of |x| rounded to `dec` decimals.
pub(crate) fn fixed_parts(x: f64, dec: usize) -> (String, String) {
    let (d, point) = round_digits(x, dec as i32);
    let mut int = String::new();
    let mut frac = String::new();
    if !d.is_empty() {
        if point > 0 {
            for k in 0..point as usize {
                int.push(d.get(k).map_or('0', |b| (b + b'0') as char));
            }
        }
        for k in 0..dec {
            let idx = point as i64 + k as i64;
            frac.push(if idx < 0 { '0' } else { d.get(idx as usize).map_or('0', |b| (b + b'0') as char) });
        }
    } else {
        frac = "0".repeat(dec);
    }
    (int, frac)
}

pub(crate) fn group_thousands(int: &str) -> String {
    let n = int.len();
    let mut out = String::with_capacity(n + n / 3);
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (n - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

// ---------------- format codes ----------------

#[derive(Clone, Debug, PartialEq)]
enum F {
    Lit(String),
    Digit(char),
    Dot,
    Comma,
    Pct,
    Exp(bool),
    At,
    General,
    Year(usize),
    Month(usize),
    Minute(usize),
    Day(usize),
    Hour(usize, bool),
    MinElapsed,
    Sec(usize, bool),
    SubSec(usize),
    AmPm(bool),
}

fn split_sections(fmt: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut it = fmt.chars().peekable();
    let mut q = false;
    while let Some(c) = it.next() {
        match c {
            '"' => {
                q = !q;
                out.last_mut().unwrap().push(c);
            }
            '\\' if !q => {
                out.last_mut().unwrap().push(c);
                if let Some(n) = it.next() {
                    out.last_mut().unwrap().push(n);
                }
            }
            ';' if !q => out.push(String::new()),
            c => out.last_mut().unwrap().push(c),
        }
    }
    out
}

fn tokenize(sec: &str) -> Vec<F> {
    let ch: Vec<char> = sec.chars().collect();
    let mut t = Vec::new();
    let mut i = 0;
    let run = |i: usize, c: char| {
        let mut j = i;
        while j < ch.len() && ch[j].eq_ignore_ascii_case(&c) {
            j += 1;
        }
        j - i
    };
    let lit = |t: &mut Vec<F>, s: &str| {
        if let Some(F::Lit(l)) = t.last_mut() {
            l.push_str(s);
        } else {
            t.push(F::Lit(s.to_string()));
        }
    };
    while i < ch.len() {
        let c = ch[i];
        let rest: String = ch[i..].iter().take(7).collect::<String>().to_ascii_lowercase();
        match c {
            '"' => {
                let mut j = i + 1;
                let mut s = String::new();
                while j < ch.len() && ch[j] != '"' {
                    s.push(ch[j]);
                    j += 1;
                }
                lit(&mut t, &s);
                i = j + 1;
            }
            '\\' => {
                if let Some(n) = ch.get(i + 1) {
                    lit(&mut t, &n.to_string());
                }
                i += 2;
            }
            '_' => {
                lit(&mut t, " ");
                i += 2;
            }
            '*' => i += 2,
            '[' => {
                let mut j = i + 1;
                let mut s = String::new();
                while j < ch.len() && ch[j] != ']' {
                    s.push(ch[j]);
                    j += 1;
                }
                i = j + 1;
                let l = s.to_ascii_lowercase();
                if !l.is_empty() && l.chars().all(|c| c == 'h') {
                    t.push(F::Hour(l.len(), true));
                } else if !l.is_empty() && l.chars().all(|c| c == 'm') {
                    t.push(F::MinElapsed);
                } else if !l.is_empty() && l.chars().all(|c| c == 's') {
                    t.push(F::Sec(l.len(), true));
                } else if let Some(cur) = s.strip_prefix('$') {
                    let sym = cur.split('-').next().unwrap_or("");
                    lit(&mut t, sym);
                }
            }
            '0' | '#' | '?' => {
                t.push(F::Digit(c));
                i += 1;
            }
            '.' => {
                if matches!(t.iter().rev().find(|x| !matches!(x, F::Lit(_))), Some(F::Sec(..))) && ch.get(i + 1) == Some(&'0') {
                    let n = run(i + 1, '0');
                    t.push(F::SubSec(n));
                    i += 1 + n;
                } else {
                    t.push(F::Dot);
                    i += 1;
                }
            }
            ',' => {
                t.push(F::Comma);
                i += 1;
            }
            '%' => {
                t.push(F::Pct);
                i += 1;
            }
            '@' => {
                t.push(F::At);
                i += 1;
            }
            'E' | 'e' if matches!(ch.get(i + 1), Some('+' | '-')) => {
                t.push(F::Exp(ch[i + 1] == '+'));
                i += 2;
            }
            _ if rest.starts_with("general") => {
                t.push(F::General);
                i += 7;
            }
            _ if rest.starts_with("am/pm") => {
                t.push(F::AmPm(true));
                i += 5;
            }
            _ if rest.starts_with("a/p") => {
                t.push(F::AmPm(false));
                i += 3;
            }
            'y' | 'Y' => {
                let n = run(i, 'y');
                t.push(F::Year(n));
                i += n;
            }
            'm' | 'M' => {
                let n = run(i, 'm');
                t.push(F::Month(n));
                i += n;
            }
            'd' | 'D' => {
                let n = run(i, 'd');
                t.push(F::Day(n));
                i += n;
            }
            'h' | 'H' => {
                let n = run(i, 'h');
                t.push(F::Hour(n, false));
                i += n;
            }
            's' | 'S' => {
                let n = run(i, 's');
                t.push(F::Sec(n, false));
                i += n;
            }
            c => {
                lit(&mut t, &c.to_string());
                i += 1;
            }
        }
    }
    // m after h / before s means minutes
    let idx: Vec<usize> = (0..t.len()).filter(|&k| !matches!(t[k], F::Lit(_))).collect();
    for (p, &k) in idx.iter().enumerate() {
        if let F::Month(n) = t[k]
            && n <= 2
        {
            let prev_h = p > 0 && matches!(t[idx[p - 1]], F::Hour(..));
            let next_s = idx.get(p + 1).is_some_and(|&q| matches!(t[q], F::Sec(..)));
            if prev_h || next_s {
                t[k] = F::Minute(n);
            }
        }
    }
    t
}

fn is_date_fmt(t: &[F]) -> bool {
    t.iter()
        .any(|x| matches!(x, F::Year(_) | F::Month(_) | F::Minute(_) | F::Day(_) | F::Hour(..) | F::MinElapsed | F::Sec(..) | F::AmPm(_)))
}

fn format_date(x: f64, t: &[F], d1904: bool) -> R<String> {
    if x < 0.0 {
        return Err(ErrorKind::Value);
    }
    let sub = t.iter().find_map(|f| if let F::SubSec(n) = f { Some(*n) } else { None }).unwrap_or(0);
    let scale = 10f64.powi(sub as i32);
    let total = (x * 86400.0 * scale).round() / scale; // seconds since epoch
    let days = (total / 86400.0).floor();
    let secs = total - days * 86400.0;
    let (y, mo, d) = dates::serial_to_ymd(days, d1904)?;
    let whole = secs.floor() as i64;
    let (h, mi, s) = (whole / 3600, (whole / 60) % 60, whole % 60);
    let frac = secs - secs.floor();
    let ampm = t.iter().any(|f| matches!(f, F::AmPm(_)));
    let mut out = String::new();
    for f in t {
        match f {
            F::Lit(s) => out.push_str(s),
            F::Year(n) => {
                if *n <= 2 {
                    out.push_str(&format!("{:02}", y % 100))
                } else {
                    out.push_str(&format!("{:04}", y))
                }
            }
            F::Month(n) => match n {
                1 => out.push_str(&mo.to_string()),
                2 => out.push_str(&format!("{:02}", mo)),
                3 => out.push_str(&dates::month_name(mo)[..3]),
                5 => out.push_str(&dates::month_name(mo)[..1]),
                _ => out.push_str(dates::month_name(mo)),
            },
            F::Day(n) => match n {
                1 => out.push_str(&d.to_string()),
                2 => out.push_str(&format!("{:02}", d)),
                3 => out.push_str(&dates::day_name(dates::weekday_sun0(days, d1904) as usize)[..3]),
                _ => out.push_str(dates::day_name(dates::weekday_sun0(days, d1904) as usize)),
            },
            F::Hour(n, elapsed) => {
                let hv = if *elapsed {
                    (total / 3600.0).floor() as i64
                } else if ampm {
                    let h12 = h % 12;
                    if h12 == 0 { 12 } else { h12 }
                } else {
                    h
                };
                out.push_str(&if *n >= 2 { format!("{:02}", hv) } else { hv.to_string() });
            }
            F::Minute(n) => out.push_str(&if *n >= 2 { format!("{:02}", mi) } else { mi.to_string() }),
            F::MinElapsed => out.push_str(&((total / 60.0).floor() as i64).to_string()),
            F::Sec(n, elapsed) => {
                let sv = if *elapsed { total.floor() as i64 } else { s };
                out.push_str(&if *n >= 2 { format!("{:02}", sv) } else { sv.to_string() });
            }
            F::SubSec(n) => {
                let v = (frac * 10f64.powi(*n as i32)).round() as i64;
                out.push('.');
                out.push_str(&format!("{:0width$}", v, width = *n));
            }
            F::AmPm(full) => {
                let pm = h >= 12;
                out.push_str(match (full, pm) {
                    (true, false) => "AM",
                    (true, true) => "PM",
                    (false, false) => "A",
                    (false, true) => "P",
                });
            }
            F::Digit(c) => out.push(*c),
            F::Dot => out.push('.'),
            F::Comma => out.push(','),
            F::Pct => out.push('%'),
            F::At | F::General => out.push_str(&num_to_text(x)),
            F::Exp(_) => {}
        }
    }
    Ok(out)
}

fn format_num_tokens(x: f64, t: &[F]) -> String {
    // x is non-negative here
    let exp_at = t.iter().position(|f| matches!(f, F::Exp(_)));
    let dot_at = t.iter().position(|f| *f == F::Dot);
    let mant_end = exp_at.unwrap_or(t.len());
    let int_end = dot_at.filter(|&d| d < mant_end).unwrap_or(mant_end);
    let int_ph: Vec<usize> = (0..int_end).filter(|&k| matches!(t[k], F::Digit(_))).collect();
    let frac_ph: Vec<usize> = (int_end..mant_end).filter(|&k| matches!(t[k], F::Digit(_))).collect();
    let exp_ph: usize = exp_at.map_or(0, |e| t[e..].iter().filter(|f| matches!(f, F::Digit(_))).count());
    let last_digit = int_ph.last().copied();
    let first_digit = int_ph.first().copied();
    // grouping comma: between integer placeholders; scaling commas: right after last digit
    let mut grouping = false;
    let mut scale = 0;
    for (k, tok) in t.iter().enumerate().take(int_end) {
        if *tok == F::Comma {
            match (first_digit, last_digit) {
                (Some(f), Some(l)) if k > f && k < l => grouping = true,
                (_, Some(l)) if k > l => scale += 1,
                _ => {}
            }
        }
    }
    if let Some(l) = frac_ph.last() {
        scale += t[l + 1..mant_end].iter().take_while(|f| **f == F::Comma).count();
    }
    let pct = t.iter().filter(|f| **f == F::Pct).count();
    let mut v = x * 100f64.powi(pct as i32) / 1000f64.powi(scale as i32);
    let mut e = 0i32;
    if exp_at.is_some() && v != 0.0 {
        let ip = int_ph.len().max(1) as i32;
        e = v.log10().floor() as i32 - (ip - 1);
        v /= 10f64.powi(e);
        let (i, _) = fixed_parts(v, frac_ph.len());
        if i.len() as i32 > ip {
            e += 1;
            v /= 10.0;
        }
    }
    let (mut int, frac) = fixed_parts(v, frac_ph.len());
    if int.is_empty() {
        int.clear();
    }
    // trim optional trailing fraction digits
    let mut frac_chars: Vec<char> = frac.chars().collect();
    for (k, &p) in frac_ph.iter().enumerate().rev() {
        if frac_chars[k] != '0' {
            break;
        }
        match t[p] {
            F::Digit('#') => frac_chars[k] = '\0',
            F::Digit('?') => frac_chars[k] = ' ',
            _ => break,
        }
    }
    let ib: Vec<char> = int.chars().collect();
    let np = int_ph.len();
    let mut out = String::new();
    let mut fi = 0;
    for (k, f) in t.iter().enumerate() {
        match f {
            F::Digit(c) if k < int_end => {
                let pk = int_ph.iter().position(|&p| p == k).unwrap_or(0);
                let emit = |out: &mut String, ch: char, q: usize| {
                    out.push(ch);
                    if grouping && q > 0 && q.is_multiple_of(3) && ch != ' ' {
                        out.push(',');
                    }
                };
                if pk == 0 && ib.len() > np {
                    for (idx, ch) in ib[..ib.len() - np + 1].iter().enumerate() {
                        emit(&mut out, *ch, ib.len() - 1 - idx);
                    }
                } else {
                    let q = np - 1 - pk; // position from right
                    if q < ib.len() {
                        emit(&mut out, ib[ib.len() - 1 - q], q);
                    } else {
                        match c {
                            '0' => emit(&mut out, '0', q),
                            '?' => out.push(' '),
                            _ => {}
                        }
                    }
                }
            }
            F::Digit(_) if k < mant_end => {
                if let Some(&ch) = frac_chars.get(fi)
                    && ch != '\0'
                {
                    out.push(ch);
                }
                fi += 1;
            }
            F::Digit(_) => {}
            F::Dot => {
                if np == 0 {
                    out.push_str(&int);
                }
                out.push('.');
            }
            F::Comma => {}
            F::Pct => out.push('%'),
            F::Exp(plus) => {
                out.push('E');
                if e < 0 {
                    out.push('-');
                } else if *plus {
                    out.push('+');
                }
                out.push_str(&format!("{:0width$}", e.abs(), width = exp_ph.max(1)));
            }
            F::Lit(s) => out.push_str(s),
            F::At | F::General => out.push_str(&num_to_text(x)),
            _ => {}
        }
    }
    out
}

/// Formats a number with an Excel format code.
pub(crate) fn format_number(x: f64, fmt: &str, d1904: bool) -> R<String> {
    let secs = split_sections(fmt);
    let (sec, neg_prefix, v) = if x < 0.0 {
        if secs.len() >= 2 && !secs[1].is_empty() { (&secs[1], false, -x) } else { (&secs[0], true, -x) }
    } else if x == 0.0 && secs.len() >= 3 {
        (&secs[2], false, x)
    } else {
        (&secs[0], false, x)
    };
    let t = tokenize(sec);
    if t.is_empty() {
        return Ok(String::new());
    }
    if t.iter().all(|f| matches!(f, F::General | F::Lit(_))) && t.contains(&F::General) {
        let s = num_to_text(v);
        let body: String = t.iter().map(|f| if let F::Lit(l) = f { l.clone() } else { s.clone() }).collect::<String>();
        return Ok(if neg_prefix { format!("-{body}") } else { body });
    }
    let body = if is_date_fmt(&t) { format_date(if neg_prefix { x } else { v }, &t, d1904)? } else { format_num_tokens(v, &t) };
    let has_digits = body.bytes().any(|b| b.is_ascii_digit() && b != b'0');
    Ok(if neg_prefix && (has_digits || !is_date_fmt(&t)) { format!("-{body}") } else { body })
}

/// Formats text with the text section (4th) or an `@` section.
pub(crate) fn format_string(s: &str, fmt: &str) -> String {
    let secs = split_sections(fmt);
    let sec = if secs.len() >= 4 {
        &secs[3]
    } else if secs.len() == 1 && secs[0].contains('@') {
        &secs[0]
    } else {
        return s.to_string();
    };
    tokenize(sec)
        .iter()
        .map(|f| match f {
            F::At => s.to_string(),
            F::Lit(l) => l.clone(),
            _ => String::new(),
        })
        .collect()
}
