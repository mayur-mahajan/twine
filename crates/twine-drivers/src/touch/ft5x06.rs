//! Focaltech FT5206 / FT5306 / FT5406 capacitive touch controllers (I2C, up to 5 points).
//!
//! | | |
//! |-|-|
//! | Datasheet | Focaltech `FT5x06` datasheet / application note (register map) |
//! | Address | `0x38` |
//! | IRQ | `INT`, active low |
//! | Rotation | reports panel coordinates: use [`TouchTransform`](super::TouchTransform) |
//!
//! The register map (`TD_STATUS` at `0x02`, first point at `0x03`–`0x06`) is identical to the
//! `FT6x36`, so [`Ft5x06`] is the [`Ft6x36`](super::Ft6x36) driver; the first of up to 5 points
//! is reported (twine's pointer input is single-touch).

/// Default I2C address.
pub const ADDR: u8 = super::ft6x36::ADDR;

/// `FT5x06` driver: the `FT6x36` driver, whose register map it shares.
pub type Ft5x06<I2C, IRQ> = super::ft6x36::Ft6x36<I2C, IRQ>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::Recorder;
    use crate::touch::TouchTransform;
    use crate::touch::test_util::Regs;
    use twine_core::Point;
    use twine_hal::{InputData, InputDevice, PointerData};

    #[test]
    fn ft5x06_reports_first_of_five() {
        let rec = Recorder::new();
        let regs = Regs::install(&rec);
        regs.set(0x02, &[0x05, 0x80, 0x64, 0x00, 0xC8]);
        let mut t: Ft5x06<_, crate::NoPin> = Ft5x06::new(rec.i2c(), None, TouchTransform::identity(800, 480));
        assert_eq!(
            t.read(),
            InputData::Pointer(PointerData {
                point: Point::new(100, 200),
                pressed: true
            })
        );
        assert_eq!(ADDR, 0x38);
    }
}
