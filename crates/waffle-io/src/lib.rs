//! File formats for Waffle.
//!
//! - [`xlsx`]: streaming reader and the "save as is" writer (untouched parts are
//!   copied byte-for-byte; structural edits are replayed over charts, tables, names…).
//! - [`csv`]: dialect sniffing, streaming load, byte-exact save of unchanged rows.
//! - [`legacy`]: xls / xlsb / ods import (values) via calamine.
//! - [`doc`]: format detection and background loading.

// The model types this crate reads into and writes from.
pub(crate) use waffle_core::{cell, cf, drawings, sheet, strings, styles, tables, workbook, xml, xmlrw};
pub(crate) use waffle_numfmt as numfmt;
pub(crate) use waffle_refs as refshift;

pub mod csv;
pub mod doc;
pub mod legacy;
pub mod xlsx;
