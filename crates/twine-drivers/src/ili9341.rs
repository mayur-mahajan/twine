//! ILI9341 240 × 320 TFT controller (the common 2.4"/2.8"/3.2" SPI modules).
//!
//! | | |
//! |-|-|
//! | Datasheet | ILITEK ILI9341 V1.11 |
//! | Max SPI clock | 10 MHz write per datasheet (t<sub>scycw</sub> = 100 ns); modules routinely run at 40–62.5 MHz |
//! | Format | `COLMOD 0x55` (RGB565) → [`ColorFormat::Rgb565Swapped`](twine_core::ColorFormat) |
//! | Quirks | BGR glass (`MADCTL.BGR`); mirrored columns at `Deg0` (Adafruit mapping), no offsets, no inversion |
//!
//! The init table is the one of `Adafruit_ILI9341` (`initcmd`, BSD license,
//! <https://github.com/adafruit/Adafruit_ILI9341>), minus the commands the generic
//! [`MipiDcs`](crate::mipi_dcs::MipiDcs) init sends itself (`MADCTL`, `COLMOD`, `SLPOUT`,
//! `DISPON`). Every command was cross-checked against the ILI9341 datasheet V1.11 (register
//! names and parameter counts noted per entry; `0xEF` is undocumented but required by many
//! panels).
//!
//! # Rotation
//!
//! `MADCTL` per rotation (Adafruit's `setRotation` values, with 90° and 270° exchanged so that
//! `Deg90` turns the picture like the engine's software rotation: the module is held turned
//! 90° clockwise): `Deg0 = MX|BGR` (240 × 320 portrait), `Deg90 = MX|MY|MV|BGR` (320 × 240
//! landscape), `Deg180 = MY|BGR`, `Deg270 = MV|BGR`.
//!
//! # Frame rate and tearing
//!
//! `FRMCTR1 = [0x00, 0x18]` selects 79 Hz (datasheet table for `DIVA = 0`, `RTNA = 0x18`). SPI
//! panels update their memory while scanning it out, so a flush that crosses the scan line
//! shows a diagonal shear (tearing). Synchronizing (`DisplayConfig::vsync`) needs the panel's
//! TE pin, which the common modules do not expose; without it tearing is expected and harmless.
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
//! use twine_drivers::ili9341;
//! use twine_drivers::testkit::Recorder;
//! use twine_hal::{DisplayDriver, Rotation};
//!
//! let rec = Recorder::new();
//! let lcd = ili9341::new(rec.spi(), rec.quiet_pin("dc"), Some(rec.pin("rst")), Rotation::Deg90, &mut rec.delay()).unwrap();
//! assert_eq!((lcd.info().width, lcd.info().height), (320, 240));
//! ```

use crate::mipi_dcs::{InitOp, Madctl, PanelSpec};

/// Vendor init table (`Adafruit_ILI9341` `initcmd`), checked against the ILI9341 datasheet V1.11.
pub static ILI9341_INIT: [InitOp; 18] = [
    InitOp::Cmd(0xEF, &[0x03, 0x80, 0x02]), // undocumented (Adafruit/ILITEK sample code) ✓ copied
    InitOp::Cmd(0xCF, &[0x00, 0xC1, 0x30]), // Power control B (3 params) ✓
    InitOp::Cmd(0xED, &[0x64, 0x03, 0x12, 0x81]), // Power on sequence control (4 params) ✓
    InitOp::Cmd(0xE8, &[0x85, 0x00, 0x78]), // Driver timing control A (3 params) ✓
    InitOp::Cmd(0xCB, &[0x39, 0x2C, 0x00, 0x34, 0x02]), // Power control A (5 params) ✓
    InitOp::Cmd(0xF7, &[0x20]),             // Pump ratio control: DDVDH = 2×VCI ✓
    InitOp::Cmd(0xEA, &[0x00, 0x00]),       // Driver timing control B (2 params) ✓
    InitOp::Cmd(0xC0, &[0x23]),             // PWCTR1: GVDD = 4.60 V ✓
    InitOp::Cmd(0xC1, &[0x10]),             // PWCTR2: step-up factor ✓
    InitOp::Cmd(0xC5, &[0x3E, 0x28]),       // VMCTR1: VCOMH 4.25 V, VCOML −1.5 V ✓
    InitOp::Cmd(0xC7, &[0x86]),             // VMCTR2: VCOM offset ✓
    InitOp::Cmd(0x37, &[0x00, 0x00]),       // VSCRSADD: scroll start 0 ( has 2 params; Adafruit sends 1) ✓
    InitOp::Cmd(0xB1, &[0x00, 0x18]),       // FRMCTR1: DIVA = 0, RTNA = 0x18 → 79 Hz ✓
    InitOp::Cmd(0xB6, &[0x08, 0x82, 0x27]), // DFUNCTR: display function control, 320 lines ✓
    InitOp::Cmd(0xF2, &[0x00]),             // Enable 3G: disabled ✓
    InitOp::Cmd(0x26, &[0x01]),             // GAMSET: gamma curve 1 ✓
    InitOp::Cmd(
        0xE0, // PGAMCTRL: positive gamma correction (15 params) ✓
        &[
            0x0F, 0x31, 0x2B, 0x0C, 0x0E, 0x08, 0x4E, 0xF1, 0x37, 0x07, 0x10, 0x03, 0x0E, 0x09, 0x00,
        ],
    ),
    InitOp::Cmd(
        0xE1, // NGAMCTRL: negative gamma correction (15 params) ✓
        &[
            0x00, 0x0E, 0x14, 0x03, 0x11, 0x07, 0x31, 0xC1, 0x48, 0x08, 0x0F, 0x0C, 0x31, 0x36, 0x0F,
        ],
    ),
];

/// The ILI9341 panel: 240 × 320, BGR, Adafruit rotation mapping, RGB565.
pub static ILI9341: PanelSpec = PanelSpec {
    name: "ILI9341",
    native_w: 240,
    native_h: 320,
    ram_w: 240,
    ram_h: 320,
    offset_x: 0,
    offset_y: 0,
    madctl: [
        Madctl::MX.union(Madctl::BGR),
        Madctl::MX.union(Madctl::MY).union(Madctl::MV).union(Madctl::BGR),
        Madctl::MY.union(Madctl::BGR),
        Madctl::MV.union(Madctl::BGR),
    ],
    colmod: 0x55,
    align: 1,
    sw_rotation: false,
    invert: false,
    init: &ILI9341_INIT,
};

crate::panel_macros::spi_panel!(Ili9341, AsyncIli9341, "ILI9341", ILI9341);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{Recorder, block_on, format_ops};
    use crate::panel_macros::test_util::{expected_init, init, window_at_origin};
    use twine_core::ColorFormat;
    use twine_hal::{AsyncDisplayDriver, DisplayDriver, Rotation};

    const FIXTURE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/ili9341_init.txt"
    ));

    #[test]
    fn ili9341_init_bytes_exact() {
        let rec = Recorder::new();
        let _lcd = new(
            rec.spi(),
            rec.quiet_pin("dc"),
            Some(rec.pin("rst")),
            Rotation::Deg90,
            &mut rec.delay(),
        )
        .unwrap();
        let log = format_ops(&rec.ops());
        assert_eq!(
            log, FIXTURE,
            "init log differs from tests/fixtures/ili9341_init.txt:\n{log}"
        );
        assert_eq!(rec.ops(), expected_init(&ILI9341, Rotation::Deg90));
    }

    #[test]
    fn ili9341_rotation_info() {
        for (rot, size) in [
            (Rotation::Deg0, (240, 320)),
            (Rotation::Deg90, (320, 240)),
            (Rotation::Deg180, (240, 320)),
            (Rotation::Deg270, (320, 240)),
        ] {
            let (_, d) = init(&ILI9341, rot);
            let info = d.info();
            assert_eq!((info.width, info.height), size);
            assert_eq!(info.format, ColorFormat::Rgb565Swapped);
            assert!(info.hw_rotation);
            assert_eq!(ILI9341.offset(rot), (0, 0));
        }
        assert_eq!(
            window_at_origin(&ILI9341, Rotation::Deg90, 320, 1),
            ([0, 0, 0x01, 0x3F], [0, 0, 0, 0])
        );
    }

    #[test]
    fn ili9341_async_matches_blocking() {
        let a = Recorder::new();
        let mut lcd = new(
            a.spi(),
            a.quiet_pin("dc"),
            Some(a.pin("rst")),
            Rotation::Deg270,
            &mut a.delay(),
        )
        .unwrap();
        let b = Recorder::new();
        let mut alcd = block_on(new_async(
            b.spi(),
            b.quiet_pin("dc"),
            Some(b.pin("rst")),
            Rotation::Deg270,
            &mut b.delay(),
        ))
        .unwrap();
        assert_eq!(a.ops(), b.ops());
        assert_eq!(lcd.info(), AsyncDisplayDriver::info(&alcd));

        let px: &'static mut [u8] = alloc::boxed::Box::leak(alloc::vec![7u8; 2 * 3 * 2].into_boxed_slice());
        let data = px.to_vec();
        lcd.begin_flush(
            twine_core::Rect::from_xywh(5, 6, 2, 3),
            twine_hal::DrawBufferMem::new(px),
        )
        .unwrap();
        block_on(alcd.flush(twine_core::Rect::from_xywh(5, 6, 2, 3), &data)).unwrap();
        assert_eq!(a.ops(), b.ops());
        assert_eq!(a.pixel_bytes(), b.pixel_bytes());
    }
}
