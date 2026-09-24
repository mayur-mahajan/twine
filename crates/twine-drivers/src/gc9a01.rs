//! GC9A01 240 × 240 round IPS controller (1.28" round modules).
//!
//! | | |
//! |-|-|
//! | Datasheet | Galaxycore GC9A01A V1.0 |
//! | Max SPI clock | 100 MHz per datasheet (t<sub>scycw</sub> = 10 ns); modules typically 40–80 MHz |
//! | Format | `COLMOD 0x55` → `Rgb565Swapped` |
//! | Quirks | long undocumented vendor init table; IPS: BGR and inversion **on**; round glass (corners of the 240 × 240 memory are invisible) |
//!
//! The vendor table is the one of mipidsi's `GC9A01` model (MIT/Apache-2.0,
//! <https://github.com/almindor/mipidsi>, itself from the manufacturer's sample code), minus
//! `MADCTL`, `COLMOD`, inversion, `SLPOUT` and `DISPON`, which the generic init sends. Rotation
//! uses [`standard_madctl`].
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
//! use twine_drivers::gc9a01;
//! use twine_drivers::testkit::Recorder;
//! use twine_hal::{DisplayDriver, Rotation};
//!
//! let rec = Recorder::new();
//! let lcd = gc9a01::new(rec.spi(), rec.quiet_pin("dc"), Some(rec.pin("rst")), Rotation::Deg0, &mut rec.delay()).unwrap();
//! assert_eq!((lcd.info().width, lcd.info().height), (240, 240));
//! ```

use crate::mipi_dcs::{InitOp, Madctl, PanelSpec, standard_madctl};

/// GC9A01 vendor init table (mipidsi `GC9A01::init`).
pub static GC9A01_INIT: [InitOp; 44] = [
    InitOp::Cmd(0xEF, &[]), // inter register enable 2
    InitOp::Cmd(0xEB, &[0x14]),
    InitOp::Cmd(0xFE, &[]), // inter register enable 1
    InitOp::Cmd(0xEF, &[]), // inter register enable 2
    InitOp::Cmd(0xEB, &[0x14]),
    InitOp::Cmd(0x84, &[0x40]),
    InitOp::Cmd(0x85, &[0xFF]),
    InitOp::Cmd(0x86, &[0xFF]),
    InitOp::Cmd(0x87, &[0xFF]),
    InitOp::Cmd(0x88, &[0x0A]),
    InitOp::Cmd(0x89, &[0x21]),
    InitOp::Cmd(0x8A, &[0x00]),
    InitOp::Cmd(0x8B, &[0x80]),
    InitOp::Cmd(0x8C, &[0x01]),
    InitOp::Cmd(0x8D, &[0x01]),
    InitOp::Cmd(0x8E, &[0xFF]),
    InitOp::Cmd(0x8F, &[0xFF]),
    InitOp::Cmd(0xB6, &[0x00, 0x20]), // display function control
    InitOp::Cmd(0x90, &[0x08, 0x08, 0x08, 0x08]),
    InitOp::Cmd(0xBD, &[0x06]),
    InitOp::Cmd(0xBC, &[0x00]),
    InitOp::Cmd(0xFF, &[0x60, 0x01, 0x04]),
    InitOp::Cmd(0xC3, &[0x13]), // power control 2
    InitOp::Cmd(0xC4, &[0x13]), // power control 3
    InitOp::Cmd(0xC9, &[0x22]), // power control 4
    InitOp::Cmd(0xBE, &[0x11]),
    InitOp::Cmd(0xE1, &[0x10, 0x0E]),
    InitOp::Cmd(0xDF, &[0x20, 0x0C, 0x02]),
    InitOp::Cmd(0xF0, &[0x45, 0x09, 0x08, 0x08, 0x26, 0x2A]), // gamma 1
    InitOp::Cmd(0xF1, &[0x43, 0x70, 0x72, 0x36, 0x37, 0x6F]), // gamma 2
    InitOp::Cmd(0xF2, &[0x45, 0x09, 0x08, 0x08, 0x26, 0x2A]), // gamma 3
    InitOp::Cmd(0xF3, &[0x43, 0x70, 0x72, 0x36, 0x37, 0x6F]), // gamma 4
    InitOp::Cmd(0xED, &[0x18, 0x0B]),
    InitOp::Cmd(0xAE, &[0x77]),
    InitOp::Cmd(0xCD, &[0x63]),
    InitOp::Cmd(0x70, &[0x07, 0x07, 0x04, 0x0E, 0x0F, 0x09, 0x07, 0x08, 0x03]),
    InitOp::Cmd(0xE8, &[0x34]), // frame rate
    InitOp::Cmd(
        0x62,
        &[
            0x18, 0x0D, 0x71, 0xED, 0x70, 0x70, 0x18, 0x0F, 0x71, 0xEF, 0x70, 0x70,
        ],
    ),
    InitOp::Cmd(
        0x63,
        &[
            0x18, 0x11, 0x71, 0xF1, 0x70, 0x70, 0x18, 0x13, 0x71, 0xF3, 0x70, 0x70,
        ],
    ),
    InitOp::Cmd(0x64, &[0x28, 0x29, 0xF1, 0x01, 0xF1, 0x00, 0x07]),
    InitOp::Cmd(
        0x66,
        &[0x3C, 0x00, 0xCD, 0x67, 0x45, 0x45, 0x10, 0x00, 0x00, 0x00],
    ),
    InitOp::Cmd(
        0x67,
        &[0x00, 0x3C, 0x00, 0x00, 0x00, 0x01, 0x54, 0x10, 0x32, 0x98],
    ),
    InitOp::Cmd(0x74, &[0x10, 0x85, 0x80, 0x00, 0x00, 0x4E, 0x00]),
    InitOp::Cmd(0x98, &[0x3E, 0x07]),
];

/// GC9A01 240 × 240 round panel, BGR, inverted.
pub static GC9A01: PanelSpec = PanelSpec {
    name: "GC9A01",
    native_w: 240,
    native_h: 240,
    ram_w: 240,
    ram_h: 240,
    offset_x: 0,
    offset_y: 0,
    madctl: standard_madctl(Madctl::BGR),
    colmod: 0x55,
    align: 1,
    sw_rotation: false,
    invert: true,
    init: &GC9A01_INIT,
};

crate::panel_macros::spi_panel!(Gc9a01, AsyncGc9a01, "GC9A01", GC9A01);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{Recorder, block_on};
    use crate::panel_macros::test_util::{expected_cmds, init, origin_windows, win10};
    use twine_hal::Rotation;

    /// Transcribed from mipidsi 0.10 `models/gc9a01.rs`.
    const MIPIDSI: [(u8, &[u8]); 44] = [
        (0xEF, &[]),
        (0xEB, &[0x14]),
        (0xFE, &[]),
        (0xEF, &[]),
        (0xEB, &[0x14]),
        (0x84, &[0x40]),
        (0x85, &[0xFF]),
        (0x86, &[0xFF]),
        (0x87, &[0xFF]),
        (0x88, &[0x0A]),
        (0x89, &[0x21]),
        (0x8A, &[0x00]),
        (0x8B, &[0x80]),
        (0x8C, &[0x01]),
        (0x8D, &[0x01]),
        (0x8E, &[0xFF]),
        (0x8F, &[0xFF]),
        (0xB6, &[0x00, 0x20]),
        (0x90, &[0x08, 0x08, 0x08, 0x08]),
        (0xBD, &[0x06]),
        (0xBC, &[0x00]),
        (0xFF, &[0x60, 0x01, 0x04]),
        (0xC3, &[0x13]),
        (0xC4, &[0x13]),
        (0xC9, &[0x22]),
        (0xBE, &[0x11]),
        (0xE1, &[0x10, 0x0E]),
        (0xDF, &[0x20, 0x0c, 0x02]),
        (0xF0, &[0x45, 0x09, 0x08, 0x08, 0x26, 0x2A]),
        (0xF1, &[0x43, 0x70, 0x72, 0x36, 0x37, 0x6f]),
        (0xF2, &[0x45, 0x09, 0x08, 0x08, 0x26, 0x2A]),
        (0xF3, &[0x43, 0x70, 0x72, 0x36, 0x37, 0x6f]),
        (0xED, &[0x18, 0x0B]),
        (0xAE, &[0x77]),
        (0xCD, &[0x63]),
        (0x70, &[0x07, 0x07, 0x04, 0x0E, 0x0F, 0x09, 0x07, 0x08, 0x03]),
        (0xE8, &[0x34]),
        (
            0x62,
            &[
                0x18, 0x0D, 0x71, 0xED, 0x70, 0x70, 0x18, 0x0F, 0x71, 0xEF, 0x70, 0x70,
            ],
        ),
        (
            0x63,
            &[
                0x18, 0x11, 0x71, 0xF1, 0x70, 0x70, 0x18, 0x13, 0x71, 0xF3, 0x70, 0x70,
            ],
        ),
        (0x64, &[0x28, 0x29, 0xF1, 0x01, 0xF1, 0x00, 0x07]),
        (
            0x66,
            &[0x3C, 0x00, 0xCD, 0x67, 0x45, 0x45, 0x10, 0x00, 0x00, 0x00],
        ),
        (
            0x67,
            &[0x00, 0x3C, 0x00, 0x00, 0x00, 0x01, 0x54, 0x10, 0x32, 0x98],
        ),
        (0x74, &[0x10, 0x85, 0x80, 0x00, 0x00, 0x4E, 0x00]),
        (0x98, &[0x3e, 0x07]),
    ];

    #[test]
    fn gc9a01_init_sequence_matches_datasheet() {
        let (rec, _) = init(&GC9A01, Rotation::Deg0);
        assert_eq!(rec.ops(), expected_cmds(&GC9A01, Rotation::Deg0, &MIPIDSI));
    }

    #[test]
    fn gc9a01_rotation_offsets_all_four() {
        assert_eq!(origin_windows(&GC9A01), [win10(0, 0); 4]);
    }

    #[test]
    fn gc9a01_async_same_bytes() {
        let (a, _) = init(&GC9A01, Rotation::Deg180);
        let b = Recorder::new();
        let _d = block_on(new_async(
            b.spi(),
            b.quiet_pin("dc"),
            Some(b.pin("rst")),
            Rotation::Deg180,
            &mut b.delay(),
        ))
        .unwrap();
        assert_eq!(a.ops(), b.ops());
    }
}
