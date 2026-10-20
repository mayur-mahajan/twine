//! [`Crc32`]: the CRC-32 checksum (IEEE 802.3) used for display-integrity checks and digests.
//!
//! Twine's display-integrity mechanisms (an independent monitor checking what reached the
//! panel, the off-screen re-render of a safety-critical node, the configuration digest of the
//! audit report) need one checksum that every part computes the same way, on every target,
//! without floating point, `unsafe` or allocation. This is the standard CRC-32 (IEEE 802.3,
//! zlib, PNG, Ethernet: reflected polynomial `0xEDB88320`, initial value and final XOR
//! `0xFFFF_FFFF`), so a monitor written in another language, or a tool on a host, gets the
//! same value.
//!
//! **Size vs speed.** The table holds one nibble (16 × 4 bytes = 64 bytes of flash) instead of
//! the usual byte table (1 KiB): two lookups per byte, about 4× faster than the bitwise loop
//! and 16× smaller than the byte table. A checksum is computed only where an integrity check
//! asks for it (never on the render path unless an observer does it), so the small table is
//! the better trade on a microcontroller.
//!
//! ```
//! use twine_core::crc::Crc32;
//!
//! // The standard check value of CRC-32.
//! assert_eq!(Crc32::of(b"123456789").value(), 0xCBF4_3926);
//!
//! // Incremental: feed the data in pieces (e.g. row by row).
//! let mut crc = Crc32::new();
//! crc.update(b"1234");
//! crc.update(b"56789");
//! assert_eq!(crc, Crc32::of(b"123456789"));
//! ```

use core::fmt;

/// Remainders of the reflected polynomial `0xEDB88320` for each nibble value.
const NIBBLE: [u32; 16] = [
    0x0000_0000,
    0x1DB7_1064,
    0x3B6E_20C8,
    0x26D9_30AC,
    0x76DC_4190,
    0x6B6B_51F4,
    0x4DB2_6158,
    0x5005_713C,
    0xEDB8_8320,
    0xF00F_9344,
    0xD6D6_A3E8,
    0xCB61_B38C,
    0x9B64_C2B0,
    0x86D3_D2D4,
    0xA00A_E278,
    0xBDBD_F21C,
];

/// One step of the nibble table: shifts four bits of `crc` out.
#[inline(always)]
fn nibble(crc: u32) -> u32 {
    // The index is masked to 0..16: `get` never misses (the fallback keeps the code panic-free).
    (crc >> 4) ^ NIBBLE.get((crc & 0xF) as usize).copied().unwrap_or(0)
}

/// A CRC-32 (IEEE 802.3) checksum, computed incrementally: [`new`](Self::new), any number of
/// [`update`](Self::update)s, then [`value`](Self::value). Two `Crc32` compare equal when
/// their values are equal, so a checksum is also the value type that integrity checks return
/// and compare (`Engine::render_region_to`, frame observers, the audit digest).
///
/// `Copy`, 4 bytes; never allocates, never panics. `Display` writes the value as 8 hex digits.
///
/// ```
/// use twine_core::crc::Crc32;
///
/// let empty = Crc32::new();
/// assert_eq!(empty.value(), 0);
/// assert_eq!(Crc32::of(b"The quick brown fox jumps over the lazy dog").value(), 0x414F_A339);
/// assert_eq!(Crc32::of(b"123456789").to_string(), "cbf43926");
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Crc32 {
    /// The running register (inverted value).
    state: u32,
}

impl Crc32 {
    /// A checksum of no data (value `0`).
    #[must_use]
    pub const fn new() -> Self {
        Crc32 { state: 0xFFFF_FFFF }
    }

    /// The checksum of `bytes`.
    ///
    /// ```
    /// use twine_core::crc::Crc32;
    /// assert_eq!(Crc32::of(b"").value(), 0);
    /// ```
    #[must_use]
    pub fn of(bytes: &[u8]) -> Self {
        let mut c = Self::new();
        c.update(bytes);
        c
    }

    /// Adds `bytes` to the checksum. O(len), two table lookups per byte.
    pub fn update(&mut self, bytes: &[u8]) {
        let mut crc = self.state;
        for &b in bytes {
            crc ^= u32::from(b);
            crc = nibble(nibble(crc));
        }
        self.state = crc;
    }

    /// Adds the little-endian bytes of `v` (for checksums of values, e.g. a widget's value).
    ///
    /// ```
    /// use twine_core::crc::Crc32;
    /// let mut a = Crc32::new();
    /// a.update_u32(0x0403_0201);
    /// assert_eq!(a, Crc32::of(&[1, 2, 3, 4]));
    /// ```
    pub fn update_u32(&mut self, v: u32) {
        self.update(&v.to_le_bytes());
    }

    /// The checksum of the data added so far.
    #[must_use]
    pub const fn value(&self) -> u32 {
        !self.state
    }
}

impl Default for Crc32 {
    /// [`Crc32::new`].
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for Crc32 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Crc32({:08x})", self.value())
    }
}

impl fmt::Display for Crc32 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:08x}", self.value())
    }
}

/// Feeds formatted text into the checksum (`write!(crc, ..)`), e.g. to digest a report without
/// building the string. Never fails.
impl fmt::Write for Crc32 {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.update(s.as_bytes());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::fmt::Write;

    /// The textbook bitwise CRC-32, as the reference.
    fn bitwise(bytes: &[u8]) -> u32 {
        let mut crc = 0xFFFF_FFFFu32;
        for &b in bytes {
            crc ^= u32::from(b);
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    #[test]
    fn table_matches_the_polynomial() {
        for (i, &t) in NIBBLE.iter().enumerate() {
            let mut c = i as u32;
            for _ in 0..4 {
                c = if c & 1 != 0 {
                    (c >> 1) ^ 0xEDB8_8320
                } else {
                    c >> 1
                };
            }
            assert_eq!(t, c, "entry {i}");
        }
    }

    #[test]
    fn matches_the_bitwise_reference() {
        let data: alloc::vec::Vec<u8> = (0..=255u8).cycle().take(1000).collect();
        for len in [0, 1, 2, 7, 64, 255, 1000] {
            assert_eq!(
                Crc32::of(&data[..len]).value(),
                bitwise(&data[..len]),
                "len {len}"
            );
        }
    }

    #[test]
    fn incremental_equals_one_shot_and_write_feeds_text() {
        let mut c = Crc32::new();
        for chunk in b"hello, world".chunks(5) {
            c.update(chunk);
        }
        assert_eq!(c, Crc32::of(b"hello, world"));
        let mut w = Crc32::new();
        let (n, s) = (12, "ab");
        write!(w, "{n}-{s}").unwrap();
        assert_eq!(w, Crc32::of(b"12-ab"));
    }
}
