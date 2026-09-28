//! Loading display-only parts that hang off a worksheet: pictures/charts
//! (drawings) and Excel tables.

use quick_xml::Reader;
use quick_xml::events::Event;

use super::{ZipArchive, find_entry, parse_rels, read_entry, rel_is, rels_path, resolve};
use crate::drawings::{Anchor, Content, Drawing, EMU_PER_PX};
use crate::sheet::Rect;
use crate::tables::Table;
use crate::xml::{attr, attr_bool, local};

/// Drawings for a sheet part (via its `<drawing r:id>` relationship).
/// Drawings for a sheet part (via its `<drawing r:id>` relationship).
pub fn load_drawings(zip: &mut ZipArchive, sheet_path: &str, suffix: &[u8]) -> Vec<Drawing> {
    let mut out = Vec::new();
    let rid = {
        let mut r = Reader::from_reader(suffix);
        r.config_mut().check_end_names = false;
        let mut found = None;
        loop {
            match r.read_event() {
                Ok(Event::Empty(e) | Event::Start(e)) if local(e.name().as_ref()) == b"drawing" => {
                    found = attr(&e, b"id");
                    break;
                }
                Ok(Event::Eof) | Err(_) => break,
                _ => {}
            }
        }
        found
    };
    let Some(rid) = rid else { return out };
    let Ok(rels_xml) = read_entry(zip, &rels_path(sheet_path)) else { return out };
    let Some(rel) = parse_rels(&rels_xml).into_iter().find(|r| r.id == rid) else { return out };
    let dpath = resolve(sheet_path, &rel.target);
    let Some(dpath) = find_entry(zip, &dpath) else { return out };
    let Ok(dxml) = read_entry(zip, &dpath) else { return out };
    let drels = read_entry(zip, &rels_path(&dpath)).map(|x| parse_rels(&x)).unwrap_or_default();

    let mut r = Reader::from_reader(dxml.as_slice());
    let mut cur: Option<Drawing> = None;
    let mut which = 0u8; // 1 from, 2 to
    let mut field: Option<Vec<u8>> = None;
    loop {
        let ev = match r.read_event() {
            Ok(Event::Eof) | Err(_) => break,
            Ok(e) => e,
        };
        match ev {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let qn = e.name();
                let n = local(qn.as_ref());
                match n {
                    b"twoCellAnchor" | b"oneCellAnchor" | b"absoluteAnchor" => {
                        cur = Some(Drawing { content: Content::Chart(String::new()), from: Anchor::default(), to: None, size: (0.0, 0.0) });
                    }
                    b"from" => which = 1,
                    b"to" => which = 2,
                    b"col" | b"colOff" | b"row" | b"rowOff" if which > 0 => field = Some(n.to_vec()),
                    b"ext" => {
                        if let Some(d) = &mut cur
                            && d.size == (0.0, 0.0)
                        {
                            let cx = attr(e, b"cx").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
                            let cy = attr(e, b"cy").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
                            d.size = (cx / EMU_PER_PX, cy / EMU_PER_PX);
                        }
                    }
                    b"pos" => {
                        if let Some(d) = &mut cur {
                            // absoluteAnchor: approximate with default cell sizes (64×20 px).
                            let x = attr(e, b"x").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0) / EMU_PER_PX;
                            let y = attr(e, b"y").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0) / EMU_PER_PX;
                            d.from = Anchor { col: (x / 64.0) as u32, dx: x % 64.0, row: (y / 20.0) as u32, dy: y % 20.0 };
                        }
                    }
                    b"blip" => {
                        if let (Some(d), Some(id)) = (&mut cur, attr(e, b"embed"))
                            && let Some(rel) = drels.iter().find(|r| r.id == id)
                        {
                            d.content = Content::Image(resolve(&dpath, &rel.target));
                        }
                    }
                    b"chart" => {
                        if let (Some(d), Some(id)) = (&mut cur, attr(e, b"id"))
                            && let Some(rel) = drels.iter().find(|r| r.id == id && rel_is(&r.kind, "chart"))
                        {
                            let cpath = resolve(&dpath, &rel.target);
                            d.content = Content::Chart(chart_title(zip, &cpath));
                        }
                    }
                    _ => {}
                }
            }
            Event::Text(t) => {
                if let (Some(f), Some(d)) = (&field, &mut cur) {
                    let v: f64 = String::from_utf8_lossy(&t).trim().parse().unwrap_or(0.0);
                    let a = if which == 2 { d.to.get_or_insert_with(Anchor::default) } else { &mut d.from };
                    match f.as_slice() {
                        b"col" => a.col = v as u32,
                        b"row" => a.row = v as u32,
                        b"colOff" => a.dx = v / EMU_PER_PX,
                        b"rowOff" => a.dy = v / EMU_PER_PX,
                        _ => {}
                    }
                }
            }
            Event::End(e) => {
                let qn = e.name();
                match local(qn.as_ref()) {
                    b"from" | b"to" => which = 0,
                    b"col" | b"colOff" | b"row" | b"rowOff" => field = None,
                    b"twoCellAnchor" | b"oneCellAnchor" | b"absoluteAnchor" => {
                        if let Some(d) = cur.take() {
                            out.push(d);
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

/// Plain text of a chart's title, if it has one.
fn chart_title(zip: &mut ZipArchive, path: &str) -> String {
    let Some(p) = find_entry(zip, path) else { return String::new() };
    let Ok(xml) = read_entry(zip, &p) else { return String::new() };
    let mut r = Reader::from_reader(xml.as_slice());
    let mut in_title = 0;
    let mut in_t = false;
    let mut out = String::new();
    loop {
        match r.read_event() {
            Ok(Event::Start(e)) => match local(e.name().as_ref()) {
                b"title" => in_title += 1,
                b"t" if in_title > 0 => in_t = true,
                _ => {}
            },
            Ok(Event::Text(t)) if in_t => out.push_str(&crate::xml::unescape(&String::from_utf8_lossy(&t))),
            Ok(Event::End(e)) => match local(e.name().as_ref()) {
                b"title" => {
                    in_title -= 1;
                    if !out.is_empty() {
                        break;
                    }
                }
                b"t" => in_t = false,
                _ => {}
            },
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    out
}

pub fn load_tables(zip: &mut ZipArchive, sheet_path: &str) -> Vec<Table> {
    let mut out = Vec::new();
    let Ok(rels_xml) = read_entry(zip, &rels_path(sheet_path)) else { return out };
    for rel in parse_rels(&rels_xml).into_iter().filter(|r| rel_is(&r.kind, "table")) {
        let path = resolve(sheet_path, &rel.target);
        let Some(p) = find_entry(zip, &path) else { continue };
        let Ok(xml) = read_entry(zip, &p) else { continue };
        let mut r = Reader::from_reader(xml.as_slice());
        let mut t: Option<Table> = None;
        loop {
            match r.read_event() {
                Ok(Event::Start(e) | Event::Empty(e)) => match local(e.name().as_ref()) {
                    b"table" => {
                        let sqref = attr(&e, b"ref").unwrap_or_default();
                        if let Some((r0, c0, r1, c1)) = crate::refshift::parse_range_ref(&sqref) {
                            t = Some(Table {
                                rect: Rect { r0, c0, r1, c1 },
                                sqref,
                                header_rows: attr(&e, b"headerRowCount").and_then(|v| v.parse().ok()).unwrap_or(1),
                                totals_rows: attr(&e, b"totalsRowCount").and_then(|v| v.parse().ok()).unwrap_or(0),
                                style: String::new(),
                                row_stripes: true,
                                col_stripes: false,
                                first_col: false,
                                last_col: false,
                            });
                        }
                    }
                    b"tableStyleInfo" => {
                        if let Some(t) = &mut t {
                            t.style = attr(&e, b"name").unwrap_or_default();
                            t.row_stripes = attr_bool(&e, b"showRowStripes").unwrap_or(false);
                            t.col_stripes = attr_bool(&e, b"showColumnStripes").unwrap_or(false);
                            t.first_col = attr_bool(&e, b"showFirstColumn").unwrap_or(false);
                            t.last_col = attr_bool(&e, b"showLastColumn").unwrap_or(false);
                        }
                    }
                    _ => {}
                },
                Ok(Event::Eof) | Err(_) => break,
                _ => {}
            }
        }
        if let Some(t) = t {
            out.push(t);
        }
    }
    out
}
