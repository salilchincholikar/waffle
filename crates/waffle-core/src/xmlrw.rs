//! Rewrite selected attributes / text nodes of an XML document while copying
//! every other byte verbatim.

use quick_xml::Reader;
use quick_xml::events::Event;

use crate::xml::{escape_attr, escape_text, local};

/// Callbacks decide replacements; return None to keep the original.
pub trait Rules {
    /// (element local name, attribute local name, unescaped value)
    fn attr(&mut self, _elem: &[u8], _attr: &[u8], _value: &str) -> Option<String> {
        None
    }
    /// Text directly inside `elem` (unescaped).
    fn text(&mut self, _elem: &[u8], _text: &str) -> Option<String> {
        None
    }
}

/// Returns None when nothing changed.
pub fn rewrite(src: &[u8], rules: &mut dyn Rules) -> Option<Vec<u8>> {
    let mut r = Reader::from_reader(src);
    r.config_mut().trim_text(false);
    let mut out: Vec<u8> = Vec::new();
    let mut copied = 0usize; // src bytes up to here are already represented in `out`
    let mut stack: Vec<Vec<u8>> = Vec::new();
    let mut changed = false;
    // Text may arrive split around entity references; gather it per element.
    let mut text_start: Option<usize> = None;

    let flush_text =
        |out: &mut Vec<u8>, copied: &mut usize, start: usize, end: usize, elem: &[u8], rules: &mut dyn Rules, changed: &mut bool| {
            let raw = String::from_utf8_lossy(&src[start..end]);
            let text = crate::xml::unescape(&raw);
            if let Some(new) = rules.text(elem, &text)
                && new != text
            {
                out.extend_from_slice(&src[*copied..start]);
                let mut s = String::new();
                escape_text(&new, &mut s);
                out.extend_from_slice(s.as_bytes());
                *copied = end;
                *changed = true;
            }
        };

    loop {
        let before = r.buffer_position() as usize;
        let ev = match r.read_event() {
            Ok(e) => e,
            Err(_) => return None,
        };
        let after = r.buffer_position() as usize;
        match &ev {
            Event::Text(_) | Event::GeneralRef(_) => {
                text_start.get_or_insert(before);
                continue;
            }
            _ => {}
        }
        if let Some(ts) = text_start.take()
            && let Some(top) = stack.last()
        {
            let top = top.clone();
            flush_text(&mut out, &mut copied, ts, before, &top, rules, &mut changed);
        }
        match ev {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let empty = matches!(ev, Event::Empty(_));
                let qn = e.name();
                let name = local(qn.as_ref()).to_vec();
                let mut new_attrs: Vec<(Vec<u8>, String)> = Vec::new();
                let mut any = false;
                for a in e.attributes().with_checks(false).flatten() {
                    let raw = String::from_utf8_lossy(&a.value).into_owned();
                    let val = crate::xml::unescape(&raw);
                    match rules.attr(&name, local(a.key.as_ref()), &val) {
                        Some(v) if v != val => {
                            any = true;
                            let mut s = String::new();
                            escape_attr(&v, &mut s);
                            new_attrs.push((a.key.as_ref().to_vec(), s));
                        }
                        _ => new_attrs.push((a.key.as_ref().to_vec(), raw.replace('"', "&quot;"))),
                    }
                }
                if any {
                    out.extend_from_slice(&src[copied..before]);
                    out.push(b'<');
                    out.extend_from_slice(qn.as_ref());
                    for (k, v) in new_attrs {
                        out.push(b' ');
                        out.extend_from_slice(&k);
                        out.extend_from_slice(b"=\"");
                        out.extend_from_slice(v.as_bytes());
                        out.push(b'"');
                    }
                    out.extend_from_slice(if empty { b"/>" } else { b">" });
                    copied = after;
                    changed = true;
                }
                if !empty {
                    stack.push(name);
                }
            }
            Event::End(_) => {
                stack.pop();
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if !changed {
        return None;
    }
    out.extend_from_slice(&src[copied..]);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct R;
    impl Rules for R {
        fn attr(&mut self, e: &[u8], a: &[u8], v: &str) -> Option<String> {
            (e == b"mergeCell" && a == b"ref").then(|| v.replace('1', "2"))
        }
        fn text(&mut self, e: &[u8], t: &str) -> Option<String> {
            (e == b"formula").then(|| format!("{t}+1"))
        }
    }

    #[test]
    fn rewrites_only_targets() {
        let src = br#"<a x='1'><mergeCell ref="A1:B1" other='q&amp;'/><formula>A1&lt;2</formula><keep>1</keep></a>"#;
        let out = String::from_utf8(rewrite(src, &mut R).unwrap()).unwrap();
        assert_eq!(out, r#"<a x='1'><mergeCell ref="A2:B2" other="q&amp;"/><formula>A1&lt;2+1</formula><keep>1</keep></a>"#);
        assert!(rewrite(b"<a><keep/></a>", &mut R).is_none());
    }
}
