//! Pictures and charts anchored on a sheet (read from the drawing part).
//! Display only — the original drawing XML is saved (with anchors shifted by the log).

use crate::refshift::{Axis, StructOp};

pub const EMU_PER_PX: f64 = 9525.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Anchor {
    pub row: u32,
    pub col: u32,
    /// Offset inside the cell, in pixels.
    pub dx: f64,
    pub dy: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Content {
    /// Zip path of the image.
    Image(String),
    /// Chart title (if any).
    Chart(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Drawing {
    pub content: Content,
    pub from: Anchor,
    /// Two-cell anchors end at a cell; one-cell anchors have a pixel size.
    pub to: Option<Anchor>,
    pub size: (f64, f64),
}

fn shift_anchor(a: &mut Anchor, op: StructOp) {
    let v = match op.axis {
        Axis::Rows => &mut a.row,
        Axis::Cols => &mut a.col,
    };
    if op.insert {
        if *v >= op.at {
            *v += op.count;
        }
    } else if *v >= op.at + op.count {
        *v -= op.count;
    } else if *v >= op.at {
        *v = op.at;
    }
}

pub fn shift(ds: &[Drawing], op: StructOp) -> Vec<Drawing> {
    ds.iter()
        .cloned()
        .map(|mut d| {
            shift_anchor(&mut d.from, op);
            if let Some(t) = &mut d.to {
                shift_anchor(t, op);
            }
            d
        })
        .collect()
}
