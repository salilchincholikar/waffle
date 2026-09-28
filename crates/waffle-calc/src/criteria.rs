//! Wildcard patterns and *IF/*IFS criteria.

use std::cmp::Ordering;

use super::value::*;

#[derive(Clone, Debug, PartialEq)]
enum PTok {
    Lit(char),
    One,
    Any,
}

/// Case-insensitive wildcard pattern (`*`, `?`, `~` escape).
#[derive(Clone, Debug)]
pub(crate) struct Pattern {
    toks: Vec<PTok>,
}

pub(crate) fn has_wildcards(s: &str) -> bool {
    s.contains(['*', '?', '~'])
}

fn lower(c: char) -> char {
    if c.is_ascii() { c.to_ascii_lowercase() } else { c.to_lowercase().next().unwrap_or(c) }
}

impl Pattern {
    pub fn new(p: &str) -> Pattern {
        let mut toks = Vec::new();
        let mut it = p.chars();
        while let Some(c) = it.next() {
            match c {
                '~' => match it.next() {
                    Some(n) => toks.push(PTok::Lit(lower(n))),
                    None => toks.push(PTok::Lit('~')),
                },
                '*' => {
                    if toks.last() != Some(&PTok::Any) {
                        toks.push(PTok::Any)
                    }
                }
                '?' => toks.push(PTok::One),
                c => toks.push(PTok::Lit(lower(c))),
            }
        }
        Pattern { toks }
    }

    /// Pattern that matches when it matches a prefix (for SEARCH).
    pub fn prefix(p: &str) -> Pattern {
        let mut pat = Pattern::new(p);
        if pat.toks.last() != Some(&PTok::Any) {
            pat.toks.push(PTok::Any);
        }
        pat
    }

    pub fn matches(&self, s: &str) -> bool {
        if s.is_ascii() {
            let b = s.as_bytes();
            return self.run(b.len(), |i| b[i].to_ascii_lowercase() as char);
        }
        let text: Vec<char> = s.chars().map(lower).collect();
        self.matches_chars(&text)
    }

    /// Matches pre-lowercased characters.
    pub fn matches_chars(&self, text: &[char]) -> bool {
        self.run(text.len(), |i| text[i])
    }

    fn run(&self, n: usize, text: impl Fn(usize) -> char) -> bool {
        let p = &self.toks;
        let (mut i, mut j) = (0usize, 0usize);
        let mut star: Option<(usize, usize)> = None;
        while i < n {
            if j < p.len() {
                match &p[j] {
                    PTok::Any => {
                        star = Some((j, i));
                        j += 1;
                        continue;
                    }
                    PTok::One => {
                        i += 1;
                        j += 1;
                        continue;
                    }
                    PTok::Lit(c) if *c == text(i) => {
                        i += 1;
                        j += 1;
                        continue;
                    }
                    _ => {}
                }
            }
            match star {
                Some((sj, si)) => {
                    j = sj + 1;
                    i = si + 1;
                    star = Some((sj, si + 1));
                }
                None => return false,
            }
        }
        while j < p.len() && p[j] == PTok::Any {
            j += 1;
        }
        j == p.len()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl Op {
    fn test(self, o: Ordering) -> bool {
        match self {
            Op::Eq => o == Ordering::Equal,
            Op::Ne => o != Ordering::Equal,
            Op::Lt => o == Ordering::Less,
            Op::Le => o != Ordering::Greater,
            Op::Gt => o == Ordering::Greater,
            Op::Ge => o != Ordering::Less,
        }
    }
}

#[derive(Clone, Debug)]
enum Kind {
    Num(f64),
    Text(String, Option<Pattern>),
    Bool(bool),
    Err(ErrorKind),
    /// `""`: empty cells and empty strings.
    Blank,
    /// `"="`: truly empty cells only.
    BlankStrict,
    /// `"<>"`: non-empty cells.
    NonBlank,
}

/// A parsed criterion such as `">=10"`, `"a*"`, `"<>"`.
#[derive(Clone, Debug)]
pub(crate) struct Crit {
    op: Op,
    kind: Kind,
}

impl Crit {
    pub fn parse(v: &Value) -> Crit {
        match v {
            Value::Number(n) => Crit { op: Op::Eq, kind: Kind::Num(*n) },
            Value::Bool(b) => Crit { op: Op::Eq, kind: Kind::Bool(*b) },
            Value::Empty => Crit { op: Op::Eq, kind: Kind::Num(0.0) },
            Value::Error(e) => Crit { op: Op::Eq, kind: Kind::Err(*e) },
            Value::Array(_) => Crit::parse(&v.top_left()),
            Value::Text(t) => Crit::parse_text(t),
        }
    }

    fn parse_text(t: &str) -> Crit {
        let (op, rest, explicit) = if let Some(r) = t.strip_prefix("<=") {
            (Op::Le, r, true)
        } else if let Some(r) = t.strip_prefix(">=") {
            (Op::Ge, r, true)
        } else if let Some(r) = t.strip_prefix("<>") {
            (Op::Ne, r, true)
        } else if let Some(r) = t.strip_prefix('<') {
            (Op::Lt, r, true)
        } else if let Some(r) = t.strip_prefix('>') {
            (Op::Gt, r, true)
        } else if let Some(r) = t.strip_prefix('=') {
            (Op::Eq, r, true)
        } else {
            (Op::Eq, t, false)
        };
        if rest.is_empty() {
            let kind = match (op, explicit) {
                (Op::Eq, false) => Kind::Blank,
                (Op::Eq, true) => Kind::BlankStrict,
                (Op::Ne, _) => Kind::NonBlank,
                _ => Kind::Text(String::new(), None),
            };
            return Crit { op, kind };
        }
        if let Some(n) = parse_num_text(rest) {
            return Crit { op, kind: Kind::Num(n) };
        }
        if rest.eq_ignore_ascii_case("TRUE") {
            return Crit { op, kind: Kind::Bool(true) };
        }
        if rest.eq_ignore_ascii_case("FALSE") {
            return Crit { op, kind: Kind::Bool(false) };
        }
        if let Some(e) = ErrorKind::from_str(rest) {
            return Crit { op, kind: Kind::Err(e) };
        }
        let pat = if matches!(op, Op::Eq | Op::Ne) && has_wildcards(rest) { Some(Pattern::new(rest)) } else { None };
        Crit { op, kind: Kind::Text(rest.to_string(), pat) }
    }

    pub fn matches(&self, v: &Value) -> bool {
        match &self.kind {
            Kind::Blank => match v {
                Value::Empty => true,
                Value::Text(t) => t.is_empty(),
                _ => false,
            },
            Kind::BlankStrict => matches!(v, Value::Empty),
            Kind::NonBlank => !matches!(v, Value::Empty),
            Kind::Num(n) => {
                let o = match v {
                    Value::Number(x) => Some(cmp_num(*x, *n)),
                    Value::Text(t) if self.op == Op::Eq || self.op == Op::Ne => parse_plain_number(t.trim()).map(|x| cmp_num(x, *n)),
                    _ => None,
                };
                match o {
                    Some(o) => self.op.test(o),
                    None => self.op == Op::Ne,
                }
            }
            Kind::Bool(b) => match v {
                Value::Bool(x) => self.op.test(x.cmp(b)),
                _ => self.op == Op::Ne,
            },
            Kind::Err(e) => match v {
                Value::Error(x) => (x == e) == (self.op == Op::Eq) || !matches!(self.op, Op::Eq | Op::Ne),
                _ => self.op == Op::Ne,
            },
            Kind::Text(s, pat) => {
                let t: &str = match v {
                    Value::Text(t) => t,
                    _ => return self.op == Op::Ne,
                };
                match self.op {
                    Op::Eq | Op::Ne => {
                        let eq = match pat {
                            Some(p) => p.matches(t),
                            None => text_eq(t, s),
                        };
                        eq == (self.op == Op::Eq)
                    }
                    _ => self.op.test(cmp_text(t, s)),
                }
            }
        }
    }
}
