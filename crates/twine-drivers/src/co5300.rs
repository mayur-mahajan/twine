//! CO5300 AMOLED controller on quad SPI (Waveshare ESP32-S3-Touch-AMOLED-2.06: 410 × 502).
//!
//! | | |
//! |-|-|
//! | Reference | Waveshare ESP32-S3-Touch-AMOLED-2.06 BSP (`Arduino_CO5300`, `Arduino_GFX`, BSD) |
//! | Interface | QSPI ([`QspiInterface`](crate::interface::QspiInterface)), up to 80 MHz on ESP32-S3 |
//! | Format | `COLMOD 0x55` → `Rgb565Swapped` |
//! | Quirks | no row/column exchange (software 90°/270°), even flush areas (`align = 2`), visible area starts at column 22 on the 2.06" glass, brightness via `0x51` |
//!
//! The init table is the vendor's (`co5300_init_operations` of
//! <https://github.com/waveshareteam/ESP32-S3-Touch-AMOLED-2.06>): `SLPOUT` + 120 ms, page 0
//! (`0xFE 0x00`), SPI mode control `0xC4 0x80`, `WRCTRLD 0x20`, high-brightness-mode level
//! `0x63 0xFF`, `WRDISBV 0xD0`, `WCE 0x00`. The column offset of 22 is the vendor example's
//! `col_offset1`.
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
//! use twine_drivers::co5300::{self, CO5300_410X502};
//! use twine_drivers::testkit::Recorder;
//! use twine_hal::{DisplayDriver, Rotation};
//!
//! let rec = Recorder::new();
//! let lcd = co5300::new(rec.qspi(), Some(rec.pin("rst")), &CO5300_410X502, Rotation::Deg0, &mut rec.delay()).unwrap();
//! assert_eq!((lcd.info().width, lcd.info().height), (410, 502));
//! ```

use crate::mipi_dcs::{InitOp, Madctl, PanelSpec, cmd};

/// CO5300 vendor init table (Waveshare `co5300_init_operations`).
pub static CO5300_INIT: [InitOp; 8] = [
    InitOp::Cmd(cmd::SLPOUT, &[]),
    InitOp::DelayMs(120),
    InitOp::Cmd(0xFE, &[0x00]), // command page 0
    InitOp::Cmd(0xC4, &[0x80]), // SPI mode control
    InitOp::Cmd(cmd::WRCTRLD, &[0x20]),
    InitOp::Cmd(0x63, &[0xFF]), // brightness in high-brightness mode
    InitOp::Cmd(cmd::WRDISBV, &[0xD0]),
    InitOp::Cmd(0x58, &[0x00]), // WCE: sunlight readability enhancement off
];

/// CO5300 2.06" 410 × 502 AMOLED (Waveshare ESP32-S3-Touch-AMOLED-2.06): columns 22–431 of
/// the controller memory, RGB order.
pub static CO5300_410X502: PanelSpec = PanelSpec {
    name: "CO5300 410x502",
    native_w: 410,
    native_h: 502,
    ram_w: 454,
    ram_h: 502,
    offset_x: 22,
    offset_y: 0,
    madctl: [Madctl::EMPTY; 4],
    colmod: 0x55,
    align: 2,
    sw_rotation: true,
    invert: false,
    init: &CO5300_INIT,
};

crate::panel_macros::qspi_panel!(Co5300, AsyncCo5300, "CO5300");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{BusOp, Recorder, block_on};
    use crate::panel_macros::test_util::{expected_qspi_init, init_qspi};
    use twine_core::Rect;
    use twine_hal::{DisplayDriver, DrawBufferMem, Rotation};

    #[test]
    fn co5300_init_bytes_exact() {
        let (rec, _) = init_qspi(&CO5300_410X502, Rotation::Deg0);
        let vendor: &[InitOp] = &[
            InitOp::Cmd(0x11, &[]),
            InitOp::DelayMs(120),
            InitOp::Cmd(0xFE, &[0x00]),
            InitOp::Cmd(0xC4, &[0x80]),
            InitOp::Cmd(0x53, &[0x20]),
            InitOp::Cmd(0x63, &[0xFF]),
            InitOp::Cmd(0x51, &[0xD0]),
            InitOp::Cmd(0x58, &[0x00]),
        ];
        assert_eq!(
            rec.ops(),
            expected_qspi_init(&CO5300_410X502, Rotation::Deg0, vendor)
        );
    }

    #[test]
    fn co5300_window_has_column_offset() {
        let (rec, mut d) = init_qspi(&CO5300_410X502, Rotation::Deg0);
        let _ = rec.take_ops();
        let buf = DrawBufferMem::new(alloc::boxed::Box::leak(
            alloc::vec![0u8; 410 * 2 * 2].into_boxed_slice(),
        ));
        d.begin_flush(Rect::from_xywh(0, 500, 410, 2), buf).unwrap();
        let ops = rec.ops();
        assert_eq!(
            ops[0],
            BusOp::QspiCmd {
                instr: 0x02,
                addr: 0x2A00,
                data: alloc::vec![0x00, 22, 0x01, 0xAF]
            }
        );
        assert_eq!(
            ops[1],
            BusOp::QspiCmd {
                instr: 0x02,
                addr: 0x2B00,
                data: alloc::vec![0x01, 0xF4, 0x01, 0xF5]
            }
        );
    }

    #[test]
    fn co5300_async_matches_blocking() {
        let (a, _) = init_qspi(&CO5300_410X502, Rotation::Deg90);
        let b = Recorder::new();
        let d = block_on(new_async(
            b.qspi(),
            Some(b.pin("rst")),
            &CO5300_410X502,
            Rotation::Deg90,
            &mut b.delay(),
        ))
        .unwrap();
        assert_eq!(a.ops(), b.ops());
        let info = twine_hal::AsyncDisplayDriver::info(&d);
        assert_eq!((info.width, info.height, info.hw_rotation), (502, 410, false));
    }
}
