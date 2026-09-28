//! Values, error kinds and Excel coercion / comparison rules.

use std::borrow::Cow;
use std::cell::Cell;
use std::cmp::Ordering;
use std::rc::Rc;

/// A cell or formula value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Empty,
    Number(f64),
    Text(Rc<str>),
    Bool(bool),
    Error(ErrorKind),
    /// Row-major 2-D array (`rows[r][c]`); never empty, all rows same length.
    Array(Rc<Vec<Vec<Value>>>),
}

/// Excel error values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    Null,
    Div0,
    Value,
    Ref,
    Name,
    Num,
    NA,
    GettingData,
    Spill,
    Calc,
}

impl ErrorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorKind::Null => "#NULL!",
            ErrorKind::Div0 => "#DIV/0!",
            ErrorKind::Value => "#VALUE!",
            ErrorKind::Ref => "#REF!",
            ErrorKind::Name => "#NAME?",
            ErrorKind::Num => "#NUM!",
            ErrorKind::NA => "#N/A",
            ErrorKind::GettingData => "#GETTING_DATA",
            ErrorKind::Spill => "#SPILL!",
            ErrorKind::Calc => "#CALC!",
        }
    }

    /// Parses an error literal such as `#N/A` (case-insensitive).
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        const ALL: [ErrorKind; 10] = [
            ErrorKind::Null,
            ErrorKind::Div0,
            ErrorKind::Value,
            ErrorKind::Ref,
            ErrorKind::Name,
            ErrorKind::Num,
            ErrorKind::NA,
            ErrorKind::GettingData,
            ErrorKind::Spill,
            ErrorKind::Calc,
        ];
        let s = s.trim();
        ALL.into_iter().find(|e| e.as_str().eq_ignore_ascii_case(s))
    }

    /// Code returned by `ERROR.TYPE`.
    pub fn code(self) -> f64 {
        match self {
            ErrorKind::Null => 1.0,
            ErrorKind::Div0 => 2.0,
            ErrorKind::Value => 3.0,
            ErrorKind::Ref => 4.0,
            ErrorKind::Name => 5.0,
            ErrorKind::Num => 6.0,
            ErrorKind::NA => 7.0,
            ErrorKind::GettingData => 8.0,
            ErrorKind::Spill => 9.0,
            ErrorKind::Calc => 14.0,
        }
    }
}

impl std::fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<ErrorKind> for Value {
    fn from(e: ErrorKind) -> Self {
        Value::Error(e)
    }
}
impl From<f64> for Value {
    fn from(n: f64) -> Self {
        num(n)
    }
}
impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Bool(b)
    }
}
impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::Text(Rc::from(s))
    }
}
impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::Text(Rc::from(s))
    }
}

impl Value {
    pub fn text(s: impl AsRef<str>) -> Value {
        Value::Text(Rc::from(s.as_ref()))
    }
    pub fn array(rows: Vec<Vec<Value>>) -> Value {
        Value::Array(Rc::new(rows))
    }
    pub fn is_error(&self) -> bool {
        matches!(self, Value::Error(_))
    }
    /// Top-left element of an array, or the value itself.
    pub fn top_left(&self) -> Value {
        match self {
            Value::Array(a) => a.first().and_then(|r| r.first()).cloned().unwrap_or(Value::Empty),
            v => v.clone(),
        }
    }
    /// Array dimensions (1x1 for scalars).
    pub(crate) fn dims(&self) -> (usize, usize) {
        match self {
            Value::Array(a) => (a.len(), a.first().map_or(0, |r| r.len())),
            _ => (1, 1),
        }
    }
}

pub(crate) type R<T> = Result<T, ErrorKind>;

thread_local! {
    static DATE1904: Cell<bool> = const { Cell::new(false) };
}
pub(crate) fn set_date1904(b: bool) {
    DATE1904.with(|c| c.set(b));
}
pub(crate) fn date1904() -> bool {
    DATE1904.with(|c| c.get())
}

/// Wraps a float, mapping non-finite results to `#NUM!`.
#[inline]
pub(crate) fn num(x: f64) -> Value {
    if x.is_finite() { Value::Number(if x == 0.0 { 0.0 } else { x }) } else { Value::Error(ErrorKind::Num) }
}

pub(crate) fn res(r: R<Value>) -> Value {
    r.unwrap_or_else(Value::Error)
}

/// Rounds to 15 significant digits (Excel's displayed/compared precision).
pub(crate) fn sig15(x: f64) -> f64 {
    if x == 0.0 || !x.is_finite() {
        return x;
    }
    format!("{:.14e}", x).parse().unwrap_or(x)
}

/// Numeric coercion used by operators (`"3"+1`, `TRUE+1`, empty = 0).
pub(crate) fn to_num(v: &Value) -> R<f64> {
    match v {
        Value::Empty => Ok(0.0),
        Value::Number(n) => Ok(*n),
        Value::Bool(b) => Ok(if *b { 1.0 } else { 0.0 }),
        Value::Text(t) => parse_num_text(t).ok_or(ErrorKind::Value),
        Value::Error(e) => Err(*e),
        Value::Array(_) => to_num(&v.top_left()),
    }
}

/// Text coercion used by `&` and text functions.
pub(crate) fn to_text(v: &Value) -> R<Cow<'_, str>> {
    match v {
        Value::Empty => Ok(Cow::Borrowed("")),
        Value::Number(n) => Ok(Cow::Owned(num_to_text(*n))),
        Value::Bool(b) => Ok(Cow::Borrowed(if *b { "TRUE" } else { "FALSE" })),
        Value::Text(t) => Ok(Cow::Borrowed(t)),
        Value::Error(e) => Err(*e),
        Value::Array(_) => match v.top_left() {
            Value::Text(t) => Ok(Cow::Owned(t.to_string())),
            o => to_text(&o).map(|c| Cow::Owned(c.into_owned())),
        },
    }
}

/// Boolean coercion (IF, AND, NOT...).
pub(crate) fn to_bool(v: &Value) -> R<bool> {
    match v {
        Value::Empty => Ok(false),
        Value::Number(n) => Ok(*n != 0.0),
        Value::Bool(b) => Ok(*b),
        Value::Text(t) => {
            if t.eq_ignore_ascii_case("TRUE") {
                Ok(true)
            } else if t.eq_ignore_ascii_case("FALSE") {
                Ok(false)
            } else {
                Err(ErrorKind::Value)
            }
        }
        Value::Error(e) => Err(*e),
        Value::Array(_) => to_bool(&v.top_left()),
    }
}

/// Formats a number the way Excel's General format does inside formulas
/// (`=""&x`): up to 15 significant digits, scientific for very large/small.
pub fn num_to_text(x: f64) -> String {
    if x == 0.0 {
        return "0".to_string();
    }
    if !x.is_finite() {
        return "#NUM!".to_string();
    }
    if x.fract() == 0.0 && x.abs() < 1e15 {
        return format!("{}", x as i64);
    }
    let s = format!("{:.14e}", x);
    let (mant, exp) = s.split_once('e').unwrap_or((&s, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let neg = mant.starts_with('-');
    let digits: String = mant.chars().filter(|c| c.is_ascii_digit()).collect();
    let digits = digits.trim_end_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };
    let mut out = String::new();
    if neg {
        out.push('-');
    }
    if !(-9..15).contains(&exp) {
        out.push_str(&digits[..1]);
        if digits.len() > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        out.push('E');
        out.push(if exp < 0 { '-' } else { '+' });
        out.push_str(&format!("{:02}", exp.abs()));
    } else if exp < 0 {
        out.push_str("0.");
        for _ in 0..(-exp - 1) {
            out.push('0');
        }
        out.push_str(digits);
    } else {
        let e = exp as usize;
        if digits.len() <= e + 1 {
            out.push_str(digits);
            for _ in digits.len()..=e {
                out.push('0');
            }
        } else {
            out.push_str(&digits[..=e]);
            out.push('.');
            out.push_str(&digits[e + 1..]);
        }
    }
    out
}

/// Parses a plain decimal number (no thousands separators); rejects inf/nan.
pub(crate) fn parse_plain_number(s: &str) -> Option<f64> {
    let b = s.as_bytes();
    if b.is_empty() {
        return None;
    }
    let mut i = 0;
    if b[i] == b'+' || b[i] == b'-' {
        i += 1;
    }
    let mut digits = 0;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
        digits += 1;
    }
    if i < b.len() && b[i] == b'.' {
        i += 1;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
            digits += 1;
        }
    }
    if digits == 0 {
        return None;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        i += 1;
        if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
            i += 1;
        }
        let st = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if st == i {
            return None;
        }
    }
    if i != b.len() {
        return None;
    }
    s.parse().ok()
}

/// Excel's text → number conversion: numbers, `1,234.5`, `$12`, `50%`,
/// `(5)`, dates and times.
pub(crate) fn parse_num_text(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    if let Some(n) = parse_plain_number(t) {
        return Some(n);
    }
    let mut body = t;
    let mut neg = false;
    if body.starts_with('(') && body.ends_with(')') && body.len() > 2 {
        neg = true;
        body = body[1..body.len() - 1].trim();
    }
    if let Some(r) = body.strip_prefix('-') {
        neg = !neg;
        body = r.trim_start();
    } else if let Some(r) = body.strip_prefix('+') {
        body = r.trim_start();
    }
    let mut pct = false;
    if let Some(r) = body.strip_suffix('%') {
        pct = true;
        body = r.trim_end();
    }
    if let Some(r) = body.strip_prefix('$') {
        body = r.trim_start();
        if let Some(r2) = body.strip_prefix('-') {
            neg = !neg;
            body = r2;
        }
    }
    let n = if body.contains(',') {
        // thousands separators: groups of 3 after the first
        let (int, frac) = match body.find('.') {
            Some(p) => (&body[..p], &body[p..]),
            None => (body, ""),
        };
        let groups: Vec<&str> = int.split(',').collect();
        let ok = !groups[0].is_empty()
            && groups[0].len() <= 3
            && groups[1..].iter().all(|g| g.len() == 3)
            && groups.iter().all(|g| g.bytes().all(|c| c.is_ascii_digit()));
        if !ok {
            return None;
        }
        parse_plain_number(&format!("{}{}", groups.concat(), frac))
    } else if body != t || pct || neg {
        parse_plain_number(body)
    } else {
        None
    };
    if let Some(mut n) = n {
        if neg {
            n = -n;
        }
        if pct {
            n /= 100.0;
        }
        return Some(n);
    }
    if pct || neg {
        return None;
    }
    super::dates::parse_datetime(t, date1904())
}

/// Numeric equality at Excel's 15 significant digits.
#[inline]
pub(crate) fn num_eq(a: f64, b: f64) -> bool {
    a == b || (a - b).abs() <= a.abs().max(b.abs()) * 1e-15
}

#[inline]
pub(crate) fn cmp_num(a: f64, b: f64) -> Ordering {
    if num_eq(a, b) { Ordering::Equal } else { a.partial_cmp(&b).unwrap_or(Ordering::Equal) }
}

/// Case-insensitive text comparison.
pub(crate) fn cmp_text(a: &str, b: &str) -> Ordering {
    if a.is_ascii() && b.is_ascii() {
        let (x, y) = (a.as_bytes(), b.as_bytes());
        for (p, q) in x.iter().zip(y) {
            let o = p.to_ascii_lowercase().cmp(&q.to_ascii_lowercase());
            if o != Ordering::Equal {
                return o;
            }
        }
        return x.len().cmp(&y.len());
    }
    let mut ia = a.chars().flat_map(char::to_lowercase);
    let mut ib = b.chars().flat_map(char::to_lowercase);
    loop {
        match (ia.next(), ib.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) => {
                if x != y {
                    return x.cmp(&y);
                }
            }
        }
    }
}

#[inline]
pub(crate) fn text_eq(a: &str, b: &str) -> bool {
    if a.len() == b.len() && a.eq_ignore_ascii_case(b) {
        return true;
    }
    if a.is_ascii() && b.is_ascii() {
        return false;
    }
    cmp_text(a, b) == Ordering::Equal
}

/// Excel ordering for comparison operators: numbers < text < booleans;
/// empty acts as 0 / "" / FALSE depending on the other side. Errors must be
/// handled by the caller.
pub(crate) fn cmp_values(a: &Value, b: &Value) -> Ordering {
    use Value::*;
    fn rank(v: &Value) -> u8 {
        match v {
            Number(_) | Empty => 0,
            Text(_) => 1,
            Bool(_) => 2,
            _ => 3,
        }
    }
    match (a, b) {
        (Empty, Empty) => Ordering::Equal,
        (Number(x), Number(y)) => cmp_num(*x, *y),
        (Empty, Number(y)) => cmp_num(0.0, *y),
        (Number(x), Empty) => cmp_num(*x, 0.0),
        (Text(x), Text(y)) => cmp_text(x, y),
        (Empty, Text(y)) => cmp_text("", y),
        (Text(x), Empty) => cmp_text(x, ""),
        (Bool(x), Bool(y)) => x.cmp(y),
        (Empty, Bool(y)) => false.cmp(y),
        (Bool(x), Empty) => x.cmp(&false),
        (Array(_), _) => cmp_values(&a.top_left(), b),
        (_, Array(_)) => cmp_values(a, &b.top_left()),
        _ => rank(a).cmp(&rank(b)),
    }
}

/// Exact-match equality used by lookups (same type required).
pub(crate) fn lookup_eq(needle: &Value, cell: &Value) -> bool {
    match (needle, cell) {
        (Value::Number(a), Value::Number(b)) => num_eq(*a, *b),
        (Value::Text(a), Value::Text(b)) => text_eq(a, b),
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::Text(a), Value::Empty) => a.is_empty(),
        _ => false,
    }
}

// ---- small argument helpers for scalar functions ----

pub(crate) fn an(a: &[Value], i: usize) -> R<f64> {
    match a.get(i) {
        Some(v) => to_num(v),
        None => Err(ErrorKind::Value),
    }
}
pub(crate) fn ao(a: &[Value], i: usize, def: f64) -> R<f64> {
    match a.get(i) {
        Some(v) => to_num(v),
        None => Ok(def),
    }
}
pub(crate) fn at(a: &[Value], i: usize) -> R<Cow<'_, str>> {
    match a.get(i) {
        Some(v) => to_text(v),
        None => Err(ErrorKind::Value),
    }
}
pub(crate) fn ab(a: &[Value], i: usize, def: bool) -> R<bool> {
    match a.get(i) {
        Some(v) => to_bool(v),
        None => Ok(def),
    }
}
