//! SH8601 AMOLED controller on quad SPI (Waveshare ESP32-S3-Touch-AMOLED-1.8: 368 × 448).
//!
//! | | |
//! |-|-|
//! | Reference | Waveshare ESP32-S3-Touch-AMOLED-1.8 BSP (`Arduino_SH8601`, `Arduino_GFX`, BSD) |
//! | Interface | QSPI ([`QspiInterface`](crate::interface::QspiInterface)), up to 80 MHz on ESP32-S3 |
//! | Format | `COLMOD 0x55` → `Rgb565Swapped` |
//! | Quirks | no row/column exchange (software 90°/270°), even flush areas (`align = 2`), brightness via `0x51` (no backlight pin) |
//!
//! The init table is the vendor's (`sh8601_init_operations` of
//! <https://github.com/waveshareteam/ESP32-S3-Touch-AMOLED-1.8>): `SLPOUT` + 120 ms, `NORON`,
//! `WRCTRLD 0x28` (brightness control and dimming on), `WRDISBV 0xD0`, `WCE 0x00` (sunlight
//! enhancement off). `INVOFF`, `COLMOD` and `DISPON` come from the generic
//! [`MipiDcs`](crate::mipi_dcs::MipiDcs) sequence.
//!
//! # Wiring
//!
//! | Panel pin | Driver argument |
//! |-----------|-----------------|
//! | `SCLK`, `SDIO0`–`SDIO3`, `CS` | `bus`, your HAL's quad-SPI implementing [`QspiBus`](crate::interface::QspiBus) / `AsyncQspiBus` (on ESP32-S3: `twine-esp`) |
//! | `RESET` | `rst`: `Some(OutputPin)`, or `None` (software reset); some boards route it through an I/O expander that the firmware releases first |
//! | `TE` | optional, not used |
//!
//! There is no backlight: set the brightness with `set_brightness` (DCS `0x51`).
//!
//! ```
//! use twine_drivers::sh8601::{self, SH8601_368X448};
//! use twine_drivers::testkit::Recorder;
//! use twine_hal::{DisplayDriver, Rotation};
//!
//! let rec = Recorder::new();
//! let lcd = sh8601::new(rec.qspi(), Some(rec.pin("rst")), &SH8601_368X448, Rotation::Deg90, &mut rec.delay()).unwrap();
//! let info = lcd.info();
//! assert_eq!((info.width, info.height, info.align, info.hw_rotation), (448, 368, 2, false));
//! ```

use crate::mipi_dcs::{InitOp, Madctl, PanelSpec, cmd};

/// SH8601 vendor init table (Waveshare `sh8601_init_operations`).
pub static SH8601_INIT: [InitOp; 6] = [
    InitOp::Cmd(cmd::SLPOUT, &[]),
    InitOp::DelayMs(120),
    InitOp::Cmd(cmd::NORON, &[]),
    InitOp::Cmd(cmd::WRCTRLD, &[0x28]),
    InitOp::Cmd(cmd::WRDISBV, &[0xD0]),
    InitOp::Cmd(0x58, &[0x00]), // WCE: sunlight readability enhancement off
];

/// SH8601 1.8" 368 × 448 AMOLED (Waveshare ESP32-S3-Touch-AMOLED-1.8), RGB order, no offsets.
pub static SH8601_368X448: PanelSpec = PanelSpec {
    name: "SH8601 368x448",
    native_w: 368,
    native_h: 448,
    ram_w: 368,
    ram_h: 448,
    offset_x: 0,
    offset_y: 0,
    madctl: [Madctl::EMPTY; 4],
    colmod: 0x55,
    align: 2,
    sw_rotation: true,
    invert: false,
    init: &SH8601_INIT,
};

crate::panel_macros::qspi_panel!(Sh8601, AsyncSh8601, "SH8601");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mipi_dcs::DcsError;
    use crate::mock::{BusOp, Recorder, block_on};
    use crate::panel_macros::test_util::{expected_qspi_init, init_qspi};
    use twine_core::Rect;
    use twine_hal::{AsyncDisplayDriver, DisplayDriver, DrawBufferMem, Rotation};

    #[test]
    fn sh8601_init_bytes_exact() {
        let (rec, _) = init_qspi(&SH8601_368X448, Rotation::Deg0);
        let vendor: &[InitOp] = &[
            InitOp::Cmd(0x11, &[]),
            InitOp::DelayMs(120),
            InitOp::Cmd(0x13, &[]),
            InitOp::Cmd(0x53, &[0x28]),
            InitOp::Cmd(0x51, &[0xD0]),
            InitOp::Cmd(0x58, &[0x00]),
        ];
        assert_eq!(
            rec.ops(),
            expected_qspi_init(&SH8601_368X448, Rotation::Deg0, vendor)
        );
        // MADCTL stays 0 in every rotation (software rotation).
        let (rec, _) = init_qspi(&SH8601_368X448, Rotation::Deg270);
        assert!(rec.ops().contains(&BusOp::QspiCmd {
            instr: 0x02,
            addr: 0x3600,
            data: alloc::vec![0x00]
        }));
    }

    #[test]
    fn sh8601_flush_framing_and_alignment() {
        let (rec, mut d) = init_qspi(&SH8601_368X448, Rotation::Deg90);
        let _ = rec.take_ops();
        let buf = DrawBufferMem::new(alloc::boxed::Box::leak(
            alloc::vec![0u8; 4 * 2 * 2].into_boxed_slice(),
        ));
        // Native coordinates (software rotation): x up to 368.
        d.begin_flush(Rect::from_xywh(364, 2, 4, 2), buf).unwrap();
        let q = |c: u32, d: &[u8]| BusOp::QspiCmd {
            instr: 0x02,
            addr: c << 8,
            data: d.to_vec(),
        };
        assert_eq!(
            rec.ops(),
            [
                q(0x2A, &[0x01, 0x6C, 0x01, 0x6F]),
                q(0x2B, &[0x00, 0x02, 0x00, 0x03]),
                BusOp::QspiPixels {
                    instr: 0x32,
                    addr: 0x2C00,
                    len: 16
                },
            ]
        );
        assert!(d.poll_flush().is_some());
        let odd = DrawBufferMem::new(alloc::boxed::Box::leak(
            alloc::vec![0u8; 3 * 2 * 2].into_boxed_slice(),
        ));
        assert_eq!(
            d.begin_flush(Rect::from_xywh(1, 0, 3, 2), odd),
            Err(DcsError::BadArea)
        );
        d.set_brightness(0x40).unwrap();
        assert_eq!(rec.ops().last(), Some(&q(0x51, &[0x40])));
    }

    #[test]
    fn sh8601_async_matches_blocking() {
        let (a, _) = init_qspi(&SH8601_368X448, Rotation::Deg180);
        let b = Recorder::new();
        let mut d = block_on(new_async(
            b.qspi(),
            Some(b.pin("rst")),
            &SH8601_368X448,
            Rotation::Deg180,
            &mut b.delay(),
        ))
        .unwrap();
        assert_eq!(a.ops(), b.ops());
        assert_eq!(AsyncDisplayDriver::info(&d).width, 368);
        block_on(d.flush(Rect::from_xywh(0, 0, 2, 2), &[1; 8])).unwrap();
        assert_eq!(b.pixel_bytes(), [1; 8]);
    }
}
