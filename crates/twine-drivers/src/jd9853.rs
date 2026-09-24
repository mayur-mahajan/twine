//! JD9853 IPS controller with a 172 × 320 panel (1.47" modules, e.g. Waveshare
//! ESP32-C6-Touch-LCD-1.47).
//!
//! | | |
//! |-|-|
//! | Reference | Waveshare ESP32-C6-Touch-LCD-1.47 demo, ESP-IDF component `esp_lcd_jd9853` (Apache-2.0) |
//! | Max SPI clock | the vendor demo runs it at 80 MHz |
//! | Format | `COLMOD 0x55` → `Rgb565Swapped` |
//! | Quirks | IPS glass: colour inversion **on**; RGB order; the 172 visible columns are columns 34–205 of a 240 × 320 controller memory (offsets) |
//!
//! | Spec | Visible | Offset (`Deg0`, `Deg180`) | Offset (`Deg90`, `Deg270`) |
//! |------|---------|---------------------------|----------------------------|
//! | [`JD9853_172X320`] | 172 × 320 | (34, 0) | (0, 34) |
//!
//! The init table is the vendor's (`vendor_specific_init_default` of `esp_lcd_jd9853.c` in
//! `ESP32-C6-Touch-LCD-1.47-Demo.zip`, `ESP-IDF/01_factory/components/esp_lcd_jd9853`, from
//! <https://www.waveshare.com/wiki/ESP32-C6-Touch-LCD-1.47>; SPDX `Apache-2.0`, Copyright
//! Espressif Systems; the Arduino demo's `lcd_reg_init` sends the same bytes). It unlocks the
//! vendor registers (`0xDF 0x98 0x53`), sets power, timing and gamma, and switches register
//! pages with `0xDE`. Left out are the commands the generic init and each flush send
//! themselves: `COLMOD`, `CASET`/`RASET` and `DISPON`. The column offset 34 is the demo's
//! `esp_lcd_panel_set_gap(34, 0)` (`(0, 34)` for 90°/270°). The controller follows the usual
//! MIPI `MADCTL` bits (the Arduino demo drives it as an ST7789), so rotation uses
//! [`standard_madctl`].
//!
//! # Wiring
//!
//! | Panel pin | Driver argument |
//! |-----------|-----------------|
//! | `SCK`, `SDI`/`MOSI` | the bus of `spi`, an `embedded_hal(_async)::spi::SpiDevice` (use a DMA-capable async one to overlap transfers with rendering) |
//! | `CS` | the chip select owned by `spi` |
//! | `DC`/`RS` | `dc`, any `OutputPin` |
//! | `RESET` | `rst`: `Some(OutputPin)`, or `None` when it is tied to the MCU reset (a software reset is sent instead) |
//! | `LED`/`BL` | not driven by the driver: switch it (or PWM it) from your firmware |
//!
//! ```
//! use twine_drivers::jd9853::{self, JD9853_172X320};
//! use twine_drivers::testkit::Recorder;
//! use twine_hal::{DisplayDriver, Rotation};
//!
//! let rec = Recorder::new();
//! let lcd = jd9853::new(rec.spi(), rec.quiet_pin("dc"), Some(rec.pin("rst")), &JD9853_172X320, Rotation::Deg90, &mut rec.delay()).unwrap();
//! assert_eq!((lcd.info().width, lcd.info().height), (320, 172));
//! ```

use crate::mipi_dcs::{InitOp, Madctl, PanelSpec, cmd, standard_madctl};

/// JD9853 vendor init table (Waveshare `esp_lcd_jd9853.c`, `vendor_specific_init_default`,
/// Apache-2.0), without `COLMOD`, `CASET`, `RASET` and `DISPON`.
pub static JD9853_INIT: [InitOp; 29] = [
    InitOp::Cmd(cmd::SLPOUT, &[]),
    InitOp::DelayMs(120),
    InitOp::Cmd(0xDF, &[0x98, 0x53]), // vendor register unlock (sent twice by the vendor)
    InitOp::Cmd(0xDF, &[0x98, 0x53]),
    InitOp::Cmd(0xB2, &[0x23]),
    InitOp::Cmd(0xB7, &[0x00, 0x47, 0x00, 0x6F]),
    InitOp::Cmd(0xBB, &[0x1C, 0x1A, 0x55, 0x73, 0x63, 0xF0]),
    InitOp::Cmd(0xC0, &[0x44, 0xA4]),
    InitOp::Cmd(0xC1, &[0x16]),
    InitOp::Cmd(0xC3, &[0x7D, 0x07, 0x14, 0x06, 0xCF, 0x71, 0x72, 0x77]),
    // Frame rate (vendor note: 00 = 60 Hz, 06 = 57 Hz, 08 = 51 Hz; 320 lines).
    InitOp::Cmd(
        0xC4,
        &[
            0x00, 0x00, 0xA0, 0x79, 0x0B, 0x0A, 0x16, 0x79, 0x0B, 0x0A, 0x16, 0x82,
        ],
    ),
    // SET_R_GAMMA
    InitOp::Cmd(
        0xC8,
        &[
            0x3F, 0x32, 0x29, 0x29, 0x27, 0x2B, 0x27, 0x28, 0x28, 0x26, 0x25, 0x17, 0x12, 0x0D, 0x04, 0x00,
            0x3F, 0x32, 0x29, 0x29, 0x27, 0x2B, 0x27, 0x28, 0x28, 0x26, 0x25, 0x17, 0x12, 0x0D, 0x04, 0x00,
        ],
    ),
    InitOp::Cmd(0xD0, &[0x04, 0x06, 0x6B, 0x0F, 0x00]),
    InitOp::Cmd(0xD7, &[0x00, 0x30]),
    InitOp::Cmd(0xE6, &[0x14]),
    InitOp::Cmd(0xDE, &[0x01]), // register page 1
    InitOp::Cmd(0xB7, &[0x03, 0x13, 0xEF, 0x35, 0x35]),
    InitOp::Cmd(0xC1, &[0x14, 0x15, 0xC0]),
    InitOp::Cmd(0xC2, &[0x06, 0x3A]),
    InitOp::Cmd(0xC4, &[0x72, 0x12]),
    InitOp::Cmd(0xBE, &[0x00]),
    InitOp::Cmd(0xDE, &[0x02]), // register page 2
    InitOp::Cmd(0xE5, &[0x00, 0x02, 0x00]),
    InitOp::Cmd(0xE5, &[0x01, 0x02, 0x00]),
    InitOp::Cmd(0xDE, &[0x00]), // register page 0
    InitOp::Cmd(cmd::TEON, &[0x00]),
    InitOp::Cmd(0xDE, &[0x02]),
    InitOp::Cmd(0xE5, &[0x00, 0x02, 0x00]),
    InitOp::Cmd(0xDE, &[0x00]),
];

/// JD9853 with a 172 × 320 IPS panel: columns 34–205 of the 240 × 320 memory, RGB, inverted.
pub static JD9853_172X320: PanelSpec = PanelSpec {
    name: "JD9853 172x320",
    native_w: 172,
    native_h: 320,
    ram_w: 240,
    ram_h: 320,
    offset_x: 34,
    offset_y: 0,
    madctl: standard_madctl(Madctl::EMPTY),
    colmod: 0x55,
    align: 1,
    sw_rotation: false,
    invert: true,
    init: &JD9853_INIT,
};

crate::panel_macros::spi_panel!(Jd9853, AsyncJd9853, "JD9853", spec);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{BusOp, Recorder, block_on};
    use crate::panel_macros::test_util::{init, origin_windows, win10};
    use alloc::vec;
    use alloc::vec::Vec;
    use twine_core::Rect;
    use twine_hal::{DisplayDriver, DrawBufferMem, Rotation};

    /// Transcribed from Waveshare's `esp_lcd_jd9853.c` (`vendor_specific_init_default`), in
    /// order, minus `3A 05` (COLMOD), `2A`/`2B` (window) and `29` (DISPON).
    const VENDOR: [(u8, &[u8]); 28] = [
        (0x11, &[]),
        (0xDF, &[0x98, 0x53]),
        (0xDF, &[0x98, 0x53]),
        (0xB2, &[0x23]),
        (0xB7, &[0x00, 0x47, 0x00, 0x6F]),
        (0xBB, &[0x1C, 0x1A, 0x55, 0x73, 0x63, 0xF0]),
        (0xC0, &[0x44, 0xA4]),
        (0xC1, &[0x16]),
        (0xC3, &[0x7D, 0x07, 0x14, 0x06, 0xCF, 0x71, 0x72, 0x77]),
        (
            0xC4,
            &[
                0x00, 0x00, 0xA0, 0x79, 0x0B, 0x0A, 0x16, 0x79, 0x0B, 0x0A, 0x16, 0x82,
            ],
        ),
        (
            0xC8,
            &[
                0x3F, 0x32, 0x29, 0x29, 0x27, 0x2B, 0x27, 0x28, 0x28, 0x26, 0x25, 0x17, 0x12, 0x0D, 0x04,
                0x00, 0x3F, 0x32, 0x29, 0x29, 0x27, 0x2B, 0x27, 0x28, 0x28, 0x26, 0x25, 0x17, 0x12, 0x0D,
                0x04, 0x00,
            ],
        ),
        (0xD0, &[0x04, 0x06, 0x6B, 0x0F, 0x00]),
        (0xD7, &[0x00, 0x30]),
        (0xE6, &[0x14]),
        (0xDE, &[0x01]),
        (0xB7, &[0x03, 0x13, 0xEF, 0x35, 0x35]),
        (0xC1, &[0x14, 0x15, 0xC0]),
        (0xC2, &[0x06, 0x3A]),
        (0xC4, &[0x72, 0x12]),
        (0xBE, &[0x00]),
        (0xDE, &[0x02]),
        (0xE5, &[0x00, 0x02, 0x00]),
        (0xE5, &[0x01, 0x02, 0x00]),
        (0xDE, &[0x00]),
        (0x35, &[0x00]),
        (0xDE, &[0x02]),
        (0xE5, &[0x00, 0x02, 0x00]),
        (0xDE, &[0x00]),
    ];

    fn push_cmd(v: &mut Vec<BusOp>, c: u8, p: &[u8]) {
        v.push(BusOp::Cmd(c));
        if !p.is_empty() {
            v.push(BusOp::Data(p.to_vec()));
        }
    }

    #[test]
    fn jd9853_init_bytes_exact() {
        let (rec, _) = init(&JD9853_172X320, Rotation::Deg0);
        let mut want = vec![
            BusOp::Pin("rst", false),
            BusOp::DelayUs(10),
            BusOp::Pin("rst", true),
            BusOp::DelayUs(120_000),
        ];
        for (i, (c, p)) in VENDOR.iter().enumerate() {
            push_cmd(&mut want, *c, p);
            if i == 0 {
                want.push(BusOp::DelayUs(120_000)); // vendor: SLPOUT, 120 ms
            }
        }
        push_cmd(&mut want, 0x3A, &[0x55]); // COLMOD RGB565
        push_cmd(&mut want, 0x36, &[0x00]); // MADCTL: RGB, no mirroring
        push_cmd(&mut want, 0x21, &[]); // INVON (vendor demo: invert_color(true))
        push_cmd(&mut want, 0x11, &[]);
        want.push(BusOp::DelayUs(120_000));
        push_cmd(&mut want, 0x29, &[]);
        assert_eq!(rec.ops(), want);
    }

    #[test]
    fn jd9853_rotation_offsets_all_four() {
        // Waveshare demo: set_gap(34, 0) at 0°/180°, (0, 34) at 90°/270°.
        assert_eq!(
            origin_windows(&JD9853_172X320),
            [win10(34, 0), win10(0, 34), win10(34, 0), win10(0, 34)]
        );
        assert_eq!(JD9853_172X320.offset(Rotation::Deg0), (34, 0));
        assert_eq!(JD9853_172X320.offset(Rotation::Deg90), (0, 34));
        assert_eq!(JD9853_172X320.offset(Rotation::Deg180), (34, 0));
        assert_eq!(JD9853_172X320.offset(Rotation::Deg270), (0, 34));
    }

    #[test]
    fn jd9853_full_screen_window_hits_visible_columns() {
        let (rec, mut d) = init(&JD9853_172X320, Rotation::Deg0);
        let _ = rec.take_ops();
        let buf = DrawBufferMem::new(alloc::boxed::Box::leak(vec![0u8; 172 * 2].into_boxed_slice()));
        d.begin_flush(Rect::from_xywh(0, 319, 172, 1), buf).unwrap();
        let ops = rec.ops();
        // Columns 34..=205 (the vendor table's own CASET 0x22..0xCD), row 319.
        assert_eq!(
            ops[..4],
            [
                BusOp::Cmd(0x2A),
                BusOp::Data(vec![0x00, 0x22, 0x00, 0xCD]),
                BusOp::Cmd(0x2B),
                BusOp::Data(vec![0x01, 0x3F, 0x01, 0x3F]),
            ]
        );
    }

    #[test]
    fn jd9853_async_same_bytes() {
        let (a, _) = init(&JD9853_172X320, Rotation::Deg270);
        let b = Recorder::new();
        let d = block_on(new_async(
            b.spi(),
            b.quiet_pin("dc"),
            Some(b.pin("rst")),
            &JD9853_172X320,
            Rotation::Deg270,
            &mut b.delay(),
        ))
        .unwrap();
        assert_eq!(a.ops(), b.ops());
        let info = twine_hal::AsyncDisplayDriver::info(&d);
        assert_eq!((info.width, info.height), (320, 172));
    }
}
