//! ST7796S 320 × 480 TFT controller (3.5"/4.0" SPI modules).
//!
//! | | |
//! |-|-|
//! | Datasheet | Sitronix ST7796S V1.0 |
//! | Max SPI clock | 15 MHz write per datasheet; modules commonly run at 40 MHz |
//! | Format | `COLMOD 0x55` → `Rgb565Swapped` |
//! | Quirks | most modules have BGR glass and no inversion; use [`PanelSpec::with_bgr`] / [`PanelSpec::with_invert`] for others |
//!
//! Init as mipidsi's `ST7796` model (which reuses its ST7789 sequence): the generic init plus
//! `NORON`. Rotation uses [`standard_madctl`].
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
//! use twine_drivers::st7796;
//! use twine_drivers::testkit::Recorder;
//! use twine_hal::{DisplayDriver, Rotation};
//!
//! let rec = Recorder::new();
//! let lcd = st7796::new(rec.spi(), rec.quiet_pin("dc"), Some(rec.pin("rst")), Rotation::Deg90, &mut rec.delay()).unwrap();
//! assert_eq!((lcd.info().width, lcd.info().height), (480, 320));
//! ```

use crate::mipi_dcs::{InitOp, Madctl, PanelSpec, cmd, standard_madctl};

static ST7796_INIT: [InitOp; 1] = [InitOp::Cmd(cmd::NORON, &[])];

/// ST7796S 320 × 480, BGR, not inverted.
pub static ST7796: PanelSpec = PanelSpec {
    name: "ST7796S",
    native_w: 320,
    native_h: 480,
    ram_w: 320,
    ram_h: 480,
    offset_x: 0,
    offset_y: 0,
    madctl: standard_madctl(Madctl::BGR),
    colmod: 0x55,
    align: 1,
    sw_rotation: false,
    invert: false,
    init: &ST7796_INIT,
};

crate::panel_macros::spi_panel!(St7796, AsyncSt7796, "ST7796S", ST7796);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{Recorder, block_on};
    use crate::panel_macros::test_util::{expected_cmds, init, origin_windows, win10};
    use twine_hal::Rotation;

    #[test]
    fn st7796_init_sequence_matches_datasheet() {
        let (rec, _) = init(&ST7796, Rotation::Deg0);
        assert_eq!(rec.ops(), expected_cmds(&ST7796, Rotation::Deg0, &[(0x13, &[])]));
    }

    #[test]
    fn st7796_rotation_offsets_all_four() {
        assert_eq!(origin_windows(&ST7796), [win10(0, 0); 4]);
        assert_eq!(ST7796.madctl.map(Madctl::bits), [0x08, 0xA8, 0xC8, 0x68]);
    }

    #[test]
    fn st7796_async_same_bytes() {
        let (a, _) = init(&ST7796, Rotation::Deg270);
        let b = Recorder::new();
        let _d = block_on(new_async(
            b.spi(),
            b.quiet_pin("dc"),
            Some(b.pin("rst")),
            Rotation::Deg270,
            &mut b.delay(),
        ))
        .unwrap();
        assert_eq!(a.ops(), b.ops());
    }
}
