//! Fixed scalar lookup and glyph cache for the graphical Display service.

use logos_abi::MAX_GLYPH_CACHE;

pub const GLYPH_WIDTH: usize = 8;
pub const GLYPH_HEIGHT: usize = 16;
pub const GLYPH_BYTES: usize = GLYPH_HEIGHT;
pub const REPLACEMENT_SCALAR: u32 = 0xfffd;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Glyph {
    pub rows: [u8; GLYPH_BYTES],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GlyphId(u16);

impl GlyphId {
    pub const fn raw(self) -> usize {
        self.0 as usize
    }
}

#[derive(Clone, Copy)]
struct GlyphSlot {
    valid: bool,
    scalar: u32,
    glyph: Glyph,
}

impl GlyphSlot {
    const EMPTY: Self = Self { valid: false, scalar: 0, glyph: Glyph { rows: [0; GLYPH_BYTES] } };
}

pub struct GlyphCache {
    slots: [GlyphSlot; MAX_GLYPH_CACHE],
    next: usize,
}

impl GlyphCache {
    pub const fn new() -> Self {
        Self { slots: [GlyphSlot::EMPTY; MAX_GLYPH_CACHE], next: 0 }
    }

    pub fn lookup(&mut self, scalar: u32) -> GlyphId {
        let scalar = normalize_scalar(scalar);
        if let Some(index) = self.slots.iter().position(|slot| slot.valid && slot.scalar == scalar)
        {
            return GlyphId(index as u16);
        }
        let index = self.next;
        self.next = (self.next + 1) % MAX_GLYPH_CACHE;
        self.slots[index] = GlyphSlot { valid: true, scalar, glyph: embedded_glyph(scalar) };
        GlyphId(index as u16)
    }

    pub fn glyph(&self, id: GlyphId) -> Option<Glyph> {
        self.slots.get(id.raw()).filter(|slot| slot.valid).map(|slot| slot.glyph)
    }

    pub fn len(&self) -> usize {
        self.slots.iter().fold(0, |count, slot| count + slot.valid as usize)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for GlyphCache {
    fn default() -> Self {
        Self::new()
    }
}

fn normalize_scalar(scalar: u32) -> u32 {
    if scalar <= 0x10ffff && !(0xd800..=0xdfff).contains(&scalar) {
        scalar
    } else {
        REPLACEMENT_SCALAR
    }
}

fn embedded_glyph(scalar: u32) -> Glyph {
    let byte = if scalar >= 'A' as u32 && scalar <= 'Z' as u32 {
        (scalar as u8).to_ascii_lowercase()
    } else {
        scalar as u8
    };
    let pattern = match byte {
        b'a' => [14, 17, 31, 17, 17, 17, 0],
        b'b' => [30, 17, 17, 30, 17, 17, 30],
        b'c' => [14, 17, 16, 16, 16, 17, 14],
        b'd' => [30, 17, 17, 17, 17, 17, 30],
        b'e' => [31, 16, 16, 30, 16, 16, 31],
        b'f' => [31, 16, 16, 30, 16, 16, 16],
        b'g' => [14, 17, 16, 23, 17, 17, 14],
        b'h' => [17, 17, 17, 31, 17, 17, 17],
        b'i' => [31, 4, 4, 4, 4, 4, 31],
        b'j' => [1, 1, 1, 1, 17, 17, 14],
        b'k' => [17, 18, 20, 24, 20, 18, 17],
        b'l' => [16, 16, 16, 16, 16, 16, 31],
        b'm' => [17, 27, 21, 21, 17, 17, 17],
        b'n' => [17, 25, 21, 19, 17, 17, 17],
        b'o' => [14, 17, 17, 17, 17, 17, 14],
        b'p' => [30, 17, 17, 30, 16, 16, 16],
        b'q' => [14, 17, 17, 17, 21, 18, 13],
        b'r' => [30, 17, 17, 30, 20, 18, 17],
        b's' => [15, 16, 16, 14, 1, 1, 30],
        b't' => [31, 4, 4, 4, 4, 4, 4],
        b'u' => [17, 17, 17, 17, 17, 17, 14],
        b'v' => [17, 17, 17, 17, 17, 10, 4],
        b'w' => [17, 17, 17, 21, 21, 21, 10],
        b'x' => [17, 17, 10, 4, 10, 17, 17],
        b'y' => [17, 17, 10, 4, 4, 4, 4],
        b'z' => [31, 1, 2, 4, 8, 16, 31],
        b'0' => [14, 17, 19, 21, 25, 17, 14],
        b'1' => [4, 12, 4, 4, 4, 4, 14],
        b'2' => [14, 17, 1, 2, 4, 8, 31],
        b'3' => [30, 1, 1, 14, 1, 1, 30],
        b'4' => [2, 6, 10, 18, 31, 2, 2],
        b'5' => [31, 16, 16, 30, 1, 1, 30],
        b'6' => [14, 16, 16, 30, 17, 17, 14],
        b'7' => [31, 1, 2, 4, 8, 8, 8],
        b'8' => [14, 17, 17, 14, 17, 17, 14],
        b'9' => [14, 17, 17, 15, 1, 1, 14],
        b' ' => [0; 7],
        b'>' => [16, 8, 4, 2, 4, 8, 16],
        b'<' => [1, 2, 4, 8, 4, 2, 1],
        b'-' => [0, 0, 0, 31, 0, 0, 0],
        b'_' => [0, 0, 0, 0, 0, 0, 31],
        _ => [31, 17, 21, 17, 21, 17, 31],
    };
    let mut rows = [0; GLYPH_BYTES];
    let mut row = 0;
    while row < pattern.len() {
        rows[row * 2] = pattern[row] as u8;
        rows[row * 2 + 1] = pattern[row] as u8;
        row += 1;
    }
    Glyph { rows }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_lookup_is_cached_and_nonempty() {
        let mut cache = GlyphCache::new();
        let first = cache.lookup('l' as u32);
        let again = cache.lookup('l' as u32);
        assert_eq!(first, again);
        assert!(cache.glyph(first).unwrap().rows.iter().any(|row| *row != 0));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn invalid_scalars_use_deterministic_fallback() {
        let mut cache = GlyphCache::new();
        let surrogate = cache.lookup(0xd800);
        let replacement = cache.lookup(REPLACEMENT_SCALAR);
        assert_eq!(surrogate, replacement);
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn cache_capacity_is_fixed() {
        let mut cache = GlyphCache::new();
        for scalar in 0..MAX_GLYPH_CACHE as u32 {
            cache.lookup(0x100 + scalar);
        }
        assert_eq!(cache.len(), MAX_GLYPH_CACHE);
        let first = cache.lookup(0x2000);
        assert!(cache.glyph(first).is_some());
        assert_eq!(cache.len(), MAX_GLYPH_CACHE);
    }
}
