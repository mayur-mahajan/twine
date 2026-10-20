//! XXH64 — the 64-bit xxHash of the golden-image manifests ([`xxh64`]).
//!
//! Chosen over the CRC-32 of `twine_core::crc` because a manifest entry is evidence that an
//! image did not change: with 64 bits the chance that a changed image keeps its hash is 2⁻⁶⁴
//! (CRC-32: 2⁻³², i.e. one in four billion over thousands of images and many runs), XXH64 is
//! specified and widely available (`xxhsum -H64` recomputes any entry from the raw RGB bytes),
//! and it needs no dependency. It is not a cryptographic hash: the threat it covers is an
//! accidental change, not a forged image.

const P1: u64 = 0x9E37_79B1_85EB_CA87;
const P2: u64 = 0xC2B2_AE3D_27D4_EB4F;
const P3: u64 = 0x1656_67B1_9E37_79F9;
const P4: u64 = 0x85EB_CA77_C2B2_AE63;
const P5: u64 = 0x27D4_EB2F_1656_67C5;

fn read_u64(b: &[u8]) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[..8]);
    u64::from_le_bytes(a)
}

fn read_u32(b: &[u8]) -> u64 {
    let mut a = [0u8; 4];
    a.copy_from_slice(&b[..4]);
    u64::from(u32::from_le_bytes(a))
}

fn round(acc: u64, input: u64) -> u64 {
    acc.wrapping_add(input.wrapping_mul(P2))
        .rotate_left(31)
        .wrapping_mul(P1)
}

fn merge(acc: u64, val: u64) -> u64 {
    (acc ^ round(0, val)).wrapping_mul(P1).wrapping_add(P4)
}

/// The XXH64 hash of `data` with `seed` (the reference algorithm, little-endian reads, so the
/// value is the same on every host).
///
/// ```
/// use twine_testing::snapshot::xxh64;
/// assert_eq!(xxh64(b"", 0), 0xEF46_DB37_51D8_E999);
/// assert_eq!(xxh64(b"abc", 0), 0x44BC_2CF5_AD77_0999);
/// ```
///
/// Never panics.
#[must_use]
pub fn xxh64(data: &[u8], seed: u64) -> u64 {
    let len = data.len() as u64;
    let mut rest = data;
    let mut h = if data.len() >= 32 {
        let mut v = [
            seed.wrapping_add(P1).wrapping_add(P2),
            seed.wrapping_add(P2),
            seed,
            seed.wrapping_sub(P1),
        ];
        while rest.len() >= 32 {
            for (i, lane) in v.iter_mut().enumerate() {
                *lane = round(*lane, read_u64(&rest[i * 8..]));
            }
            rest = &rest[32..];
        }
        let mut h = v[0]
            .rotate_left(1)
            .wrapping_add(v[1].rotate_left(7))
            .wrapping_add(v[2].rotate_left(12))
            .wrapping_add(v[3].rotate_left(18));
        for lane in v {
            h = merge(h, lane);
        }
        h
    } else {
        seed.wrapping_add(P5)
    };
    h = h.wrapping_add(len);
    while rest.len() >= 8 {
        h ^= round(0, read_u64(rest));
        h = h.rotate_left(27).wrapping_mul(P1).wrapping_add(P4);
        rest = &rest[8..];
    }
    if rest.len() >= 4 {
        h ^= read_u32(rest).wrapping_mul(P1);
        h = h.rotate_left(23).wrapping_mul(P2).wrapping_add(P3);
        rest = &rest[4..];
    }
    for &b in rest {
        h ^= u64::from(b).wrapping_mul(P5);
        h = h.rotate_left(11).wrapping_mul(P1);
    }
    h ^= h >> 33;
    h = h.wrapping_mul(P2);
    h ^= h >> 29;
    h = h.wrapping_mul(P3);
    h ^ (h >> 32)
}

#[cfg(test)]
mod tests {
    use super::xxh64;

    /// Reference values computed with `xxhsum -H1` (xxHash 0.8).
    #[test]
    fn matches_reference_implementation() {
        assert_eq!(xxh64(b"", 0), 0xef46_db37_51d8_e999);
        assert_eq!(xxh64(b"a", 0), 0xd24e_c4f1_a98c_6e5b);
        assert_eq!(xxh64(b"abc", 0), 0x44bc_2cf5_ad77_0999);
        assert_eq!(
            xxh64(b"The quick brown fox jumps over the lazy dog", 0),
            0x0b24_2d36_1fda_71bc
        );
        let ramp: Vec<u8> = (0..=255u8).cycle().take(768).collect();
        assert_eq!(xxh64(&ramp, 0), 0x8e03_c838_c596_036f);
    }

    #[test]
    fn every_tail_length_differs() {
        let data: Vec<u8> = (0..64u8).collect();
        let hashes: std::collections::BTreeSet<u64> =
            (0..=data.len()).map(|n| xxh64(&data[..n], 0)).collect();
        assert_eq!(hashes.len(), data.len() + 1);
    }
}
