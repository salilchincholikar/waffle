//! Append-only string storage: one byte arena plus an end-offset per string,
//! so each string costs its bytes + 4 bytes. Interning is optional and adds
//! a 4-byte open-addressing slot per string.

use std::sync::Arc;

#[derive(Default, Clone)]
pub struct StringPool {
    bytes: Vec<u8>,
    ends: Vec<u32>,
    index: Vec<u32>, // open addressing; 0 = empty slot, otherwise id + 1
}

#[inline]
fn hash(s: &[u8]) -> u64 {
    // FNV-1a over 8-byte words; strings are short and this is plenty.
    let mut h: u64 = 0xcbf29ce484222325;
    let (words, rest) = s.as_chunks::<8>();
    for w in words {
        h = (h ^ u64::from_le_bytes(*w)).wrapping_mul(0x100000001b3);
        h ^= h >> 29;
    }
    for &b in rest {
        h = (h ^ b as u64).wrapping_mul(0x100000001b3);
    }
    h ^ (h >> 32)
}

impl StringPool {
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.ends.len()
    }
    pub fn is_empty(&self) -> bool {
        self.ends.is_empty()
    }

    #[inline]
    pub fn get(&self, id: u32) -> &str {
        let id = id as usize;
        let start = if id == 0 { 0 } else { self.ends[id - 1] as usize };
        let end = self.ends[id] as usize;
        // SAFETY: only valid UTF-8 is ever pushed.
        unsafe { std::str::from_utf8_unchecked(&self.bytes[start..end]) }
    }

    /// Append without deduplication.
    pub fn push(&mut self, s: &str) -> u32 {
        let id = self.ends.len() as u32;
        self.bytes.extend_from_slice(s.as_bytes());
        self.ends.push(self.bytes.len() as u32);
        if !self.index.is_empty() {
            self.index_insert(id);
        }
        id
    }

    /// Return the id of an equal string, adding it if missing.
    pub fn intern(&mut self, s: &str) -> u32 {
        if self.index.is_empty() {
            self.rebuild_index(64.max(self.len() * 2));
        }
        let mask = self.index.len() - 1;
        let mut slot = hash(s.as_bytes()) as usize & mask;
        loop {
            match self.index[slot] {
                0 => break,
                v if self.get(v - 1) == s => return v - 1,
                _ => slot = (slot + 1) & mask,
            }
        }
        self.push(s)
    }

    fn index_insert(&mut self, id: u32) {
        if (self.len()) * 2 > self.index.len() {
            self.rebuild_index(self.index.len() * 2);
            return;
        }
        let mask = self.index.len() - 1;
        let mut slot = hash(self.get(id).as_bytes()) as usize & mask;
        while self.index[slot] != 0 {
            slot = (slot + 1) & mask;
        }
        self.index[slot] = id + 1;
    }

    fn rebuild_index(&mut self, cap: usize) {
        let cap = cap.next_power_of_two();
        self.index = vec![0; cap];
        let mask = cap - 1;
        for id in 0..self.len() as u32 {
            let mut slot = hash(self.get(id).as_bytes()) as usize & mask;
            while self.index[slot] != 0 {
                slot = (slot + 1) & mask;
            }
            self.index[slot] = id + 1;
        }
    }

    /// Drop the dedup index (e.g. once a bulk load finishes).
    pub fn drop_index(&mut self) {
        self.index = Vec::new();
    }

    pub fn shrink(&mut self) {
        self.bytes.shrink_to_fit();
        self.ends.shrink_to_fit();
    }

    pub fn heap_bytes(&self) -> usize {
        self.bytes.capacity() + self.ends.capacity() * 4 + self.index.capacity() * 4
    }
}

/// A sheet's view of strings: the workbook's shared-string table followed by
/// strings created by this sheet (inline strings, formula results, edits).
#[derive(Clone)]
pub struct SheetStrings {
    pub shared: Arc<StringPool>,
    pub local: StringPool,
}

impl SheetStrings {
    pub fn new(shared: Arc<StringPool>) -> Self {
        Self { shared, local: StringPool::new() }
    }
    #[inline]
    pub fn get(&self, id: u32) -> &str {
        let n = self.shared.len() as u32;
        if id < n { self.shared.get(id) } else { self.local.get(id - n) }
    }
    #[inline]
    pub fn is_shared(&self, id: u32) -> bool {
        (id as usize) < self.shared.len()
    }
    pub fn add(&mut self, s: &str) -> u32 {
        self.shared.len() as u32 + self.local.intern(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_basics() {
        let mut p = StringPool::new();
        let a = p.push("hello");
        let b = p.push("");
        let c = p.intern("hello");
        assert_eq!(c, a);
        let d = p.intern("wörld");
        assert_eq!(p.get(a), "hello");
        assert_eq!(p.get(b), "");
        assert_eq!(p.get(d), "wörld");
        for i in 0..10_000 {
            let s = format!("s{}", i % 500);
            let id = p.intern(&s);
            assert_eq!(p.get(id), s);
        }
        assert_eq!(p.len(), 3 + 500);
    }

    #[test]
    fn sheet_strings() {
        let mut shared = StringPool::new();
        shared.push("a");
        let mut s = SheetStrings::new(Arc::new(shared));
        let id = s.add("b");
        assert_eq!(id, 1);
        assert_eq!(s.get(0), "a");
        assert_eq!(s.get(1), "b");
        assert_eq!(s.add("b"), 1);
    }
}
