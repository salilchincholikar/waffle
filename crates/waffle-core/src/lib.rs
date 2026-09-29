//! The Waffle spreadsheet model.
//!
//! - [`sheet`]: compact block storage (8-byte cells, 256×16 blocks) with row/column
//!   maps, so structural edits and sorting never move cell data.
//! - [`workbook`]: sheets, styles, the structural change log and undo history.
//! - [`ops`]: every editing operation (each is one undo step).
//! - [`view`]: what the grid shows (display text, alignment, conditional looks).
//! - [`recalc`]: dependency-ordered recalculation using `waffle-calc`.
//!
//! File formats live in `waffle-io`; the C API for the macOS app in `waffle-ffi`.

pub use waffle_calc as calc;
pub use waffle_numfmt as numfmt;
pub use waffle_refs as refshift;

pub mod autofilter;
pub mod cell;
pub mod cf;
pub mod delimited;
pub mod drawings;
pub mod ops;
pub mod recalc;
pub mod sheet;
pub mod source;
pub mod strings;
pub mod styles;
pub mod tables;
pub mod view;
pub mod workbook;
pub mod xml;
pub mod xmlrw;
