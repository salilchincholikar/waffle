//! Conditional formatting: parsed from the sheet XML, evaluated per visible cell.
//! Display only — the original XML is what gets saved.

use std::collections::HashMap;
use std::sync::Arc;

use quick_xml::Reader;
use quick_xml::events::Event;

use crate::cell::{Cell, Kind};
use crate::refshift;
use crate::sheet::{Rect, Sheet};
use crate::styles::Styles;
use crate::xml::{attr, attr_bool, local};

#[derive(Clone, Debug)]
pub enum Operand {
    Num(f64),
    Text(String),
    /// Anything else: evaluated by the formula engine when available.
    Formula(String),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    Lt,
    Le,
    Eq,
    Ne,
    Ge,
    Gt,
    Between,
    NotBetween,
}

#[derive(Clone, Debug)]
pub enum Cfvo {
    Min,
    Max,
    Num(f64),
    Percent(f64),
    Percentile(f64),
    Formula(String),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TextMode {
    Contains,
    NotContains,
    Begins,
    Ends,
}

#[derive(Clone, Debug)]
pub enum Kind2 {
    CellIs { op: Op, a: Operand, b: Option<Operand> },
    Expression(String),
    Text { mode: TextMode, text: String },
    Blanks(bool),
    Errors(bool),
    Duplicate { unique: bool },
    Top { rank: u32, percent: bool, bottom: bool },
    Average { above: bool, equal: bool, std_dev: u32 },
    ColorScale(Vec<(Cfvo, u32)>),
    DataBar { min: Cfvo, max: Cfvo, color: u32 },
    Unsupported,
}

#[derive(Clone, Debug)]
pub struct Rule {
    pub priority: i32,
    pub kind: Kind2,
    pub dxf: Option<u32>,
    pub stop: bool,
}

#[derive(Clone, Debug)]
pub struct Block {
    pub sqref: String,
    pub rects: Vec<Rect>,
    pub rules: Vec<Rule>,
}

impl Block {
    pub fn contains(&self, r: u32, c: u32) -> bool {
        self.rects.iter().any(|x| x.contains(r, c))
    }
}

/// Parse all `<conditionalFormatting>` blocks from a sheet's XML suffix.
pub fn parse(xml: &[u8], styles: &Styles) -> Vec<Block> {
    let mut out = Vec::new();
    let mut r = Reader::from_reader(xml);
    r.config_mut().check_end_names = false;
    let mut cur: Option<Block> = None;
    let mut rule: Option<(Rule, String, Option<String>, Option<String>)> = None; // rule, type, operator, text
    let mut formulas: Vec<String> = Vec::new();
    let mut cfvos: Vec<Cfvo> = Vec::new();
    let mut colors: Vec<u32> = Vec::new();
    let mut in_formula = false;
    let mut text = String::new();
    let mut depth_ext = 0; // skip x14 extensions
    loop {
        let ev = match r.read_event() {
            Ok(Event::Eof) | Err(_) => break,
            Ok(e) => e,
        };
        match ev {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let empty = matches!(ev, Event::Empty(_));
                let qn = e.name();
                let n = local(qn.as_ref());
                if n == b"extLst" && !empty {
                    depth_ext += 1;
                }
                if depth_ext > 0 {
                    continue;
                }
                match n {
                    b"conditionalFormatting" => {
                        let sqref = attr(e, b"sqref").unwrap_or_default();
                        cur = Some(Block { rects: parse_sqref(&sqref), sqref, rules: Vec::new() });
                    }
                    b"cfRule" => {
                        let t = attr(e, b"type").unwrap_or_default();
                        let mut rl = Rule {
                            priority: attr(e, b"priority").and_then(|v| v.parse().ok()).unwrap_or(0),
                            kind: Kind2::Unsupported,
                            dxf: attr(e, b"dxfId").and_then(|v| v.parse().ok()),
                            stop: attr_bool(e, b"stopIfTrue").unwrap_or(false),
                        };
                        rl.kind = match t.as_str() {
                            "top10" => Kind2::Top {
                                rank: attr(e, b"rank").and_then(|v| v.parse().ok()).unwrap_or(10),
                                percent: attr_bool(e, b"percent").unwrap_or(false),
                                bottom: attr_bool(e, b"bottom").unwrap_or(false),
                            },
                            "aboveAverage" => Kind2::Average {
                                above: attr_bool(e, b"aboveAverage").unwrap_or(true),
                                equal: attr_bool(e, b"equalAverage").unwrap_or(false),
                                std_dev: attr(e, b"stdDev").and_then(|v| v.parse().ok()).unwrap_or(0),
                            },
                            "duplicateValues" => Kind2::Duplicate { unique: false },
                            "uniqueValues" => Kind2::Duplicate { unique: true },
                            "containsBlanks" => Kind2::Blanks(true),
                            "notContainsBlanks" => Kind2::Blanks(false),
                            "containsErrors" => Kind2::Errors(true),
                            "notContainsErrors" => Kind2::Errors(false),
                            _ => Kind2::Unsupported,
                        };
                        rule = Some((rl, t, attr(e, b"operator"), attr(e, b"text")));
                        formulas.clear();
                        cfvos.clear();
                        colors.clear();
                        if empty {
                            finish_rule(&mut rule, &mut cur, &formulas, &cfvos, &colors);
                        }
                    }
                    b"formula" => {
                        in_formula = true;
                        text.clear();
                    }
                    b"cfvo" => {
                        let val = attr(e, b"val").unwrap_or_default();
                        let num = val.parse::<f64>().ok();
                        cfvos.push(match attr(e, b"type").as_deref() {
                            Some("min") => Cfvo::Min,
                            Some("max") => Cfvo::Max,
                            Some("percent") => Cfvo::Percent(num.unwrap_or(0.0)),
                            Some("percentile") => Cfvo::Percentile(num.unwrap_or(50.0)),
                            Some("num") => num.map_or(Cfvo::Formula(val.clone()), Cfvo::Num),
                            _ => Cfvo::Formula(val),
                        });
                    }
                    b"color" if rule.is_some() => colors.push(styles.resolve_color(e).unwrap_or(0x638EC6)),
                    _ => {}
                }
            }
            Event::Text(t) if in_formula => text.push_str(&crate::xml::unescape(&String::from_utf8_lossy(&t))),
            Event::GeneralRef(g) if in_formula => {
                text.push_str(&crate::xml::unescape(&format!("&{};", String::from_utf8_lossy(&g))));
            }
            Event::End(e) => {
                let qn = e.name();
                let n = local(qn.as_ref());
                if n == b"extLst" {
                    depth_ext -= 1;
                    continue;
                }
                if depth_ext > 0 {
                    continue;
                }
                match n {
                    b"formula" => {
                        in_formula = false;
                        formulas.push(std::mem::take(&mut text));
                    }
                    b"cfRule" => finish_rule(&mut rule, &mut cur, &formulas, &cfvos, &colors),
                    b"conditionalFormatting" => {
                        if let Some(mut b) = cur.take() {
                            b.rules.sort_by_key(|r| r.priority);
                            out.push(b);
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    out
}

fn operand(f: &str) -> Operand {
    let t = f.trim();
    if let Ok(n) = t.parse::<f64>() {
        return Operand::Num(n);
    }
    if t.len() >= 2 && t.starts_with('"') && t.ends_with('"') {
        return Operand::Text(t[1..t.len() - 1].replace("\"\"", "\""));
    }
    Operand::Formula(t.to_string())
}

fn finish_rule(
    rule: &mut Option<(Rule, String, Option<String>, Option<String>)>,
    cur: &mut Option<Block>,
    formulas: &[String],
    cfvos: &[Cfvo],
    colors: &[u32],
) {
    let Some((mut rl, t, op, text)) = rule.take() else { return };
    match t.as_str() {
        "cellIs" => {
            let op = match op.as_deref() {
                Some("lessThan") => Op::Lt,
                Some("lessThanOrEqual") => Op::Le,
                Some("equal") => Op::Eq,
                Some("notEqual") => Op::Ne,
                Some("greaterThanOrEqual") => Op::Ge,
                Some("greaterThan") => Op::Gt,
                Some("between") => Op::Between,
                Some("notBetween") => Op::NotBetween,
                _ => Op::Eq,
            };
            if let Some(a) = formulas.first() {
                rl.kind = Kind2::CellIs { op, a: operand(a), b: formulas.get(1).map(|f| operand(f)) };
            }
        }
        "expression" => {
            if let Some(f) = formulas.first() {
                rl.kind = Kind2::Expression(f.clone());
            }
        }
        "containsText" | "notContainsText" | "beginsWith" | "endsWith" => {
            let mode = match t.as_str() {
                "notContainsText" => TextMode::NotContains,
                "beginsWith" => TextMode::Begins,
                "endsWith" => TextMode::Ends,
                _ => TextMode::Contains,
            };
            rl.kind = Kind2::Text { mode, text: text.unwrap_or_default().to_lowercase() };
        }
        "colorScale" if cfvos.len() >= 2 && colors.len() >= cfvos.len() => {
            rl.kind = Kind2::ColorScale(cfvos.iter().cloned().zip(colors.iter().copied()).collect());
        }
        "dataBar" if cfvos.len() >= 2 => {
            rl.kind = Kind2::DataBar { min: cfvos[0].clone(), max: cfvos[1].clone(), color: colors.first().copied().unwrap_or(0x638EC6) };
        }
        _ => {}
    }
    if let Some(b) = cur {
        b.rules.push(rl);
    }
}

pub fn parse_sqref(s: &str) -> Vec<Rect> {
    s.split_whitespace()
        .filter_map(|p| refshift::parse_range_ref(p.trim_start_matches('$')))
        .map(|(r0, c0, r1, c1)| Rect { r0, c0, r1, c1 })
        .collect()
}

/// Shift blocks after a structural edit on their sheet.
pub fn shift(blocks: &[Block], op: refshift::StructOp) -> Vec<Block> {
    blocks
        .iter()
        .filter_map(|b| {
            let sq = refshift::shift_sqref(&b.sqref, op).unwrap_or_else(|| b.sqref.clone());
            if sq.is_empty() {
                return None;
            }
            Some(Block { rects: parse_sqref(&sq), sqref: sq, rules: b.rules.clone() })
        })
        .collect()
}

// ---- evaluation -----------------------------------------------------------------

/// Formatting a cell gets from its conditional rules.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Look {
    pub fill: Option<u32>,
    pub font: Option<u32>,
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub underline: bool,
    /// Data bar length 0..1 and colour.
    pub bar: Option<(f32, u32)>,
}

/// Aggregates over a block's range, cached per sheet version.
#[derive(Default)]
struct Stats {
    sorted: Vec<f64>,
    mean: f64,
    std: f64,
    counts: HashMap<u64, u32>,
}

#[derive(Default)]
pub struct Cache {
    version: u64,
    stats: HashMap<(usize, usize), Arc<Stats>>,
}

/// Evaluate hook for formula operands/expressions (set once the engine is present).
pub type FormulaEval<'a> = &'a dyn Fn(&str, u32, u32) -> Option<Cell>;

fn key_of(s: &Sheet, v: Cell) -> u64 {
    match v.as_str_id() {
        Some(id) => {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            s.strings.get(id).to_lowercase().hash(&mut h);
            h.finish() | 1
        }
        None => v.bits() & !1,
    }
}

fn stats_for(s: &Sheet, b: &Block) -> Stats {
    let mut st = Stats::default();
    let (rows, cols) = (s.row_count(), s.col_count());
    for rect in &b.rects {
        if rect.r0 >= rows || rect.c0 >= cols {
            continue;
        }
        for r in rect.r0..=rect.r1.min(rows - 1) {
            for c in rect.c0..=rect.c1.min(cols - 1) {
                let v = s.get(r, c);
                if v.is_empty() {
                    continue;
                }
                *st.counts.entry(key_of(s, v)).or_insert(0) += 1;
                if let Some(n) = v.as_number() {
                    st.sorted.push(n);
                }
            }
        }
    }
    st.sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = st.sorted.len() as f64;
    if n > 0.0 {
        st.mean = st.sorted.iter().sum::<f64>() / n;
        st.std = (st.sorted.iter().map(|x| (x - st.mean).powi(2)).sum::<f64>() / n.max(1.0)).sqrt();
    }
    st
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let k = (p / 100.0).clamp(0.0, 1.0) * (sorted.len() - 1) as f64;
    let (lo, hi) = (k.floor() as usize, k.ceil() as usize);
    sorted[lo] + (sorted[hi] - sorted[lo]) * (k - lo as f64)
}

fn cfvo_value(v: &Cfvo, st: &Stats, eval: Option<FormulaEval<'_>>, r: u32, c: u32) -> f64 {
    let (min, max) = (st.sorted.first().copied().unwrap_or(0.0), st.sorted.last().copied().unwrap_or(0.0));
    match v {
        Cfvo::Min => min,
        Cfvo::Max => max,
        Cfvo::Num(n) => *n,
        Cfvo::Percent(p) => min + (max - min) * p / 100.0,
        Cfvo::Percentile(p) => percentile(&st.sorted, *p),
        Cfvo::Formula(f) => eval.and_then(|e| e(f, r, c)).and_then(|v| v.as_number()).unwrap_or(min),
    }
}

fn lerp(a: u32, b: u32, t: f64) -> u32 {
    let ch = |s: u32| [(a >> s) & 0xFF, (b >> s) & 0xFF];
    let mix = |s: u32| {
        let [x, y] = ch(s);
        ((x as f64 + (y as f64 - x as f64) * t).round() as u32) << s
    };
    mix(16) | mix(8) | mix(0)
}

fn compare(v: Cell, s: &Sheet, operand: &Operand, eval: Option<FormulaEval<'_>>, r: u32, c: u32) -> Option<std::cmp::Ordering> {
    let rhs: Cell = match operand {
        Operand::Num(n) => Cell::number(*n),
        Operand::Text(_) => Cell::EMPTY,
        Operand::Formula(f) => eval?(f, r, c)?,
    };
    match (v.kind(), operand) {
        (Kind::Str, Operand::Text(t)) => Some(s.strings.get(v.as_str_id()?).to_lowercase().cmp(&t.to_lowercase())),
        (Kind::Number, _) | (Kind::Empty, _) => {
            let a = v.as_number().unwrap_or(0.0);
            let b = rhs.as_number()?;
            a.partial_cmp(&b)
        }
        _ => None,
    }
}

impl Cache {
    fn get(&mut self, s: &Sheet, bi: usize, b: &Block) -> Arc<Stats> {
        if self.version != s.version {
            self.stats.clear();
            self.version = s.version;
        }
        self.stats.entry((bi, 0)).or_insert_with(|| Arc::new(stats_for(s, b))).clone()
    }
}

/// Conditional look of one cell (None when no rule applies).
pub fn look(s: &Sheet, cache: &mut Cache, styles: &Styles, r: u32, c: u32, eval: Option<FormulaEval<'_>>) -> Option<Look> {
    let blocks = s.grid.cf.clone();
    if blocks.is_empty() {
        return None;
    }
    let mut hits: Vec<(i32, &Rule, usize)> = Vec::new();
    for (bi, b) in blocks.iter().enumerate() {
        if b.contains(r, c) {
            for rule in &b.rules {
                hits.push((rule.priority, rule, bi));
            }
        }
    }
    if hits.is_empty() {
        return None;
    }
    hits.sort_by_key(|h| h.0);
    let v = s.get(r, c);
    let mut look = Look::default();
    let mut any = false;
    let (mut fill_set, mut font_set, mut b_set, mut i_set, mut s_set, mut u_set) = (false, false, false, false, false, false);
    for (_, rule, bi) in hits {
        let b = &blocks[bi];
        let matched = match &rule.kind {
            Kind2::CellIs { op, a, b: second } => {
                if v.is_empty() && !matches!(a, Operand::Num(_)) {
                    false
                } else {
                    match op {
                        Op::Between | Op::NotBetween => {
                            let lo = compare(v, s, a, eval, r, c);
                            let hi = second.as_ref().and_then(|x| compare(v, s, x, eval, r, c));
                            let inside = matches!(lo, Some(o) if o != std::cmp::Ordering::Less)
                                && matches!(hi, Some(o) if o != std::cmp::Ordering::Greater);
                            if *op == Op::Between { inside } else { lo.is_some() && hi.is_some() && !inside }
                        }
                        _ => match compare(v, s, a, eval, r, c) {
                            Some(o) => match op {
                                Op::Lt => o.is_lt(),
                                Op::Le => o.is_le(),
                                Op::Eq => o.is_eq(),
                                Op::Ne => o.is_ne(),
                                Op::Ge => o.is_ge(),
                                Op::Gt => o.is_gt(),
                                _ => false,
                            },
                            None => *op == Op::Ne,
                        },
                    }
                }
            }
            Kind2::Expression(f) => eval.and_then(|e| e(f, r, c)).is_some_and(|x| match x.kind() {
                Kind::Bool => x.as_bool().unwrap(),
                Kind::Number => x.as_number().unwrap() != 0.0,
                _ => false,
            }),
            Kind2::Text { mode, text } => {
                let t = match v.as_str_id() {
                    Some(id) => s.strings.get(id).to_lowercase(),
                    None => v.as_number().map(|n| n.to_string()).unwrap_or_default(),
                };
                match mode {
                    TextMode::Contains => !v.is_empty() && t.contains(text.as_str()),
                    TextMode::NotContains => !t.contains(text.as_str()),
                    TextMode::Begins => t.starts_with(text.as_str()),
                    TextMode::Ends => t.ends_with(text.as_str()),
                }
            }
            Kind2::Blanks(want) => {
                let blank = v.is_empty() || v.as_str_id().is_some_and(|id| s.strings.get(id).trim().is_empty());
                blank == *want
            }
            Kind2::Errors(want) => (v.kind() == Kind::Error) == *want,
            Kind2::Duplicate { unique } => {
                if v.is_empty() {
                    false
                } else {
                    let st = cache.get(s, bi, b);
                    let n = st.counts.get(&key_of(s, v)).copied().unwrap_or(0);
                    if *unique { n == 1 } else { n > 1 }
                }
            }
            Kind2::Top { rank, percent, bottom } => match v.as_number() {
                Some(x) => {
                    let st = cache.get(s, bi, b);
                    let n = st.sorted.len();
                    if n == 0 {
                        false
                    } else {
                        let k =
                            if *percent { ((n as f64 * *rank as f64 / 100.0).floor() as usize).max(1) } else { *rank as usize }.clamp(1, n);
                        if *bottom { x <= st.sorted[k - 1] } else { x >= st.sorted[n - k] }
                    }
                }
                None => false,
            },
            Kind2::Average { above, equal, std_dev } => match v.as_number() {
                Some(x) => {
                    let st = cache.get(s, bi, b);
                    let t = st.mean + if *above { 1.0 } else { -1.0 } * *std_dev as f64 * st.std;
                    if *above { x > t || (*equal && x == t) } else { x < t || (*equal && x == t) }
                }
                None => false,
            },
            Kind2::ColorScale(stops) => {
                if let Some(x) = v.as_number() {
                    let st = cache.get(s, bi, b);
                    let pts: Vec<(f64, u32)> = stops.iter().map(|(cv, col)| (cfvo_value(cv, &st, eval, r, c), *col)).collect();
                    let color = if x <= pts[0].0 {
                        pts[0].1
                    } else if x >= pts[pts.len() - 1].0 {
                        pts[pts.len() - 1].1
                    } else {
                        let i = pts.windows(2).position(|w| x >= w[0].0 && x <= w[1].0).unwrap_or(0);
                        let (a, bb) = (pts[i], pts[i + 1]);
                        let t = if bb.0 > a.0 { (x - a.0) / (bb.0 - a.0) } else { 0.0 };
                        lerp(a.1, bb.1, t)
                    };
                    if !fill_set {
                        look.fill = Some(color);
                        fill_set = true;
                        any = true;
                    }
                }
                continue;
            }
            Kind2::DataBar { min, max, color } => {
                if let Some(x) = v.as_number()
                    && look.bar.is_none()
                {
                    let st = cache.get(s, bi, b);
                    let (lo, hi) = (cfvo_value(min, &st, eval, r, c), cfvo_value(max, &st, eval, r, c));
                    let t = if hi > lo { ((x - lo) / (hi - lo)).clamp(0.0, 1.0) } else { 1.0 };
                    // Excel's default bars span 10%–90% of the cell.
                    look.bar = Some((0.1 + 0.8 * t as f32, *color));
                    any = true;
                }
                continue;
            }
            Kind2::Unsupported => false,
        };
        if !matched {
            continue;
        }
        if let Some(d) = rule.dxf.and_then(|i| styles.dxfs.get(i as usize)) {
            any = true;
            if !fill_set && let Some(f) = d.fill {
                look.fill = Some(f);
                fill_set = true;
            }
            if !font_set && let Some(f) = d.font_color {
                look.font = Some(f);
                font_set = true;
            }
            if !b_set && let Some(x) = d.bold {
                look.bold = x;
                b_set = true;
            }
            if !i_set && let Some(x) = d.italic {
                look.italic = x;
                i_set = true;
            }
            if !s_set && let Some(x) = d.strike {
                look.strike = x;
                s_set = true;
            }
            if !u_set && let Some(x) = d.underline {
                look.underline = x;
                u_set = true;
            }
        }
        if rule.stop {
            break;
        }
    }
    any.then_some(look)
}
