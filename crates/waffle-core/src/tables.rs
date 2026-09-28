//! Excel tables (ListObjects): range + built-in style, for display.

use crate::refshift::StructOp;
use crate::sheet::Rect;
use crate::styles::{Styles, apply_tint};

#[derive(Clone, Debug)]
pub struct Table {
    pub rect: Rect,
    pub sqref: String,
    pub header_rows: u32,
    pub totals_rows: u32,
    pub style: String,
    pub row_stripes: bool,
    pub col_stripes: bool,
    pub first_col: bool,
    pub last_col: bool,
}

pub fn shift(ts: &[Table], op: StructOp) -> Vec<Table> {
    ts.iter()
        .filter_map(|t| {
            let sq = crate::refshift::shift_sqref(&t.sqref, op).unwrap_or_else(|| t.sqref.clone());
            let (r0, c0, r1, c1) = crate::refshift::parse_range_ref(&sq)?;
            Some(Table { rect: Rect { r0, c0, r1, c1 }, sqref: sq, ..t.clone() })
        })
        .collect()
}

/// Fill / font colour / bold for a cell inside a table, approximating Excel's built-in styles.
pub fn look(t: &Table, styles: &Styles, r: u32, c: u32) -> Option<(Option<u32>, Option<u32>, bool)> {
    if !t.rect.contains(r, c) {
        return None;
    }
    let name = t.style.strip_prefix("TableStyle")?;
    let (family, n) = name.split_at(name.find(|ch: char| ch.is_ascii_digit())?);
    let n: usize = n.parse().ok()?;
    // Built-in styles cycle through: dark (0) then accents 1–6.
    let slot = (n - 1) % 7;
    let base = if slot == 0 { 0x000000 } else { styles.theme_color(3 + slot) };
    let tint = |t: f64| if slot == 0 { apply_tint(0x808080, t + 0.2) } else { apply_tint(base, t) };
    let header = r < t.rect.r0 + t.header_rows;
    let totals = t.totals_rows > 0 && r > t.rect.r1 - t.totals_rows;
    let body_index = r.saturating_sub(t.rect.r0 + t.header_rows);
    let striped = t.row_stripes && body_index.is_multiple_of(2) && !header && !totals;
    let col_striped = t.col_stripes && (c - t.rect.c0).is_multiple_of(2) && !header && !totals;
    let emph_col = (t.first_col && c == t.rect.c0) || (t.last_col && c == t.rect.c1);
    let (fill, font, bold) = match family {
        "Light" => {
            if header || totals {
                if (8..=14).contains(&n) { (Some(base), Some(0xFFFFFF), true) } else { (None, None, true) }
            } else {
                ((striped || col_striped).then(|| tint(0.8)), None, emph_col)
            }
        }
        "Medium" => {
            if header {
                if (15..=21).contains(&n) { (Some(0x000000), Some(0xFFFFFF), true) } else { (Some(base), Some(0xFFFFFF), true) }
            } else if totals {
                (Some(tint(0.8)), None, true)
            } else if (8..=14).contains(&n) || n >= 22 {
                (Some(if striped || col_striped { tint(0.6) } else { tint(0.8) }), None, emph_col)
            } else {
                ((striped || col_striped).then(|| tint(0.8)), None, emph_col)
            }
        }
        "Dark" => {
            if header {
                (Some(0x000000), Some(0xFFFFFF), true)
            } else {
                let dark = if slot == 0 { 0x404040 } else { apply_tint(base, -0.25) };
                (Some(if striped { apply_tint(dark, -0.15) } else { dark }), Some(0xFFFFFF), emph_col || totals)
            }
        }
        _ => return None,
    };
    Some((fill, font, bold))
}
