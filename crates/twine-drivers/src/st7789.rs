//! ST7789 / ST7789V 240 × 320 IPS controller and its common module variants.
//!
//! | | |
//! |-|-|
//! | Datasheet | Sitronix ST7789V V1.0 |
//! | Max SPI clock | 15 MHz write per datasheet (t<sub>scycw</sub> = 66 ns); modules usually run at 40–80 MHz |
//! | Format | `COLMOD 0x55` → `Rgb565Swapped` |
//! | Quirks | IPS glass: colour inversion **on**; RGB order; small panels show a window of the 240 × 320 memory (offsets) |
//!
//! | Spec | Visible | Offset (`Deg0`) | Typical module |
//! |------|---------|-----------------|----------------|
//! | [`ST7789`] | 240 × 320 | (0, 0) | 2.0"/2.4" IPS |
//! | [`ST7789_240X240`] | 240 × 240 | (0, 0); (0, 80) at `Deg180` | 1.3"/1.54" square |
//! | [`ST7789_135X240`] | 135 × 240 | (52, 40); (53, 40) at `Deg180` | Lilygo T-Display 1.14" |
//!
//! Init: after reset the generic sequence (`COLMOD`, `MADCTL`, `INVON`, `SLPOUT`, `DISPON`) is
//! all the ST7789 needs; the vendor table only adds `NORON` (as mipidsi's `ST7789` model and
//! Adafruit's `generic_st7789` table do). Rotation uses [`standard_madctl`].
//!
//! ```
//! use twine_drivers::st7789::{self, ST7789_135X240};
//! use twine_drivers::testkit::Recorder;
//! use twine_hal::{DisplayDriver, Rotation};
//!
//! let rec = Recorder::new();
//! let lcd = st7789::new(rec.spi(), rec.quiet_pin("dc"), Some(rec.pin("rst")), &ST7789_135X240, Rotation::Deg90, &mut rec.delay()).unwrap();
//! assert_eq!((lcd.info().width, lcd.info().height), (240, 135));
//! ```

use crate::mipi_dcs::{InitOp, Madctl, PanelSpec, cmd, standard_madctl};

static ST7789_INIT: [InitOp; 1] = [InitOp::Cmd(cmd::NORON, &[])];

/// ST7789 240 × 320 (full controller memory), RGB, inverted.
pub static ST7789: PanelSpec = PanelSpec {
    name: "ST7789",
    native_w: 240,
    native_h: 320,
    ram_w: 240,
    ram_h: 320,
    offset_x: 0,
    offset_y: 0,
    madctl: standard_madctl(Madctl::EMPTY),
    colmod: 0x55,
    invert: true,
    init: &ST7789_INIT,
};

/// ST7789 240 × 240 square module (top of the 240 × 320 memory).
pub static ST7789_240X240: PanelSpec = ST7789.with_size(240, 240, 0, 0).with_name("ST7789 240x240");

/// ST7789 135 × 240 module (Lilygo T-Display): window at (52, 40).
pub static ST7789_135X240: PanelSpec = ST7789.with_size(135, 240, 52, 40).with_name("ST7789 135x240");

crate::panel_macros::spi_panel!(St7789, AsyncSt7789, "ST7789", spec);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{BusOp, Recorder, block_on};
    use crate::panel_macros::test_util::{expected_cmds, init, origin_windows, win10};
    use twine_hal::{DisplayDriver, Rotation};

    #[test]
    fn st7789_init_sequence_matches_datasheet() {
        for spec in [&ST7789, &ST7789_240X240, &ST7789_135X240] {
            let (rec, _) = init(spec, Rotation::Deg0);
            assert_eq!(rec.ops(), expected_cmds(spec, Rotation::Deg0, &[(0x13, &[])]));
            // Datasheet: COLMOD 0x55 = 65K colours, 16 bit/pixel; INVON for IPS.
            assert!(rec.ops().contains(&BusOp::Cmd(0x21)));
        }
    }

    #[test]
    fn st7789_rotation_offsets_all_four() {
        assert_eq!(origin_windows(&ST7789), [win10(0, 0); 4]);
        assert_eq!(
            origin_windows(&ST7789_240X240),
            [win10(0, 0), win10(0, 0), win10(0, 80), win10(80, 0)]
        );
        // Matches TFT_eSPI's T-Display offsets (rotation 0..3: 52/40, 40/53, 53/40, 40/52).
        assert_eq!(
            origin_windows(&ST7789_135X240),
            [win10(52, 40), win10(40, 53), win10(53, 40), win10(40, 52)]
        );
    }

    #[test]
    fn st7789_async_same_bytes() {
        let (a, _) = init(&ST7789_240X240, Rotation::Deg180);
        let b = Recorder::new();
        let d = block_on(new_async(
            b.spi(),
            b.quiet_pin("dc"),
            Some(b.pin("rst")),
            &ST7789_240X240,
            Rotation::Deg180,
            &mut b.delay(),
        ))
        .unwrap();
        assert_eq!(a.ops(), b.ops());
        assert_eq!(twine_hal::AsyncDisplayDriver::info(&d).width, 240);
        let rec = Recorder::new();
        let d = new(
            rec.spi(),
            rec.quiet_pin("dc"),
            Some(rec.pin("rst")),
            &ST7789,
            Rotation::Deg270,
            &mut rec.delay(),
        )
        .unwrap();
        assert_eq!((d.info().width, d.info().height), (320, 240));
    }
}
