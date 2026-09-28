//! Cell styles: parses `styles.xml` (+ theme colours) into render-ready
//! styles, and derives new styles by *appending* fonts/fills/borders/xfs so
//! existing indices never change — the file saves back byte-for-byte except
//! for the appended entries.

use std::collections::HashMap;
use std::sync::Arc;

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

use crate::numfmt::{self, NumFmt};
use crate::xml::{self, attr, attr_bool, attr_f32, attr_u32, local};

#[derive(Clone, Debug, PartialEq)]
pub struct Font {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub size: f32,
    pub name: String,
    /// Resolved RGB, None = automatic (black).
    pub color: Option<u32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Side {
    pub style: u8,
    pub color: Option<u32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Xf {
    pub numfmt: u32,
    pub font: u32,
    pub fill: u32,
    pub border: u32,
    pub halign: u8,
    pub valign: u8,
    pub wrap: bool,
    pub indent: u8,
    pub rotation: i16,
}

#[derive(Clone)]
struct FontEntry {
    f: Font,
    color_raw: Option<String>,
    rest_raw: String,
}

#[derive(Clone)]
struct XfEntry {
    x: Xf,
    attrs_raw: String,
    align_extra: String,
    tail_raw: String,
}

pub const HALIGN: [&str; 8] = ["general", "left", "center", "right", "fill", "justify", "centerContinuous", "distributed"];
pub const VALIGN: [&str; 5] = ["bottom", "center", "top", "justify", "distributed"];
pub const BORDER_STYLES: [&str; 14] = [
    "none",
    "thin",
    "medium",
    "dashed",
    "dotted",
    "thick",
    "double",
    "hair",
    "mediumDashed",
    "dashDot",
    "mediumDashDot",
    "dashDotDot",
    "mediumDashDotDot",
    "slantDashDot",
];

/// A requested formatting change.
#[derive(Clone, Debug)]
pub enum StyleChange {
    Bold(bool),
    Italic(bool),
    Underline(bool),
    Strike(bool),
    FontSize(f32),
    FontName(String),
    TextColor(Option<u32>),
    Fill(Option<u32>),
    NumFmt(String),
    HAlign(u8),
    VAlign(u8),
    Wrap(bool),
    /// left, right, top, bottom
    Borders([Option<Side>; 4]),
}

/// Differential format used by conditional formatting.
#[derive(Clone, Debug, Default)]
pub struct Dxf {
    pub font_color: Option<u32>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub strike: Option<bool>,
    pub fill: Option<u32>,
    pub numfmt: Option<String>,
}

pub struct Styles {
    pub source: Option<Vec<u8>>,
    pub dxfs: Vec<Dxf>,
    theme: Vec<u32>,
    indexed: Vec<u32>,
    fonts: Vec<FontEntry>,
    fills: Vec<(Option<u32>, String)>,
    borders: Vec<([Side; 4], String)>,
    xfs: Vec<XfEntry>,
    pub numfmts: HashMap<u32, String>,
    orig: (usize, usize, usize, usize),
    new_numfmts: Vec<u32>,
    compiled: HashMap<u32, Arc<NumFmt>>,
    xf_by_raw: HashMap<String, u16>,
    /// Bumped on every change so the UI knows to refresh its style cache.
    pub generation: u32,
}

const MINIMAL_STYLES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><fonts count="1"><font><sz val="11"/><name val="Calibri"/><family val="2"/></font></fonts><fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills><borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders><cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs><cellXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/></cellXfs><cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles></styleSheet>"#;

/// Office default theme colours in `theme="n"` order (lt1, dk1, lt2, dk2, accent1–6, hlink, folHlink).
const DEFAULT_THEME: [u32; 12] =
    [0xFFFFFF, 0x000000, 0xE7E6E6, 0x44546A, 0x4472C4, 0xED7D31, 0xA5A5A5, 0xFFC000, 0x5B9BD5, 0x70AD47, 0x0563C1, 0x954F72];

pub const DEFAULT_INDEXED: [u32; 64] = [
    0x000000, 0xFFFFFF, 0xFF0000, 0x00FF00, 0x0000FF, 0xFFFF00, 0xFF00FF, 0x00FFFF, 0x000000, 0xFFFFFF, 0xFF0000, 0x00FF00, 0x0000FF,
    0xFFFF00, 0xFF00FF, 0x00FFFF, 0x800000, 0x008000, 0x000080, 0x808000, 0x800080, 0x008080, 0xC0C0C0, 0x808080, 0x9999FF, 0x993366,
    0xFFFFCC, 0xCCFFFF, 0x660066, 0xFF8080, 0x0066CC, 0xCCCCFF, 0x000080, 0xFF00FF, 0xFFFF00, 0x00FFFF, 0x800080, 0x800000, 0x008080,
    0x0000FF, 0x00CCFF, 0xCCFFFF, 0xCCFFCC, 0xFFFF99, 0x99CCFF, 0xFF99CC, 0xCC99FF, 0xFFCC99, 0x3366FF, 0x33CCCC, 0x99CC00, 0xFFCC00,
    0xFF9900, 0xFF6600, 0x666699, 0x969696, 0x003366, 0x339966, 0x003300, 0x333300, 0x993300, 0x993366, 0x333399, 0x333333,
];

impl Styles {
    pub fn minimal() -> Styles {
        Styles::parse(MINIMAL_STYLES.as_bytes().to_vec(), None)
    }

    pub fn parse(xml: Vec<u8>, theme_xml: Option<&[u8]>) -> Styles {
        let theme = theme_xml.map(parse_theme).unwrap_or_else(|| DEFAULT_THEME.to_vec());
        let mut s = Styles {
            source: None,
            dxfs: Vec::new(),
            theme,
            indexed: DEFAULT_INDEXED.to_vec(),
            fonts: Vec::new(),
            fills: Vec::new(),
            borders: Vec::new(),
            xfs: Vec::new(),
            numfmts: HashMap::new(),
            orig: (0, 0, 0, 0),
            new_numfmts: Vec::new(),
            compiled: HashMap::new(),
            xf_by_raw: HashMap::new(),
            generation: 0,
        };
        s.read(&xml);
        if s.fonts.is_empty() {
            s.fonts.push(FontEntry { f: default_font(), color_raw: None, rest_raw: String::new() });
        }
        if s.fills.is_empty() {
            s.fills.push((None, "<fill><patternFill patternType=\"none\"/></fill>".into()));
        }
        if s.borders.is_empty() {
            s.borders.push(([Side::default(); 4], "<border><left/><right/><top/><bottom/><diagonal/></border>".into()));
        }
        if s.xfs.is_empty() {
            s.xfs.push(XfEntry { x: Xf::default(), attrs_raw: " xfId=\"0\"".into(), align_extra: String::new(), tail_raw: String::new() });
        }
        s.orig = (s.fonts.len(), s.fills.len(), s.borders.len(), s.xfs.len());
        s.source = Some(xml);
        s
    }

    fn read(&mut self, xml: &[u8]) {
        // Custom indexed palette first, since colours below may refer to it.
        if let Some(sec) = xml::find_section(xml, b"indexedColors", b"rgbColor") {
            let mut pal = Vec::new();
            for span in sec.children {
                let mut r = Reader::from_reader(&xml[span]);
                if let Ok(Event::Empty(e) | Event::Start(e)) = r.read_event() {
                    pal.push(attr(&e, b"rgb").and_then(|v| parse_argb(&v)).unwrap_or(0));
                }
            }
            if !pal.is_empty() {
                self.indexed = pal;
            }
        }
        if let Some(sec) = xml::find_section(xml, b"numFmts", b"numFmt") {
            for span in sec.children {
                let mut r = Reader::from_reader(&xml[span]);
                if let Ok(Event::Empty(e) | Event::Start(e)) = r.read_event()
                    && let (Some(id), Some(code)) = (attr_u32(&e, b"numFmtId"), attr(&e, b"formatCode"))
                {
                    self.numfmts.insert(id, code);
                }
            }
        }
        if let Some(sec) = xml::find_section(xml, b"fonts", b"font") {
            for span in sec.children {
                let entry = self.parse_font(&xml[span]);
                self.fonts.push(entry);
            }
        }
        if let Some(sec) = xml::find_section(xml, b"fills", b"fill") {
            for span in sec.children {
                let raw = String::from_utf8_lossy(&xml[span.clone()]).into_owned();
                self.fills.push((self.parse_fill(&xml[span]), raw));
            }
        }
        if let Some(sec) = xml::find_section(xml, b"borders", b"border") {
            for span in sec.children {
                let raw = String::from_utf8_lossy(&xml[span.clone()]).into_owned();
                self.borders.push((self.parse_border(&xml[span]), raw));
            }
        }
        if let Some(sec) = xml::find_section(xml, b"dxfs", b"dxf") {
            for span in sec.children {
                let d = self.parse_dxf(&xml[span]);
                self.dxfs.push(d);
            }
        }
        if let Some(sec) = xml::find_section(xml, b"cellXfs", b"xf") {
            for span in sec.children {
                let raw = String::from_utf8_lossy(&xml[span.clone()]).into_owned();
                let e = parse_xf(&xml[span]);
                self.xf_by_raw.entry(raw).or_insert(self.xfs.len() as u16);
                self.xfs.push(e);
            }
        }
    }

    fn parse_dxf(&self, raw: &[u8]) -> Dxf {
        let mut d = Dxf::default();
        let mut r = Reader::from_reader(raw);
        let mut ctx: Vec<Vec<u8>> = Vec::new();
        let mut pattern: Option<String> = None;
        let (mut fg, mut bg) = (None, None);
        loop {
            match r.read_event() {
                Ok(ev @ (Event::Start(_) | Event::Empty(_))) => {
                    let empty = matches!(ev, Event::Empty(_));
                    let (Event::Start(e) | Event::Empty(e)) = ev else { unreachable!() };
                    let qn = e.name();
                    let n = local(qn.as_ref()).to_vec();
                    let in_font = ctx.iter().any(|c| c == b"font");
                    let val_on = || attr(&e, b"val").as_deref().is_none_or(|v| v != "0" && v != "false");
                    match n.as_slice() {
                        b"b" if in_font => d.bold = Some(val_on()),
                        b"i" if in_font => d.italic = Some(val_on()),
                        b"strike" if in_font => d.strike = Some(val_on()),
                        b"u" if in_font => d.underline = Some(attr(&e, b"val").as_deref() != Some("none")),
                        b"color" if in_font => d.font_color = self.color(&e),
                        b"patternFill" => pattern = attr(&e, b"patternType"),
                        b"fgColor" => fg = self.color(&e),
                        b"bgColor" => bg = self.color(&e),
                        b"numFmt" => d.numfmt = attr(&e, b"formatCode"),
                        _ => {}
                    }
                    if !empty {
                        ctx.push(n);
                    }
                }
                Ok(Event::End(_)) => {
                    ctx.pop();
                }
                Ok(Event::Eof) | Err(_) => break,
                _ => {}
            }
        }
        // In a dxf, a solid fill's colour is usually in bgColor.
        d.fill = match pattern.as_deref() {
            Some("none") => None,
            Some("solid") | None => bg.or(fg),
            _ => fg.or(bg),
        };
        d
    }

    /// Theme colour by `theme="n"` index.
    pub fn theme_color(&self, i: usize) -> u32 {
        self.theme.get(i).copied().unwrap_or(0)
    }

    /// Resolve a colour element (rgb / theme+tint / indexed).
    pub fn resolve_color(&self, e: &BytesStart<'_>) -> Option<u32> {
        self.color(e)
    }

    fn color(&self, e: &BytesStart<'_>) -> Option<u32> {
        if attr_bool(e, b"auto") == Some(true) {
            return None;
        }
        let base = if let Some(v) = attr(e, b"rgb") {
            parse_argb(&v)?
        } else if let Some(t) = attr_u32(e, b"theme") {
            *self.theme.get(t as usize)?
        } else if let Some(i) = attr_u32(e, b"indexed") {
            if i >= 64 {
                return None; // system foreground/background
            }
            *self.indexed.get(i as usize)?
        } else {
            return None;
        };
        let tint = attr_f32(e, b"tint").unwrap_or(0.0);
        Some(if tint != 0.0 { apply_tint(base, tint as f64) } else { base })
    }

    fn parse_font(&self, raw: &[u8]) -> FontEntry {
        let mut f = default_font();
        f.name.clear();
        let mut color_raw = None;
        let mut rest = String::new();
        let mut r = Reader::from_reader(raw);
        let mut depth = 0;
        loop {
            let start = r.buffer_position() as usize;
            match r.read_event() {
                Ok(Event::Start(e)) => {
                    depth += 1;
                    if depth == 1 {
                        continue;
                    }
                    // Unexpected nested content: keep raw.
                    let _ = r.read_to_end(e.name());
                    rest.push_str(&String::from_utf8_lossy(&raw[start..r.buffer_position() as usize]));
                }
                Ok(Event::Empty(e)) => {
                    let val = attr(&e, b"val");
                    let on = val.as_deref().is_none_or(|v| v != "0" && v != "false");
                    let text = String::from_utf8_lossy(&raw[start..r.buffer_position() as usize]).into_owned();
                    match local(e.name().as_ref()) {
                        b"b" => f.bold = on,
                        b"i" => f.italic = on,
                        b"strike" => f.strike = on,
                        b"u" => f.underline = val.as_deref() != Some("none"),
                        b"sz" => f.size = val.and_then(|v| v.parse().ok()).unwrap_or(11.0),
                        b"name" | b"rFont" => f.name = val.unwrap_or_default(),
                        b"color" => {
                            f.color = self.color(&e);
                            color_raw = Some(text);
                        }
                        b"font" => {}
                        _ => rest.push_str(&text),
                    }
                }
                Ok(Event::Eof) | Err(_) => break,
                _ => {}
            }
        }
        if f.name.is_empty() {
            f.name = "Calibri".into();
        }
        FontEntry { f, color_raw, rest_raw: rest }
    }

    fn parse_fill(&self, raw: &[u8]) -> Option<u32> {
        let mut r = Reader::from_reader(raw);
        let mut pattern = None::<String>;
        let mut fg = None;
        let mut bg = None;
        let mut gradient = false;
        loop {
            match r.read_event() {
                Ok(Event::Start(e) | Event::Empty(e)) => match local(e.name().as_ref()) {
                    b"patternFill" => pattern = Some(attr(&e, b"patternType").unwrap_or_else(|| "none".into())),
                    b"fgColor" => fg = self.color(&e),
                    b"bgColor" => bg = self.color(&e),
                    b"gradientFill" => gradient = true,
                    b"color" if gradient && fg.is_none() => fg = self.color(&e),
                    _ => {}
                },
                Ok(Event::Eof) | Err(_) => break,
                _ => {}
            }
        }
        if gradient {
            return fg;
        }
        match pattern.as_deref() {
            None | Some("none") => None,
            Some("solid") => fg.or(bg),
            Some(_) => fg.or(bg), // patterns are approximated as their foreground colour
        }
    }

    fn parse_border(&self, raw: &[u8]) -> [Side; 4] {
        let mut sides = [Side::default(); 4];
        let mut r = Reader::from_reader(raw);
        let mut cur: Option<usize> = None;
        loop {
            match r.read_event() {
                Ok(ev @ (Event::Start(_) | Event::Empty(_))) => {
                    let empty = matches!(ev, Event::Empty(_));
                    let (Event::Start(e) | Event::Empty(e)) = ev else { unreachable!() };
                    let idx = match local(e.name().as_ref()) {
                        b"left" | b"start" => Some(0),
                        b"right" | b"end" => Some(1),
                        b"top" => Some(2),
                        b"bottom" => Some(3),
                        b"color" => {
                            if let Some(i) = cur {
                                sides[i].color = self.color(&e);
                            }
                            None
                        }
                        _ => None,
                    };
                    if let Some(i) = idx {
                        let st = attr(&e, b"style").unwrap_or_default();
                        sides[i].style = BORDER_STYLES.iter().position(|s| *s == st).unwrap_or(0) as u8;
                        cur = if empty { None } else { Some(i) };
                    }
                }
                Ok(Event::End(e)) => {
                    if matches!(local(e.name().as_ref()), b"left" | b"start" | b"right" | b"end" | b"top" | b"bottom") {
                        cur = None;
                    }
                }
                Ok(Event::Eof) | Err(_) => break,
                _ => {}
            }
        }
        sides
    }

    // ---- lookups ----------------------------------------------------------

    pub fn xf_count(&self) -> usize {
        self.xfs.len()
    }
    pub fn xf(&self, id: u16) -> &Xf {
        &self.xfs.get(id as usize).unwrap_or(&self.xfs[0]).x
    }
    pub fn font(&self, id: u32) -> &Font {
        &self.fonts.get(id as usize).unwrap_or(&self.fonts[0]).f
    }
    pub fn fill(&self, id: u32) -> Option<u32> {
        self.fills.get(id as usize).and_then(|f| f.0)
    }
    pub fn border(&self, id: u32) -> [Side; 4] {
        self.borders.get(id as usize).map_or([Side::default(); 4], |b| b.0)
    }

    pub fn numfmt_code(&self, id: u32) -> &str {
        if let Some(c) = self.numfmts.get(&id) {
            return c;
        }
        numfmt::builtin_code(id).unwrap_or("General")
    }

    /// Compiled number format for an xf (cached).
    pub fn numfmt(&mut self, xf: u16) -> Arc<NumFmt> {
        let id = self.xf(xf).numfmt;
        if let Some(f) = self.compiled.get(&id) {
            return f.clone();
        }
        let f = Arc::new(NumFmt::parse(self.numfmt_code(id)));
        self.compiled.insert(id, f.clone());
        f
    }

    pub fn is_date_xf(&mut self, xf: u16) -> bool {
        self.numfmt(xf).is_date()
    }

    // ---- deriving new styles ---------------------------------------------

    /// xf equal to `base` with `change` applied (appending entries as needed).
    pub fn derive(&mut self, base: u16, change: &StyleChange) -> u16 {
        let mut e = self.xfs.get(base as usize).unwrap_or(&self.xfs[0]).clone();
        match change {
            StyleChange::Bold(_)
            | StyleChange::Italic(_)
            | StyleChange::Underline(_)
            | StyleChange::Strike(_)
            | StyleChange::FontSize(_)
            | StyleChange::FontName(_)
            | StyleChange::TextColor(_) => {
                let mut fe = self.fonts.get(e.x.font as usize).unwrap_or(&self.fonts[0]).clone();
                match change {
                    StyleChange::Bold(v) => fe.f.bold = *v,
                    StyleChange::Italic(v) => fe.f.italic = *v,
                    StyleChange::Underline(v) => fe.f.underline = *v,
                    StyleChange::Strike(v) => fe.f.strike = *v,
                    StyleChange::FontSize(v) => fe.f.size = *v,
                    StyleChange::FontName(v) => {
                        fe.f.name = v.clone();
                        // A theme font scheme would override the explicit name.
                        fe.rest_raw = strip_elements(&fe.rest_raw, &["scheme"]);
                    }
                    StyleChange::TextColor(c) => {
                        fe.f.color = *c;
                        fe.color_raw = c.map(|c| format!("<color rgb=\"FF{c:06X}\"/>"));
                    }
                    _ => unreachable!(),
                }
                e.x.font = self.find_or_add_font(fe);
            }
            StyleChange::Fill(c) => {
                e.x.fill = match c {
                    None => 0,
                    Some(c) => {
                        let raw = format!(
                            "<fill><patternFill patternType=\"solid\"><fgColor rgb=\"FF{c:06X}\"/><bgColor indexed=\"64\"/></patternFill></fill>"
                        );
                        match self.fills.iter().position(|f| f.1 == raw) {
                            Some(i) => i as u32,
                            None => {
                                self.fills.push((Some(*c), raw));
                                self.fills.len() as u32 - 1
                            }
                        }
                    }
                };
            }
            StyleChange::NumFmt(code) => e.x.numfmt = self.numfmt_id(code),
            StyleChange::HAlign(a) => e.x.halign = *a,
            StyleChange::VAlign(a) => e.x.valign = *a,
            StyleChange::Wrap(w) => e.x.wrap = *w,
            StyleChange::Borders(sides) => {
                let mut cur = self.border(e.x.border);
                for (i, s) in sides.iter().enumerate() {
                    if let Some(s) = s {
                        cur[i] = *s;
                    }
                }
                let raw = border_xml(&cur);
                e.x.border = match self.borders.iter().position(|b| b.1 == raw) {
                    Some(i) => i as u32,
                    None => {
                        self.borders.push((cur, raw));
                        self.borders.len() as u32 - 1
                    }
                };
            }
        }
        let raw = xf_xml(&e);
        if let Some(&i) = self.xf_by_raw.get(&raw) {
            return i;
        }
        if self.xfs.len() >= u16::MAX as usize {
            return base;
        }
        self.xfs.push(e);
        let id = self.xfs.len() as u16 - 1;
        self.xf_by_raw.insert(raw, id);
        self.generation += 1;
        id
    }

    fn find_or_add_font(&mut self, fe: FontEntry) -> u32 {
        let raw = font_xml(&fe);
        if let Some(i) = self.fonts.iter().position(|f| font_xml(f) == raw) {
            return i as u32;
        }
        self.fonts.push(fe);
        self.fonts.len() as u32 - 1
    }

    pub fn numfmt_id(&mut self, code: &str) -> u32 {
        for id in 0..50 {
            if numfmt::builtin_code(id) == Some(code) {
                return id;
            }
        }
        if let Some((&id, _)) = self.numfmts.iter().find(|(_, c)| *c == code) {
            return id;
        }
        let id = self.numfmts.keys().copied().max().unwrap_or(163).max(163) + 1;
        self.numfmts.insert(id, code.to_string());
        self.new_numfmts.push(id);
        id
    }

    pub fn is_modified(&self) -> bool {
        self.xfs.len() != self.orig.3 || !self.new_numfmts.is_empty()
    }

    /// styles.xml with appended entries spliced in (unchanged bytes otherwise).
    pub fn to_xml(&self) -> Vec<u8> {
        let src = self.source.clone().unwrap_or_else(|| MINIMAL_STYLES.as_bytes().to_vec());
        if !self.is_modified() {
            return src;
        }
        struct Splice {
            at: usize,
            text: String,
            count_attr: Option<(std::ops::Range<usize>, usize)>,
        }
        let mut splices: Vec<Splice> = Vec::new();
        let mut add = |parent: &[u8], child: &[u8], items: Vec<String>, total: usize, before_if_missing: &[u8]| {
            if items.is_empty() {
                return;
            }
            match xml::find_section(&src, parent, child) {
                Some(sec) if sec.close.start != sec.close.end => splices.push(Splice {
                    at: sec.close.start,
                    text: items.concat(),
                    count_attr: count_span(&src[sec.open.clone()]).map(|r| (r.start + sec.open.start..r.end + sec.open.start, total)),
                }),
                Some(sec) => {
                    // Self-closing <fonts/>: replace it entirely.
                    let name = String::from_utf8_lossy(parent).into_owned();
                    splices.push(Splice {
                        at: sec.open.start,
                        text: format!("<{name} count=\"{total}\">{}</{name}>", items.concat()),
                        count_attr: Some((sec.open.clone(), usize::MAX)),
                    });
                }
                None => {
                    let name = String::from_utf8_lossy(parent).into_owned();
                    let at = find_tag_start(&src, before_if_missing).unwrap_or(0);
                    splices.push(Splice { at, text: format!("<{name} count=\"{total}\">{}</{name}>", items.concat()), count_attr: None });
                }
            }
        };
        let nf: Vec<String> = self
            .new_numfmts
            .iter()
            .map(|id| {
                let mut s = format!("<numFmt numFmtId=\"{id}\" formatCode=\"");
                xml::escape_attr(&self.numfmts[id], &mut s);
                s.push_str("\"/>");
                s
            })
            .collect();
        let old_nf = self.numfmts.len() - self.new_numfmts.len();
        add(b"numFmts", b"numFmt", nf, old_nf + self.new_numfmts.len(), b"fonts");
        add(b"fonts", b"font", self.fonts[self.orig.0..].iter().map(font_xml).collect(), self.fonts.len(), b"fills");
        add(b"fills", b"fill", self.fills[self.orig.1..].iter().map(|f| f.1.clone()).collect(), self.fills.len(), b"borders");
        add(b"borders", b"border", self.borders[self.orig.2..].iter().map(|b| b.1.clone()).collect(), self.borders.len(), b"cellStyleXfs");
        add(b"cellXfs", b"xf", self.xfs[self.orig.3..].iter().map(xf_xml).collect(), self.xfs.len(), b"cellStyles");

        splices.sort_by_key(|s| s.at);
        let mut out = Vec::with_capacity(src.len() + 4096);
        let mut pos = 0;
        // Count attributes sit before their section's closing tag, so process in order.
        let mut edits: Vec<(std::ops::Range<usize>, String)> = Vec::new();
        for s in splices {
            match s.count_attr {
                Some((range, usize::MAX)) => edits.push((range, s.text)),
                Some((range, total)) => {
                    edits.push((range, format!("count=\"{total}\"")));
                    edits.push((s.at..s.at, s.text));
                }
                None => edits.push((s.at..s.at, s.text)),
            }
        }
        edits.sort_by_key(|e| (e.0.start, e.0.end));
        for (range, text) in edits {
            out.extend_from_slice(&src[pos..range.start]);
            out.extend_from_slice(text.as_bytes());
            pos = range.end;
        }
        out.extend_from_slice(&src[pos..]);
        out
    }
}

fn find_tag_start(xml: &[u8], name: &[u8]) -> Option<usize> {
    let mut r = Reader::from_reader(xml);
    loop {
        let before = r.buffer_position() as usize;
        match r.read_event() {
            Ok(Event::Start(e) | Event::Empty(e)) if local(e.name().as_ref()) == name => return Some(before),
            Ok(Event::End(e)) if local(e.name().as_ref()) == b"styleSheet" => return Some(before),
            Ok(Event::Eof) | Err(_) => return None,
            _ => {}
        }
    }
}

/// Byte range of `count="…"` inside an opening tag.
fn count_span(open_tag: &[u8]) -> Option<std::ops::Range<usize>> {
    let i = memchr::memmem::find(open_tag, b" count=\"")? + 1;
    let end = memchr::memchr(b'"', &open_tag[i + 7..])? + i + 8;
    Some(i..end)
}

fn default_font() -> Font {
    Font { bold: false, italic: false, underline: false, strike: false, size: 11.0, name: "Calibri".into(), color: None }
}

fn parse_xf(raw: &[u8]) -> XfEntry {
    let mut r = Reader::from_reader(raw);
    let mut x = Xf::default();
    let mut attrs_raw = String::new();
    let mut align_extra = String::new();
    let mut tail = String::new();
    loop {
        let start = r.buffer_position() as usize;
        match r.read_event() {
            Ok(Event::Start(e) | Event::Empty(e)) => match local(e.name().as_ref()) {
                b"xf" => {
                    x.numfmt = attr_u32(&e, b"numFmtId").unwrap_or(0);
                    x.font = attr_u32(&e, b"fontId").unwrap_or(0);
                    x.fill = attr_u32(&e, b"fillId").unwrap_or(0);
                    x.border = attr_u32(&e, b"borderId").unwrap_or(0);
                    attrs_raw = xml::other_attrs(
                        &e,
                        &[
                            b"numFmtId",
                            b"fontId",
                            b"fillId",
                            b"borderId",
                            b"applyNumberFormat",
                            b"applyFont",
                            b"applyFill",
                            b"applyBorder",
                            b"applyAlignment",
                        ],
                    );
                }
                b"alignment" => {
                    let h = attr(&e, b"horizontal").unwrap_or_default();
                    x.halign = HALIGN.iter().position(|s| *s == h).unwrap_or(0) as u8;
                    let v = attr(&e, b"vertical").unwrap_or_default();
                    x.valign = VALIGN.iter().position(|s| *s == v).unwrap_or(0) as u8;
                    x.wrap = attr_bool(&e, b"wrapText").unwrap_or(false);
                    x.indent = attr_u32(&e, b"indent").unwrap_or(0).min(250) as u8;
                    x.rotation = attr_u32(&e, b"textRotation").unwrap_or(0) as i16;
                    align_extra = xml::other_attrs(&e, &[b"horizontal", b"vertical", b"wrapText", b"indent", b"textRotation"]);
                }
                _ => {
                    // protection / extLst: keep verbatim.
                    if let Ok(Event::Start(ref s)) = Reader::from_reader(&raw[start..]).read_event() {
                        let _ = r.read_to_end(s.name());
                    }
                    tail.push_str(&String::from_utf8_lossy(&raw[start..r.buffer_position() as usize]));
                }
            },
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    XfEntry { x, attrs_raw, align_extra, tail_raw: tail }
}

fn font_xml(fe: &FontEntry) -> String {
    let f = &fe.f;
    let mut s = String::from("<font>");
    if f.bold {
        s.push_str("<b/>");
    }
    if f.italic {
        s.push_str("<i/>");
    }
    if f.strike {
        s.push_str("<strike/>");
    }
    if f.underline {
        s.push_str("<u/>");
    }
    s.push_str(&format!("<sz val=\"{}\"/>", f.size));
    if let Some(c) = &fe.color_raw {
        s.push_str(c);
    }
    s.push_str("<name val=\"");
    xml::escape_attr(&f.name, &mut s);
    s.push_str("\"/>");
    s.push_str(&fe.rest_raw);
    s.push_str("</font>");
    s
}

fn xf_xml(e: &XfEntry) -> String {
    let x = &e.x;
    let mut s =
        format!("<xf numFmtId=\"{}\" fontId=\"{}\" fillId=\"{}\" borderId=\"{}\"{}", x.numfmt, x.font, x.fill, x.border, e.attrs_raw);
    let has_align = x.halign != 0 || x.valign != 0 || x.wrap || x.indent != 0 || x.rotation != 0 || !e.align_extra.is_empty();
    s.push_str(" applyNumberFormat=\"1\" applyFont=\"1\" applyFill=\"1\" applyBorder=\"1\"");
    if has_align {
        s.push_str(" applyAlignment=\"1\"");
    }
    if !has_align && e.tail_raw.is_empty() {
        s.push_str("/>");
        return s;
    }
    s.push('>');
    if has_align {
        s.push_str("<alignment");
        if x.halign != 0 {
            s.push_str(&format!(" horizontal=\"{}\"", HALIGN[x.halign as usize]));
        }
        if x.valign != 0 {
            s.push_str(&format!(" vertical=\"{}\"", VALIGN[x.valign as usize]));
        }
        if x.rotation != 0 {
            s.push_str(&format!(" textRotation=\"{}\"", x.rotation));
        }
        if x.wrap {
            s.push_str(" wrapText=\"1\"");
        }
        if x.indent != 0 {
            s.push_str(&format!(" indent=\"{}\"", x.indent));
        }
        s.push_str(&e.align_extra);
        s.push_str("/>");
    }
    s.push_str(&e.tail_raw);
    s.push_str("</xf>");
    s
}

fn border_xml(sides: &[Side; 4]) -> String {
    let mut s = String::from("<border>");
    for (i, name) in ["left", "right", "top", "bottom"].iter().enumerate() {
        let side = sides[i];
        if side.style == 0 {
            s.push_str(&format!("<{name}/>"));
        } else {
            s.push_str(&format!("<{name} style=\"{}\">", BORDER_STYLES[side.style as usize]));
            match side.color {
                Some(c) => s.push_str(&format!("<color rgb=\"FF{c:06X}\"/>")),
                None => s.push_str("<color indexed=\"64\"/>"),
            }
            s.push_str(&format!("</{name}>"));
        }
    }
    s.push_str("<diagonal/></border>");
    s
}

fn strip_elements(raw: &str, names: &[&str]) -> String {
    let mut out = String::new();
    let mut r = Reader::from_str(raw);
    loop {
        let start = r.buffer_position() as usize;
        match r.read_event() {
            Ok(Event::Empty(e)) => {
                let keep = !names.iter().any(|n| local(e.name().as_ref()) == n.as_bytes());
                if keep {
                    out.push_str(&raw[start..r.buffer_position() as usize]);
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            Ok(_) => out.push_str(&raw[start..r.buffer_position() as usize]),
        }
    }
    out
}

fn parse_argb(v: &str) -> Option<u32> {
    let v = v.trim();
    let hex = if v.len() == 8 { &v[2..] } else { v };
    u32::from_str_radix(hex, 16).ok().map(|c| c & 0xFFFFFF)
}

fn parse_theme(xml: &[u8]) -> Vec<u32> {
    let mut r = Reader::from_reader(xml);
    let mut in_scheme = false;
    let mut current: Option<usize> = None;
    let order = [
        b"dk1".as_slice(),
        b"lt1",
        b"dk2",
        b"lt2",
        b"accent1",
        b"accent2",
        b"accent3",
        b"accent4",
        b"accent5",
        b"accent6",
        b"hlink",
        b"folHlink",
    ];
    let mut scheme = [None; 12];
    loop {
        match r.read_event() {
            Ok(Event::Start(e) | Event::Empty(e)) => {
                let qn = e.name();
                let n = local(qn.as_ref());
                if n == b"clrScheme" {
                    in_scheme = true;
                } else if in_scheme {
                    if let Some(i) = order.iter().position(|o| *o == n) {
                        current = Some(i);
                    } else if let Some(i) = current {
                        let v = match n {
                            b"srgbClr" => attr(&e, b"val"),
                            b"sysClr" => attr(&e, b"lastClr"),
                            _ => None,
                        };
                        if let Some(c) = v.and_then(|v| u32::from_str_radix(&v, 16).ok()) {
                            scheme[i].get_or_insert(c);
                        }
                    }
                }
            }
            Ok(Event::End(e)) if local(e.name().as_ref()) == b"clrScheme" => break,
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    // Theme indices swap the first two pairs relative to clrScheme order.
    let pick = |i: usize| scheme[i].unwrap_or(DEFAULT_THEME[[1, 0, 3, 2, 4, 5, 6, 7, 8, 9, 10, 11][i]]);
    vec![pick(1), pick(0), pick(3), pick(2), pick(4), pick(5), pick(6), pick(7), pick(8), pick(9), pick(10), pick(11)]
}

/// Excel's tint: adjust HSL luminance towards black (tint < 0) or white (tint > 0).
pub fn apply_tint(rgb: u32, tint: f64) -> u32 {
    let (r, g, b) = (((rgb >> 16) & 0xFF) as f64 / 255.0, ((rgb >> 8) & 0xFF) as f64 / 255.0, (rgb & 0xFF) as f64 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let mut l = (max + min) / 2.0;
    let d = max - min;
    let (h, s) = if d == 0.0 {
        (0.0, 0.0)
    } else {
        let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
        let h = if max == r {
            (g - b) / d + if g < b { 6.0 } else { 0.0 }
        } else if max == g {
            (b - r) / d + 2.0
        } else {
            (r - g) / d + 4.0
        };
        (h / 6.0, s)
    };
    l = if tint < 0.0 { l * (1.0 + tint) } else { l * (1.0 - tint) + tint };
    let hue = |p: f64, q: f64, mut t: f64| {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    let (r, g, b) = if s == 0.0 {
        (l, l, l)
    } else {
        let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
        let p = 2.0 * l - q;
        (hue(p, q, h + 1.0 / 3.0), hue(p, q, h), hue(p, q, h - 1.0 / 3.0))
    };
    let c = |v: f64| (v * 255.0).round().clamp(0.0, 255.0) as u32;
    (c(r) << 16) | (c(g) << 8) | c(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_and_derive() {
        let mut s = Styles::minimal();
        assert_eq!(s.xf_count(), 1);
        let bold = s.derive(0, &StyleChange::Bold(true));
        assert_eq!(bold, 1);
        assert!(s.font(s.xf(bold).font).bold);
        assert_eq!(s.derive(0, &StyleChange::Bold(true)), bold, "deduplicated");
        let red = s.derive(bold, &StyleChange::Fill(Some(0xFF0000)));
        assert_eq!(s.fill(s.xf(red).fill), Some(0xFF0000));
        let pct = s.derive(0, &StyleChange::NumFmt("0.0%".into()));
        assert_eq!(s.numfmt_code(s.xf(pct).numfmt), "0.0%");
        let out = String::from_utf8(s.to_xml()).unwrap();
        assert!(out.contains("<cellXfs count=\"4\">"), "{out}");
        assert!(out.contains("<fonts count=\"2\">"));
        assert!(out.contains("<numFmts count=\"1\"><numFmt numFmtId=\"164\" formatCode=\"0.0%\"/></numFmts><fonts"));
        let reparsed = Styles::parse(out.into_bytes(), None);
        assert_eq!(reparsed.xf_count(), 4);
        assert!(reparsed.font(reparsed.xf(1).font).bold);
    }

    #[test]
    fn colors() {
        assert_eq!(apply_tint(0x000000, 0.5), 0x808080);
        assert_eq!(apply_tint(0xFFFFFF, -0.5), 0x808080);
        let xml = br#"<styleSheet><fonts count="1"><font><sz val="12"/><color theme="4" tint="0.0"/><name val="Arial"/><family val="2"/></font></fonts>
        <fills count="1"><fill><patternFill patternType="solid"><fgColor indexed="10"/></patternFill></fill></fills>
        <borders count="1"><border><left style="thin"><color rgb="FF00FF00"/></left><right/><top/><bottom style="thick"/><diagonal/></border></borders>
        <cellXfs count="1"><xf numFmtId="14" fontId="0" fillId="0" borderId="0" xfId="0" applyAlignment="1"><alignment horizontal="center" wrapText="1"/><protection locked="0"/></xf></cellXfs></styleSheet>"#;
        let mut s = Styles::parse(xml.to_vec(), None);
        assert_eq!(s.font(0).color, Some(0x4472C4));
        assert_eq!(s.font(0).name, "Arial");
        assert_eq!(s.fill(0), Some(0xFF0000));
        assert_eq!(s.border(0)[0], Side { style: 1, color: Some(0x00FF00) });
        assert_eq!(s.border(0)[3].style, 5);
        assert_eq!(s.xf(0).halign, 2);
        assert!(s.xf(0).wrap);
        assert!(s.is_date_xf(0));
        let b = s.derive(0, &StyleChange::Bold(true));
        let out = String::from_utf8(s.to_xml()).unwrap();
        assert!(out.contains("<protection locked=\"0\"/></xf></cellXfs>"), "{out}");
        assert!(out.contains("<family val=\"2\"/></font></fonts>"));
        assert_eq!(b, 1);
    }
}
