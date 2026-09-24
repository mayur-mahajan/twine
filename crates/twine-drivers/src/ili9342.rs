//! ILI9342C 320 × 240 TFT controller (landscape-native ILI9341 sibling: `M5Stack` Core,
//! ESP32-S3-BOX-3).
//!
//! | | |
//! |-|-|
//! | Datasheet | ILITEK ILI9342C V1.0 |
//! | Max SPI clock | 10 MHz write per datasheet; boards run it at 40 MHz |
//! | Format | `COLMOD 0x55` → `Rgb565Swapped` |
//! | Quirks | IPS glass on the common boards: BGR and inversion **on** (use [`PanelSpec::with_invert`] for TN panels) |
//!
//! Init as mipidsi's `ili934x::init_common`: `0xB4 [0x00]` (display inversion control: dot
//! inversion) and `NORON`, plus the generic sequence. Rotation uses [`standard_madctl`];
//! `Deg0` is landscape 320 × 240.
//!
//! # Wiring
//!
//! | Panel pin | Driver argument |
//! |-----------|-----------------|
//! | `SCK`, `SDI`/`MOSI` (`SDO`/`MISO` is not needed) | the bus of `spi`, an `embedded_hal(_async)::spi::SpiDevice` (use a DMA-capable async one to overlap transfers with rendering) |
//! | `CS` | the chip select owned by `spi` |
//! | `DC`/`RS` | `dc`, any `OutputPin` |
//! | `RESET` | `rst`: `Some(OutputPin)`, or `None` when it is tied to the MCU reset (a software reset is sent instead) |
//! | `LED`/`BL` | not driven by the driver: switch it (or PWM it) from your firmware |
//!
//! ```
//! use twine_drivers::ili9342;
//! use twine_drivers::testkit::Recorder;
//! use twine_hal::{DisplayDriver, Rotation};
//!
//! let rec = Recorder::new();
//! let lcd = ili9342::new(rec.spi(), rec.quiet_pin("dc"), Some(rec.pin("rst")), Rotation::Deg0, &mut rec.delay()).unwrap();
//! assert_eq!((lcd.info().width, lcd.info().height), (320, 240));
//! ```

use crate::mipi_dcs::{InitOp, Madctl, PanelSpec, cmd, standard_madctl};

static ILI9342_INIT: [InitOp; 2] = [
    InitOp::Cmd(0xB4, &[0x00]), // Display inversion control: dot inversion
    InitOp::Cmd(cmd::NORON, &[]),
];

/// ILI9342C 320 × 240 (landscape native), BGR, inverted.
pub static ILI9342C: PanelSpec = PanelSpec {
    name: "ILI9342C",
    native_w: 320,
    native_h: 240,
    ram_w: 320,
    ram_h: 240,
    offset_x: 0,
    offset_y: 0,
    madctl: standard_madctl(Madctl::BGR),
    colmod: 0x55,
    align: 1,
    sw_rotation: false,
    invert: true,
    init: &ILI9342_INIT,
};

crate::panel_macros::spi_panel!(Ili9342, AsyncIli9342, "ILI9342C", ILI9342C);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{Recorder, block_on};
    use crate::panel_macros::test_util::{expected_cmds, init, origin_windows, win10};
    use twine_hal::{DisplayDriver, Rotation};

    #[test]
    fn ili9342_init_sequence_matches_datasheet() {
        let (rec, d) = init(&ILI9342C, Rotation::Deg0);
        assert_eq!(
            rec.ops(),
            expected_cmds(&ILI9342C, Rotation::Deg0, &[(0xB4, &[0x00]), (0x13, &[])])
        );
        assert_eq!((d.info().width, d.info().height), (320, 240));
    }

    #[test]
    fn ili9342_rotation_offsets_all_four() {
        assert_eq!(origin_windows(&ILI9342C), [win10(0, 0); 4]);
    }

    #[test]
    fn ili9342_async_same_bytes() {
        let (a, _) = init(&ILI9342C, Rotation::Deg90);
        let b = Recorder::new();
        let d = block_on(new_async(
            b.spi(),
            b.quiet_pin("dc"),
            Some(b.pin("rst")),
            Rotation::Deg90,
            &mut b.delay(),
        ))
        .unwrap();
        assert_eq!(a.ops(), b.ops());
        assert_eq!(twine_hal::AsyncDisplayDriver::info(&d).width, 240);
    }
}
