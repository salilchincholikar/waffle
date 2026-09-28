//! Excel number-format codes: compile once with [`NumFmt::parse`], render many times.
//!
//! Std-only. The format path does no heap allocation beyond appending to the caller's `String`.
//! Numbers are rendered from their 15-significant-digit decimal form (as Excel does) and rounded
//! half away from zero at the displayed precision.

use std::fmt::{self, Write as _};

// ---------------------------------------------------------------------------------------------
// Compiled representation
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ph {
    Zero,
    Hash,
    Q,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Unit {
    H,
    M,
    S,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Tok {
    /// Literal text: (start, len) into `NumFmt::lits`.
    Lit(u32, u32),
    Int(Ph),
    Frac(Ph),
    ExpD(Ph),
    Num(Ph),
    Den(Ph),
    DenFixed(u32),
    Point,
    /// `E+` (true) or `E-` (false).
    Exp(bool),
    Slash,
    Text,
    General,
    Year(u8),
    Month(u8),
    Day(u8),
    Hour(u8),
    Minute(u8),
    Second(u8),
    SubSec(u8),
    /// 0 = AM/PM, 1 = am/pm, 2 = A/P, 3 = a/p
    AmPm(u8),
    Elapsed(Unit, u8),
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Cmp {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum Kind {
    #[default]
    Number,
    Date,
    Text,
}

#[derive(Clone, Debug, Default)]
struct Section {
    toks: Vec<Tok>,
    color: Option<u32>,
    cond: Option<(Cmp, f64)>,
    kind: Kind,
    grouping: bool,
    percent: bool,
    exp: bool,
    eng: bool,
    frac: bool,
    ampm: bool,
    ymd: bool,
    /// Power-of-ten scale: +2 per `%`, -3 per trailing `,`.
    scale: i32,
    n_int: u16,
    n_frac: u16,
    n_exp: u16,
    n_num: u16,
    n_den: u16,
    den_fixed: u32,
    subsec: u8,
}

/// A compiled Excel number format. Parse once, format many times.
#[derive(Clone, Debug)]
pub struct NumFmt {
    secs: Vec<Section>,
    lits: String,
    text_idx: Option<u8>,
    /// Number of sections usable for numbers (excludes the text section).
    n_num: u8,
}

impl Default for NumFmt {
    fn default() -> Self {
        NumFmt::general()
    }
}

const HASHES: &str = "########";

const PALETTE: [u32; 56] = [
    0x000000, 0xFFFFFF, 0xFF0000, 0x00FF00, 0x0000FF, 0xFFFF00, 0xFF00FF, 0x00FFFF, // 1-8
    0x800000, 0x008000, 0x000080, 0x808000, 0x800080, 0x008080, 0xC0C0C0, 0x808080, // 9-16
    0x9999FF, 0x993366, 0xFFFFCC, 0xCCFFFF, 0x660066, 0xFF8080, 0x0066CC, 0xCCCCFF, // 17-24
    0x000080, 0xFF00FF, 0xFFFF00, 0x00FFFF, 0x800080, 0x800000, 0x008080, 0x0000FF, // 25-32
    0x00CCFF, 0xCCFFFF, 0xCCFFCC, 0xFFFF99, 0x99CCFF, 0xFF99CC, 0xCC99FF, 0xFFCC99, // 33-40
    0x3366FF, 0x33CCCC, 0x99CC00, 0xFFCC00, 0xFF9900, 0xFF6600, 0x666699, 0x969696, // 41-48
    0x003366, 0x339966, 0x003300, 0x333300, 0x993300, 0x993366, 0x333399, 0x333333, // 49-56
];

const MONTHS: [&str; 12] =
    ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

fn named_color(s: &str) -> Option<u32> {
    Some(match s {
        "black" => 0x000000,
        "white" => 0xFFFFFF,
        "red" => 0xFF0000,
        "green" => 0x00FF00,
        "blue" => 0x0000FF,
        "yellow" => 0xFFFF00,
        "magenta" => 0xFF00FF,
        "cyan" => 0x00FFFF,
        _ => return None,
    })
}

// ---------------------------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------------------------

fn push_lit(toks: &mut Vec<Tok>, lits: &mut String, s: &str) {
    if s.is_empty() {
        return;
    }
    let start = lits.len() as u32;
    lits.push_str(s);
    if let Some(Tok::Lit(st, len)) = toks.last_mut()
        && *st + *len == start
    {
        *len += s.len() as u32;
        return;
    }
    toks.push(Tok::Lit(start, s.len() as u32));
}

fn push_ch(toks: &mut Vec<Tok>, lits: &mut String, c: char) {
    let mut b = [0u8; 4];
    push_lit(toks, lits, c.encode_utf8(&mut b));
}

fn starts_with_ci(cs: &[char], i: usize, pat: &str) -> bool {
    let n = pat.len();
    cs.len() >= i + n && cs[i..i + n].iter().zip(pat.chars()).all(|(c, p)| c.eq_ignore_ascii_case(&p))
}

fn run_len(cs: &[char], i: usize) -> usize {
    let c = cs[i].to_ascii_lowercase();
    cs[i..].iter().take_while(|x| x.to_ascii_lowercase() == c).count()
}

fn is_ph(c: char) -> bool {
    matches!(c, '0' | '#' | '?')
}

fn bracket(s: &mut Section, body: &str, lits: &mut String) {
    let lc = body.trim().to_ascii_lowercase();
    if let Some(c) = named_color(&lc) {
        s.color = Some(c);
    } else if let Some(rest) = lc.strip_prefix("color") {
        if let Ok(n) = rest.trim().parse::<usize>()
            && (1..=56).contains(&n)
        {
            s.color = Some(PALETTE[n - 1]);
        }
    } else if let Some(r) = body.strip_prefix('$') {
        let sym = r.split('-').next().unwrap_or("");
        push_lit(&mut s.toks, lits, sym);
    } else if lc.starts_with(['<', '>', '=']) {
        let (op, rest) = if let Some(r) = lc.strip_prefix("<=") {
            (Cmp::Le, r)
        } else if let Some(r) = lc.strip_prefix(">=") {
            (Cmp::Ge, r)
        } else if let Some(r) = lc.strip_prefix("<>") {
            (Cmp::Ne, r)
        } else if let Some(r) = lc.strip_prefix('<') {
            (Cmp::Lt, r)
        } else if let Some(r) = lc.strip_prefix('>') {
            (Cmp::Gt, r)
        } else {
            (Cmp::Eq, &lc[1..])
        };
        if let Ok(x) = rest.trim().parse::<f64>() {
            s.cond = Some((op, x));
        }
    } else if !lc.is_empty() {
        let w = lc.len().min(255) as u8;
        let unit = if lc.bytes().all(|b| b == b'h') {
            Some(Unit::H)
        } else if lc.bytes().all(|b| b == b'm') {
            Some(Unit::M)
        } else if lc.bytes().all(|b| b == b's') {
            Some(Unit::S)
        } else {
            None // [DBNum1], [$-F800] handled above, etc.: ignored
        };
        if let Some(u) = unit {
            s.toks.push(Tok::Elapsed(u, w));
        }
    }
}

/// Parses one section starting at `i`; returns it and the start of the next section, if any.
fn parse_section(cs: &[char], mut i: usize, lits: &mut String) -> (Section, Option<usize>) {
    #[derive(PartialEq, Clone, Copy)]
    enum Mode {
        Int,
        Frac,
        Exp,
        Den,
    }
    let n = cs.len();
    let mut s = Section::default();
    let mut mode = Mode::Int;
    let mut next = None;
    let seen_ph = |t: &[Tok]| t.iter().any(|t| matches!(t, Tok::Int(_) | Tok::Frac(_)));

    while i < n {
        let c = cs[i];
        match c {
            ';' => {
                next = Some(i + 1);
                break;
            }
            '"' => {
                let mut j = i + 1;
                while j < n && cs[j] != '"' {
                    push_ch(&mut s.toks, lits, cs[j]);
                    j += 1;
                }
                i = j + 1;
                continue;
            }
            '\\' | '!' => {
                if i + 1 < n {
                    push_ch(&mut s.toks, lits, cs[i + 1]);
                }
                i += 2;
                continue;
            }
            '_' => {
                push_ch(&mut s.toks, lits, ' ');
                i += 2;
                continue;
            }
            '*' => {
                i += 2;
                continue;
            }
            '[' => {
                let mut j = i + 1;
                while j < n && cs[j] != ']' {
                    j += 1;
                }
                let body: String = cs[i + 1..j].iter().collect();
                bracket(&mut s, &body, lits);
                i = j + 1;
                continue;
            }
            '0' | '#' | '?' => {
                let ph = match c {
                    '0' => Ph::Zero,
                    '#' => Ph::Hash,
                    _ => Ph::Q,
                };
                s.toks.push(match mode {
                    Mode::Int => Tok::Int(ph),
                    Mode::Frac => Tok::Frac(ph),
                    Mode::Exp => Tok::ExpD(ph),
                    Mode::Den => Tok::Den(ph),
                });
            }
            '.' => {
                let after_sec =
                    matches!(s.toks.iter().rev().find(|t| !matches!(t, Tok::Lit(..))), Some(Tok::Second(_) | Tok::Elapsed(Unit::S, _)));
                if after_sec && i + 1 < n && cs[i + 1] == '0' {
                    let mut j = i + 1;
                    while j < n && cs[j] == '0' {
                        j += 1;
                    }
                    s.toks.push(Tok::SubSec((j - i - 1).min(3) as u8));
                    i = j;
                    continue;
                } else if mode == Mode::Int {
                    s.toks.push(Tok::Point);
                    mode = Mode::Frac;
                } else {
                    push_ch(&mut s.toks, lits, '.');
                }
            }
            ',' => {
                let mut j = i;
                while j < n && cs[j] == ',' {
                    j += 1;
                }
                let next_ph = j < n && is_ph(cs[j]);
                if !seen_ph(&s.toks) {
                    for _ in i..j {
                        push_ch(&mut s.toks, lits, ',');
                    }
                } else if next_ph {
                    if mode == Mode::Int {
                        s.grouping = true;
                    }
                } else {
                    s.scale -= 3 * (j - i) as i32;
                }
                i = j;
                continue;
            }
            '%' => {
                push_ch(&mut s.toks, lits, '%');
                s.scale += 2;
                s.percent = true;
            }
            'E' | 'e' if i + 1 < n && matches!(cs[i + 1], '+' | '-') && seen_ph(&s.toks) => {
                s.toks.push(Tok::Exp(cs[i + 1] == '+'));
                s.exp = true;
                mode = Mode::Exp;
                i += 2;
                continue;
            }
            '/' if mode == Mode::Int && matches!(s.toks.last(), Some(Tok::Int(_))) => {
                // The run of placeholders just before '/' is the numerator.
                for t in s.toks.iter_mut().rev() {
                    match *t {
                        Tok::Int(p) => *t = Tok::Num(p),
                        _ => break,
                    }
                }
                s.toks.push(Tok::Slash);
                s.frac = true;
                mode = Mode::Den;
                let mut j = i + 1;
                let mut d: u32 = 0;
                if j < n && matches!(cs[j], '1'..='9') {
                    while j < n && cs[j].is_ascii_digit() {
                        d = d.saturating_mul(10).saturating_add(cs[j] as u32 - '0' as u32);
                        j += 1;
                    }
                    s.toks.push(Tok::DenFixed(d));
                    s.den_fixed = d;
                }
                i = j;
                continue;
            }
            '@' => s.toks.push(Tok::Text),
            _ if c.is_ascii_alphabetic() => {
                let lc = c.to_ascii_lowercase();
                let tok = match lc {
                    'y' | 'e' => {
                        let r = run_len(cs, i);
                        i += r;
                        Some(Tok::Year(if lc == 'y' && r <= 2 { 2 } else { 4 }))
                    }
                    'm' | 'd' | 'h' | 's' => {
                        let r = run_len(cs, i).min(255) as u8;
                        i += r as usize;
                        Some(match lc {
                            'm' => Tok::Month(r.min(5)),
                            'd' => Tok::Day(r.min(4)),
                            'h' => Tok::Hour(r.min(2)),
                            _ => Tok::Second(r.min(2)),
                        })
                    }
                    'a' if starts_with_ci(cs, i, "am/pm") => {
                        i += 5;
                        Some(Tok::AmPm(if c == 'a' { 1 } else { 0 }))
                    }
                    'a' if starts_with_ci(cs, i, "a/p") => {
                        i += 3;
                        Some(Tok::AmPm(if c == 'a' { 3 } else { 2 }))
                    }
                    'g' if starts_with_ci(cs, i, "general") => {
                        i += 7;
                        Some(Tok::General)
                    }
                    _ => None,
                };
                match tok {
                    Some(t) => s.toks.push(t),
                    None => {
                        push_ch(&mut s.toks, lits, c);
                        i += 1;
                    }
                }
                continue;
            }
            _ => push_ch(&mut s.toks, lits, c),
        }
        i += 1;
    }
    finish_section(&mut s);
    (s, next)
}

fn finish_section(s: &mut Section) {
    // m/mm directly after an hour or directly before a second means minutes.
    let dt: Vec<usize> = s
        .toks
        .iter()
        .enumerate()
        .filter(|(_, t)| {
            matches!(t, Tok::Year(_) | Tok::Month(_) | Tok::Day(_) | Tok::Hour(_) | Tok::Minute(_) | Tok::Second(_) | Tok::Elapsed(..))
        })
        .map(|(i, _)| i)
        .collect();
    for (k, &i) in dt.iter().enumerate() {
        if let Tok::Month(n) = s.toks[i]
            && n <= 2
        {
            let prev = k > 0 && matches!(s.toks[dt[k - 1]], Tok::Hour(_) | Tok::Elapsed(Unit::H, _));
            let next = k + 1 < dt.len() && matches!(s.toks[dt[k + 1]], Tok::Second(_) | Tok::Elapsed(Unit::S, _));
            if prev || next {
                s.toks[i] = Tok::Minute(n);
            }
        }
    }

    let mut has_date = false;
    let mut has_num = false;
    let mut has_text = false;
    let mut int_hash = false;
    for t in &s.toks {
        match *t {
            Tok::Int(p) => {
                s.n_int += 1;
                int_hash |= p == Ph::Hash;
                has_num = true;
            }
            Tok::Frac(_) => {
                s.n_frac += 1;
                has_num = true;
            }
            Tok::ExpD(_) => s.n_exp += 1,
            Tok::Num(_) => {
                s.n_num += 1;
                has_num = true;
            }
            Tok::Den(_) => s.n_den += 1,
            Tok::General => has_num = true,
            Tok::Text => has_text = true,
            Tok::SubSec(n) => s.subsec = s.subsec.max(n),
            Tok::AmPm(_) => {
                s.ampm = true;
                has_date = true;
            }
            Tok::Year(_) | Tok::Month(_) | Tok::Day(_) => {
                s.ymd = true;
                has_date = true;
            }
            Tok::Hour(_) | Tok::Minute(_) | Tok::Second(_) | Tok::Elapsed(..) => has_date = true,
            _ => {}
        }
    }
    s.kind = if has_date {
        Kind::Date
    } else if has_text && !has_num {
        Kind::Text
    } else {
        Kind::Number
    };
    s.eng = s.exp && s.n_int > 1 && int_hash;
    s.toks.shrink_to_fit();
}

// ---------------------------------------------------------------------------------------------
// Decimal digits (15 significant, like Excel)
// ---------------------------------------------------------------------------------------------

struct StackBuf {
    b: [u8; 40],
    n: usize,
}

impl fmt::Write for StackBuf {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let bytes = s.as_bytes();
        let end = self.n + bytes.len();
        if end > self.b.len() {
            return Err(fmt::Error);
        }
        self.b[self.n..end].copy_from_slice(bytes);
        self.n = end;
        Ok(())
    }
}

/// value = d[0].d[1]d[2]... × 10^exp
#[derive(Clone, Copy)]
struct Dec {
    d: [u8; 15],
    exp: i32,
    zero: bool,
}

impl Dec {
    fn new(a: f64) -> Dec {
        let mut dec = Dec { d: [0; 15], exp: 0, zero: true };
        if a == 0.0 || !a.is_finite() {
            return dec;
        }
        let mut buf = StackBuf { b: [0; 40], n: 0 };
        let _ = write!(buf, "{:.14e}", a.abs());
        let s = &buf.b[..buf.n];
        // "d.dddddddddddddde[-]x"
        dec.d[0] = s[0] - b'0';
        for k in 1..15 {
            dec.d[k] = s[k + 1] - b'0';
        }
        let mut e = 0i32;
        let mut neg = false;
        for &b in &s[17..] {
            if b == b'-' {
                neg = true;
            } else {
                e = e * 10 + (b - b'0') as i32;
            }
        }
        dec.exp = if neg { -e } else { e };
        dec.zero = false;
        dec
    }

    /// ASCII digit at power-of-ten position `pos`.
    #[inline]
    fn dg(&self, pos: i32) -> u8 {
        let i = self.exp - pos;
        if self.zero || !(0..15).contains(&i) { b'0' } else { b'0' + self.d[i as usize] }
    }

    fn int_len(&self) -> i32 {
        if self.zero { 0 } else { (self.exp + 1).max(0) }
    }

    /// Number of fractional digits up to the last non-zero one.
    fn frac_len(&self) -> i32 {
        if self.zero {
            return 0;
        }
        let last = (0..15).rev().find(|&k| self.d[k] != 0).unwrap_or(0) as i32;
        (last - self.exp).max(0)
    }

    /// Keep `n` significant digits, rounding half away from zero.
    fn round_sig(&mut self, n: i32) {
        if self.zero || n >= 15 {
            return;
        }
        if n < 0 {
            self.zero = true;
            return;
        }
        let n = n as usize;
        let up = self.d[n] >= 5;
        self.d[n..].fill(0);
        if !up {
            if n == 0 {
                self.zero = true;
            }
            return;
        }
        let mut k = n;
        loop {
            if k == 0 {
                self.d = [0; 15];
                self.d[0] = 1;
                self.exp += 1;
                return;
            }
            k -= 1;
            if self.d[k] == 9 {
                self.d[k] = 0;
            } else {
                self.d[k] += 1;
                return;
            }
        }
    }

    fn round_dec(&mut self, frac: i32) {
        self.round_sig(self.exp + 1 + frac)
    }
}

fn u64_digits(mut n: u64) -> ([u8; 20], i32) {
    let mut b = [0u8; 20];
    let mut i = 20;
    loop {
        i -= 1;
        b[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    b.copy_within(i.., 0);
    (b, (20 - i) as i32)
}

/// Emit the `k`-th of `n_ph` right-aligned integer placeholders for a number with `nd` digits.
#[inline]
fn fill_digit<F: Fn(i32) -> u8>(out: &mut String, ph: Ph, k: i32, n_ph: i32, nd: i32, dg: &F, group: bool) {
    let sep = |out: &mut String, p: i32| {
        if group && p > 0 && p % 3 == 0 {
            out.push(',');
        }
    };
    if k == 0 && nd > n_ph {
        for q in (n_ph..nd).rev() {
            out.push(dg(q) as char);
            sep(out, q);
        }
    }
    let p = n_ph - 1 - k;
    if p < nd {
        out.push(dg(p) as char);
        sep(out, p);
    } else {
        match ph {
            Ph::Zero => {
                out.push('0');
                sep(out, p);
            }
            Ph::Q => out.push(' '),
            Ph::Hash => {}
        }
    }
}

fn push_num(out: &mut String, n: i64, width: u8) {
    let _ = write!(out, "{:0w$}", n, w = width as usize);
}

// ---------------------------------------------------------------------------------------------
// Formatting
// ---------------------------------------------------------------------------------------------

fn cond_ok((op, x): (Cmp, f64), v: f64) -> bool {
    match op {
        Cmp::Lt => v < x,
        Cmp::Le => v <= x,
        Cmp::Gt => v > x,
        Cmp::Ge => v >= x,
        Cmp::Eq => v == x,
        Cmp::Ne => v != x,
    }
}

fn implies_neg(c: Option<(Cmp, f64)>) -> bool {
    matches!(c, Some((Cmp::Lt, x)) if x <= 0.0) || matches!(c, Some((Cmp::Le, x)) if x < 0.0)
}

impl NumFmt {
    /// Compile a format code. Never fails; an empty code behaves like General.
    pub fn parse(code: &str) -> NumFmt {
        if code.trim().is_empty() {
            return NumFmt::general();
        }
        let cs: Vec<char> = code.chars().collect();
        let mut lits = String::new();
        let mut secs = Vec::with_capacity(1);
        let mut i = 0;
        loop {
            let (sec, next) = parse_section(&cs, i, &mut lits);
            secs.push(sec);
            match next {
                Some(j) if secs.len() < 4 => i = j,
                _ => break,
            }
        }
        let len = secs.len();
        let text_idx = if len == 4 || secs[len - 1].kind == Kind::Text { Some(len - 1) } else { None };
        let n_num = len - usize::from(text_idx.is_some());
        lits.shrink_to_fit();
        secs.shrink_to_fit();
        NumFmt { secs, lits, text_idx: text_idx.map(|x| x as u8), n_num: n_num as u8 }
    }

    pub fn general() -> NumFmt {
        let sec = Section { toks: vec![Tok::General], ..Section::default() };
        NumFmt { secs: vec![sec], lits: String::new(), text_idx: None, n_num: 1 }
    }

    /// Parse the built-in format for `id`, or General if unknown.
    pub fn builtin(id: u32) -> NumFmt {
        builtin_code(id).map_or_else(NumFmt::general, NumFmt::parse)
    }

    #[inline]
    fn lit(&self, s: u32, l: u32) -> &str {
        &self.lits[s as usize..(s + l) as usize]
    }

    /// Returns (section index, whether to prefix '-' for negatives).
    fn choose(&self, v: f64) -> (usize, bool) {
        let n = self.n_num as usize;
        let s = &self.secs;
        if s[..n].iter().all(|x| x.cond.is_none()) {
            return match n {
                1 => (0, true),
                2 => {
                    if v >= 0.0 {
                        (0, true)
                    } else {
                        (1, false)
                    }
                }
                _ => {
                    if v > 0.0 {
                        (0, true)
                    } else if v < 0.0 {
                        (1, false)
                    } else {
                        (2, true)
                    }
                }
            };
        }
        let c0 = s[0].cond.unwrap_or((if n >= 3 { Cmp::Gt } else { Cmp::Ge }, 0.0));
        if n == 1 || cond_ok(c0, v) {
            return (0, true);
        }
        if n == 2 {
            return (1, !implies_neg(s[1].cond));
        }
        let c1 = s[1].cond.unwrap_or((Cmp::Lt, 0.0));
        if cond_ok(c1, v) { (1, !implies_neg(Some(c1))) } else { (2, true) }
    }

    /// Append the display text for a number to `out`; returns the section's color (0xRRGGBB).
    pub fn format_number(&self, v: f64, date1904: bool, out: &mut String) -> Option<u32> {
        if !v.is_finite() {
            out.push_str("#NUM!");
            return None;
        }
        let v = if v == 0.0 { 0.0 } else { v }; // -0 → 0
        if self.n_num == 0 {
            format_general(v, out);
            return None;
        }
        let (idx, sign) = self.choose(v);
        let sec = &self.secs[idx];
        if sec.kind == Kind::Date {
            self.fmt_date(sec, v, date1904, out);
        } else {
            if sign && v < 0.0 {
                out.push('-');
            }
            if sec.frac {
                self.fmt_fraction(sec, v.abs(), out);
            } else {
                self.fmt_decimal(sec, v.abs(), out);
            }
        }
        sec.color
    }

    /// Text-section rendering; without a text section the text is shown as-is.
    pub fn format_text(&self, s: &str, out: &mut String) -> Option<u32> {
        let Some(i) = self.text_idx else {
            out.push_str(s);
            return None;
        };
        let sec = &self.secs[i as usize];
        for t in &sec.toks {
            match *t {
                Tok::Text => out.push_str(s),
                Tok::Lit(a, l) => out.push_str(self.lit(a, l)),
                _ => {}
            }
        }
        sec.color
    }

    pub fn is_date(&self) -> bool {
        self.n_num > 0 && self.secs[0].kind == Kind::Date
    }

    pub fn is_general(&self) -> bool {
        self.secs.len() == 1 && self.secs[0].toks == [Tok::General]
    }

    pub fn is_percent(&self) -> bool {
        self.secs[0].percent
    }

    pub fn is_text(&self) -> bool {
        self.n_num == 0
    }

    pub fn decimals(&self) -> u8 {
        let s = &self.secs[0];
        if s.kind == Kind::Date { s.subsec } else { s.n_frac.min(255) as u8 }
    }

    fn fmt_decimal(&self, sec: &Section, a: f64, out: &mut String) {
        let mut d = Dec::new(a);
        if !d.zero {
            d.exp += sec.scale;
        }
        let n_int = sec.n_int as i32;
        let n_frac = sec.n_frac as i32;
        let mut expv = 0i32;
        if sec.exp {
            if !d.zero {
                let step = if sec.eng { n_int } else { 0 };
                let pick = |e: i32| {
                    if step > 0 { e.div_euclid(step) * step } else { e + 1 - n_int }
                };
                let e0 = pick(d.exp);
                d.round_sig(d.exp - e0 + 1 + n_frac);
                expv = pick(d.exp);
                d.exp -= expv;
            }
        } else {
            d.round_dec(n_frac);
        }
        let nd = d.int_len();
        let fl = d.frac_len();
        let (eb, ne) = u64_digits(expv.unsigned_abs() as u64);
        let edg = |p: i32| eb[(ne - 1 - p) as usize];
        let idg = |p: i32| d.dg(p);
        let (mut ii, mut fi, mut ei) = (0i32, 0i32, 0i32);
        for t in &sec.toks {
            match *t {
                Tok::Lit(s, l) => out.push_str(self.lit(s, l)),
                Tok::Int(ph) => {
                    fill_digit(out, ph, ii, n_int, nd, &idg, sec.grouping);
                    ii += 1;
                }
                Tok::Point => {
                    if n_int == 0 {
                        for q in (0..nd).rev() {
                            out.push(d.dg(q) as char);
                        }
                    }
                    out.push('.');
                }
                Tok::Frac(ph) => {
                    fi += 1;
                    if ph == Ph::Zero || fi <= fl {
                        out.push(d.dg(-fi) as char);
                    } else if ph == Ph::Q {
                        out.push(' ');
                    }
                }
                Tok::Exp(plus) => {
                    out.push('E');
                    if expv < 0 {
                        out.push('-');
                    } else if plus {
                        out.push('+');
                    }
                }
                Tok::ExpD(ph) => {
                    fill_digit(out, ph, ei, sec.n_exp as i32, ne, &edg, false);
                    ei += 1;
                }
                Tok::General | Tok::Text => format_general(a, out),
                _ => {}
            }
        }
    }

    fn fmt_fraction(&self, sec: &Section, a: f64, out: &mut String) {
        let a = if sec.scale != 0 { a * 10f64.powi(sec.scale) } else { a };
        let has_whole = sec.n_int > 0;
        let (mut whole, f) = if has_whole { (a.trunc(), a.fract()) } else { (0.0, a) };
        let (mut num, den) = if sec.den_fixed > 0 {
            let d = sec.den_fixed as f64;
            ((f * d).round(), d)
        } else {
            best_fraction(f, 10u32.pow(sec.n_den.clamp(1, 4) as u32) - 1)
        };
        if has_whole && num >= den {
            whole += 1.0;
            num = 0.0;
        }
        let blank = has_whole && num == 0.0;
        let w = whole.min(u64::MAX as f64) as u64;
        let (wb, wn) = u64_digits(w);
        let wnd = if w == 0 && !blank { 0 } else { wn };
        let (nb, nn) = u64_digits(num as u64);
        let (db, dn) = u64_digits(den as u64);
        let wdg = |p: i32| wb[(wn - 1 - p) as usize];
        let ndg = |p: i32| nb[(nn - 1 - p) as usize];
        let (mut ii, mut ni, mut di) = (0i32, 0i32, 0i32);
        for t in &sec.toks {
            match *t {
                Tok::Lit(s, l) => out.push_str(self.lit(s, l)),
                Tok::Int(ph) => {
                    fill_digit(out, ph, ii, sec.n_int as i32, wnd, &wdg, sec.grouping);
                    ii += 1;
                }
                Tok::Num(ph) => {
                    if blank {
                        out.push(' ');
                    } else {
                        fill_digit(out, ph, ni, sec.n_num as i32, nn, &ndg, false);
                    }
                    ni += 1;
                }
                Tok::Slash => out.push(if blank { ' ' } else { '/' }),
                Tok::Den(ph) => {
                    if blank {
                        out.push(' ');
                    } else if di < dn {
                        out.push(db[di as usize] as char);
                    } else {
                        match ph {
                            Ph::Zero => out.push('0'),
                            Ph::Q => out.push(' '),
                            Ph::Hash => {}
                        }
                    }
                    di += 1;
                }
                Tok::DenFixed(_) => {
                    for &b in &db[..dn as usize] {
                        out.push(if blank { ' ' } else { b as char });
                    }
                }
                Tok::General | Tok::Text => format_general(a, out),
                _ => {}
            }
        }
    }

    fn fmt_date(&self, sec: &Section, v: f64, date1904: bool, out: &mut String) {
        if v < 0.0 {
            out.push_str(HASHES);
            return;
        }
        let p = sec.subsec as u32;
        let mul = 10i64.pow(p);
        let units = (v * 86400.0 * mul as f64).round();
        if units > 9.0e15 {
            out.push_str(HASHES);
            return;
        }
        let units = units as i64;
        let per_day = 86400 * mul;
        let days = units / per_day;
        let rem = units % per_day;
        let sod = rem / mul;
        let sub = rem % mul;
        let total = units / mul;
        if sec.ymd && days > max_day(date1904) {
            out.push_str(HASHES);
            return;
        }
        let (y, mo, dd) = days_to_ymd(days, date1904);
        let wd = (days + if date1904 { 5 } else { 6 }) % 7;
        let h = sod / 3600;
        let mi = sod / 60 % 60;
        let s = sod % 60;
        for t in &sec.toks {
            match *t {
                Tok::Lit(a, l) => out.push_str(self.lit(a, l)),
                Tok::Year(2) => push_num(out, y.rem_euclid(100), 2),
                Tok::Year(_) => push_num(out, y, 4),
                Tok::Month(n) => match n {
                    1 | 2 => push_num(out, mo as i64, n),
                    3 => out.push_str(&MONTHS[mo as usize - 1][..3]),
                    4 => out.push_str(MONTHS[mo as usize - 1]),
                    _ => out.push_str(&MONTHS[mo as usize - 1][..1]),
                },
                Tok::Day(n) => match n {
                    1 | 2 => push_num(out, dd as i64, n),
                    3 => out.push_str(&DAYS[wd as usize][..3]),
                    _ => out.push_str(DAYS[wd as usize]),
                },
                Tok::Hour(n) => push_num(out, if sec.ampm { (h + 11) % 12 + 1 } else { h }, n),
                Tok::Minute(n) => push_num(out, mi, n),
                Tok::Second(n) => push_num(out, s, n),
                Tok::SubSec(n) => {
                    out.push('.');
                    let mut div = mul / 10;
                    for _ in 0..n {
                        if div == 0 {
                            break;
                        }
                        out.push((b'0' + (sub / div % 10) as u8) as char);
                        div /= 10;
                    }
                }
                Tok::AmPm(k) => out.push_str(match (k, h >= 12) {
                    (0, false) => "AM",
                    (0, true) => "PM",
                    (1, false) => "am",
                    (1, true) => "pm",
                    (2, false) => "A",
                    (2, true) => "P",
                    (_, false) => "a",
                    (_, true) => "p",
                }),
                Tok::Elapsed(u, w) => push_num(
                    out,
                    match u {
                        Unit::H => total / 3600,
                        Unit::M => total / 60,
                        Unit::S => total,
                    },
                    w,
                ),
                _ => {}
            }
        }
    }
}

fn best_fraction(x: f64, maxd: u32) -> (f64, f64) {
    let whole = x.trunc();
    let f = x - whole;
    let mut best = (f.round(), 1.0, (f - f.round()).abs());
    let mut d = 2u32;
    while d <= maxd && best.2 > 1e-12 {
        let df = d as f64;
        let n = (f * df).round();
        let err = (f - n / df).abs();
        if err < best.2 {
            best = (n, df, err);
        }
        d += 1;
    }
    (whole * best.1 + best.0, best.1)
}

// ---------------------------------------------------------------------------------------------
// General
// ---------------------------------------------------------------------------------------------

/// Excel "General": up to ~11 characters; scientific for very large/small magnitudes.
pub fn format_general(v: f64, out: &mut String) {
    if !v.is_finite() {
        out.push_str("#NUM!");
        return;
    }
    if v == 0.0 {
        out.push('0');
        return;
    }
    if v < 0.0 {
        out.push('-');
    }
    let a = v.abs();
    let mut d = Dec::new(a);
    let e = d.exp;
    if (-5..=10).contains(&e) && e != -5 {
        d.round_sig(if e < 0 { 10 + e } else { (e + 1).max(10) });
        if d.exp <= 10 {
            let nd = d.int_len();
            if nd == 0 {
                out.push('0');
            }
            for q in (0..nd).rev() {
                out.push(d.dg(q) as char);
            }
            let fl = d.frac_len();
            if fl > 0 {
                out.push('.');
                for k in 1..=fl {
                    out.push(d.dg(-k) as char);
                }
            }
            return;
        }
        d = Dec::new(a);
    }
    d.round_sig(6);
    out.push((b'0' + d.d[0]) as char);
    if let Some(last) = (1..6).rev().find(|&k| d.d[k] != 0) {
        out.push('.');
        for &x in &d.d[1..=last] {
            out.push((b'0' + x) as char);
        }
    }
    out.push('E');
    out.push(if d.exp < 0 { '-' } else { '+' });
    push_num(out, d.exp.abs() as i64, 2);
}

// ---------------------------------------------------------------------------------------------
// Built-ins
// ---------------------------------------------------------------------------------------------

/// Built-in Excel numFmtId → format code (en-US). None for unknown / locale-specific ids.
pub fn builtin_code(id: u32) -> Option<&'static str> {
    Some(match id {
        0 => "General",
        1 => "0",
        2 => "0.00",
        3 => "#,##0",
        4 => "#,##0.00",
        5 => r##""$"#,##0_);\("$"#,##0\)"##,
        6 => r##""$"#,##0_);[Red]\("$"#,##0\)"##,
        7 => r##""$"#,##0.00_);\("$"#,##0.00\)"##,
        8 => r##""$"#,##0.00_);[Red]\("$"#,##0.00\)"##,
        9 => "0%",
        10 => "0.00%",
        11 => "0.00E+00",
        12 => "# ?/?",
        13 => "# ??/??",
        14 => "m/d/yyyy",
        15 => "d-mmm-yy",
        16 => "d-mmm",
        17 => "mmm-yy",
        18 => "h:mm AM/PM",
        19 => "h:mm:ss AM/PM",
        20 => "h:mm",
        21 => "h:mm:ss",
        22 => "m/d/yyyy h:mm",
        37 => "#,##0 ;(#,##0)",
        38 => "#,##0 ;[Red](#,##0)",
        39 => "#,##0.00;(#,##0.00)",
        40 => "#,##0.00;[Red](#,##0.00)",
        41 => r##"_(* #,##0_);_(* \(#,##0\);_(* "-"_);_(@_)"##,
        42 => r##"_("$"* #,##0_);_("$"* \(#,##0\);_("$"* "-"_);_(@_)"##,
        43 => r##"_(* #,##0.00_);_(* \(#,##0.00\);_(* "-"??_);_(@_)"##,
        44 => r##"_("$"* #,##0.00_);_("$"* \(#,##0.00\);_("$"* "-"??_);_(@_)"##,
        45 => "mm:ss",
        46 => "[h]:mm:ss",
        47 => "mmss.0",
        48 => "##0.0E+0",
        49 => "@",
        _ => return None,
    })
}

// ---------------------------------------------------------------------------------------------
// Dates
// ---------------------------------------------------------------------------------------------

/// Days since 1970-01-01 (proleptic Gregorian).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

const EPOCH_1900: i64 = -25569; // days_from_civil(1899, 12, 30)
const EPOCH_1904: i64 = -24107; // days_from_civil(1904, 1, 1)

fn max_day(date1904: bool) -> i64 {
    if date1904 { 2957003 } else { 2958465 } // 9999-12-31
}

fn days_to_ymd(days: i64, date1904: bool) -> (i64, u32, u32) {
    if date1904 {
        civil_from_days(days + EPOCH_1904)
    } else if days == 0 {
        (1900, 1, 0)
    } else if days == 60 {
        (1900, 2, 29)
    } else if days < 60 {
        civil_from_days(days + EPOCH_1900 + 1)
    } else {
        civil_from_days(days + EPOCH_1900)
    }
}

fn days_in_month(y: i32, m: u32) -> u32 {
    match m {
        4 | 6 | 9 | 11 => 30,
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        _ => 31,
    }
}

/// Excel serial → (year, month, day, hour, minute, second, millis). Serial 0 is 1900-01-00 and
/// serial 60 is the fictitious 1900-02-29 in the 1900 system.
pub fn serial_to_datetime(serial: f64, date1904: bool) -> Option<(i32, u32, u32, u32, u32, u32, u32)> {
    if !serial.is_finite() || serial < 0.0 {
        return None;
    }
    let units = (serial * 86_400_000.0).round() as i64;
    let days = units / 86_400_000;
    if days > max_day(date1904) {
        return None;
    }
    let ms = (units % 86_400_000) as u32;
    let (y, m, d) = days_to_ymd(days, date1904);
    Some((y as i32, m, d, ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000))
}

/// Calendar date/time → Excel serial. None for invalid or out-of-range input.
pub fn datetime_to_serial(y: i32, m: u32, d: u32, h: u32, mi: u32, s: u32, date1904: bool) -> Option<f64> {
    if !(1..=12).contains(&m) || h > 23 || mi > 59 || s > 59 || y > 9999 {
        return None;
    }
    let time = (h * 3600 + mi * 60 + s) as f64 / 86400.0;
    if !date1904 && y == 1900 && m == 2 && d == 29 {
        return Some(60.0 + time);
    }
    if d < 1 || d > days_in_month(y, m) {
        return None;
    }
    let z = days_from_civil(y as i64, m, d);
    let serial = if date1904 {
        z - EPOCH_1904
    } else {
        let s = z - EPOCH_1900;
        if s < 61 { s - 1 } else { s }
    };
    if serial < i64::from(!date1904) {
        return None;
    }
    Some(serial as f64 + time)
}

// ---------------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn f(code: &str, v: f64) -> String {
        let mut s = String::new();
        NumFmt::parse(code).format_number(v, false, &mut s);
        s
    }

    fn fc(code: &str, v: f64) -> (String, Option<u32>) {
        let mut s = String::new();
        let c = NumFmt::parse(code).format_number(v, false, &mut s);
        (s, c)
    }

    fn ft(code: &str, t: &str) -> String {
        let mut s = String::new();
        NumFmt::parse(code).format_text(t, &mut s);
        s
    }

    fn g(v: f64) -> String {
        let mut s = String::new();
        format_general(v, &mut s);
        s
    }

    #[test]
    fn general() {
        assert_eq!(g(0.0), "0");
        assert_eq!(g(-0.0), "0");
        assert_eq!(g(1.0), "1");
        assert_eq!(g(-1.5), "-1.5");
        assert_eq!(g(0.1), "0.1");
        assert_eq!(g(0.1 + 0.2), "0.3");
        assert_eq!(g(1234567.891), "1234567.891");
        assert_eq!(g(1.0 / 3.0), "0.333333333");
        assert_eq!(g(200.0 / 3.0), "66.66666667");
        assert_eq!(g(12345678901.0), "12345678901");
        assert_eq!(g(123456789012.0), "1.23457E+11");
        assert_eq!(g(0.000012345), "1.2345E-05");
        assert_eq!(g(0.0001234), "0.0001234");
        assert_eq!(g(1e100), "1E+100");
        assert_eq!(g(-2e-20), "-2E-20");
        assert_eq!(f("General", 42.5), "42.5");
        assert_eq!(f("", 7.0), "7");
        assert_eq!(f("0", f64::NAN), "#NUM!");
        let (s, c) = fc("General;[Red]-General", -5.0);
        assert_eq!((s.as_str(), c), ("-5", Some(0xFF0000)));
    }

    #[test]
    fn integers_and_decimals() {
        assert_eq!(f("0", 1234.5), "1235");
        assert_eq!(f("0", 2.5), "3");
        assert_eq!(f("0", -2.5), "-3");
        assert_eq!(f("0", 0.4), "0");
        assert_eq!(f("0.00", std::f64::consts::PI), "3.14");
        assert_eq!(f("0.00", 2.675), "2.68");
        assert_eq!(f("0.00", 1.005), "1.01");
        assert_eq!(f("0.0", 0.15), "0.2");
        assert_eq!(f("0.00", -0.0), "0.00");
        assert_eq!(f("000", 5.0), "005");
        assert_eq!(f("#.##", 0.5), ".5");
        assert_eq!(f("#.##", 5.0), "5.");
        assert_eq!(f("#", 0.0), "");
        assert_eq!(f("0.0#", 1.5), "1.5");
        assert_eq!(f("0.0#", 1.567), "1.57");
        assert_eq!(f("?.??", 1.5), "1.5 ");
        assert_eq!(f("0", 1e20), "100000000000000000000");
        assert_eq!(f("0", 12345678901234567890.0), "12345678901234600000");
    }

    #[test]
    fn thousands_and_scaling() {
        assert_eq!(f("#,##0", 1234567.0), "1,234,567");
        assert_eq!(f("#,##0", 999.5), "1,000");
        assert_eq!(f("#,##0", 0.0), "0");
        assert_eq!(f("#,##0", -1234.0), "-1,234");
        assert_eq!(f("#,##0.00", 1234.567), "1,234.57");
        assert_eq!(f("#,##0.00", 1e15), "1,000,000,000,000,000.00");
        assert_eq!(f("#,##0,", 1234567.0), "1,235");
        assert_eq!(f("0.0,,\"M\"", 1234567.0), "1.2M");
        assert_eq!(f("0,\"K\"", 12500.0), "13K");
    }

    #[test]
    fn sections_and_colors() {
        let acc = "#,##0.00_);(#,##0.00)";
        assert_eq!(f(acc, 1234.5), "1,234.50 ");
        assert_eq!(f(acc, -1234.5), "(1,234.50)");
        assert_eq!(f(acc, 0.0), "0.00 ");
        let red = "#,##0.00_);[Red](#,##0.00)";
        assert_eq!(fc(red, -5.0), ("(5.00)".to_string(), Some(0xFF0000)));
        assert_eq!(fc(red, 5.0), ("5.00 ".to_string(), None));
        assert_eq!(f("0;(0);\"zero\"", 0.0), "zero");
        assert_eq!(f("0;(0);\"zero\"", -3.0), "(3)");
        assert_eq!(fc("#,##0.00;[Red]-#,##0.00", -5.0), ("-5.00".to_string(), Some(0xFF0000)));
        assert_eq!(fc("[Color10]0", 1.0).1, Some(0x008000));
        assert_eq!(fc("[Blue]0", 1.0).1, Some(0x0000FF));
        assert_eq!(fc("[Magenta]0", 1.0).1, Some(0xFF00FF));
        assert_eq!(fc("[Color56]0", 1.0).1, Some(0x333333));
        assert_eq!(f(";;;", 5.0), "");
        assert_eq!(ft(";;;", "x"), "");
    }

    #[test]
    fn conditions() {
        let c = "[Red][<=100]0;[Blue][>100]0";
        assert_eq!(fc(c, 50.0), ("50".to_string(), Some(0xFF0000)));
        assert_eq!(fc(c, 150.0), ("150".to_string(), Some(0x0000FF)));
        assert_eq!(f("[>=100]0.0;0.00", 150.0), "150.0");
        assert_eq!(f("[>=100]0.0;0.00", 5.0), "5.00");
        assert_eq!(f("[>=100]0.0;0.00", -5.0), "-5.00");
        assert_eq!(f("[>100]\"big\";[<=-5]\"neg\";0", 500.0), "big");
        assert_eq!(f("[>100]\"big\";[<=-5]\"neg\";0", -50.0), "neg");
        assert_eq!(f("[>100]\"big\";[<=-5]\"neg\";0", 3.0), "3");
        let phone = "[<=9999999]###-####;(###) ###-####";
        assert_eq!(f(phone, 5551234.0), "555-1234");
        assert_eq!(f(phone, 2125551234.0), "(212) 555-1234");
    }

    #[test]
    fn indian_grouping() {
        let c = r"[>=10000000]##\,##\,##\,##0;[>=100000]##\,##\,##0;##,##0";
        assert_eq!(f(c, 12345678.0), "1,23,45,678");
        assert_eq!(f(c, 1234567.0), "12,34,567");
        assert_eq!(f(c, 123456.0), "1,23,456");
        assert_eq!(f(c, 12345.0), "12,345");
        assert_eq!(f(c, 0.0), "0");
    }

    #[test]
    fn percent_and_scientific() {
        assert_eq!(f("0%", 0.256), "26%");
        assert_eq!(f("0.00%", 0.12345), "12.35%");
        assert_eq!(f("0.00%", 1.0), "100.00%");
        assert_eq!(f("0.00%", 0.07), "7.00%");
        assert_eq!(f(r"0\%", 5.0), "5%");
        assert_eq!(f("0.00E+00", 12345.678), "1.23E+04");
        assert_eq!(f("0.00E+00", 0.00012345), "1.23E-04");
        assert_eq!(f("0.00E+00", 0.0), "0.00E+00");
        assert_eq!(f("0.00E+00", -12345.0), "-1.23E+04");
        assert_eq!(f("0.00E+00", 1e100), "1.00E+100");
        assert_eq!(f("0.00E+00", 9.999), "1.00E+01");
        assert_eq!(f("0.0E-0", 12345.0), "1.2E4");
        assert_eq!(f("##0.0E+0", 12345.0), "12.3E+3");
        assert_eq!(f("##0.0E+0", 0.00012), "120.0E-6");
    }

    #[test]
    fn fractions() {
        assert_eq!(f("# ?/?", 1.25), "1 1/4");
        assert_eq!(f("# ?/?", 0.5), " 1/2");
        assert_eq!(f("# ?/?", 5.0), "5    ");
        assert_eq!(f("# ?/?", 0.333), " 1/3");
        assert_eq!(f("# ?/?", -1.25), "-1 1/4");
        assert_eq!(f("# ?/?", 1.99), "2    ");
        assert_eq!(f("# ??/??", std::f64::consts::PI), "3 14/99");
        assert_eq!(f("# ??/??", 1.5), "1  1/2 ");
        assert_eq!(f("?/8", 1.5), "12/8");
        assert_eq!(f("?/8", 0.3), "2/8");
        assert_eq!(f("# ?/10", 2.35), "2 4/10");
        assert_eq!(f("?/?", 0.75), "3/4");
    }

    #[test]
    fn literals() {
        assert_eq!(f("\"USD \"0.00", 5.0), "USD 5.00");
        assert_eq!(f("\"USD \"0.00", -5.0), "-USD 5.00");
        assert_eq!(f("0_)", 5.0), "5 ");
        assert_eq!(f("*-0", 5.0), "5");
        assert_eq!(f("$#,##0", 1234.0), "$1,234");
        assert_eq!(f("(0)", 1.0), "(1)");
        assert_eq!(f("[$₹-4009] #,##0.00", 1234567.891), "₹ 1,234,567.89");
        assert_eq!(f("[$$-409]#,##0", 5.0), "$5");
        assert_eq!(f("[$€-2] #,##0.00", 9.5), "€ 9.50");
        assert_eq!(f("#,##0 \"items\"", 3.0), "3 items");
    }

    #[test]
    fn accounting() {
        let c = builtin_code(44).unwrap();
        assert_eq!(c, r##"_("$"* #,##0.00_);_("$"* \(#,##0.00\);_("$"* "-"??_);_(@_)"##);
        assert_eq!(f(c, 1234.5), " $1,234.50 ");
        assert_eq!(f(c, -1234.5), " $(1,234.50)");
        assert_eq!(f(c, 0.0), " $-   ");
        assert_eq!(ft(c, "abc"), " abc ");
        assert_eq!(f(builtin_code(5).unwrap(), -1234.0), "($1,234)");
    }

    #[test]
    fn text() {
        assert_eq!(ft("@", "hello"), "hello");
        assert_eq!(f("@", 12.5), "12.5");
        assert!(NumFmt::parse("@").is_text());
        assert_eq!(ft("0;0;0;\"t: \"@", "abc"), "t: abc");
        assert_eq!(ft("0.00", "abc"), "abc");
        assert_eq!(ft("0.00;@\" kg\"", "5"), "5 kg");
        assert_eq!(f("0.00;@\" kg\"", -5.0), "-5.00");
        let mut s = String::new();
        assert_eq!(NumFmt::parse("[Blue]@").format_text("x", &mut s), Some(0x0000FF));
    }

    #[test]
    fn dates() {
        // 45000 = 2023-03-15 (Wednesday)
        assert_eq!(f("m/d/yyyy", 45000.0), "3/15/2023");
        assert_eq!(f("d-mmm-yy", 45000.0), "15-Mar-23");
        assert_eq!(f("dd/mm/yyyy", 45000.0), "15/03/2023");
        assert_eq!(f("yyyy-mm-dd hh:mm:ss", 45000.5), "2023-03-15 12:00:00");
        assert_eq!(f("mmm d, yyyy", 45000.0), "Mar 15, 2023");
        assert_eq!(f("dddd, mmmm d", 45000.0), "Wednesday, March 15");
        assert_eq!(f("ddd mmmmm", 45000.0), "Wed M");
        assert_eq!(f("e", 45000.0), "2023");
        assert_eq!(f("yy", 45000.0), "23");
        assert_eq!(f("m/d/yyyy", 60.0), "2/29/1900");
        assert_eq!(f("m/d/yyyy", 61.0), "3/1/1900");
        assert_eq!(f("m/d/yyyy", 1.0), "1/1/1900");
        assert_eq!(f("m/d/yyyy", 0.0), "1/0/1900");
        assert_eq!(f("dddd", 1.0), "Sunday");
        assert_eq!(f("m/d/yyyy", -1.0), HASHES);
        assert_eq!(f("m/d/yyyy", 3e6), HASHES);
        let mut s = String::new();
        NumFmt::parse("m/d/yyyy").format_number(0.0, true, &mut s);
        assert_eq!(s, "1/1/1904");
        s.clear();
        NumFmt::parse("dddd").format_number(0.0, true, &mut s);
        assert_eq!(s, "Friday");
        assert_eq!(f("m/d/yyyy h:mm", 45000.75), "3/15/2023 18:00");
    }

    #[test]
    fn times() {
        assert_eq!(f("h:mm AM/PM", 0.75), "6:00 PM");
        assert_eq!(f("h:mm AM/PM", 0.0), "12:00 AM");
        assert_eq!(f("h:mm am/pm", 0.5), "12:00 pm");
        assert_eq!(f("h:mm:ss A/P", 0.25), "6:00:00 A");
        assert_eq!(f("[h]:mm:ss", 1.5), "36:00:00");
        assert_eq!(f("[h]:mm", 2.5), "60:00");
        assert_eq!(f("[mm]:ss", 1.0 / 24.0), "60:00");
        assert_eq!(f("[ss]", 1.0 / 1440.0), "60");
        assert_eq!(f("h:mm", 0.5 + 5.0 / 1440.0), "12:05");
        assert_eq!(f("mm:ss", 2.5 / 1440.0), "02:30");
        assert_eq!(f("h:mm:ss.00", 0.5 + 1.5 / 86400.0), "12:00:01.50");
        assert_eq!(f("mmss.0", 61.3 / 86400.0), "0101.3");
        assert_eq!(f("hh:mm:ss", 0.999_999_999), "00:00:00");
        assert_eq!(f("h:mm", 10.0 / 24.0 + 29.5 / 1440.0), "10:29");
        assert_eq!(f("[$-409]h:mm:ss AM/PM", 0.5), "12:00:00 PM");
        assert_eq!(f("yyyy mm", 45000.0), "2023 03");
    }

    #[test]
    fn serial_conversion() {
        assert_eq!(serial_to_datetime(45000.25, false), Some((2023, 3, 15, 6, 0, 0, 0)));
        assert_eq!(serial_to_datetime(60.0, false), Some((1900, 2, 29, 0, 0, 0, 0)));
        assert_eq!(serial_to_datetime(59.0, false), Some((1900, 2, 28, 0, 0, 0, 0)));
        assert_eq!(serial_to_datetime(0.0, true), Some((1904, 1, 1, 0, 0, 0, 0)));
        assert_eq!(serial_to_datetime(0.5 + 1.5 / 86400.0, false), Some((1900, 1, 0, 12, 0, 1, 500)));
        assert_eq!(serial_to_datetime(2958465.0, false), Some((9999, 12, 31, 0, 0, 0, 0)));
        assert_eq!(serial_to_datetime(-1.0, false), None);
        assert_eq!(datetime_to_serial(2023, 3, 15, 0, 0, 0, false), Some(45000.0));
        assert_eq!(datetime_to_serial(2023, 3, 15, 6, 0, 0, false), Some(45000.25));
        assert_eq!(datetime_to_serial(1900, 2, 29, 0, 0, 0, false), Some(60.0));
        assert_eq!(datetime_to_serial(1900, 3, 1, 0, 0, 0, false), Some(61.0));
        assert_eq!(datetime_to_serial(1900, 1, 1, 0, 0, 0, false), Some(1.0));
        assert_eq!(datetime_to_serial(1904, 1, 2, 0, 0, 0, true), Some(1.0));
        assert_eq!(datetime_to_serial(2023, 2, 29, 0, 0, 0, false), None);
        assert_eq!(datetime_to_serial(1899, 12, 31, 0, 0, 0, false), None);
        assert_eq!(datetime_to_serial(2024, 2, 29, 0, 0, 0, false), Some(45351.0));
        for serial in [1.0, 59.0, 61.0, 1000.0, 45000.0, 2958465.0] {
            let (y, m, d, ..) = serial_to_datetime(serial, false).unwrap();
            assert_eq!(datetime_to_serial(y, m, d, 0, 0, 0, false), Some(serial));
            if let Some((y, m, d, ..)) = serial_to_datetime(serial, true) {
                assert_eq!(datetime_to_serial(y, m, d, 0, 0, 0, true), Some(serial));
            }
        }
    }

    #[test]
    fn predicates_and_builtins() {
        assert!(NumFmt::parse("m/d/yyyy").is_date());
        assert!(NumFmt::parse("[h]:mm").is_date());
        assert!(!NumFmt::parse("0.00").is_date());
        assert!(!NumFmt::parse("\"d\"0").is_date());
        assert!(NumFmt::parse("General").is_general());
        assert!(NumFmt::parse("").is_general());
        assert!(NumFmt::general().is_general());
        assert!(!NumFmt::parse("0").is_general());
        assert!(NumFmt::parse("0%").is_percent());
        assert!(!NumFmt::parse("0").is_percent());
        assert_eq!(NumFmt::parse("#,##0.00").decimals(), 2);
        assert_eq!(NumFmt::parse("0").decimals(), 0);
        assert_eq!(NumFmt::parse("0.000%").decimals(), 3);
        assert_eq!(builtin_code(0), Some("General"));
        assert_eq!(builtin_code(14), Some("m/d/yyyy"));
        assert_eq!(builtin_code(49), Some("@"));
        assert_eq!(builtin_code(23), None);
        assert_eq!(builtin_code(50), None);
        for id in 0..50 {
            if let Some(c) = builtin_code(id) {
                let mut s = String::new();
                NumFmt::parse(c).format_number(45000.5, false, &mut s);
                assert!(!s.is_empty(), "id {id}");
            }
        }
        assert_eq!(f(builtin_code(22).unwrap(), 45000.5), "3/15/2023 12:00");
        assert_eq!(f(builtin_code(18).unwrap(), 45000.5), "12:00 PM");
        assert_eq!(f(builtin_code(47).unwrap(), 45000.5), "0000.0");
        assert_eq!(f(builtin_code(37).unwrap(), -1234.0), "(1,234)");
        assert!(NumFmt::builtin(14).is_date());
    }

    #[test]
    fn reuse_appends() {
        let nf = NumFmt::parse("#,##0.00");
        let mut s = String::from("x=");
        nf.format_number(1.0, false, &mut s);
        nf.format_number(2.0, false, &mut s);
        assert_eq!(s, "x=1.002.00");
    }
}
