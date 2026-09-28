//! Tokenizer and precedence-climbing parser producing the compiled AST.

use std::rc::Rc;

use super::funcs::{self, Func};
use super::value::{ErrorKind, Value};
use super::{MAX_COL, MAX_ROW};

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SheetSpec {
    Host,
    One(Rc<str>),
    Span(Rc<str>, Rc<str>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RefKind {
    Cell,
    Area,
    Cols,
    Rows,
}

/// A resolved A1 reference (0-based, inclusive).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RefExpr {
    pub sheet: SheetSpec,
    pub kind: RefKind,
    pub r0: u32,
    pub c0: u32,
    pub r1: u32,
    pub c1: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
    Concat,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Range,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UnOp {
    Neg,
    Plus,
    Pct,
}

#[derive(Clone, Debug)]
pub(crate) enum Expr {
    Num(f64),
    Str(Rc<str>),
    Bool(bool),
    Err(ErrorKind),
    Array(Rc<Vec<Vec<Value>>>),
    /// An omitted function argument (`IF(A1,,1)`).
    Missing,
    Ref(Box<RefExpr>),
    Name(Rc<str>),
    Unary(UnOp, Box<Expr>),
    Bin(BinOp, Box<(Expr, Expr)>),
    Call(Func, Box<[Expr]>),
    /// Unknown / unsupported function (evaluates to #NAME?).
    Unknown(Rc<str>, Box<[Expr]>),
    /// Structured or external reference (evaluates to #REF!).
    BadRef,
}

#[derive(Clone, Debug)]
enum Tok {
    Num(f64),
    Str(Rc<str>),
    Err(ErrorKind),
    Bool(bool),
    Name(String),
    Func(String),
    Ref(RefExpr),
    BadRef,
    Op(BinOp),
    Pct,
    LParen,
    RParen,
    LBrace,
    RBrace,
    Comma,
    Semi,
    Colon,
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '.' || c == '\\' || (c as u32) > 127
}

struct Lexer<'a> {
    s: &'a [char],
    i: usize,
    out: Vec<Tok>,
}

impl Lexer<'_> {
    fn peek(&self, k: usize) -> Option<char> {
        self.s.get(self.i + k).copied()
    }
    fn at(&self, j: usize) -> Option<char> {
        self.s.get(j).copied()
    }

    /// `$?[A-Z]{1,3}` not followed by a letter/digit → (col, end).
    fn col_at(&self, mut j: usize) -> Option<(u32, usize)> {
        if self.at(j) == Some('$') {
            j += 1;
        }
        let st = j;
        let mut c: u32 = 0;
        while let Some(ch) = self.at(j).filter(|c| c.is_ascii_alphabetic()) {
            c = c * 26 + (ch.to_ascii_uppercase() as u32 - 'A' as u32 + 1);
            j += 1;
            if j - st > 3 {
                return None;
            }
        }
        if j == st || c - 1 > MAX_COL {
            return None;
        }
        Some((c - 1, j))
    }

    fn row_at(&self, mut j: usize) -> Option<(u32, usize)> {
        if self.at(j) == Some('$') {
            j += 1;
        }
        let st = j;
        let mut r: u64 = 0;
        while let Some(ch) = self.at(j).filter(|c| c.is_ascii_digit()) {
            r = r * 10 + (ch as u64 - '0' as u64);
            j += 1;
            if j - st > 7 {
                return None;
            }
        }
        if j == st || r == 0 || r - 1 > MAX_ROW as u64 {
            return None;
        }
        Some(((r - 1) as u32, j))
    }

    fn cell_at(&self, j: usize) -> Option<(u32, u32, usize)> {
        let (c, j) = self.col_at(j)?;
        let (r, j) = self.row_at(j)?;
        Some((r, c, j))
    }

    fn boundary(&self, j: usize) -> bool {
        match self.at(j) {
            None => true,
            Some(c) => !(is_word(c) || c == '(' || c == '$' || c == '!' || c == '['),
        }
    }

    /// Tries to read an A1 reference starting at `j`.
    fn ref_at(&self, j: usize, sheet: &SheetSpec) -> Option<(RefExpr, usize)> {
        let mk = |kind, r0: u32, c0: u32, r1: u32, c1: u32| RefExpr {
            sheet: sheet.clone(),
            kind,
            r0: r0.min(r1),
            c0: c0.min(c1),
            r1: r0.max(r1),
            c1: c0.max(c1),
        };
        if let Some((r0, c0, p)) = self.cell_at(j) {
            if self.at(p) == Some(':')
                && let Some((r1, c1, q)) = self.cell_at(p + 1)
                && self.boundary(q)
            {
                return Some((mk(RefKind::Area, r0, c0, r1, c1), q));
            }
            if self.boundary(p) {
                return Some((mk(RefKind::Cell, r0, c0, r0, c0), p));
            }
        }
        if let Some((c0, p)) = self.col_at(j)
            && self.at(p) == Some(':')
            && let Some((c1, q)) = self.col_at(p + 1)
            && self.boundary(q)
            && !self.at(p - 1).is_some_and(|c| c.is_ascii_digit())
        {
            return Some((mk(RefKind::Cols, 0, c0, MAX_ROW, c1), q));
        }
        if let Some((r0, p)) = self.row_at(j)
            && self.at(p) == Some(':')
            && let Some((r1, q)) = self.row_at(p + 1)
            && self.boundary(q)
        {
            return Some((mk(RefKind::Rows, r0, 0, r1, MAX_COL), q));
        }
        None
    }

    fn word_end(&self, mut j: usize) -> usize {
        while self.at(j).is_some_and(is_word) {
            j += 1;
        }
        j
    }

    fn text(&self, a: usize, b: usize) -> String {
        self.s[a..b].iter().collect()
    }

    /// After `Sheet!`: a reference, `#REF!`, or a sheet-scoped name.
    fn ref_body(&mut self, sheet: SheetSpec) -> Result<(), String> {
        if self.peek(0) == Some('#') {
            let rest: String = self.s[self.i..].iter().take(5).collect();
            if rest.eq_ignore_ascii_case("#REF!") {
                self.i += 5;
                self.out.push(Tok::Err(ErrorKind::Ref));
                return Ok(());
            }
        }
        if let Some((r, end)) = self.ref_at(self.i, &sheet) {
            self.i = end;
            self.out.push(Tok::Ref(r));
            return Ok(());
        }
        let e = self.word_end(self.i);
        if e > self.i {
            let w = self.text(self.i, e);
            self.i = e;
            self.out.push(Tok::Name(w));
            return Ok(());
        }
        Err(format!("bad reference at {}", self.i))
    }

    /// Consumes an external (`[1]Sheet1!A1`) or structured (`T[Col]`) ref.
    fn bad_ref(&mut self) {
        let mut depth = 0i32;
        let mut quoted = false;
        while let Some(c) = self.peek(0) {
            if quoted {
                if c == '\'' {
                    quoted = false;
                }
                self.i += 1;
                continue;
            }
            match c {
                '[' => depth += 1,
                ']' => depth -= 1,
                '\'' if depth == 0 => quoted = true,
                _ if depth > 0 => {}
                c if is_word(c) || c == '!' || c == '$' || c == ':' || c == '#' => {}
                _ => break,
            }
            self.i += 1;
        }
        self.out.push(Tok::BadRef);
    }

    fn run(&mut self) -> Result<(), String> {
        while let Some(c) = self.peek(0) {
            match c {
                ' ' | '\t' | '\r' | '\n' | '@' => self.i += 1,
                '"' => {
                    let mut s = String::new();
                    self.i += 1;
                    loop {
                        match self.peek(0) {
                            None => return Err("unterminated string".into()),
                            Some('"') if self.peek(1) == Some('"') => {
                                s.push('"');
                                self.i += 2;
                            }
                            Some('"') => {
                                self.i += 1;
                                break;
                            }
                            Some(ch) => {
                                s.push(ch);
                                self.i += 1;
                            }
                        }
                    }
                    self.out.push(Tok::Str(Rc::from(s)));
                }
                '#' => {
                    const ERRS: [&str; 10] =
                        ["#GETTING_DATA", "#DIV/0!", "#VALUE!", "#SPILL!", "#CALC!", "#NULL!", "#NAME?", "#REF!", "#NUM!", "#N/A"];
                    let rest: String = self.s[self.i..].iter().take(13).collect::<String>().to_ascii_uppercase();
                    let e = ERRS.iter().find(|e| rest.starts_with(*e)).ok_or("bad error literal")?;
                    self.i += e.chars().count();
                    self.out.push(Tok::Err(ErrorKind::from_str(e).unwrap_or(ErrorKind::Value)));
                }
                '\'' => {
                    let mut name = String::new();
                    self.i += 1;
                    loop {
                        match self.peek(0) {
                            None => return Err("unterminated sheet name".into()),
                            Some('\'') if self.peek(1) == Some('\'') => {
                                name.push('\'');
                                self.i += 2;
                            }
                            Some('\'') => {
                                self.i += 1;
                                break;
                            }
                            Some(ch) => {
                                name.push(ch);
                                self.i += 1;
                            }
                        }
                    }
                    if self.peek(0) != Some('!') {
                        return Err("expected ! after sheet name".into());
                    }
                    self.i += 1;
                    if name.starts_with('[') {
                        // external workbook reference
                        self.i -= 1;
                        self.bad_ref();
                        continue;
                    }
                    let spec = match name.split_once(':') {
                        Some((a, b)) => SheetSpec::Span(Rc::from(a), Rc::from(b)),
                        None => SheetSpec::One(Rc::from(name.as_str())),
                    };
                    self.ref_body(spec)?;
                }
                '[' => self.bad_ref(),
                '{' => {
                    self.out.push(Tok::LBrace);
                    self.i += 1
                }
                '}' => {
                    self.out.push(Tok::RBrace);
                    self.i += 1
                }
                '(' => {
                    self.out.push(Tok::LParen);
                    self.i += 1
                }
                ')' => {
                    self.out.push(Tok::RParen);
                    self.i += 1
                }
                ',' => {
                    self.out.push(Tok::Comma);
                    self.i += 1
                }
                ';' => {
                    self.out.push(Tok::Semi);
                    self.i += 1
                }
                ':' => {
                    self.out.push(Tok::Colon);
                    self.i += 1
                }
                '%' => {
                    self.out.push(Tok::Pct);
                    self.i += 1
                }
                '+' | '-' | '*' | '/' | '^' | '&' | '=' => {
                    self.out.push(Tok::Op(match c {
                        '+' => BinOp::Add,
                        '-' => BinOp::Sub,
                        '*' => BinOp::Mul,
                        '/' => BinOp::Div,
                        '^' => BinOp::Pow,
                        '&' => BinOp::Concat,
                        _ => BinOp::Eq,
                    }));
                    self.i += 1;
                }
                '<' => {
                    let op = match self.peek(1) {
                        Some('=') => BinOp::Le,
                        Some('>') => BinOp::Ne,
                        _ => BinOp::Lt,
                    };
                    self.i += if op == BinOp::Lt { 1 } else { 2 };
                    self.out.push(Tok::Op(op));
                }
                '>' => {
                    let op = if self.peek(1) == Some('=') { BinOp::Ge } else { BinOp::Gt };
                    self.i += if op == BinOp::Gt { 1 } else { 2 };
                    self.out.push(Tok::Op(op));
                }
                '$' => match self.ref_at(self.i, &SheetSpec::Host) {
                    Some((r, e)) => {
                        self.i = e;
                        self.out.push(Tok::Ref(r));
                    }
                    None => return Err("bad $ reference".into()),
                },
                c if c.is_ascii_digit() || c == '.' => {
                    if c != '.'
                        && let Some((r, e)) = self.ref_at(self.i, &SheetSpec::Host)
                        && r.kind == RefKind::Rows
                    {
                        self.i = e;
                        self.out.push(Tok::Ref(r));
                        continue;
                    }
                    // unquoted sheet names may start with a digit only if quoted; plain number
                    let st = self.i;
                    while self.peek(0).is_some_and(|c| c.is_ascii_digit()) {
                        self.i += 1;
                    }
                    if self.peek(0) == Some('.') {
                        self.i += 1;
                        while self.peek(0).is_some_and(|c| c.is_ascii_digit()) {
                            self.i += 1;
                        }
                    }
                    if matches!(self.peek(0), Some('e' | 'E')) {
                        let save = self.i;
                        self.i += 1;
                        if matches!(self.peek(0), Some('+' | '-')) {
                            self.i += 1;
                        }
                        if self.peek(0).is_some_and(|c| c.is_ascii_digit()) {
                            while self.peek(0).is_some_and(|c| c.is_ascii_digit()) {
                                self.i += 1;
                            }
                        } else {
                            self.i = save;
                        }
                    }
                    let t = self.text(st, self.i);
                    let n: f64 = t.parse().map_err(|_| format!("bad number {t}"))?;
                    self.out.push(Tok::Num(n));
                }
                c if is_word(c) => {
                    let st = self.i;
                    let e = self.word_end(st);
                    // Sheet!ref or Sheet1:Sheet3!ref
                    if self.at(e) == Some('!') {
                        let w = self.text(st, e);
                        self.i = e + 1;
                        self.ref_body(SheetSpec::One(Rc::from(w.as_str())))?;
                        continue;
                    }
                    if self.at(e) == Some(':') {
                        let e2 = self.word_end(e + 1);
                        if e2 > e + 1 && self.at(e2) == Some('!') {
                            let a = self.text(st, e);
                            let b = self.text(e + 1, e2);
                            self.i = e2 + 1;
                            self.ref_body(SheetSpec::Span(Rc::from(a.as_str()), Rc::from(b.as_str())))?;
                            continue;
                        }
                    }
                    if self.at(e) == Some('(') {
                        self.out.push(Tok::Func(self.text(st, e)));
                        self.i = e;
                        continue;
                    }
                    if self.at(e) == Some('[') {
                        self.bad_ref();
                        continue;
                    }
                    if let Some((r, end)) = self.ref_at(st, &SheetSpec::Host) {
                        self.i = end;
                        self.out.push(Tok::Ref(r));
                        continue;
                    }
                    let w = self.text(st, e);
                    self.i = e;
                    if w.eq_ignore_ascii_case("TRUE") {
                        self.out.push(Tok::Bool(true));
                    } else if w.eq_ignore_ascii_case("FALSE") {
                        self.out.push(Tok::Bool(false));
                    } else {
                        self.out.push(Tok::Name(w));
                    }
                }
                other => return Err(format!("unexpected character '{other}'")),
            }
        }
        Ok(())
    }
}

struct Parser {
    toks: Vec<Tok>,
    p: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.p)
    }
    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.p).cloned();
        self.p += 1;
        t
    }
    fn peek_op(&self) -> Option<BinOp> {
        match self.peek() {
            Some(Tok::Op(o)) => Some(*o),
            _ => None,
        }
    }

    fn expr(&mut self) -> Result<Expr, String> {
        let mut l = self.concat()?;
        while let Some(op @ (BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge)) = self.peek_op() {
            self.p += 1;
            let r = self.concat()?;
            l = Expr::Bin(op, Box::new((l, r)));
        }
        Ok(l)
    }
    fn concat(&mut self) -> Result<Expr, String> {
        let mut l = self.additive()?;
        while self.peek_op() == Some(BinOp::Concat) {
            self.p += 1;
            let r = self.additive()?;
            l = Expr::Bin(BinOp::Concat, Box::new((l, r)));
        }
        Ok(l)
    }
    fn additive(&mut self) -> Result<Expr, String> {
        let mut l = self.term()?;
        while let Some(op @ (BinOp::Add | BinOp::Sub)) = self.peek_op() {
            self.p += 1;
            let r = self.term()?;
            l = Expr::Bin(op, Box::new((l, r)));
        }
        Ok(l)
    }
    fn term(&mut self) -> Result<Expr, String> {
        let mut l = self.power()?;
        while let Some(op @ (BinOp::Mul | BinOp::Div)) = self.peek_op() {
            self.p += 1;
            let r = self.power()?;
            l = Expr::Bin(op, Box::new((l, r)));
        }
        Ok(l)
    }
    fn power(&mut self) -> Result<Expr, String> {
        let mut l = self.unary()?;
        while self.peek_op() == Some(BinOp::Pow) {
            self.p += 1;
            let r = self.unary()?;
            l = Expr::Bin(BinOp::Pow, Box::new((l, r)));
        }
        Ok(l)
    }
    fn unary(&mut self) -> Result<Expr, String> {
        match self.peek_op() {
            Some(BinOp::Sub) => {
                self.p += 1;
                let e = self.unary()?;
                Ok(match e {
                    Expr::Num(n) => Expr::Num(-n),
                    e => Expr::Unary(UnOp::Neg, Box::new(e)),
                })
            }
            Some(BinOp::Add) => {
                self.p += 1;
                let e = self.unary()?;
                Ok(Expr::Unary(UnOp::Plus, Box::new(e)))
            }
            _ => self.postfix(),
        }
    }
    fn postfix(&mut self) -> Result<Expr, String> {
        let mut e = self.range()?;
        while matches!(self.peek(), Some(Tok::Pct)) {
            self.p += 1;
            e = Expr::Unary(UnOp::Pct, Box::new(e));
        }
        Ok(e)
    }
    fn range(&mut self) -> Result<Expr, String> {
        let mut l = self.primary()?;
        while matches!(self.peek(), Some(Tok::Colon)) {
            self.p += 1;
            let r = self.primary()?;
            l = Expr::Bin(BinOp::Range, Box::new((l, r)));
        }
        Ok(l)
    }

    fn primary(&mut self) -> Result<Expr, String> {
        match self.next() {
            Some(Tok::Num(n)) => Ok(Expr::Num(n)),
            Some(Tok::Str(s)) => Ok(Expr::Str(s)),
            Some(Tok::Bool(b)) => Ok(Expr::Bool(b)),
            Some(Tok::Err(e)) => Ok(Expr::Err(e)),
            Some(Tok::Ref(r)) => Ok(Expr::Ref(Box::new(r))),
            Some(Tok::BadRef) => Ok(Expr::BadRef),
            Some(Tok::Name(n)) => Ok(Expr::Name(Rc::from(n.as_str()))),
            Some(Tok::LParen) => {
                let e = self.expr()?;
                match self.next() {
                    Some(Tok::RParen) => Ok(e),
                    _ => Err("expected )".into()),
                }
            }
            Some(Tok::LBrace) => self.array(),
            Some(Tok::Func(name)) => self.call(name),
            t => Err(format!("unexpected token {t:?}")),
        }
    }

    fn call(&mut self, name: String) -> Result<Expr, String> {
        if !matches!(self.next(), Some(Tok::LParen)) {
            return Err("expected (".into());
        }
        let mut args = Vec::new();
        if matches!(self.peek(), Some(Tok::RParen)) {
            self.p += 1;
        } else {
            loop {
                if matches!(self.peek(), Some(Tok::Comma | Tok::RParen)) {
                    args.push(Expr::Missing);
                } else {
                    args.push(self.expr()?);
                }
                match self.next() {
                    Some(Tok::Comma) => continue,
                    Some(Tok::RParen) => break,
                    _ => return Err("expected , or )".into()),
                }
            }
        }
        let mut up = name.to_ascii_uppercase();
        while let Some(r) = up.strip_prefix("_XLFN.").or_else(|| up.strip_prefix("_XLWS.")) {
            up = r.to_string();
        }
        Ok(match funcs::lookup(&up) {
            Some(f) => Expr::Call(f, args.into_boxed_slice()),
            None => Expr::Unknown(Rc::from(up.as_str()), args.into_boxed_slice()),
        })
    }

    fn array(&mut self) -> Result<Expr, String> {
        let mut rows: Vec<Vec<Value>> = vec![Vec::new()];
        loop {
            let neg = match self.peek() {
                Some(Tok::Op(BinOp::Sub)) => {
                    self.p += 1;
                    true
                }
                Some(Tok::Op(BinOp::Add)) => {
                    self.p += 1;
                    false
                }
                _ => false,
            };
            let v = match self.next() {
                Some(Tok::Num(n)) => Value::Number(if neg { -n } else { n }),
                Some(Tok::Str(s)) if !neg => Value::Text(s),
                Some(Tok::Bool(b)) if !neg => Value::Bool(b),
                Some(Tok::Err(e)) if !neg => Value::Error(e),
                t => return Err(format!("bad array element {t:?}")),
            };
            rows.last_mut().expect("row").push(v);
            match self.next() {
                Some(Tok::Comma) => {}
                Some(Tok::Semi) => rows.push(Vec::new()),
                Some(Tok::RBrace) => break,
                _ => return Err("bad array constant".into()),
            }
        }
        let w = rows[0].len();
        if rows.iter().any(|r| r.len() != w) {
            return Err("ragged array constant".into());
        }
        Ok(Expr::Array(Rc::new(rows)))
    }
}

/// Parses formula text (storage form, optional leading `=`).
pub(crate) fn parse(text: &str) -> Result<Expr, String> {
    let t = text.trim();
    let t = t.strip_prefix('=').unwrap_or(t);
    let chars: Vec<char> = t.chars().collect();
    let mut lx = Lexer { s: &chars, i: 0, out: Vec::new() };
    lx.run()?;
    if lx.out.is_empty() {
        return Err("empty formula".into());
    }
    let mut p = Parser { toks: lx.out, p: 0 };
    let e = p.expr()?;
    if p.p != p.toks.len() {
        return Err(format!("unexpected trailing token {:?}", p.toks[p.p]));
    }
    Ok(e)
}
