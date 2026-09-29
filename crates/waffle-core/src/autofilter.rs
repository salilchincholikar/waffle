//! A worksheet's saved AutoFilter (`<autoFilter>`): Excel stores the criteria there and the
//! rows they exclude as hidden rows. Waffle shows it as an active filter and can clear it.

use quick_xml::Reader;
use quick_xml::events::Event;

use crate::cf::parse_sqref;
use crate::sheet::Rect;
use crate::xml::{attr, attr_u32, local};

#[derive(Clone, Debug, PartialEq)]
pub struct AutoFilter {
    /// The filtered range; its first row is the header.
    pub range: Rect,
    /// Columns (absolute) that have criteria. Empty: the filter buttons are on, nothing filtered.
    pub cols: Vec<u32>,
}

/// The `<autoFilter>` in a worksheet part's XML (after `<sheetData>`), if any.
pub fn parse(xml: &[u8]) -> Option<AutoFilter> {
    let mut r = Reader::from_reader(xml);
    r.config_mut().check_end_names = false;
    r.config_mut().allow_unmatched_ends = true; // a sheet suffix starts with </sheetData>
    let mut found: Option<AutoFilter> = None;
    let mut depth = 0u32; // > 0 while inside <autoFilter>
    loop {
        match r.read_event() {
            Ok(Event::Start(e)) => match local(e.name().as_ref()) {
                b"autoFilter" if depth == 0 => {
                    let range = parse_sqref(&attr(&e, b"ref")?).into_iter().next()?;
                    found = Some(AutoFilter { range, cols: Vec::new() });
                    depth = 1;
                }
                b"filterColumn" if depth > 0 => {
                    if let (Some(f), Some(id)) = (found.as_mut(), attr_u32(&e, b"colId")) {
                        f.cols.push(f.range.c0 + id);
                    }
                    depth += 1;
                }
                _ if depth > 0 => depth += 1,
                _ => {}
            },
            Ok(Event::Empty(e)) => match local(e.name().as_ref()) {
                // <autoFilter ref="…"/>: buttons shown, no criteria.
                b"autoFilter" if depth == 0 => {
                    let range = parse_sqref(&attr(&e, b"ref")?).into_iter().next()?;
                    return Some(AutoFilter { range, cols: Vec::new() });
                }
                // An empty <filterColumn/> carries no criteria.
                _ => {}
            },
            Ok(Event::End(_)) if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    return found;
                }
            }
            Ok(Event::Eof) | Err(_) => return found,
            _ => {}
        }
    }
}

/// `xml` without the `<filterColumn>` criteria inside its `<autoFilter>` (the filter
/// buttons stay; nothing is filtered any more).
pub fn strip_criteria(xml: &[u8]) -> Vec<u8> {
    let mut r = Reader::from_reader(xml);
    r.config_mut().check_end_names = false;
    r.config_mut().allow_unmatched_ends = true; // a sheet suffix starts with </sheetData>
    let mut cut: Vec<(usize, usize)> = Vec::new();
    let mut in_filter = false;
    loop {
        let before = r.buffer_position() as usize;
        match r.read_event() {
            Ok(Event::Start(e)) => match local(e.name().as_ref()) {
                b"autoFilter" => in_filter = true,
                b"filterColumn" if in_filter => {
                    let name = e.name().as_ref().to_vec();
                    if r.read_to_end(quick_xml::name::QName(&name)).is_err() {
                        break;
                    }
                    cut.push((before, r.buffer_position() as usize));
                }
                _ => {}
            },
            Ok(Event::Empty(e)) if in_filter && local(e.name().as_ref()) == b"filterColumn" => {
                cut.push((before, r.buffer_position() as usize));
            }
            Ok(Event::End(e)) if local(e.name().as_ref()) == b"autoFilter" => break,
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    let mut out = Vec::with_capacity(xml.len());
    let mut at = 0;
    for (a, b) in cut {
        out.extend_from_slice(&xml[at..a]);
        at = b;
    }
    out.extend_from_slice(&xml[at..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const XML: &[u8] = br#"</sheetData><autoFilter ref="A1:D20"><filterColumn colId="2"><filters><filter val="North"/></filters></filterColumn></autoFilter><pageMargins left="0.7"/>"#;

    #[test]
    fn parses_range_and_filtered_columns() {
        let f = parse(XML).unwrap();
        assert_eq!(f.range, Rect { r0: 0, c0: 0, r1: 19, c1: 3 });
        assert_eq!(f.cols, vec![2]);
        assert_eq!(parse(br#"<autoFilter ref="B2:C9"/>"#).unwrap().cols, Vec::<u32>::new());
        assert!(parse(b"<pageMargins/>").is_none());
    }

    #[test]
    fn strips_only_the_criteria() {
        let out = String::from_utf8(strip_criteria(XML)).unwrap();
        assert_eq!(out, r#"</sheetData><autoFilter ref="A1:D20"></autoFilter><pageMargins left="0.7"/>"#);
        assert_eq!(parse(out.as_bytes()).unwrap().cols, Vec::<u32>::new());
    }
}
