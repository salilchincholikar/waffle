//! An 8-byte cell value.
//!
//! Numbers are stored as plain `f64` bits. Everything else lives in the
//! negative quiet-NaN space: the top 16 bits are a tag and the low 32 bits a
//! payload. Real NaNs never reach a sheet (Excel has none), so this is lossless.

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct Cell(u64);

const TAG_SHIFT: u32 = 48;
const TAG_EMPTY: u64 = 0xFFF9;
const TAG_STR: u64 = 0xFFFA;
const TAG_BOOL: u64 = 0xFFFB;
const TAG_ERR: u64 = 0xFFFC;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Empty,
    Number,
    Str,
    Bool,
    Error,
}

/// Excel error values, in the order used by the `Error` payload.
pub const ERRORS: [&str; 8] = ["#NULL!", "#DIV/0!", "#VALUE!", "#REF!", "#NAME?", "#NUM!", "#N/A", "#GETTING_DATA"];

impl Cell {
    pub const EMPTY: Cell = Cell(TAG_EMPTY << TAG_SHIFT);

    #[inline]
    pub fn number(v: f64) -> Cell {
        if v.is_nan() {
            return Cell::error(2);
        }
        // Normalise -0.0 so equality and hashing behave.
        Cell(if v == 0.0 { 0 } else { v.to_bits() })
    }
    #[inline]
    pub fn string(id: u32) -> Cell {
        Cell((TAG_STR << TAG_SHIFT) | id as u64)
    }
    #[inline]
    pub fn boolean(b: bool) -> Cell {
        Cell((TAG_BOOL << TAG_SHIFT) | b as u64)
    }
    #[inline]
    pub fn error(code: u8) -> Cell {
        Cell((TAG_ERR << TAG_SHIFT) | code as u64)
    }
    pub fn error_from_str(s: &str) -> Cell {
        let code = ERRORS.iter().position(|e| e.eq_ignore_ascii_case(s)).unwrap_or(2);
        Cell::error(code as u8)
    }

    #[inline]
    fn tag(self) -> u64 {
        self.0 >> TAG_SHIFT
    }
    #[inline]
    pub fn kind(self) -> Kind {
        match self.tag() {
            TAG_EMPTY => Kind::Empty,
            TAG_STR => Kind::Str,
            TAG_BOOL => Kind::Bool,
            TAG_ERR => Kind::Error,
            _ => Kind::Number,
        }
    }
    #[inline]
    pub fn is_empty(self) -> bool {
        self.0 == Cell::EMPTY.0
    }
    #[inline]
    pub fn as_number(self) -> Option<f64> {
        (self.kind() == Kind::Number).then(|| f64::from_bits(self.0))
    }
    #[inline]
    pub fn as_str_id(self) -> Option<u32> {
        (self.tag() == TAG_STR).then_some(self.0 as u32)
    }
    #[inline]
    pub fn as_bool(self) -> Option<bool> {
        (self.tag() == TAG_BOOL).then_some(self.0 & 1 == 1)
    }
    #[inline]
    pub fn as_error(self) -> Option<&'static str> {
        (self.tag() == TAG_ERR).then(|| ERRORS[(self.0 as usize & 0xFF).min(ERRORS.len() - 1)])
    }
    #[inline]
    pub fn bits(self) -> u64 {
        self.0
    }
}

impl Default for Cell {
    fn default() -> Self {
        Cell::EMPTY
    }
}

impl std::fmt::Debug for Cell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.kind() {
            Kind::Empty => write!(f, "Empty"),
            Kind::Number => write!(f, "Num({})", f64::from_bits(self.0)),
            Kind::Str => write!(f, "Str#{}", self.0 as u32),
            Kind::Bool => write!(f, "Bool({})", self.0 & 1 == 1),
            Kind::Error => write!(f, "Err({})", self.as_error().unwrap()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips() {
        assert_eq!(std::mem::size_of::<Cell>(), 8);
        for v in [0.0, -0.0, 1.5, -2.25, f64::MAX, f64::MIN_POSITIVE, f64::INFINITY, -1e300] {
            let c = Cell::number(v);
            assert_eq!(c.kind(), Kind::Number);
            assert_eq!(c.as_number().unwrap(), if v == 0.0 { 0.0 } else { v });
        }
        assert_eq!(Cell::string(u32::MAX).as_str_id(), Some(u32::MAX));
        assert_eq!(Cell::boolean(true).as_bool(), Some(true));
        assert_eq!(Cell::boolean(false).as_bool(), Some(false));
        assert_eq!(Cell::error_from_str("#N/A").as_error(), Some("#N/A"));
        assert!(Cell::EMPTY.is_empty());
        assert_eq!(Cell::number(f64::NAN).kind(), Kind::Error);
        assert_eq!(Cell::default().kind(), Kind::Empty);
    }
}
