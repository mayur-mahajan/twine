//! RM67162 AMOLED controller on quad SPI (`LilyGO` T-Display-S3 AMOLED 1.91": 240 × 536).
//!
//! | | |
//! |-|-|
//! | Reference | `LilyGO` `LilyGo-AMOLED-Series` (`rm67162_cmd`, MIT) |
//! | Interface | QSPI ([`QspiInterface`](crate::interface::QspiInterface)), vendor runs 75 MHz |
//! | Format | `COLMOD 0x55` → `Rgb565Swapped` |
//! | Quirks | hardware rotation with [`standard_madctl`], RGB order, even flush areas (`align = 2`), brightness via `0x51` |
//!
//! The init table is the vendor's QSPI table (`rm67162_cmd` of
//! <https://github.com/Xinyuan-LilyGO/LilyGo-AMOLED-Series>, `src/initSequence.cpp`):
//! `0xFE 0x00`, `SLPOUT` + 120 ms, page 5 `0x05 0x05` (ELVSS −3.95 V), page 1 `0x73 0x25`
//! (OVSS −4.0 V), page 0, then `WRDISBV`. The vendor switches the display on at brightness 0 and
//! raises it afterwards; this table sets `WRDISBV 0xD0` before the generic `DISPON` instead, and
//! leaves `MADCTL`/`COLMOD` to the generic sequence.
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
//! use twine_drivers::rm67162::{self, RM67162_240X536};
//! use twine_drivers::testkit::Recorder;
//! use twine_hal::{DisplayDriver, Rotation};
//!
//! let rec = Recorder::new();
//! let lcd = rm67162::new(rec.qspi(), Some(rec.pin("rst")), &RM67162_240X536, Rotation::Deg270, &mut rec.delay()).unwrap();
//! assert_eq!((lcd.info().width, lcd.info().height, lcd.info().hw_rotation), (536, 240, true));
//! ```

use crate::mipi_dcs::{InitOp, Madctl, PanelSpec, cmd, standard_madctl};

/// RM67162 vendor init table (`LilyGO` `rm67162_cmd`, see the module docs).
pub static RM67162_INIT: [InitOp; 9] = [
    InitOp::Cmd(0xFE, &[0x00]), // command page 0
    InitOp::Cmd(cmd::SLPOUT, &[]),
    InitOp::DelayMs(120),
    InitOp::Cmd(0xFE, &[0x05]), // page 5
    InitOp::Cmd(0x05, &[0x05]), // OVSS control: ELVSS −3.95 V
    InitOp::Cmd(0xFE, &[0x01]), // page 1
    InitOp::Cmd(0x73, &[0x25]), // OVSS voltage level −4.0 V
    InitOp::Cmd(0xFE, &[0x00]), // page 0
    InitOp::Cmd(cmd::WRDISBV, &[0xD0]),
];

/// RM67162 1.91" 240 × 536 AMOLED (`LilyGO` T-Display-S3 AMOLED), RGB order, no offsets.
pub static RM67162_240X536: PanelSpec = PanelSpec {
    name: "RM67162 240x536",
    native_w: 240,
    native_h: 536,
    ram_w: 240,
    ram_h: 536,
    offset_x: 0,
    offset_y: 0,
    madctl: standard_madctl(Madctl::EMPTY),
    colmod: 0x55,
    align: 2,
    sw_rotation: false,
    invert: false,
    init: &RM67162_INIT,
};

crate::panel_macros::qspi_panel!(Rm67162, AsyncRm67162, "RM67162");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{BusOp, Recorder, block_on};
    use crate::panel_macros::test_util::{expected_qspi_init, init_qspi};
    use twine_core::Rect;
    use twine_hal::{DisplayDriver, DrawBufferMem, Rotation};

    #[test]
    fn rm67162_init_bytes_exact() {
        let (rec, _) = init_qspi(&RM67162_240X536, Rotation::Deg270);
        let vendor: &[InitOp] = &[
            InitOp::Cmd(0xFE, &[0x00]),
            InitOp::Cmd(0x11, &[]),
            InitOp::DelayMs(120),
            InitOp::Cmd(0xFE, &[0x05]),
            InitOp::Cmd(0x05, &[0x05]),
            InitOp::Cmd(0xFE, &[0x01]),
            InitOp::Cmd(0x73, &[0x25]),
            InitOp::Cmd(0xFE, &[0x00]),
            InitOp::Cmd(0x51, &[0xD0]),
        ];
        assert_eq!(
            rec.ops(),
            expected_qspi_init(&RM67162_240X536, Rotation::Deg270, vendor)
        );
        // Deg270 = MX|MV: the vendor's default landscape (`0x60`).
        assert_eq!(RM67162_240X536.madctl_for(Rotation::Deg270).bits(), 0x60);
    }

    #[test]
    fn rm67162_landscape_window() {
        let (rec, mut d) = init_qspi(&RM67162_240X536, Rotation::Deg90);
        let _ = rec.take_ops();
        let buf = DrawBufferMem::new(alloc::boxed::Box::leak(
            alloc::vec![0u8; 536 * 2 * 2].into_boxed_slice(),
        ));
        d.begin_flush(Rect::from_xywh(0, 238, 536, 2), buf).unwrap();
        let ops = rec.ops();
        assert_eq!(
            ops[0],
            BusOp::QspiCmd {
                instr: 0x02,
                addr: 0x2A00,
                data: alloc::vec![0x00, 0x00, 0x02, 0x17]
            }
        );
        assert_eq!(
            ops[2],
            BusOp::QspiPixels {
                instr: 0x32,
                addr: 0x2C00,
                len: 536 * 2 * 2
            }
        );
    }

    #[test]
    fn rm67162_async_matches_blocking() {
        let (a, _) = init_qspi(&RM67162_240X536, Rotation::Deg0);
        let b = Recorder::new();
        let _d = block_on(new_async(
            b.qspi(),
            Some(b.pin("rst")),
            &RM67162_240X536,
            Rotation::Deg0,
            &mut b.delay(),
        ))
        .unwrap();
        assert_eq!(a.ops(), b.ops());
    }
}
