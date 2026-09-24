//! [`Code128`]: the Code 128 symbology (ISO/IEC 15417).

use alloc::vec::Vec;

/// Bar/space widths (in modules) of symbol values 0–105, bar first. Every pattern has three bars
/// and three spaces totalling 11 modules.
const PATTERNS: [[u8; 6]; 106] = [
    [2, 1, 2, 2, 2, 2],
    [2, 2, 2, 1, 2, 2],
    [2, 2, 2, 2, 2, 1],
    [1, 2, 1, 2, 2, 3],
    [1, 2, 1, 3, 2, 2],
    [1, 3, 1, 2, 2, 2],
    [1, 2, 2, 2, 1, 3],
    [1, 2, 2, 3, 1, 2],
    [1, 3, 2, 2, 1, 2],
    [2, 2, 1, 2, 1, 3],
    [2, 2, 1, 3, 1, 2],
    [2, 3, 1, 2, 1, 2],
    [1, 1, 2, 2, 3, 2],
    [1, 2, 2, 1, 3, 2],
    [1, 2, 2, 2, 3, 1],
    [1, 1, 3, 2, 2, 2],
    [1, 2, 3, 1, 2, 2],
    [1, 2, 3, 2, 2, 1],
    [2, 2, 3, 2, 1, 1],
    [2, 2, 1, 1, 3, 2],
    [2, 2, 1, 2, 3, 1],
    [2, 1, 3, 2, 1, 2],
    [2, 2, 3, 1, 1, 2],
    [3, 1, 2, 1, 3, 1],
    [3, 1, 1, 2, 2, 2],
    [3, 2, 1, 1, 2, 2],
    [3, 2, 1, 2, 2, 1],
    [3, 1, 2, 2, 1, 2],
    [3, 2, 2, 1, 1, 2],
    [3, 2, 2, 2, 1, 1],
    [2, 1, 2, 1, 2, 3],
    [2, 1, 2, 3, 2, 1],
    [2, 3, 2, 1, 2, 1],
    [1, 1, 1, 3, 2, 3],
    [1, 3, 1, 1, 2, 3],
    [1, 3, 1, 3, 2, 1],
    [1, 1, 2, 3, 1, 3],
    [1, 3, 2, 1, 1, 3],
    [1, 3, 2, 3, 1, 1],
    [2, 1, 1, 3, 1, 3],
    [2, 3, 1, 1, 1, 3],
    [2, 3, 1, 3, 1, 1],
    [1, 1, 2, 1, 3, 3],
    [1, 1, 2, 3, 3, 1],
    [1, 3, 2, 1, 3, 1],
    [1, 1, 3, 1, 2, 3],
    [1, 1, 3, 3, 2, 1],
    [1, 3, 3, 1, 2, 1],
    [3, 1, 3, 1, 2, 1],
    [2, 1, 1, 3, 3, 1],
    [2, 3, 1, 1, 3, 1],
    [2, 1, 3, 1, 1, 3],
    [2, 1, 3, 3, 1, 1],
    [2, 1, 3, 1, 3, 1],
    [3, 1, 1, 1, 2, 3],
    [3, 1, 1, 3, 2, 1],
    [3, 3, 1, 1, 2, 1],
    [3, 1, 2, 1, 1, 3],
    [3, 1, 2, 3, 1, 1],
    [3, 3, 2, 1, 1, 1],
    [3, 1, 4, 1, 1, 1],
    [2, 2, 1, 4, 1, 1],
    [4, 3, 1, 1, 1, 1],
    [1, 1, 1, 2, 2, 4],
    [1, 1, 1, 4, 2, 2],
    [1, 2, 1, 1, 2, 4],
    [1, 2, 1, 4, 2, 1],
    [1, 4, 1, 1, 2, 2],
    [1, 4, 1, 2, 2, 1],
    [1, 1, 2, 2, 1, 4],
    [1, 1, 2, 4, 1, 2],
    [1, 2, 2, 1, 1, 4],
    [1, 2, 2, 4, 1, 1],
    [1, 4, 2, 1, 1, 2],
    [1, 4, 2, 2, 1, 1],
    [2, 4, 1, 2, 1, 1],
    [2, 2, 1, 1, 1, 4],
    [4, 1, 3, 1, 1, 1],
    [2, 4, 1, 1, 1, 2],
    [1, 3, 4, 1, 1, 1],
    [1, 1, 1, 2, 4, 2],
    [1, 2, 1, 1, 4, 2],
    [1, 2, 1, 2, 4, 1],
    [1, 1, 4, 2, 1, 2],
    [1, 2, 4, 1, 1, 2],
    [1, 2, 4, 2, 1, 1],
    [4, 1, 1, 2, 1, 2],
    [4, 2, 1, 1, 1, 2],
    [4, 2, 1, 2, 1, 1],
    [2, 1, 2, 1, 4, 1],
    [2, 1, 4, 1, 2, 1],
    [4, 1, 2, 1, 2, 1],
    [1, 1, 1, 1, 4, 3],
    [1, 1, 1, 3, 4, 1],
    [1, 3, 1, 1, 4, 1],
    [1, 1, 4, 1, 1, 3],
    [1, 1, 4, 3, 1, 1],
    [4, 1, 1, 1, 1, 3],
    [4, 1, 1, 3, 1, 1],
    [1, 1, 3, 1, 4, 1],
    [1, 1, 4, 1, 3, 1],
    [3, 1, 1, 1, 4, 1],
    [4, 1, 1, 1, 3, 1],
    [2, 1, 1, 4, 1, 2],
    [2, 1, 1, 2, 1, 4],
    [2, 1, 1, 2, 3, 2],
];

/// The stop pattern: 4 bars and 3 spaces, 13 modules (ending in the 2-module termination bar).
const STOP: [u8; 7] = [2, 3, 3, 1, 1, 1, 2];

/// Symbol value of the stop pattern.
pub(crate) const STOP_VALUE: u8 = 106;

const SHIFT: u8 = 98;
const CODE_C: u8 = 99;
/// `CODE B` in sets A and C.
const CODE_B: u8 = 100;
/// `CODE A` in sets B and C.
const CODE_A: u8 = 101;
const START_A: u8 = 103;
const START_B: u8 = 104;
const START_C: u8 = 105;

/// Why text could not be encoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum Code128Error {
    /// The text is empty.
    #[error("empty barcode text")]
    Empty,
    /// The text contains a character outside ASCII (Code 128 encodes code points 0–127).
    #[error("character {ch:?} at byte {index} cannot be encoded in Code 128")]
    InvalidChar {
        /// Byte offset of the character in the text.
        index: usize,
        /// The character.
        ch: char,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Set {
    A,
    B,
    C,
}

impl Set {
    fn has(self, c: u8) -> bool {
        match self {
            Set::A => c < 96,
            Set::B => (32..128).contains(&c),
            Set::C => false,
        }
    }

    /// Symbol value of `c` in set A or B (the caller checked [`has`](Self::has)).
    fn value(self, c: u8) -> u8 {
        match self {
            Set::A if c < 32 => c + 64,
            _ => c - 32,
        }
    }
}

/// Number of ASCII digits at the start of `s`.
fn digit_run(s: &[u8]) -> usize {
    s.iter().take_while(|c| c.is_ascii_digit()).count()
}

/// Set A when a control character comes before any lower-case letter (or DEL), else B.
fn pick_ab(s: &[u8]) -> Set {
    for &c in s {
        if c < 32 {
            return Set::A;
        }
        if c >= 96 {
            return Set::B;
        }
    }
    Set::B
}

/// A Code 128 symbol: the sequence of symbol values from the start code to the stop code.
///
/// Every symbol is 11 modules wide (the stop symbol 13), alternating bars and spaces, bar first.
/// A scanner also needs a light quiet zone of at least 10 modules on both sides; drawing adds it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Code128 {
    symbols: Vec<u8>,
}

impl Code128 {
    /// Encodes ASCII `text`.
    ///
    /// Code sets are chosen greedily to keep the symbol short: set C (two digits per symbol) for
    /// runs of 4 or more digits (an odd run leaves its first digit in the current set), set A when
    /// a control character is needed before a lower-case letter, set B otherwise; a single
    /// character from the other of A/B uses `SHIFT`.
    ///
    /// ```
    /// use twine_extra::barcode::{Code128, Code128Error};
    ///
    /// let code = Code128::encode("Wikipedia").unwrap();
    /// assert_eq!(code.symbols()[0], 104); // Start B
    /// assert_eq!(code.check_symbol(), 88);
    ///
    /// assert_eq!(Code128::encode("€1"), Err(Code128Error::InvalidChar { index: 0, ch: '€' }));
    /// ```
    pub fn encode(text: &str) -> Result<Self, Code128Error> {
        if let Some((index, ch)) = text.char_indices().find(|(_, c)| !c.is_ascii()) {
            return Err(Code128Error::InvalidChar { index, ch });
        }
        let s = text.as_bytes();
        if s.is_empty() {
            return Err(Code128Error::Empty);
        }
        let mut symbols = Vec::with_capacity(s.len() + 4);
        let lead = digit_run(s);
        let mut set = if lead >= 4 || (lead == 2 && s.len() == 2) {
            Set::C
        } else {
            pick_ab(s)
        };
        symbols.push(match set {
            Set::A => START_A,
            Set::B => START_B,
            Set::C => START_C,
        });
        let mut i = 0;
        while i < s.len() {
            if set == Set::C {
                if digit_run(&s[i..]) >= 2 {
                    symbols.push((s[i] - b'0') * 10 + (s[i + 1] - b'0'));
                    i += 2;
                } else {
                    set = pick_ab(&s[i..]);
                    symbols.push(if set == Set::A { CODE_A } else { CODE_B });
                }
                continue;
            }
            let run = digit_run(&s[i..]);
            if run >= 4 {
                if run % 2 == 1 {
                    symbols.push(set.value(s[i]));
                    i += 1;
                }
                symbols.push(CODE_C);
                set = Set::C;
                continue;
            }
            let c = s[i];
            if set.has(c) {
                symbols.push(set.value(c));
                i += 1;
                continue;
            }
            let other = if set == Set::A { Set::B } else { Set::A };
            if s.get(i + 1).is_some_and(|&n| set.has(n)) {
                symbols.push(SHIFT);
                symbols.push(other.value(c));
                i += 1;
            } else {
                symbols.push(if other == Set::A { CODE_A } else { CODE_B });
                set = other;
            }
        }
        let check = symbols
            .iter()
            .enumerate()
            .fold(u32::from(symbols[0]), |acc, (pos, &v)| {
                (acc + pos as u32 * u32::from(v)) % 103
            });
        symbols.push(check as u8);
        symbols.push(STOP_VALUE);
        Ok(Self { symbols })
    }

    /// Symbol values: start code, data (including code set switches and shifts), check symbol,
    /// stop code (106).
    #[must_use]
    pub fn symbols(&self) -> &[u8] {
        &self.symbols
    }

    /// The modulo-103 check symbol.
    #[must_use]
    pub fn check_symbol(&self) -> u8 {
        self.symbols[self.symbols.len() - 2]
    }

    /// Width of the symbol in modules, without quiet zones.
    #[must_use]
    pub fn module_count(&self) -> u32 {
        11 * (self.symbols.len() as u32 - 1) + 13
    }

    /// Bar and space widths in modules, alternating, bar first (the pattern of every symbol in
    /// order).
    pub fn widths(&self) -> impl Iterator<Item = u8> + '_ {
        self.symbols
            .iter()
            .flat_map(|&v| -> &'static [u8] {
                if v == STOP_VALUE {
                    &STOP
                } else {
                    &PATTERNS[usize::from(v)]
                }
            })
            .copied()
    }

    /// The bars as `(first module, width in modules)`, left to right.
    ///
    /// ```
    /// use twine_extra::barcode::Code128;
    ///
    /// let code = Code128::encode("A").unwrap();
    /// // Start B = 2 1 1 2 1 4: bars at modules 0 (2 wide), 3 (1 wide) and 6 (1 wide).
    /// let bars: Vec<_> = code.bars().take(3).collect();
    /// assert_eq!(bars, [(0, 2), (3, 1), (6, 1)]);
    /// ```
    #[must_use]
    pub fn bars(&self) -> Bars<'_> {
        Bars {
            symbols: &self.symbols,
            symbol: 0,
            element: 0,
            module: 0,
        }
    }
}

/// Iterator over the bars of a [`Code128`], see [`Code128::bars`].
#[derive(Clone, Debug)]
pub struct Bars<'a> {
    symbols: &'a [u8],
    symbol: usize,
    element: usize,
    module: u32,
}

impl Iterator for Bars<'_> {
    type Item = (u32, u8);

    fn next(&mut self) -> Option<(u32, u8)> {
        let &v = self.symbols.get(self.symbol)?;
        let pattern: &[u8] = if v == STOP_VALUE {
            &STOP
        } else {
            &PATTERNS[usize::from(v)]
        };
        let bar = (self.module, pattern[self.element]);
        self.module += u32::from(pattern[self.element]);
        if let Some(&space) = pattern.get(self.element + 1) {
            self.module += u32::from(space);
            self.element += 2;
        } else {
            self.element += 1;
        }
        if self.element >= pattern.len() {
            self.symbol += 1;
            self.element = 0;
        }
        Some(bar)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns_are_well_formed_and_distinct() {
        for (v, p) in PATTERNS.iter().enumerate() {
            assert_eq!(p.iter().map(|&w| u32::from(w)).sum::<u32>(), 11, "value {v}");
            assert!(p.iter().all(|&w| (1..=4).contains(&w)), "value {v}");
            // Bars (even elements) sum to an even number of modules (parity check of the spec).
            assert_eq!((p[0] + p[2] + p[4]) % 2, 0, "value {v}");
            for q in &PATTERNS[..v] {
                assert_ne!(p, q, "value {v} duplicated");
            }
        }
        assert_eq!(STOP.iter().map(|&w| u32::from(w)).sum::<u32>(), 13);
    }

    #[test]
    fn bars_match_widths() {
        let code = Code128::encode("Mixed\tcase 12345 x").unwrap();
        let widths: Vec<u8> = code.widths().collect();
        let mut expected = Vec::new();
        let mut m = 0u32;
        for (i, &w) in widths.iter().enumerate() {
            if i % 2 == 0 {
                expected.push((m, w));
            }
            m += u32::from(w);
        }
        assert_eq!(m, code.module_count());
        assert_eq!(code.bars().collect::<Vec<_>>(), expected);
    }

    #[test]
    fn shift_for_single_character_of_the_other_set() {
        // "a\tb": start B, 'a', SHIFT, TAB (A value 73), 'b'.
        let code = Code128::encode("a\tb").unwrap();
        assert_eq!(&code.symbols()[..5], &[START_B, 65, SHIFT, 73, 66]);
        // Two control characters in a row switch to A (and back to B for the final letter).
        let code = Code128::encode("a\t\tb").unwrap();
        assert_eq!(&code.symbols()[..7], &[START_B, 65, CODE_A, 73, 73, CODE_B, 66]);
    }

    #[test]
    fn odd_digit_run_keeps_first_digit() {
        // "x12345": B 'x' '1', CODE C, 23, 45.
        let code = Code128::encode("x12345").unwrap();
        assert_eq!(&code.symbols()[..6], &[START_B, 88, 17, CODE_C, 23, 45]);
        // Leading odd run: C pairs, then the last digit in B.
        let code = Code128::encode("12345").unwrap();
        assert_eq!(&code.symbols()[..5], &[START_C, 12, 34, CODE_B, 21]);
    }
}
