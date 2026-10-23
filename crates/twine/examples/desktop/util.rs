//! Small, dependency-free helpers shared by the whole player: a seeded random number
//! generator (so the catalogue, the shuffle order and the simulated audio are the same on
//! every run, which keeps scripted runs and README recordings reproducible) and the time
//! formats the UI shows.

/// `SplitMix64`: a tiny, fast, well-distributed generator. Every random choice in the app is
/// drawn from one of these with a fixed seed, never from the system clock.
#[derive(Clone, Copy, Debug)]
pub struct Rng(u64);

impl Rng {
    /// A generator whose sequence is fully determined by `seed`.
    pub const fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// The next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        mix(self.0)
    }

    /// A value in `lo..=hi` (`hi < lo` gives `lo`).
    pub fn range(&mut self, lo: u32, hi: u32) -> u32 {
        let span = u64::from(hi.saturating_sub(lo)) + 1;
        lo + (self.next_u64() % span) as u32
    }

    /// One element of `items` (which must not be empty).
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[(self.next_u64() % items.len() as u64) as usize]
    }
}

/// The `SplitMix64` finaliser: a stateless hash of `x`. The simulated audio uses it to derive
/// a note, an accent or a bit of jitter from (track, bar, beat) without keeping any state.
pub const fn mix(mut x: u64) -> u64 {
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// `m:ss` of a duration in seconds (`4:07`), or `h:mm:ss` from an hour on.
pub fn clock(secs: u32) -> String {
    if secs >= 3600 {
        format!("{}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60)
    } else {
        format!("{}:{:02}", secs / 60, secs % 60)
    }
}

/// A long duration for summaries: `34 hr 12 min`, `47 min`.
pub fn summary(secs: u32) -> String {
    let (h, m) = (secs / 3600, secs / 60 % 60);
    if h > 0 {
        format!("{h} hr {m} min")
    } else {
        format!("{m} min")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(clock(247), "4:07");
        assert_eq!(clock(3725), "1:02:05");
        assert_eq!(summary(123_120), "34 hr 12 min");
    }
}
