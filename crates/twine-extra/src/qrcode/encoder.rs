//! [`QrMatrix`]: encoding bytes into a packed QR code module matrix.

use alloc::vec;
use alloc::vec::Vec;

use qrcodegen_no_heap::{QrCode, QrCodeEcc, Version};

/// Error correction level. Higher levels survive more damage but need a larger code.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Ecc {
    /// About 7 % of the codewords can be restored.
    Low,
    /// About 15 % of the codewords can be restored (the default, like LVGL).
    #[default]
    Medium,
    /// About 25 % of the codewords can be restored.
    Quartile,
    /// About 30 % of the codewords can be restored.
    High,
}

impl Ecc {
    const fn to_qrcodegen(self) -> QrCodeEcc {
        match self {
            Ecc::Low => QrCodeEcc::Low,
            Ecc::Medium => QrCodeEcc::Medium,
            Ecc::Quartile => QrCodeEcc::Quartile,
            Ecc::High => QrCodeEcc::High,
        }
    }

    /// Reads the level from the format information next to the top-left finder pattern: its
    /// two most significant bits, stored at (0, 8) and (1, 8), XOR the format mask `0b10`.
    /// (`qrcodegen-no-heap` 1.8's `error_correction_level()` maps these bits wrongly.)
    fn from_format_bits(qr: &QrCode<'_>) -> Self {
        let raw = u8::from(qr.get_module(0, 8)) << 1 | u8::from(qr.get_module(1, 8));
        match raw ^ 0b10 {
            0b01 => Ecc::Low,
            0b00 => Ecc::Medium,
            0b11 => Ecc::Quartile,
            _ => Ecc::High,
        }
    }
}

/// Why data could not be encoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum QrError {
    /// The data does not fit in a version 40 code at the requested error correction level.
    #[error("data of {len} bytes does not fit in a QR code at this error correction level")]
    DataTooLong {
        /// Length of the rejected data in bytes.
        len: usize,
    },
}

/// Largest payload of any QR code (7089 digits, version 40, level L); longer data is rejected
/// before any allocation.
const MAX_DATA_LEN: usize = 7089;

/// Largest version tried for each step of the encoding ladder: the scratch buffers grow only as
/// far as the data needs, so short texts do not allocate the 2 × 3.9 KiB of version 40.
const VERSION_LADDER: [u8; 6] = [4, 10, 16, 24, 32, 40];

/// A QR code as a square matrix of dark and light modules.
///
/// Rows are packed MSB first, each row starting on a byte boundary. The matrix does not include
/// the quiet zone (the light margin around the code); drawing adds it.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct QrMatrix {
    size: u8,
    stride: u8,
    ecc: Ecc,
    bits: Vec<u8>,
}

impl core::fmt::Debug for QrMatrix {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("QrMatrix")
            .field("version", &self.version())
            .field("size", &self.size)
            .field("ecc", &self.ecc)
            .finish_non_exhaustive()
    }
}

impl QrMatrix {
    /// Encodes `data` into the smallest QR code that holds it with at least the error correction
    /// level `ecc`.
    ///
    /// Valid UTF-8 made only of digits uses numeric mode; digits, upper-case letters and
    /// ` $%*+-./:` use alphanumeric mode; anything else is stored as bytes (UTF-8 text is read
    /// back as text by scanners). The level is raised while the data still fits the chosen
    /// version (see [`ecc`](Self::ecc)); the mask is chosen by the standard penalty rules.
    ///
    /// ```
    /// use twine_extra::qrcode::{Ecc, QrError, QrMatrix};
    ///
    /// let qr = QrMatrix::encode(b"HELLO WORLD", Ecc::Quartile).unwrap();
    /// assert_eq!((qr.version(), qr.ecc()), (1, Ecc::Quartile));
    ///
    /// let too_long = [b'x'; 3000];
    /// assert_eq!(QrMatrix::encode(&too_long, Ecc::Low), Err(QrError::DataTooLong { len: 3000 }));
    /// ```
    pub fn encode(data: &[u8], ecc: Ecc) -> Result<Self, QrError> {
        let too_long = QrError::DataTooLong { len: data.len() };
        if data.len() > MAX_DATA_LEN {
            return Err(too_long);
        }
        let text = core::str::from_utf8(data).ok();
        let mut temp = Vec::new();
        let mut out = Vec::new();
        for max in VERSION_LADDER {
            let max = Version::new(max);
            let len = max.buffer_len();
            if len < data.len() {
                // Byte mode copies the data into the scratch buffer first.
                continue;
            }
            temp.resize(len, 0);
            out.resize(len, 0);
            let ecl = ecc.to_qrcodegen();
            let qr = if let Some(t) = text {
                QrCode::encode_text(t, &mut temp, &mut out, ecl, Version::MIN, max, None, true)
            } else {
                temp[..data.len()].copy_from_slice(data);
                QrCode::encode_binary(
                    &mut temp,
                    data.len(),
                    &mut out,
                    ecl,
                    Version::MIN,
                    max,
                    None,
                    true,
                )
            };
            if let Ok(qr) = qr {
                return Ok(Self::from_qrcodegen(&qr));
            }
        }
        Err(too_long)
    }

    fn from_qrcodegen(qr: &QrCode<'_>) -> Self {
        let size = qr.size() as u8;
        let stride = size.div_ceil(8);
        let mut bits = vec![0u8; usize::from(stride) * usize::from(size)];
        for y in 0..size {
            let row = &mut bits[usize::from(y) * usize::from(stride)..][..usize::from(stride)];
            for x in 0..size {
                if qr.get_module(i32::from(x), i32::from(y)) {
                    row[usize::from(x / 8)] |= 0x80 >> (x % 8);
                }
            }
        }
        Self {
            size,
            stride,
            ecc: Ecc::from_format_bits(qr),
            bits,
        }
    }

    /// Modules per side, `17 + 4 × version` (21–177).
    #[must_use]
    pub fn size(&self) -> u8 {
        self.size
    }

    /// The QR code version (1–40).
    #[must_use]
    pub fn version(&self) -> u8 {
        (self.size - 17) / 4
    }

    /// The error correction level actually used: at least the requested one, possibly higher
    /// when the data fits the same version at a higher level.
    #[must_use]
    pub fn ecc(&self) -> Ecc {
        self.ecc
    }

    /// Whether the module at column `x`, row `y` is dark (`false` outside the matrix).
    #[must_use]
    pub fn get(&self, x: u8, y: u8) -> bool {
        x < self.size
            && y < self.size
            && self.bits[usize::from(y) * usize::from(self.stride) + usize::from(x / 8)] & (0x80 >> (x % 8))
                != 0
    }

    /// The packed modules of row `y` (MSB first, `ceil(size / 8)` bytes; bits past `size` are 0).
    ///
    /// # Panics
    /// If `y >= size`.
    #[must_use]
    pub fn row(&self, y: u8) -> &[u8] {
        assert!(y < self.size, "row {y} out of range");
        &self.bits[usize::from(y) * usize::from(self.stride)..][..usize::from(self.stride)]
    }

    /// The runs of dark modules in row `y` as `(first column, length)`, left to right (empty
    /// outside the matrix).
    ///
    /// ```
    /// use twine_extra::qrcode::{Ecc, QrMatrix};
    ///
    /// let qr = QrMatrix::encode(b"1", Ecc::Low).unwrap();
    /// // The top row starts with the 7-module finder pattern.
    /// assert_eq!(qr.dark_runs(0).next(), Some((0, 7)));
    /// ```
    #[must_use]
    pub fn dark_runs(&self, y: u8) -> DarkRuns<'_> {
        let row: &[u8] = if y < self.size { self.row(y) } else { &[] };
        DarkRuns {
            row,
            size: if y < self.size { self.size } else { 0 },
            x: 0,
        }
    }
}

/// Iterator over the dark runs of one matrix row, see [`QrMatrix::dark_runs`].
#[derive(Clone, Debug)]
pub struct DarkRuns<'a> {
    row: &'a [u8],
    size: u8,
    x: u8,
}

impl DarkRuns<'_> {
    fn bit(&self, x: u8) -> bool {
        self.row[usize::from(x / 8)] & (0x80 >> (x % 8)) != 0
    }
}

impl Iterator for DarkRuns<'_> {
    type Item = (u8, u8);

    fn next(&mut self) -> Option<(u8, u8)> {
        while self.x < self.size && !self.bit(self.x) {
            // Skip whole light bytes quickly.
            if self.x % 8 == 0 && self.row[usize::from(self.x / 8)] == 0 {
                self.x = self.x.saturating_add(8).min(self.size);
            } else {
                self.x += 1;
            }
        }
        if self.x >= self.size {
            return None;
        }
        let start = self.x;
        while self.x < self.size && self.bit(self.x) {
            self.x += 1;
        }
        Some((start, self.x - start))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_cover_exactly_the_dark_modules() {
        let qr = QrMatrix::encode(b"twine runs test 0123456789", Ecc::High).unwrap();
        for y in 0..qr.size() {
            let mut dark = [false; 177];
            for (x, n) in qr.dark_runs(y) {
                assert!(n > 0);
                for i in x..x + n {
                    dark[usize::from(i)] = true;
                }
            }
            for x in 0..qr.size() {
                assert_eq!(dark[usize::from(x)], qr.get(x, y), "({x}, {y})");
            }
        }
        assert_eq!(qr.dark_runs(qr.size()).count(), 0);
    }

    #[test]
    fn ladder_reaches_version_40() {
        let data = alloc::vec![b'a'; 2953];
        let qr = QrMatrix::encode(&data, Ecc::Low).unwrap();
        assert_eq!((qr.version(), qr.size()), (40, 177));
        assert!(QrMatrix::encode(&alloc::vec![b'a'; 2954], Ecc::Low).is_err());
    }

    #[test]
    fn modes_are_chosen_automatically() {
        // Version 1-L capacities: 41 digits, 25 alphanumeric characters, 17 bytes.
        assert_eq!(QrMatrix::encode(&[b'7'; 41], Ecc::Low).unwrap().version(), 1);
        assert_eq!(QrMatrix::encode(&[b'A'; 25], Ecc::Low).unwrap().version(), 1);
        assert_eq!(QrMatrix::encode(&[b'a'; 17], Ecc::Low).unwrap().version(), 1);
        assert_eq!(QrMatrix::encode(&[b'a'; 18], Ecc::Low).unwrap().version(), 2);
        // Invalid UTF-8 goes through byte mode.
        assert_eq!(QrMatrix::encode(&[0xFF; 17], Ecc::Low).unwrap().version(), 1);
    }

    #[test]
    fn empty_data_is_a_version_1_code() {
        let qr = QrMatrix::encode(b"", Ecc::High).unwrap();
        assert_eq!((qr.version(), qr.ecc()), (1, Ecc::High));
    }
}
