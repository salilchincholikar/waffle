use std::collections::HashMap;
use std::rc::Rc;

use super::*;

// ---------------------------------------------------------------- test grid

#[derive(Default)]
struct TestGrid {
    cells: HashMap<(usize, u32, u32), Value>,
    sheets: Vec<String>,
    names: HashMap<String, String>,
    d1904: bool,
    now: f64,
}

fn a1(s: &str) -> (u32, u32) {
    let s = s.to_ascii_uppercase();
    let letters: String = s.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    let digits = &s[letters.len()..];
    let mut c = 0u32;
    for ch in letters.chars() {
        c = c * 26 + (ch as u32 - 'A' as u32 + 1);
    }
    (digits.parse::<u32>().unwrap() - 1, c - 1)
}

impl TestGrid {
    fn new() -> Self {
        TestGrid {
            sheets: vec!["Sheet1".into(), "Sheet2".into(), "Sheet3".into(), "My Sheet".into()],
            now: 45306.75, // 2024-01-15 18:00
            ..Default::default()
        }
    }
    fn set(&mut self, cell: &str, v: impl Into<Value>) -> &mut Self {
        self.set_on(0, cell, v)
    }
    fn set_on(&mut self, sheet: usize, cell: &str, v: impl Into<Value>) -> &mut Self {
        let (r, c) = a1(cell);
        self.cells.insert((sheet, r, c), v.into());
        self
    }
    fn name(&mut self, n: &str, f: &str) -> &mut Self {
        self.names.insert(n.to_ascii_uppercase(), f.to_string());
        self
    }
}

impl Grid for TestGrid {
    fn value(&self, sheet: usize, row: u32, col: u32) -> Value {
        self.cells.get(&(sheet, row, col)).cloned().unwrap_or(Value::Empty)
    }
    fn extent(&self, sheet: usize) -> (u32, u32) {
        let (mut r, mut c) = (0, 0);
        for &(s, rr, cc) in self.cells.keys() {
            if s == sheet {
                r = r.max(rr + 1);
                c = c.max(cc + 1);
            }
        }
        (r, c)
    }
    fn sheet_index(&self, name: &str) -> Option<usize> {
        self.sheets.iter().position(|s| s.eq_ignore_ascii_case(name))
    }
    fn sheet_count(&self) -> usize {
        self.sheets.len()
    }
    fn defined_name(&self, name: &str, _host: usize) -> Option<String> {
        self.names.get(&name.to_ascii_uppercase()).cloned()
    }
    fn date1904(&self) -> bool {
        self.d1904
    }
    fn now(&self) -> f64 {
        self.now
    }
}

const HOST: (usize, u32, u32) = (0, 999, 25); // Sheet1!Z1000

fn ev(g: &TestGrid, f: &str) -> Value {
    let c = compile(f).unwrap_or_else(|e| panic!("compile {f}: {e}"));
    c.eval(g, HOST.0, HOST.1, HOST.2)
}

#[track_caller]
fn n(g: &TestGrid, f: &str, want: f64) {
    match ev(g, f).top_left() {
        Value::Number(x) => {
            let tol = 1e-9 * want.abs().max(1.0);
            assert!((x - want).abs() <= tol, "{f}: got {x}, want {want}");
        }
        v => panic!("{f}: got {v:?}, want {want}"),
    }
}

#[track_caller]
fn t(g: &TestGrid, f: &str, want: &str) {
    match ev(g, f).top_left() {
        Value::Text(s) => assert_eq!(&*s, want, "{f}"),
        v => panic!("{f}: got {v:?}, want {want:?}"),
    }
}

#[track_caller]
fn b(g: &TestGrid, f: &str, want: bool) {
    assert_eq!(ev(g, f).top_left(), Value::Bool(want), "{f}");
}

#[track_caller]
fn e(g: &TestGrid, f: &str, want: ErrorKind) {
    assert_eq!(ev(g, f).top_left(), Value::Error(want), "{f}");
}

#[track_caller]
fn arr(g: &TestGrid, f: &str, want: Vec<Vec<Value>>) {
    match ev(g, f) {
        Value::Array(a) => assert_eq!(*a, want, "{f}"),
        v => panic!("{f}: got {v:?}, want array"),
    }
}

fn nv(x: f64) -> Value {
    Value::Number(x)
}
fn tv(s: &str) -> Value {
    Value::text(s)
}

/// A1:A5 = 1..5, B1:B5 = 10..50, fruit table D1:F4, H1:K2 horizontal table.
fn base() -> TestGrid {
    let mut g = TestGrid::new();
    for i in 1..=5 {
        g.set(&format!("A{i}"), i as f64);
        g.set(&format!("B{i}"), i as f64 * 10.0);
    }
    let fruit = [("apple", 1.5, 10.0), ("banana", 0.25, 20.0), ("cherry", 3.0, 30.0), ("date", 7.0, 40.0)];
    for (i, (name, p, q)) in fruit.iter().enumerate() {
        g.set(&format!("D{}", i + 1), *name);
        g.set(&format!("E{}", i + 1), *p);
        g.set(&format!("F{}", i + 1), *q);
    }
    for (i, c) in ["H", "I", "J", "K"].iter().enumerate() {
        g.set(&format!("{c}1"), (i as f64 + 1.0) * 10.0);
        g.set(&format!("{c}2"), ["a", "b", "c", "d"][i]);
    }
    g.set_on(1, "A1", 100.0).set_on(1, "B2", "two");
    g.set_on(2, "A1", 1000.0);
    g.set_on(3, "A1", 7.0).set_on(3, "B2", 8.0);
    g
}

// ---------------------------------------------------------------- parsing

#[test]
fn parse_forms() {
    let g = base();
    n(&g, "=1+2", 3.0);
    n(&g, "A1+A2", 3.0);
    n(&g, "$A$1+A$2+$A3", 6.0);
    n(&g, "Sheet2!A1", 100.0);
    n(&g, "sheet2!$A$1*2", 200.0);
    n(&g, "SUM('My Sheet'!$A$1:B2)", 15.0);
    n(&g, "SUM(Sheet1:Sheet3!A1)", 1101.0);
    n(&g, "SUM('Sheet1:Sheet2'!A1)", 101.0);
    t(&g, "\"say \"\"hi\"\"\"", "say \"hi\"");
    n(&g, "LEN(\"a\"\"b\")", 3.0);
    e(&g, "#N/A", ErrorKind::NA);
    b(&g, "ISERROR(#REF!)", true);
    e(&g, "#DIV/0!+1", ErrorKind::Div0);
    n(&g, "SUM(A:A)", 15.0);
    n(&g, "SUM($A:$B)", 165.0);
    n(&g, "SUM(1:1)", 122.5);
    n(&g, "SUM(2:3)", 108.25);
    n(&g, "{1,2;3,4}", 1.0);
    n(&g, "SUM({1,2;3,4})", 10.0);
    n(&g, "SUM({1,-2,3.5})", 2.5);
    n(&g, "_xlfn.STDEV.S({1,2,3})", 1.0);
    t(&g, "_xlfn.CONCAT(\"a\",\"b\")", "ab");
    n(&g, "SUM(_xlfn._xlws.SORT({3;1;2}))", 6.0);
    n(&g, "  SUM( A1 , A2 )  ", 3.0);
    n(&g, "sum(a1:a3)", 6.0);
    b(&g, "true", true);
    n(&g, "1.5e2", 150.0);
    n(&g, ".5+.5", 1.0);
    assert!(compile("1+").is_err());
    assert!(compile("SUM(1,2").is_err());
    assert!(compile("\"abc").is_err());
    e(&g, "NoSuchSheet!A1", ErrorKind::Ref);
    e(&g, "Sheet1!#REF!", ErrorKind::Ref);
}

#[test]
fn unsupported_and_flags() {
    let g = base();
    let f = compile("Table1[Col]").unwrap();
    assert!(f.uses_unsupported());
    assert_eq!(f.eval(&g, 0, 0, 0), Value::Error(ErrorKind::Ref));
    let f = compile("SUM(Table1[[#This Row],[Qty]])*2").unwrap();
    assert!(f.uses_unsupported());
    let f = compile("[1]Sheet1!A1+1").unwrap();
    assert!(f.uses_unsupported());
    assert_eq!(f.eval(&g, 0, 0, 0), Value::Error(ErrorKind::Ref));
    let f = compile("'[2]My Sheet'!A1").unwrap();
    assert!(f.uses_unsupported());
    let f = compile("FOOBAR(1)+1").unwrap();
    assert!(f.uses_unsupported());
    assert_eq!(f.unsupported_functions(), vec!["FOOBAR".to_string()]);
    assert_eq!(f.eval(&g, 0, 0, 0), Value::Error(ErrorKind::Name));
    assert!(!compile("SUM(A1:A3)").unwrap().uses_unsupported());
    assert!(compile("TODAY()+1").unwrap().is_volatile());
    assert!(compile("SUM(OFFSET(A1,0,0,2,1))").unwrap().is_volatile());
    assert!(compile("INDIRECT(\"A1\")").unwrap().is_volatile());
    assert!(compile("RAND()").unwrap().is_volatile());
    assert!(!compile("SUM(A1:A3)").unwrap().is_volatile());
    assert!(SUPPORTED_FUNCTIONS.len() > 190, "{}", SUPPORTED_FUNCTIONS.len());
    assert!(SUPPORTED_FUNCTIONS.contains(&"XLOOKUP"));
}

#[test]
fn precedents_list() {
    let f = compile("SUM(A1:B2,Sheet2!C3,A:A,3:5,'My Sheet'!$D$4,Sheet1:Sheet3!E5)+Rate").unwrap();
    let p = f.precedents();
    let pr = |s: Option<&str>, r0, c0, r1, c1| Precedent { sheet: s.map(String::from), sheet_last: None, r0, c0, r1, c1 };
    assert_eq!(p[0], pr(None, 0, 0, 1, 1));
    assert_eq!(p[1], pr(Some("Sheet2"), 2, 2, 2, 2));
    assert_eq!(p[2], pr(None, 0, 0, WHOLE, 0));
    assert_eq!(p[3], pr(None, 2, 0, 4, WHOLE));
    assert_eq!(p[4], pr(Some("My Sheet"), 3, 3, 3, 3));
    assert_eq!(p[5].sheet_last.as_deref(), Some("Sheet3"));
    assert_eq!(p.len(), 6);
    assert_eq!(f.names(), vec!["Rate".to_string()]);
}

// ---------------------------------------------------------------- operators & coercion

#[test]
fn operators() {
    let g = base();
    n(&g, "1+2*3", 7.0);
    n(&g, "(1+2)*3", 9.0);
    n(&g, "1-2-3", -4.0);
    n(&g, "2^10", 1024.0);
    n(&g, "-2^2", 4.0);
    n(&g, "2^3^2", 64.0);
    n(&g, "50%", 0.5);
    n(&g, "10*50%", 5.0);
    n(&g, "2*3%", 0.06);
    n(&g, "--\"3\"", 3.0);
    n(&g, "+A1", 1.0);
    n(&g, "-A2", -2.0);
    e(&g, "1/0", ErrorKind::Div0);
    e(&g, "0^0", ErrorKind::Num);
    e(&g, "(-8)^(1/3)", ErrorKind::Num);
    t(&g, "\"a\"&\"b\"", "ab");
    t(&g, "1&2", "12");
    t(&g, "3&TRUE", "3TRUE");
    t(&g, "\"\"&1.5", "1.5");
    t(&g, "1/3&\"\"", "0.333333333333333");
    t(&g, "1E+20&\"\"", "1E+20");
    t(&g, "0.0001&\"\"", "0.0001");
    t(&g, "1E-10&\"\"", "1E-10");
    t(&g, "123456789012345678&\"\"", "1.23456789012346E+17");
    t(&g, "-2.5&\"x\"", "-2.5x");
    t(&g, "Z99&\"x\"", "x");
}

#[test]
fn coercion_and_comparison() {
    let g = base();
    n(&g, "\"3\"+1", 4.0);
    e(&g, "\"abc\"+1", ErrorKind::Value);
    n(&g, "TRUE+1", 2.0);
    n(&g, "\"1/1/2024\"+0", 45292.0);
    n(&g, "\"2024-01-01\"+0", 45292.0);
    n(&g, "\"50%\"*2", 1.0);
    n(&g, "\"$1,000\"+0", 1000.0);
    n(&g, "\" 12 \"*1", 12.0);
    n(&g, "\"12:00\"*2", 1.0);
    e(&g, "\"1,2\"+0", ErrorKind::Value);
    n(&g, "A1+Z99", 1.0);
    n(&g, "Z99", 0.0);
    e(&g, "#N/A+1", ErrorKind::NA);
    e(&g, "(1/0)+#N/A", ErrorKind::Div0);
    b(&g, "0.1+0.2=0.3", true);
    b(&g, "0.1+0.2<>0.3", false);
    b(&g, "\"abc\"=\"ABC\"", true);
    b(&g, "\"a\"<\"b\"", true);
    b(&g, "\"B\">\"a\"", true);
    b(&g, "1<\"a\"", true);
    b(&g, "\"a\"<TRUE", true);
    b(&g, "2>TRUE", false);
    b(&g, "\"1\"=1", false);
    b(&g, "Z99=0", true);
    b(&g, "Z99=\"\"", true);
    b(&g, "Z99=FALSE", true);
    b(&g, "A1:A3=2", false); // top-left of array
    n(&g, "SUM(--(A1:A5>2))", 3.0);
    n(&g, "SUM({1,2,3}*2)", 12.0);
    n(&g, "SUM({1,2,3}*{1;2})", 18.0);
    n(&g, "SUM(A1:A5*B1:B5)", 550.0);
    arr(&g, "{1,2}+{10;20}", vec![vec![nv(11.0), nv(12.0)], vec![nv(21.0), nv(22.0)]]);
    arr(&g, "{1,2,3}+{1,2}", vec![vec![nv(2.0), nv(4.0), Value::Error(ErrorKind::NA)]]);
    arr(&g, "A1:A2", vec![vec![nv(1.0)], vec![nv(2.0)]]);
    arr(&g, "Z1:Z2", vec![vec![nv(0.0)], vec![nv(0.0)]]);
}

#[test]
fn names() {
    let mut g = base();
    g.name("Rate", "0.1").name("Data", "Sheet1!$A$1:$A$5").name("Nested", "Data").name("Loop", "Loop+1");
    n(&g, "Rate*100", 10.0);
    n(&g, "SUM(Data)", 15.0);
    n(&g, "SUM(Nested)", 15.0);
    n(&g, "INDEX(Data,2)", 2.0);
    e(&g, "Missing+1", ErrorKind::Name);
    e(&g, "Loop", ErrorKind::Name);
}

// ---------------------------------------------------------------- math

#[test]
fn math_functions() {
    let g = base();
    n(&g, "SUM(A1:A5,10,\"5\",TRUE)", 31.0);
    e(&g, "SUM(A1,\"x\")", ErrorKind::Value);
    n(&g, "SUM(A1:B5)", 165.0);
    n(&g, "SUM(D1:D4)", 0.0);
    n(&g, "PRODUCT(A1:A5)", 120.0);
    n(&g, "PRODUCT(Z1:Z5)", 0.0);
    n(&g, "SUMSQ(1,2,3)", 14.0);
    n(&g, "SUMPRODUCT(A1:A3,B1:B3)", 140.0);
    n(&g, "SUMPRODUCT((A1:A5>2)*A1:A5)", 12.0);
    n(&g, "SUMPRODUCT(D1:D4,F1:F4)", 0.0);
    e(&g, "SUMPRODUCT(A1:A3,B1:B4)", ErrorKind::Value);
    n(&g, "ABS(-5)", 5.0);
    n(&g, "ROUND(2.675,2)", 2.68);
    n(&g, "ROUND(-2.5,0)", -3.0);
    n(&g, "ROUND(1234.5678,-2)", 1200.0);
    n(&g, "ROUND(1.005,2)", 1.01);
    n(&g, "ROUNDUP(3.2,0)", 4.0);
    n(&g, "ROUNDUP(-3.2,0)", -4.0);
    n(&g, "ROUNDUP(0.1+0.2,1)", 0.3);
    n(&g, "ROUNDUP(1234,-2)", 1300.0);
    n(&g, "ROUNDDOWN(3.7,0)", 3.0);
    n(&g, "ROUNDDOWN(-4.56789,3)", -4.567);
    n(&g, "MROUND(10,3)", 9.0);
    n(&g, "MROUND(-10,-3)", -9.0);
    n(&g, "MROUND(1.3,0.2)", 1.4);
    e(&g, "MROUND(5,-2)", ErrorKind::Num);
    n(&g, "INT(-8.9)", -9.0);
    n(&g, "INT(8.9)", 8.0);
    n(&g, "TRUNC(-8.9)", -8.0);
    n(&g, "TRUNC(8.987,2)", 8.98);
    n(&g, "CEILING(2.5,1)", 3.0);
    n(&g, "CEILING(-2.5,2)", -2.0);
    n(&g, "CEILING(-2.5,-2)", -4.0);
    n(&g, "CEILING(0.3,0.1)", 0.3);
    e(&g, "CEILING(2.5,-1)", ErrorKind::Num);
    n(&g, "FLOOR(3.7,2)", 2.0);
    n(&g, "FLOOR(-2.5,-2)", -2.0);
    n(&g, "FLOOR(-2.5,2)", -4.0);
    e(&g, "FLOOR(2,0)", ErrorKind::Div0);
    n(&g, "CEILING.MATH(6.3)", 7.0);
    n(&g, "CEILING.MATH(-5.5,2)", -4.0);
    n(&g, "CEILING.MATH(-5.5,2,-1)", -6.0);
    n(&g, "FLOOR.MATH(-5.5,2)", -6.0);
    n(&g, "FLOOR.MATH(-5.5,2,-1)", -4.0);
    n(&g, "FLOOR.MATH(24.3,5)", 20.0);
    n(&g, "_xlfn.CEILING.PRECISE(-4.1)", -4.0);
    n(&g, "MOD(3,2)", 1.0);
    n(&g, "MOD(-3,2)", 1.0);
    n(&g, "MOD(3,-2)", -1.0);
    n(&g, "MOD(5.5,1)", 0.5);
    e(&g, "MOD(5,0)", ErrorKind::Div0);
    n(&g, "POWER(2,10)", 1024.0);
    n(&g, "SQRT(16)", 4.0);
    e(&g, "SQRT(-1)", ErrorKind::Num);
    n(&g, "EXP(1)", std::f64::consts::E);
    n(&g, "LN(EXP(2))", 2.0);
    e(&g, "LN(0)", ErrorKind::Num);
    n(&g, "LOG(8,2)", 3.0);
    n(&g, "LOG(100)", 2.0);
    n(&g, "LOG10(1000)", 3.0);
    n(&g, "PI()", std::f64::consts::PI);
    n(&g, "SIGN(-3)", -1.0);
    n(&g, "SIGN(0)", 0.0);
    n(&g, "QUOTIENT(7,2)", 3.0);
    n(&g, "QUOTIENT(-7,2)", -3.0);
    n(&g, "GCD(12,18)", 6.0);
    n(&g, "GCD(A1:A5,10)", 1.0);
    n(&g, "LCM(4,6)", 12.0);
    n(&g, "LCM(2,3,4)", 12.0);
    n(&g, "FACT(5)", 120.0);
    n(&g, "COMBIN(5,2)", 10.0);
    n(&g, "EVEN(3)", 4.0);
    n(&g, "EVEN(-1)", -2.0);
    n(&g, "ODD(2)", 3.0);
    n(&g, "ODD(0)", 1.0);
    n(&g, "DEGREES(PI())", 180.0);
    n(&g, "SIN(0)+COS(0)", 1.0);
    n(&g, "ATAN2(1,1)", std::f64::consts::FRAC_PI_4);
    n(&g, "SUBTOTAL(9,A1:A5)", 15.0);
    n(&g, "SUBTOTAL(101,A1:A5)", 3.0);
    n(&g, "SUBTOTAL(2,A1:A5,D1:D4)", 5.0);
    n(&g, "SUBTOTAL(3,A1:A5,D1:D4)", 9.0);
    n(&g, "SUBTOTAL(104,A1:A5)", 5.0);
    e(&g, "SUBTOTAL(12,A1:A5)", ErrorKind::Value);
    let mut g2 = base();
    g2.set("A3", Value::Error(ErrorKind::Div0));
    e(&g2, "SUM(A1:A5)", ErrorKind::Div0);
    n(&g2, "AGGREGATE(9,6,A1:A5)", 12.0);
    n(&g2, "AGGREGATE(4,6,A1:A5)", 5.0);
    n(&g2, "AGGREGATE(14,6,A1:A5,2)", 4.0);
    // rand
    for _ in 0..20 {
        let r = match ev(&g, "RAND()") {
            Value::Number(x) => x,
            v => panic!("{v:?}"),
        };
        assert!((0.0..1.0).contains(&r));
        let r = match ev(&g, "RANDBETWEEN(1,6)") {
            Value::Number(x) => x,
            v => panic!("{v:?}"),
        };
        assert!((1.0..=6.0).contains(&r) && r.fract() == 0.0);
    }
}

// ---------------------------------------------------------------- stats

fn stats_grid() -> TestGrid {
    let mut g = TestGrid::new();
    for (i, v) in [2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0].iter().enumerate() {
        g.set(&format!("A{}", i + 1), *v);
    }
    g.set("B1", 1.0).set("B2", "text").set("B3", true).set("B4", 3.0);
    g
}

#[test]
fn stats_functions() {
    let g = stats_grid();
    n(&g, "AVERAGE(A1:A8)", 5.0);
    n(&g, "AVERAGE(B1:B4)", 2.0);
    n(&g, "AVERAGEA(B1:B4)", 5.0 / 4.0);
    e(&g, "AVERAGE(Z1:Z3)", ErrorKind::Div0);
    n(&g, "COUNT(A1:A8,B1:B4)", 10.0);
    n(&g, "COUNT(1,\"2\",\"x\",TRUE)", 3.0);
    n(&g, "COUNTA(B1:B6)", 4.0);
    n(&g, "COUNTBLANK(B1:B6)", 2.0);
    n(&g, "MAX(A1:A8)", 9.0);
    n(&g, "MIN(A1:A8,1)", 1.0);
    n(&g, "MAX(Z1:Z3)", 0.0);
    n(&g, "MAXA(B1:B4)", 3.0);
    n(&g, "MINA(B1:B4)", 0.0);
    n(&g, "MEDIAN(A1:A8)", 4.5);
    n(&g, "MEDIAN(1,3,2)", 2.0);
    n(&g, "MODE(A1:A8)", 4.0);
    n(&g, "MODE.SNGL(1,2,2,3,3)", 2.0);
    e(&g, "MODE(1,2,3)", ErrorKind::NA);
    n(&g, "LARGE(A1:A8,2)", 7.0);
    n(&g, "SMALL(A1:A8,3)", 4.0);
    e(&g, "LARGE(A1:A8,9)", ErrorKind::Num);
    n(&g, "SUM(LARGE(A1:A8,{1,2}))", 16.0);
    n(&g, "RANK(7,A1:A8)", 2.0);
    n(&g, "RANK.EQ(4,A1:A8,1)", 2.0);
    e(&g, "RANK(6,A1:A8)", ErrorKind::NA);
    n(&g, "STDEV.P(A1:A8)", 2.0);
    n(&g, "STDEV(A1:A8)", 2.138_089_935_299_395);
    n(&g, "VAR.P(A1:A8)", 4.0);
    n(&g, "VAR(A1:A8)", 4.571_428_571_428_571);
    e(&g, "STDEV(1)", ErrorKind::Div0);
    n(&g, "PERCENTILE(A1:A8,0.25)", 4.0);
    n(&g, "PERCENTILE.INC(A1:A8,0.9)", 7.6);
    n(&g, "PERCENTILE.EXC(A1:A8,0.25)", 4.0);
    n(&g, "QUARTILE(A1:A8,3)", 5.5);
    n(&g, "QUARTILE.INC(A1:A8,0)", 2.0);
    e(&g, "QUARTILE(A1:A8,5)", ErrorKind::Num);
}

// ---------------------------------------------------------------- criteria

fn crit_grid() -> TestGrid {
    let mut g = base();
    g.set("G1", 5.0)
        .set("G2", 10.0)
        .set("G3", "5")
        .set("G4", "apple")
        .set("G5", "Apricot")
        .set("G7", true)
        .set("G8", "")
        .set("G9", -3.0)
        .set("G10", "a*b");
    g
}

#[test]
fn criteria_functions() {
    let g = crit_grid();
    n(&g, "COUNTIF(G1:G10,5)", 2.0);
    n(&g, "COUNTIF(G1:G10,\"5\")", 2.0);
    n(&g, "COUNTIF(G1:G10,\">4\")", 2.0);
    n(&g, "COUNTIF(G1:G10,\"<>5\")", 8.0);
    n(&g, "COUNTIF(G1:G10,\"a*\")", 3.0);
    n(&g, "COUNTIF(G1:G10,\"A~*b\")", 1.0);
    n(&g, "COUNTIF(G1:G10,\"?pple\")", 1.0);
    n(&g, "COUNTIF(G1:G10,\"\")", 2.0);
    n(&g, "COUNTIF(G1:G10,\"=\")", 1.0);
    n(&g, "COUNTIF(G1:G10,\"<>\")", 9.0);
    n(&g, "COUNTIF(G1:G10,TRUE)", 1.0);
    n(&g, "COUNTIF(G1:G10,\"<0\")", 1.0);
    n(&g, "COUNTIF(G1:G10,\">=apple\")", 2.0);
    n(&g, "COUNTIF(G1:G10,\"=APPLE\")", 1.0);
    n(&g, "COUNTIF(G:G,\"\")", 1_048_568.0);
    n(&g, "COUNTIF(G:G,\"<>apple\")", 1_048_575.0);
    n(&g, "COUNTBLANK(G1:G10)", 2.0);
    n(&g, "COUNTA(G1:G10)", 9.0);
    n(&g, "COUNT(G1:G10)", 3.0);
    n(&g, "SUMIF(G1:G10,\">0\")", 15.0);
    n(&g, "SUMIF(D1:D4,\"b*\",F1:F4)", 20.0);
    n(&g, "SUMIF(D1:D4,\"c*\",F1)", 30.0);
    n(&g, "SUMIF(F1:F4,\">=\"&20)", 90.0);
    n(&g, "SUMIFS(F1:F4,E1:E4,\">1\",D1:D4,\"<>date\")", 40.0);
    n(&g, "SUMIFS(F:F,D:D,\"*e*\")", 10.0 + 30.0 + 40.0);
    e(&g, "SUMIFS(F1:F4,E1:E3,\">1\")", ErrorKind::Value);
    n(&g, "AVERAGEIF(F1:F4,\">15\")", 30.0);
    n(&g, "AVERAGEIFS(F1:F4,D1:D4,\"*a*\")", 70.0 / 3.0);
    e(&g, "AVERAGEIF(F1:F4,\">100\")", ErrorKind::Div0);
    n(&g, "MAXIFS(F1:F4,E1:E4,\"<5\")", 30.0);
    n(&g, "MINIFS(F1:F4,E1:E4,\">1\")", 10.0);
    n(&g, "MAXIFS(F1:F4,E1:E4,\">100\")", 0.0);
    n(&g, "COUNTIFS(D1:D4,\"*a*\",F1:F4,\">15\")", 2.0);
    n(&g, "COUNTIFS(A1:A5,\">1\",B1:B5,\"<50\")", 3.0);
    n(&g, "SUM(COUNTIF(D1:D4,{\"apple\",\"date\"}))", 2.0);
    n(&g, "COUNTIF(A1:A5,Z99)", 0.0);
}

// ---------------------------------------------------------------- logic

#[test]
fn logic_functions() {
    let g = base();
    t(&g, "IF(1>2,\"a\",\"b\")", "b");
    n(&g, "IF(TRUE,1)", 1.0);
    b(&g, "IF(FALSE,1)", false);
    n(&g, "IF(FALSE,1,)", 0.0);
    e(&g, "IF(\"x\",1,2)", ErrorKind::Value);
    n(&g, "IF(TRUE,1,1/0)", 1.0);
    n(&g, "SUM(IF(A1:A5>2,A1:A5,0))", 12.0);
    n(&g, "SUM(IF(TRUE,A1:A2,B1:B2))", 3.0);
    t(&g, "IFS(1>2,\"a\",TRUE,\"c\")", "c");
    e(&g, "IFS(FALSE,1)", ErrorKind::NA);
    t(&g, "IFERROR(1/0,\"x\")", "x");
    n(&g, "IFERROR(5,\"x\")", 5.0);
    n(&g, "SUM(IFERROR({1,#N/A,3},0))", 4.0);
    n(&g, "IFNA(NA(),5)", 5.0);
    e(&g, "IFNA(1/0,5)", ErrorKind::Div0);
    b(&g, "AND(TRUE,1)", true);
    b(&g, "AND(TRUE,0)", false);
    b(&g, "AND(A1:A5)", true);
    b(&g, "OR(FALSE,0)", false);
    b(&g, "OR(A1>4,A2>1)", true);
    e(&g, "AND(\"x\")", ErrorKind::Value);
    e(&g, "AND(D1:D2)", ErrorKind::Value);
    b(&g, "XOR(TRUE,TRUE,TRUE)", true);
    b(&g, "XOR(TRUE,TRUE)", false);
    b(&g, "NOT(0)", true);
    b(&g, "TRUE()", true);
    b(&g, "FALSE()", false);
    t(&g, "SWITCH(2,1,\"a\",2,\"b\",\"z\")", "b");
    t(&g, "SWITCH(9,1,\"a\",\"z\")", "z");
    e(&g, "SWITCH(9,1,\"a\")", ErrorKind::NA);
    t(&g, "SWITCH(\"B\",\"a\",1,\"b\",\"yes\")", "yes");
    t(&g, "CHOOSE(2,\"x\",\"y\",\"z\")", "y");
    e(&g, "CHOOSE(4,\"x\")", ErrorKind::Value);
    n(&g, "SUM(CHOOSE(2,A1:A2,B1:B2))", 30.0);
}

// ---------------------------------------------------------------- lookup

#[test]
fn lookup_functions() {
    let g = base();
    n(&g, "VLOOKUP(\"banana\",D1:F4,3,FALSE)", 20.0);
    n(&g, "VLOOKUP(\"BANANA\",D1:F4,2,0)", 0.25);
    n(&g, "VLOOKUP(\"ch*\",D1:F4,3,FALSE)", 30.0);
    n(&g, "VLOOKUP(\"banana\",D:F,3,FALSE)", 20.0);
    e(&g, "VLOOKUP(\"zzz\",D1:F4,2,FALSE)", ErrorKind::NA);
    e(&g, "VLOOKUP(\"apple\",D1:F4,4,FALSE)", ErrorKind::Ref);
    e(&g, "VLOOKUP(\"apple\",D1:F4,0,FALSE)", ErrorKind::Value);
    n(&g, "VLOOKUP(25,F1:F4,1,TRUE)", 20.0);
    n(&g, "VLOOKUP(40,F1:F4,1)", 40.0);
    n(&g, "VLOOKUP(99,F1:F4,1)", 40.0);
    e(&g, "VLOOKUP(5,F1:F4,1)", ErrorKind::NA);
    t(&g, "HLOOKUP(30,H1:K2,2,FALSE)", "c");
    t(&g, "HLOOKUP(35,H1:K2,2)", "c");
    n(&g, "INDEX(D1:F4,2,3)", 20.0);
    t(&g, "INDEX(D1:D4,3)", "cherry");
    n(&g, "INDEX(H1:K1,2)", 20.0);
    n(&g, "SUM(INDEX(D1:F4,0,3))", 100.0);
    n(&g, "SUM(INDEX(D1:F4,2,0))", 20.25);
    n(&g, "SUM(D1:INDEX(F1:F4,2))", 31.75);
    e(&g, "INDEX(D1:F4,5,1)", ErrorKind::Ref);
    n(&g, "INDEX({1,2;3,4},2,1)", 3.0);
    n(&g, "MATCH(\"cherry\",D1:D4,0)", 3.0);
    n(&g, "MATCH(25,F1:F4,1)", 2.0);
    n(&g, "MATCH(25,F1:F4)", 2.0);
    n(&g, "MATCH(25,{40,30,20,10},-1)", 2.0);
    n(&g, "MATCH(\"b?nana\",D1:D4,0)", 2.0);
    n(&g, "MATCH(30,H1:K1,0)", 3.0);
    e(&g, "MATCH(\"x\",D1:D4,0)", ErrorKind::NA);
    e(&g, "MATCH(1,D1:F4,0)", ErrorKind::NA);
    n(&g, "XLOOKUP(\"date\",D1:D4,F1:F4)", 40.0);
    t(&g, "XLOOKUP(\"zz\",D1:D4,F1:F4,\"none\")", "none");
    e(&g, "XLOOKUP(\"zz\",D1:D4,F1:F4)", ErrorKind::NA);
    t(&g, "XLOOKUP(25,F1:F4,D1:D4,,-1)", "banana");
    t(&g, "XLOOKUP(25,F1:F4,D1:D4,,1)", "cherry");
    n(&g, "XLOOKUP(\"*an*\",D1:D4,F1:F4,,2)", 20.0);
    n(&g, "XLOOKUP(\"*a*\",D1:D4,F1:F4,,2,-1)", 40.0);
    arr(&g, "XLOOKUP(\"cherry\",D1:D4,E1:F4)", vec![vec![nv(3.0), nv(30.0)]]);
    t(&g, "XLOOKUP(20,H1:K1,H2:K2)", "b");
    n(&g, "XMATCH(\"cherry\",D1:D4)", 3.0);
    n(&g, "XMATCH(25,F1:F4,1)", 3.0);
    t(&g, "LOOKUP(25,F1:F4,D1:D4)", "banana");
    t(&g, "LOOKUP(35,H1:K1,H2:K2)", "c");
    n(&g, "OFFSET(D1,1,2)", 20.0);
    n(&g, "SUM(OFFSET(F1,0,0,3,1))", 60.0);
    n(&g, "SUM(OFFSET(A1:A2,1,1))", 50.0);
    e(&g, "OFFSET(A1,-1,0)", ErrorKind::Ref);
    n(&g, "INDIRECT(\"F3\")", 30.0);
    n(&g, "SUM(INDIRECT(\"F1:F4\"))", 100.0);
    n(&g, "INDIRECT(\"Sheet2!A1\")", 100.0);
    n(&g, "INDIRECT(\"'My Sheet'!B2\")", 8.0);
    n(&g, "INDIRECT(\"A\"&2)", 2.0);
    e(&g, "INDIRECT(\"bad ref!!\")", ErrorKind::Ref);
    n(&g, "ROW()", 1000.0);
    n(&g, "COLUMN()", 26.0);
    n(&g, "ROW(C5)", 5.0);
    n(&g, "COLUMN(C5)", 3.0);
    n(&g, "SUM(ROW(A1:A4))", 10.0);
    n(&g, "ROWS(D1:F4)", 4.0);
    n(&g, "COLUMNS(D1:F4)", 3.0);
    n(&g, "ROWS(A:A)", 1_048_576.0);
    n(&g, "ROWS({1,2;3,4})", 2.0);
    t(&g, "ADDRESS(2,3)", "$C$2");
    t(&g, "ADDRESS(2,3,4)", "C2");
    t(&g, "ADDRESS(5,28,2)", "AB$5");
    t(&g, "ADDRESS(1,1,1,TRUE,\"My Sheet\")", "'My Sheet'!$A$1");
    t(&g, "ADDRESS(3,4,1,FALSE)", "R3C4");
    arr(&g, "TRANSPOSE({1,2,3})", vec![vec![nv(1.0)], vec![nv(2.0)], vec![nv(3.0)]]);
    arr(&g, "TRANSPOSE(A1:B1)", vec![vec![nv(1.0)], vec![nv(10.0)]]);
}

// ---------------------------------------------------------------- text

#[test]
fn text_functions() {
    let g = base();
    t(&g, "LEFT(\"hello\",2)", "he");
    t(&g, "LEFT(\"hello\")", "h");
    t(&g, "LEFT(123.5,2)", "12");
    e(&g, "LEFT(\"a\",-1)", ErrorKind::Value);
    t(&g, "RIGHT(\"hello\",3)", "llo");
    t(&g, "RIGHT(\"hi\",10)", "hi");
    t(&g, "MID(\"hello\",2,3)", "ell");
    t(&g, "MID(\"hello\",10,2)", "");
    e(&g, "MID(\"hello\",0,2)", ErrorKind::Value);
    n(&g, "LEN(\"héllo\")", 5.0);
    n(&g, "LEN(12.50)", 4.0);
    t(&g, "LOWER(\"ABC\")", "abc");
    t(&g, "UPPER(\"abc\")", "ABC");
    t(&g, "PROPER(\"hello wORLD o'neil\")", "Hello World O'Neil");
    t(&g, "TRIM(\"  a   b  \")", "a b");
    t(&g, "CLEAN(CHAR(7)&\"x\")", "x");
    t(&g, "SUBSTITUTE(\"aaa\",\"a\",\"b\")", "bbb");
    t(&g, "SUBSTITUTE(\"aaa\",\"a\",\"b\",2)", "aba");
    t(&g, "SUBSTITUTE(\"abc\",\"\",\"x\")", "abc");
    t(&g, "REPLACE(\"abcdef\",2,3,\"X\")", "aXef");
    n(&g, "FIND(\"l\",\"hello\")", 3.0);
    n(&g, "FIND(\"l\",\"hello\",4)", 4.0);
    e(&g, "FIND(\"L\",\"hello\")", ErrorKind::Value);
    n(&g, "SEARCH(\"L\",\"hello\")", 3.0);
    n(&g, "SEARCH(\"l?o\",\"hello\")", 3.0);
    n(&g, "SEARCH(\"e*o\",\"hello\")", 2.0);
    n(&g, "SEARCH(\"~*\",\"a*b\")", 2.0);
    e(&g, "SEARCH(\"z\",\"hello\")", ErrorKind::Value);
    b(&g, "EXACT(\"a\",\"A\")", false);
    b(&g, "EXACT(\"a\",\"a\")", true);
    t(&g, "REPT(\"ab\",3)", "ababab");
    n(&g, "VALUE(\"1,234.5\")", 1234.5);
    n(&g, "VALUE(\"-1e3\")", -1000.0);
    n(&g, "VALUE(\"(5)\")", -5.0);
    e(&g, "VALUE(\"abc\")", ErrorKind::Value);
    n(&g, "NUMBERVALUE(\"1.234,5\",\",\",\".\")", 1234.5);
    n(&g, "NUMBERVALUE(\"12%\")", 0.12);
    t(&g, "CHAR(65)", "A");
    t(&g, "CHAR(128)", "€");
    n(&g, "CODE(\"A\")", 65.0);
    n(&g, "CODE(\"€\")", 128.0);
    t(&g, "UNICHAR(8364)", "€");
    n(&g, "UNICODE(\"€\")", 8364.0);
    t(&g, "T(1)", "");
    t(&g, "T(\"a\")", "a");
    n(&g, "N(TRUE)", 1.0);
    n(&g, "N(\"a\")", 0.0);
    t(&g, "CONCATENATE(\"a\",1,TRUE)", "a1TRUE");
    t(&g, "CONCAT(D1:D2)", "applebanana");
    t(&g, "CONCAT(A1:B2,\"!\")", "110220!");
    t(&g, "TEXTJOIN(\", \",TRUE,\"a\",\"\",\"b\")", "a, b");
    t(&g, "TEXTJOIN(\"-\",FALSE,\"a\",\"\",\"b\")", "a--b");
    t(&g, "TEXTJOIN(\",\",TRUE,D1:D4,Z1)", "apple,banana,cherry,date");
    t(&g, "DOLLAR(1234.567)", "$1,234.57");
    t(&g, "DOLLAR(-1234.567,-2)", "($1,200)");
    t(&g, "FIXED(1234.567,1)", "1,234.6");
    t(&g, "FIXED(1234.567,1,TRUE)", "1234.6");
    t(&g, "FIXED(-1234.567,-1)", "-1,230");
    t(&g, "FIXED(0.5,0)", "1");
    arr(&g, "LEFT(D1:D2,1)", vec![vec![tv("a")], vec![tv("b")]]);
    n(&g, "SUMPRODUCT(--(LEFT(D1:D4,1)=\"c\"))", 1.0);
}

#[test]
fn text_format() {
    let g = base();
    t(&g, "TEXT(1234.567,\"#,##0.00\")", "1,234.57");
    t(&g, "TEXT(1234567,\"#,##0\")", "1,234,567");
    t(&g, "TEXT(-1234.5,\"#,##0\")", "-1,235");
    t(&g, "TEXT(0.256,\"0.0%\")", "25.6%");
    t(&g, "TEXT(0.5,\"0%\")", "50%");
    t(&g, "TEXT(5,\"0\")", "5");
    t(&g, "TEXT(3.14159,\"0.00\")", "3.14");
    t(&g, "TEXT(0.5,\"#.00\")", ".50");
    t(&g, "TEXT(7,\"000\")", "007");
    t(&g, "TEXT(1234.5,\"$#,##0.00\")", "$1,234.50");
    t(&g, "TEXT(12345.678,\"0.00E+00\")", "1.23E+04");
    t(&g, "TEXT(-5,\"0;(0)\")", "(5)");
    t(&g, "TEXT(0,\"0;-0;\"\"zero\"\"\")", "zero");
    t(&g, "TEXT(1.5,\"0.0#\")", "1.5");
    t(&g, "TEXT(45292,\"yyyy-mm-dd\")", "2024-01-01");
    t(&g, "TEXT(45292,\"dd/mm/yyyy\")", "01/01/2024");
    t(&g, "TEXT(45306,\"mm/dd/yyyy\")", "01/15/2024");
    t(&g, "TEXT(45292,\"dd-mmm-yyyy\")", "01-Jan-2024");
    t(&g, "TEXT(45292,\"mmm\")", "Jan");
    t(&g, "TEXT(45292,\"mmmm\")", "January");
    t(&g, "TEXT(45292,\"dddd\")", "Monday");
    t(&g, "TEXT(45292,\"ddd d mmm yy\")", "Mon 1 Jan 24");
    t(&g, "TEXT(0.75,\"hh:mm\")", "18:00");
    t(&g, "TEXT(0.5+5/86400,\"hh:mm:ss\")", "12:00:05");
    t(&g, "TEXT(0.75,\"h:mm AM/PM\")", "6:00 PM");
    t(&g, "TEXT(45292.25,\"yyyy-mm-dd hh:mm\")", "2024-01-01 06:00");
    t(&g, "TEXT(1.25,\"[h]:mm\")", "30:00");
    t(&g, "TEXT(\"abc\",\"@\")", "abc");
    t(&g, "TEXT(\"abc\",\"0.00\")", "abc");
    t(&g, "TEXT(\"12\",\"0.00\")", "12.00");
    t(&g, "TEXT(1234.5,\"General\")", "1234.5");
    t(&g, "TEXT(12,\"0 \"\"units\"\"\")", "12 units");
}

// ---------------------------------------------------------------- dates

#[test]
fn date_functions() {
    let g = base();
    n(&g, "DATE(2024,1,1)", 45292.0);
    n(&g, "DATE(1900,1,1)", 1.0);
    n(&g, "DATE(1900,2,29)", 60.0);
    n(&g, "DATE(1900,3,1)", 61.0);
    n(&g, "DATE(2024,2,29)", 45351.0);
    n(&g, "DATE(2023,14,1)", 45292.0 + 31.0);
    n(&g, "DATE(2024,1,0)", 45291.0);
    n(&g, "DATE(24,1,1)", 8767.0); // year 1924
    e(&g, "DATE(10000,1,1)", ErrorKind::Num);
    n(&g, "DATEVALUE(\"2024-01-01\")", 45292.0);
    n(&g, "DATEVALUE(\"15-Jan-2024\")", 45306.0);
    n(&g, "DATEVALUE(\"1/15/2024\")", 45306.0);
    n(&g, "DATEVALUE(\"January 15, 2024\")", 45306.0);
    n(&g, "DATEVALUE(\"2024-01-15 10:30\")", 45306.0);
    e(&g, "DATEVALUE(\"hello\")", ErrorKind::Value);
    e(&g, "DATEVALUE(\"2023-02-29\")", ErrorKind::Value);
    n(&g, "TIME(12,30,0)", 0.520_833_333_333_333_3);
    n(&g, "TIME(25,0,0)", 1.0 / 24.0);
    n(&g, "TIMEVALUE(\"6:00 PM\")", 0.75);
    n(&g, "TIMEVALUE(\"2024-01-01 06:00\")", 0.25);
    n(&g, "TODAY()", 45306.0);
    n(&g, "NOW()", 45306.75);
    n(&g, "YEAR(TODAY())", 2024.0);
    n(&g, "MONTH(45351)", 2.0);
    n(&g, "DAY(45351)", 29.0);
    n(&g, "DAY(60)", 29.0);
    n(&g, "MONTH(61)", 3.0);
    n(&g, "YEAR(\"2020-06-30\")", 2020.0);
    e(&g, "YEAR(-1)", ErrorKind::Num);
    n(&g, "HOUR(0.75)", 18.0);
    n(&g, "MINUTE(\"12:45\")", 45.0);
    n(&g, "SECOND(TIME(1,2,3))", 3.0);
    n(&g, "HOUR(NOW())", 18.0);
    n(&g, "WEEKDAY(DATE(2024,1,1))", 2.0);
    n(&g, "WEEKDAY(DATE(2024,1,1),2)", 1.0);
    n(&g, "WEEKDAY(DATE(2024,1,1),3)", 0.0);
    n(&g, "WEEKDAY(DATE(2024,1,7))", 1.0);
    n(&g, "WEEKDAY(1)", 1.0);
    n(&g, "WEEKNUM(DATE(2024,1,1))", 1.0);
    n(&g, "WEEKNUM(DATE(2024,1,7))", 2.0);
    n(&g, "WEEKNUM(DATE(2024,1,7),2)", 1.0);
    n(&g, "WEEKNUM(DATE(2024,12,31))", 53.0);
    n(&g, "ISOWEEKNUM(DATE(2021,1,1))", 53.0);
    n(&g, "ISOWEEKNUM(DATE(2024,1,1))", 1.0);
    n(&g, "EDATE(DATE(2024,1,31),1)", 45351.0);
    n(&g, "EDATE(DATE(2024,3,15),-2)", 45306.0);
    n(&g, "EOMONTH(DATE(2024,1,15),1)", 45351.0);
    n(&g, "EOMONTH(DATE(2024,1,15),-1)", 45291.0);
    n(&g, "DATEDIF(DATE(2020,1,15),DATE(2024,3,10),\"Y\")", 4.0);
    n(&g, "DATEDIF(DATE(2020,1,15),DATE(2024,3,10),\"M\")", 49.0);
    n(&g, "DATEDIF(DATE(2020,1,15),DATE(2024,3,10),\"D\")", 1516.0);
    n(&g, "DATEDIF(DATE(2020,1,15),DATE(2024,3,10),\"YM\")", 1.0);
    n(&g, "DATEDIF(DATE(2020,1,15),DATE(2024,3,10),\"MD\")", 24.0);
    n(&g, "DATEDIF(DATE(2020,1,15),DATE(2024,3,10),\"YD\")", 55.0);
    e(&g, "DATEDIF(DATE(2024,1,2),DATE(2024,1,1),\"D\")", ErrorKind::Num);
    n(&g, "DAYS(DATE(2024,3,1),DATE(2024,1,1))", 60.0);
    n(&g, "DAYS(\"2024-03-01\",\"2024-01-01\")", 60.0);
    n(&g, "NETWORKDAYS(DATE(2024,1,1),DATE(2024,1,31))", 23.0);
    n(&g, "NETWORKDAYS(DATE(2024,1,1),DATE(2024,1,31),DATE(2024,1,1))", 22.0);
    n(&g, "NETWORKDAYS(DATE(2024,1,31),DATE(2024,1,1))", -23.0);
    n(&g, "NETWORKDAYS(DATE(2024,1,1),DATE(2024,1,31),{45292,45293,45297})", 21.0);
    n(&g, "NETWORKDAYS.INTL(DATE(2024,1,1),DATE(2024,1,7),11)", 6.0);
    n(&g, "NETWORKDAYS.INTL(DATE(2024,1,1),DATE(2024,1,7),\"0000011\")", 5.0);
    n(&g, "NETWORKDAYS.INTL(DATE(2024,1,1),DATE(2024,1,7),7)", 5.0);
    e(&g, "NETWORKDAYS.INTL(DATE(2024,1,1),DATE(2024,1,7),\"1111111\")", ErrorKind::Value);
    n(&g, "WORKDAY(DATE(2024,1,5),1)", 45299.0);
    n(&g, "WORKDAY(DATE(2024,1,8),-1)", 45296.0);
    n(&g, "WORKDAY(DATE(2024,1,1),10)", 45306.0);
    n(&g, "WORKDAY(DATE(2024,1,5),1,DATE(2024,1,8))", 45300.0);
    n(&g, "WORKDAY.INTL(DATE(2024,1,5),1,11)", 45297.0);
    n(&g, "YEARFRAC(DATE(2024,1,1),DATE(2024,7,1))", 0.5);
    n(&g, "YEARFRAC(DATE(2024,1,1),DATE(2024,7,1),1)", 182.0 / 366.0);
    n(&g, "YEARFRAC(DATE(2024,1,1),DATE(2024,7,1),2)", 182.0 / 360.0);
    n(&g, "YEARFRAC(DATE(2024,1,1),DATE(2024,7,1),3)", 182.0 / 365.0);
    n(&g, "YEARFRAC(DATE(2024,1,31),DATE(2024,3,31),4)", 60.0 / 360.0);
    n(&g, "YEARFRAC(DATE(2023,1,1),DATE(2025,1,1),1)", 731.0 / (1096.0 / 3.0));
    e(&g, "YEARFRAC(1,2,5)", ErrorKind::Num);
}

#[test]
fn date1904_system() {
    let mut g = base();
    g.d1904 = true;
    n(&g, "DATE(2024,1,1)", 43830.0);
    n(&g, "YEAR(0)", 1904.0);
    n(&g, "WEEKDAY(DATE(2024,1,1))", 2.0);
    t(&g, "TEXT(43830,\"yyyy-mm-dd\")", "2024-01-01");
    n(&g, "DATEVALUE(\"2024-01-01\")", 43830.0);
    assert_eq!(date_to_serial(2024, 1, 1, true), Some(43830.0));
    assert_eq!(serial_to_date(45292.0, false), Some((2024, 1, 1)));
}

// ---------------------------------------------------------------- info & financial

#[test]
fn info_functions() {
    let g = base();
    b(&g, "ISBLANK(Z99)", true);
    b(&g, "ISBLANK(A1)", false);
    b(&g, "ISNUMBER(1)", true);
    b(&g, "ISNUMBER(\"1\")", false);
    b(&g, "ISTEXT(D1)", true);
    b(&g, "ISNONTEXT(A1)", true);
    b(&g, "ISLOGICAL(TRUE)", true);
    b(&g, "ISERROR(1/0)", true);
    b(&g, "ISERR(NA())", false);
    b(&g, "ISERR(1/0)", true);
    b(&g, "ISNA(NA())", true);
    b(&g, "ISEVEN(4)", true);
    b(&g, "ISODD(3.7)", true);
    b(&g, "ISREF(A1)", true);
    b(&g, "ISREF(1)", false);
    b(&g, "ISREF(OFFSET(A1,1,1))", true);
    n(&g, "ERROR.TYPE(1/0)", 2.0);
    n(&g, "ERROR.TYPE(NA())", 7.0);
    e(&g, "ERROR.TYPE(1)", ErrorKind::NA);
    n(&g, "TYPE(\"a\")", 2.0);
    n(&g, "TYPE(A1:A2)", 64.0);
    n(&g, "TYPE(TRUE)", 4.0);
    n(&g, "TYPE(1/0)", 16.0);
    n(&g, "TYPE(A1)", 1.0);
    e(&g, "NA()", ErrorKind::NA);
    arr(&g, "ISNUMBER(C1:D1)", vec![vec![Value::Bool(false), Value::Bool(false)]]);
}

#[test]
fn financial_functions() {
    let g = base();
    n(&g, "PMT(0.05/12,360,200000)", -1_073.643_246_024_28);
    n(&g, "PMT(0,10,1000)", -100.0);
    n(&g, "FV(0.06/12,10,-200,-500,1)", 2_581.403_373_775_62);
    n(&g, "PV(0.08/12,12*20,500)", -59_777.145_851_187_8);
    n(&g, "NPER(0.12/12,-100,-1000,10000,1)", 59.673_865_674_294_6);
    n(&g, "RATE(4*12,-200,8000)", 0.007_701_472_488_246);
    n(&g, "NPV(0.1,-10000,3000,4200,6800)", 1_188.443_412_335_1);
    n(&g, "IRR({-70000,12000,15000,18000,21000,26000})", 0.086_630_948_036_1);
    e(&g, "IRR({1,2,3})", ErrorKind::Num);
}

// ---------------------------------------------------------------- dynamic arrays

#[test]
fn dynamic_arrays() {
    let g = base();
    arr(&g, "SEQUENCE(3)", vec![vec![nv(1.0)], vec![nv(2.0)], vec![nv(3.0)]]);
    arr(&g, "SEQUENCE(2,3,10,5)", vec![vec![nv(10.0), nv(15.0), nv(20.0)], vec![nv(25.0), nv(30.0), nv(35.0)]]);
    n(&g, "SUM(SEQUENCE(10))", 55.0);
    n(&g, "ROWS(SEQUENCE(5))", 5.0);
    arr(&g, "UNIQUE({1;2;1;3})", vec![vec![nv(1.0)], vec![nv(2.0)], vec![nv(3.0)]]);
    arr(&g, "UNIQUE({\"a\";\"A\";\"b\"})", vec![vec![tv("a")], vec![tv("b")]]);
    arr(&g, "UNIQUE({1;2;1},FALSE,TRUE)", vec![vec![nv(2.0)]]);
    arr(&g, "UNIQUE({1,1,2},TRUE)", vec![vec![nv(1.0), nv(2.0)]]);
    arr(&g, "FILTER(D1:D4,F1:F4>15)", vec![vec![tv("banana")], vec![tv("cherry")], vec![tv("date")]]);
    t(&g, "FILTER(D1:D4,F1:F4>100,\"none\")", "none");
    e(&g, "FILTER(D1:D4,F1:F4>100)", ErrorKind::Calc);
    arr(&g, "FILTER(H1:K2,H1:K1>=30)", vec![vec![nv(30.0), nv(40.0)], vec![tv("c"), tv("d")]]);
    arr(&g, "SORT({3;1;2})", vec![vec![nv(1.0)], vec![nv(2.0)], vec![nv(3.0)]]);
    arr(&g, "SORT({3;1;2},1,-1)", vec![vec![nv(3.0)], vec![nv(2.0)], vec![nv(1.0)]]);
    t(&g, "SORT(D1:E4,2,-1)", "date");
    arr(&g, "SORTBY(D1:D4,E1:E4)", vec![vec![tv("banana")], vec![tv("apple")], vec![tv("cherry")], vec![tv("date")]]);
    arr(&g, "SORT({\"b\";\"a\";\"C\"})", vec![vec![tv("a")], vec![tv("b")], vec![tv("C")]]);
}

// ---------------------------------------------------------------- mini workbook

#[test]
fn mini_workbook() {
    let mut g = TestGrid::new();
    g.sheets = vec!["Sales".into(), "Prices".into(), "Report".into()];
    // Sales: Date | Region | Product | Qty
    let rows: [(f64, &str, &str, f64); 8] = [
        (45292.0, "East", "Widget", 10.0), // 2024-01-01
        (45300.0, "West", "Gadget", 5.0),
        (45310.0, "East", "Gadget", 7.0),
        (45320.0, "East", "Widget", 3.0),
        (45323.0, "East", "Widget", 4.0), // 2024-02-01
        (45330.0, "West", "Widget", 8.0),
        (45340.0, "East", "Doohickey", 2.0),
        (45350.0, "West", "Gadget", 1.0),
    ];
    g.set("A1", "Date").set("B1", "Region").set("C1", "Product").set("D1", "Qty");
    for (i, (d, r, p, q)) in rows.iter().enumerate() {
        let row = i + 2;
        g.set(&format!("A{row}"), *d);
        g.set(&format!("B{row}"), *r);
        g.set(&format!("C{row}"), *p);
        g.set(&format!("D{row}"), *q);
    }
    // Prices sheet
    for (i, (p, price)) in [("Widget", 2.5), ("Gadget", 10.0), ("Doohickey", 99.0)].iter().enumerate() {
        g.set_on(1, &format!("A{}", i + 1), *p);
        g.set_on(1, &format!("B{}", i + 1), *price);
    }
    g.name("PriceTable", "Prices!$A$1:$B$3");
    let host = |f: &str| compile(f).unwrap().eval(&g, 2, 0, 0).top_left();
    // SUMIFS with dates: East in January 2024
    assert_eq!(host("SUMIFS(Sales!D:D,Sales!B:B,\"East\",Sales!A:A,\">=\"&DATE(2024,1,1),Sales!A:A,\"<\"&DATE(2024,2,1))"), nv(20.0));
    assert_eq!(host("SUMIFS(Sales!D:D,Sales!A:A,\">=2024-02-01\")"), nv(15.0));
    assert_eq!(host("COUNTIFS(Sales!B:B,\"West\",Sales!C:C,\"Gadget\")"), nv(2.0));
    // VLOOKUP over a table on another sheet via a defined name
    assert_eq!(host("VLOOKUP(\"gadget\",PriceTable,2,FALSE)"), nv(10.0));
    assert_eq!(host("IFERROR(VLOOKUP(\"Gizmo\",PriceTable,2,FALSE),0)"), nv(0.0));
    // revenue via SUMPRODUCT of qty * looked-up price
    assert_eq!(host("SUMPRODUCT(Sales!D2:D9,SUMIF(Prices!A1:A3,Sales!C2:C9,Prices!B1:B3))"), nv(25.0 * 2.5 + 13.0 * 10.0 + 2.0 * 99.0));
    // nested IF grading
    let grade = |x: f64| {
        let f = format!("IF({x}>=90,\"A\",IF({x}>=80,\"B\",IF({x}>=70,\"C\",\"F\")))");
        host(&f)
    };
    assert_eq!(grade(95.0), tv("A"));
    assert_eq!(grade(85.0), tv("B"));
    assert_eq!(grade(70.0), tv("C"));
    assert_eq!(grade(10.0), tv("F"));
    // month summary through TEXT + SUMPRODUCT
    assert_eq!(host("SUMPRODUCT((TEXT(Sales!A2:A9,\"yyyy-mm\")=\"2024-02\")*Sales!D2:D9)"), nv(15.0));
    assert_eq!(host("MAXIFS(Sales!A:A,Sales!B:B,\"West\")-MINIFS(Sales!A:A,Sales!B:B,\"West\")"), nv(50.0));
    assert_eq!(host("INDEX(Sales!C:C,MATCH(MAX(Sales!D:D),Sales!D:D,0))"), tv("Widget"));
    assert_eq!(host("XLOOKUP(45310,Sales!A2:A9,Sales!B2:B9)"), tv("East"));
    assert_eq!(host("COUNTA(UNIQUE(Sales!C2:C9))"), nv(3.0));
    assert_eq!(host("TEXTJOIN(\",\",TRUE,SORT(UNIQUE(Sales!B2:B9)))"), tv("East,West"));
}

// ---------------------------------------------------------------- benchmark

struct DenseGrid {
    cols: Vec<Vec<Value>>,
    rows: u32,
}

impl Grid for DenseGrid {
    fn value(&self, _s: usize, row: u32, col: u32) -> Value {
        self.cols.get(col as usize).and_then(|c| c.get(row as usize)).cloned().unwrap_or(Value::Empty)
    }
    fn extent(&self, _s: usize) -> (u32, u32) {
        (self.rows, self.cols.len() as u32)
    }
    fn sheet_index(&self, _n: &str) -> Option<usize> {
        Some(0)
    }
    fn sheet_count(&self) -> usize {
        1
    }
    fn defined_name(&self, _n: &str, _h: usize) -> Option<String> {
        None
    }
    fn now(&self) -> f64 {
        45306.0
    }
}

#[test]
#[ignore]
fn bench_sumifs_100k() {
    let n = 100_000u32;
    let regions: Vec<Rc<str>> = ["East", "West", "North", "South"].iter().map(|s| Rc::from(*s)).collect();
    let g = DenseGrid {
        rows: n,
        cols: vec![
            (0..n).map(|i| Value::Number(45292.0 + (i % 366) as f64)).collect(),
            (0..n).map(|i| Value::Text(regions[(i % 4) as usize].clone())).collect(),
            (0..n).map(|i| Value::Number((i % 100) as f64)).collect(),
        ],
    };
    let cases = [
        "SUMIFS(C:C,B:B,\"East\",A:A,\">=\"&DATE(2024,6,1))",
        "SUM(C:C)",
        "COUNTIF(B:B,\"we*\")",
        "SUMPRODUCT((B1:B100000=\"North\")*C1:C100000)",
        "VLOOKUP(99,C1:C100000,1,FALSE)",
    ];
    for f in cases {
        let c = compile(f).unwrap();
        let v = c.eval(&g, 0, 0, 5);
        let iters = 20;
        let t0 = std::time::Instant::now();
        for _ in 0..iters {
            std::hint::black_box(c.eval(&g, 0, 0, 5));
        }
        let per = t0.elapsed().as_secs_f64() * 1000.0 / iters as f64;
        println!("{f:<60} {per:8.3} ms/eval  -> {v:?}");
    }
}

#[test]
fn regressions() {
    let g = base();
    arr(&g, "IF(A1:A3>1,\"y\",\"n\")", vec![vec![tv("n")], vec![tv("y")], vec![tv("y")]]);
    arr(&g, "IF({TRUE,FALSE},A1:A2,0)", vec![vec![nv(1.0), nv(0.0)], vec![nv(2.0), nv(0.0)]]);
    n(&g, "SUMPRODUCT(A:A,(B:B>20)*1)", 3.0 + 4.0 + 5.0);
    n(&g, "SUMPRODUCT((A:A>2)*B:B)", 120.0);
    n(&g, "TRUNC(1E+300,20)", 1e300);
    n(&g, "ROUNDUP(0.1+0.2,20)", 0.3);
    e(&g, "COMBIN(1E+9,5E+8)", ErrorKind::Num);
    e(&g, "\"1.2.3\"+0", ErrorKind::Value);
    t(&g, "TEXT(12.5,\".00\")", "12.50");
    n(&g, "SUM(A1:A3 )", 6.0);
}
