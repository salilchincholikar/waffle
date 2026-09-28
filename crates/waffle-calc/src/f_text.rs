//! Text functions.

use super::criteria::Pattern;
use super::eval::Ctx;
use super::format::{fixed_parts, format_number, format_string, group_thousands, round_half_away};
use super::funcs::Func;
use super::parser::Expr;
use super::value::*;

const MAX_TEXT: usize = 32767;

/// Windows-1252 code points 128..=159.
const CP1252: [u32; 32] = [
    0x20AC, 0x81, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0x8D, 0x017D, 0x8F, 0x90, 0x2018,
    0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0x9D, 0x017E, 0x0178,
];

fn chars(s: &str) -> Vec<char> {
    s.chars().collect()
}

fn text_val(s: String) -> R<Value> {
    if s.chars().count() > MAX_TEXT {
        return Err(ErrorKind::Value);
    }
    Ok(Value::from(s))
}

fn money(n: f64, dec: f64, commas: bool, dollar: bool) -> R<Value> {
    let dec = dec.trunc().clamp(-127.0, 127.0) as i32;
    let r = round_half_away(n, dec);
    let (int, frac) = fixed_parts(r.abs(), dec.max(0) as usize);
    let int = if int.is_empty() { "0".to_string() } else { int };
    let mut body = if commas { group_thousands(&int) } else { int };
    if dec > 0 {
        body.push('.');
        body.push_str(&frac);
    }
    let neg = r < 0.0;
    Ok(Value::from(match (dollar, neg) {
        (true, true) => format!("(${body})"),
        (true, false) => format!("${body}"),
        (false, true) => format!("-{body}"),
        (false, false) => body,
    }))
}

impl Ctx<'_> {
    pub(crate) fn text_scalar(&self, f: Func, a: &[Value]) -> Option<Value> {
        use Func::*;
        let r = (|| -> R<Option<Value>> {
            Ok(Some(match f {
                Concatenate => {
                    let mut s = String::new();
                    for v in a {
                        s.push_str(&to_text(v)?);
                    }
                    text_val(s)?
                }
                Left | Right => {
                    let s = at(a, 0)?;
                    let n = ao(a, 1, 1.0)?;
                    if n < 0.0 {
                        return Err(ErrorKind::Value);
                    }
                    let c = chars(&s);
                    let n = (n.trunc() as usize).min(c.len());
                    let out: String = if f == Left { c[..n].iter().collect() } else { c[c.len() - n..].iter().collect() };
                    Value::from(out)
                }
                Mid => {
                    let s = at(a, 0)?;
                    let st = an(a, 1)?.trunc();
                    let n = an(a, 2)?.trunc();
                    if st < 1.0 || n < 0.0 {
                        return Err(ErrorKind::Value);
                    }
                    let c = chars(&s);
                    let st = (st as usize - 1).min(c.len());
                    let en = (st + n as usize).min(c.len());
                    Value::from(c[st..en].iter().collect::<String>())
                }
                Len => num(at(a, 0)?.chars().count() as f64),
                Lower => Value::from(at(a, 0)?.to_lowercase()),
                Upper => Value::from(at(a, 0)?.to_uppercase()),
                Proper => {
                    let mut out = String::new();
                    let mut prev_letter = false;
                    for ch in at(a, 0)?.chars() {
                        if ch.is_alphabetic() {
                            if prev_letter {
                                out.extend(ch.to_lowercase());
                            } else {
                                out.extend(ch.to_uppercase());
                            }
                            prev_letter = true;
                        } else {
                            out.push(ch);
                            prev_letter = false;
                        }
                    }
                    Value::from(out)
                }
                Trim => Value::from(at(a, 0)?.split(' ').filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")),
                Clean => Value::from(at(a, 0)?.chars().filter(|c| (*c as u32) >= 32).collect::<String>()),
                Substitute => {
                    let s = at(a, 0)?;
                    let old = at(a, 1)?;
                    let new = at(a, 2)?;
                    if old.is_empty() {
                        return Ok(Some(Value::from(s.into_owned())));
                    }
                    match a.get(3) {
                        None => text_val(s.replace(&*old, &new))?,
                        Some(v) => {
                            let k = to_num(v)?.trunc();
                            if k < 1.0 {
                                return Err(ErrorKind::Value);
                            }
                            let k = k as usize;
                            match s.match_indices(&*old).nth(k - 1) {
                                Some((p, _)) => {
                                    let mut out = String::with_capacity(s.len());
                                    out.push_str(&s[..p]);
                                    out.push_str(&new);
                                    out.push_str(&s[p + old.len()..]);
                                    text_val(out)?
                                }
                                None => Value::from(s.into_owned()),
                            }
                        }
                    }
                }
                Replace => {
                    let s = chars(&at(a, 0)?);
                    let st = an(a, 1)?.trunc();
                    let n = an(a, 2)?.trunc();
                    let new = at(a, 3)?;
                    if st < 1.0 || n < 0.0 {
                        return Err(ErrorKind::Value);
                    }
                    let st = (st as usize - 1).min(s.len());
                    let en = (st + n as usize).min(s.len());
                    let mut out: String = s[..st].iter().collect();
                    out.push_str(&new);
                    out.extend(&s[en..]);
                    text_val(out)?
                }
                Find | Search => {
                    let needle = at(a, 0)?;
                    let hay = chars(&at(a, 1)?);
                    let st = ao(a, 2, 1.0)?.trunc();
                    if st < 1.0 || st as usize > hay.len() + 1 {
                        return Err(ErrorKind::Value);
                    }
                    let st = st as usize - 1;
                    if needle.is_empty() {
                        return Ok(Some(num(st as f64 + 1.0)));
                    }
                    let pos = if f == Find {
                        let n = chars(&needle);
                        (st..hay.len()).find(|&i| hay[i..].starts_with(&n))
                    } else {
                        let p = Pattern::prefix(&needle);
                        let low: Vec<char> = hay.iter().map(|c| c.to_lowercase().next().unwrap_or(*c)).collect();
                        (st..low.len()).find(|&i| p.matches_chars(&low[i..]))
                    };
                    num(pos.ok_or(ErrorKind::Value)? as f64 + 1.0)
                }
                Exact => Value::Bool(at(a, 0)? == at(a, 1)?),
                Rept => {
                    let s = at(a, 0)?;
                    let n = an(a, 1)?.trunc();
                    if n < 0.0 || s.chars().count() as f64 * n > MAX_TEXT as f64 {
                        return Err(ErrorKind::Value);
                    }
                    Value::from(s.repeat(n as usize))
                }
                ValueFn => match a.first() {
                    Some(Value::Number(n)) => num(*n),
                    Some(Value::Empty) | None => num(0.0),
                    Some(Value::Text(t)) => num(parse_num_text(t).ok_or(ErrorKind::Value)?),
                    Some(Value::Error(e)) => return Err(*e),
                    _ => return Err(ErrorKind::Value),
                },
                NumberValue => {
                    let s = at(a, 0)?;
                    let dec = match a.get(1) {
                        Some(v) => to_text(v)?.chars().next().ok_or(ErrorKind::Value)?,
                        None => '.',
                    };
                    let grp: Vec<char> = match a.get(2) {
                        Some(v) => to_text(v)?.chars().collect(),
                        None => vec![','],
                    };
                    let mut t: String = s.chars().filter(|c| !c.is_whitespace()).collect();
                    if t.is_empty() {
                        return Ok(Some(num(0.0)));
                    }
                    let mut pct = 0;
                    while let Some(r) = t.strip_suffix('%') {
                        t = r.to_string();
                        pct += 1;
                    }
                    let dpos = t.find(dec);
                    let mut out = String::new();
                    for (i, c) in t.char_indices() {
                        if Some(i) == dpos {
                            out.push('.');
                        } else if grp.contains(&c) && dpos.is_none_or(|d| i < d) {
                            continue;
                        } else {
                            out.push(c);
                        }
                    }
                    let n = parse_plain_number(&out).ok_or(ErrorKind::Value)?;
                    num(n / 100f64.powi(pct))
                }
                Text => {
                    let fmt = at(a, 1)?;
                    match a.first().unwrap_or(&Value::Empty) {
                        Value::Error(e) => return Err(*e),
                        Value::Bool(b) => Value::from(if *b { "TRUE" } else { "FALSE" }),
                        Value::Text(t) => match parse_num_text(t) {
                            Some(n) => Value::from(format_number(n, &fmt, self.d1904)?),
                            None => Value::from(format_string(t, &fmt)),
                        },
                        v => Value::from(format_number(to_num(v)?, &fmt, self.d1904)?),
                    }
                }
                Char => {
                    let n = an(a, 0)?.trunc();
                    if !(1.0..=255.0).contains(&n) {
                        return Err(ErrorKind::Value);
                    }
                    let n = n as u32;
                    let cp = if (128..160).contains(&n) { CP1252[(n - 128) as usize] } else { n };
                    Value::from(char::from_u32(cp).unwrap_or('?').to_string())
                }
                Code | Unicode => {
                    let s = at(a, 0)?;
                    let c = s.chars().next().ok_or(ErrorKind::Value)? as u32;
                    if f == Unicode || c < 128 || (160..256).contains(&c) {
                        num(c as f64)
                    } else {
                        num(CP1252.iter().position(|&x| x == c).map_or(63.0, |p| (p + 128) as f64))
                    }
                }
                Unichar => {
                    let n = an(a, 0)?.trunc();
                    if n < 1.0 {
                        return Err(ErrorKind::Value);
                    }
                    Value::from(char::from_u32(n as u32).ok_or(ErrorKind::Value)?.to_string())
                }
                T => match a.first() {
                    Some(Value::Text(t)) => Value::Text(t.clone()),
                    Some(Value::Error(e)) => return Err(*e),
                    _ => Value::from(""),
                },
                N => match a.first() {
                    Some(Value::Number(n)) => num(*n),
                    Some(Value::Bool(b)) => num(if *b { 1.0 } else { 0.0 }),
                    Some(Value::Error(e)) => return Err(*e),
                    _ => num(0.0),
                },
                Dollar => money(an(a, 0)?, ao(a, 1, 2.0)?, true, true)?,
                Fixed => money(an(a, 0)?, ao(a, 1, 2.0)?, !ab(a, 2, false)?, false)?,
                _ => return Ok(None),
            }))
        })();
        match r {
            Ok(v) => v,
            Err(e) => Some(Value::Error(e)),
        }
    }

    pub(crate) fn text_special(&self, f: Func, args: &[Expr]) -> Option<Value> {
        use Func::*;
        let r = (|| -> R<Option<Value>> {
            Ok(Some(match f {
                Concat => {
                    let mut s = String::new();
                    self.visit(args, &mut |v, _| {
                        s.push_str(&to_text(v)?);
                        if s.len() > MAX_TEXT * 4 {
                            return Err(ErrorKind::Value);
                        }
                        Ok(())
                    })?;
                    text_val(s)?
                }
                TextJoin => {
                    let delim = self.text(args, 0)?;
                    let ignore = self.opt_bool(args, 1, true)?;
                    let mut s = String::new();
                    let mut first = true;
                    self.visit(args.get(2..).unwrap_or(&[]), &mut |v, _| {
                        let t = to_text(v)?;
                        if ignore && t.is_empty() {
                            return Ok(());
                        }
                        if !first {
                            s.push_str(&delim);
                        }
                        first = false;
                        s.push_str(&t);
                        if s.len() > MAX_TEXT * 4 {
                            return Err(ErrorKind::Value);
                        }
                        Ok(())
                    })?;
                    text_val(s)?
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
