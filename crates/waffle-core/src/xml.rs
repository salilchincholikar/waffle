//! Small helpers around quick-xml.

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

/// Local name of a (possibly prefixed) tag, e.g. `x:row` → `row`.
#[inline]
pub fn local(name: &[u8]) -> &[u8] {
    match memchr::memrchr(b':', name) {
        Some(i) => &name[i + 1..],
        None => name,
    }
}

/// Raw (still escaped) attribute value.
pub fn attr_raw<'a>(e: &'a BytesStart<'a>, key: &[u8]) -> Option<std::borrow::Cow<'a, [u8]>> {
    e.attributes().flatten().find(|a| local(a.key.as_ref()) == key).map(|a| a.value)
}

/// Unescaped attribute value as a String.
pub fn attr(e: &BytesStart<'_>, key: &[u8]) -> Option<String> {
    e.attributes().flatten().find(|a| local(a.key.as_ref()) == key).map(|a| {
        let raw = String::from_utf8_lossy(&a.value).into_owned();
        unescape(&raw)
    })
}

pub fn attr_u32(e: &BytesStart<'_>, key: &[u8]) -> Option<u32> {
    parse_u32(&attr_raw(e, key)?)
}

pub fn attr_f32(e: &BytesStart<'_>, key: &[u8]) -> Option<f32> {
    std::str::from_utf8(&attr_raw(e, key)?).ok()?.trim().parse().ok()
}

pub fn attr_bool(e: &BytesStart<'_>, key: &[u8]) -> Option<bool> {
    let v = attr_raw(e, key)?;
    Some(matches!(&*v, b"1" | b"true" | b"on"))
}

#[inline]
pub fn parse_u32(b: &[u8]) -> Option<u32> {
    if b.is_empty() || b.len() > 10 {
        return None;
    }
    let mut v: u64 = 0;
    for &c in b {
        if !c.is_ascii_digit() {
            return None;
        }
        v = v * 10 + (c - b'0') as u64;
    }
    u32::try_from(v).ok()
}

/// Every attribute except the listed ones, re-serialised as ` k="v"` (raw).
pub fn other_attrs(e: &BytesStart<'_>, skip: &[&[u8]]) -> String {
    let mut out = String::new();
    for a in e.attributes().flatten() {
        let k = a.key.as_ref();
        if skip.contains(&local(k)) {
            continue;
        }
        out.push(' ');
        out.push_str(&String::from_utf8_lossy(k));
        out.push_str("=\"");
        // Attribute values stay escaped; re-quote safely with double quotes.
        let v = String::from_utf8_lossy(&a.value);
        out.push_str(&v.replace('"', "&quot;"));
        out.push('"');
    }
    out
}

/// XML-unescape including numeric references. Excel's `_xHHHH_` escapes are left alone.
pub fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    match quick_xml::escape::unescape(s) {
        Ok(c) => c.into_owned(),
        Err(_) => s.to_string(),
    }
}

/// Decode Excel's `_xHHHH_` escapes used in shared strings for control characters.
pub fn decode_ooxml_escapes(s: &str) -> std::borrow::Cow<'_, str> {
    if !s.contains("_x") {
        return s.into();
    }
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'_'
            && b.get(i + 1) == Some(&b'x')
            && b.get(i + 6) == Some(&b'_')
            && b[i + 2..i + 6].iter().all(u8::is_ascii_hexdigit)
            && let Ok(code) = u32::from_str_radix(&s[i + 2..i + 6], 16)
            && let Some(ch) = char::from_u32(code)
        {
            out.push(ch);
            i += 7;
            continue;
        }
        let ch = s[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out.into()
}

/// Escape text for element content, also encoding control chars Excel-style.
pub fn escape_text(s: &str, out: &mut String) {
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\t' | '\n' | '\r' => out.push(ch),
            c if (c as u32) < 0x20 => out.push_str(&format!("_x{:04X}_", c as u32)),
            c => out.push(c),
        }
    }
}

pub fn escape_attr(s: &str, out: &mut String) {
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            '\t' => out.push_str("&#9;"),
            c => out.push(c),
        }
    }
}

/// Byte span of each top-level child element of the first element named `parent`.
/// Returns (children spans, parent inner range, parent open-tag span).
pub struct Section {
    pub open: std::ops::Range<usize>,
    pub close: std::ops::Range<usize>,
    pub children: Vec<std::ops::Range<usize>>,
}

pub fn find_section(xml: &[u8], parent: &[u8], child: &[u8]) -> Option<Section> {
    let mut r = Reader::from_reader(xml);
    r.config_mut().trim_text(false);
    let mut depth = 0usize;
    let mut open = None;
    let mut children = Vec::new();
    let mut child_start = None;
    loop {
        let before = r.buffer_position() as usize;
        let ev = r.read_event().ok()?;
        let after = r.buffer_position() as usize;
        match ev {
            Event::Start(e) => {
                if open.is_none() {
                    if local(e.name().as_ref()) == parent {
                        open = Some(before..after);
                        depth = 0;
                    }
                } else {
                    if depth == 0 && local(e.name().as_ref()) == child {
                        child_start = Some(before);
                    }
                    depth += 1;
                }
            }
            Event::Empty(e) => {
                if open.is_none() {
                    if local(e.name().as_ref()) == parent {
                        return Some(Section { open: before..after, close: after..after, children });
                    }
                } else if depth == 0 && local(e.name().as_ref()) == child {
                    children.push(before..after);
                }
            }
            Event::End(e) => {
                if let Some(o) = &open {
                    if depth == 0 {
                        debug_assert_eq!(local(e.name().as_ref()), parent);
                        return Some(Section { open: o.clone(), close: before..after, children });
                    }
                    depth -= 1;
                    if depth == 0
                        && let Some(s) = child_start.take()
                    {
                        children.push(s..after);
                    }
                }
            }
            Event::Eof => return None,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections() {
        let xml = br#"<a><x:fonts count="2"><x:font><b/></x:font><x:font/></x:fonts><fills/></a>"#;
        let s = find_section(xml, b"fonts", b"font").unwrap();
        assert_eq!(s.children.len(), 2);
        assert_eq!(&xml[s.children[0].clone()], b"<x:font><b/></x:font>");
        assert_eq!(&xml[s.close.clone()], b"</x:fonts>");
        let f = find_section(xml, b"fills", b"fill").unwrap();
        assert!(f.children.is_empty());
    }

    #[test]
    fn escapes() {
        assert_eq!(decode_ooxml_escapes("a_x000D_b"), "a\rb");
        let mut o = String::new();
        escape_text("a<b&\u{1}", &mut o);
        assert_eq!(o, "a&lt;b&amp;_x0001_");
    }
}
