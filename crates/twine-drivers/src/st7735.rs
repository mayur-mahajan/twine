//! ST7735R / ST7735S 128 × 160 TFT controller ("tab" variants of the common 1.8" and 0.96"
//! modules).
//!
//! | | |
//! |-|-|
//! | Datasheet | Sitronix ST7735R V2.1 / ST7735S V1.1 |
//! | Max SPI clock | 15 MHz write (t<sub>scycw</sub> = 66 ns); modules usually work at 20–40 MHz |
//! | Format | `COLMOD 0x05` (16-bit) → `Rgb565Swapped` |
//! | Quirks | the protective-film tab colour tells the variant: offsets and RGB/BGR differ |
//!
//! | Spec | Visible | Controller memory | Offset at `Deg0` | Colour order | Inversion |
//! |------|---------|-------------------|------------------|--------------|-----------|
//! | [`ST7735R_GREENTAB`] | 128 × 160 | 132 × 162 | (2, 1) | BGR | off |
//! | [`ST7735R_REDTAB`] | 128 × 160 | 128 × 160 | (0, 0) | BGR | off |
//! | [`ST7735R_BLACKTAB`] | 128 × 160 | 128 × 160 | (0, 0) | RGB | off |
//! | [`ST7735_80X160`] | 80 × 160 (0.96" IPS mini) | 132 × 162 | (26, 1) | BGR | on |
//!
//! Init tables and rotation mapping follow Adafruit-ST7735-Library (`Rcmd1`, `Rcmd3`,
//! `setRotation`; BSD license, <https://github.com/adafruit/Adafruit-ST7735-Library>), minus
//! the commands the generic init sends (`SWRESET`, `SLPOUT`, `INVOFF`, `MADCTL`, `COLMOD`,
//! `DISPON`) and the address commands of `Rcmd2*` (every flush sets its window). Rotation:
//! `Deg0 = MX|MY`, `Deg90 = MY|MV`, `Deg180 = —`, `Deg270 = MX|MV`.
//!
//! ```
//! use twine_drivers::st7735::{self, ST7735R_GREENTAB};
//! use twine_drivers::testkit::Recorder;
//! use twine_drivers::NoPin;
//! use twine_hal::{DisplayDriver, Rotation};
//!
//! let rec = Recorder::new();
//! let lcd = st7735::new(rec.spi(), rec.quiet_pin("dc"), None::<NoPin>, &ST7735R_GREENTAB, Rotation::Deg0, &mut rec.delay()).unwrap();
//! assert_eq!((lcd.info().width, lcd.info().height), (128, 160));
//! ```

use crate::mipi_dcs::{InitOp, Madctl, PanelSpec, cmd};

/// ST7735R vendor init (Adafruit `Rcmd1` frame rate / power settings, `Rcmd3` gamma, `NORON`).
static ST7735R_INIT: [InitOp; 14] = [
    InitOp::Cmd(0xB1, &[0x01, 0x2C, 0x2D]), // FRMCTR1: normal mode frame rate ✓
    InitOp::Cmd(0xB2, &[0x01, 0x2C, 0x2D]), // FRMCTR2: idle mode ✓
    InitOp::Cmd(0xB3, &[0x01, 0x2C, 0x2D, 0x01, 0x2C, 0x2D]), // FRMCTR3: partial mode ✓
    InitOp::Cmd(0xB4, &[0x07]),             // INVCTR: no line inversion ✓
    InitOp::Cmd(0xC0, &[0xA2, 0x02, 0x84]), // PWCTR1: −4.6 V, AUTO ✓
    InitOp::Cmd(0xC1, &[0xC5]),             // PWCTR2: VGH25 2.4, VGSEL −10 ✓
    InitOp::Cmd(0xC2, &[0x0A, 0x00]),       // PWCTR3: opamp current small ✓
    InitOp::Cmd(0xC3, &[0x8A, 0x2A]),       // PWCTR4: BCLK/2 ✓
    InitOp::Cmd(0xC4, &[0x8A, 0xEE]),       // PWCTR5 ✓
    InitOp::Cmd(0xC5, &[0x0E]),             // VMCTR1: VCOM ✓
    InitOp::Cmd(
        0xE0, // GMCTRP1: positive gamma (16 params) ✓
        &[
            0x02, 0x1C, 0x07, 0x12, 0x37, 0x32, 0x29, 0x2D, 0x29, 0x25, 0x2B, 0x39, 0x00, 0x01, 0x03, 0x10,
        ],
    ),
    InitOp::Cmd(
        0xE1, // GMCTRN1: negative gamma (16 params) ✓
        &[
            0x03, 0x1D, 0x07, 0x06, 0x2E, 0x2C, 0x29, 0x2D, 0x2E, 0x2E, 0x37, 0x3F, 0x00, 0x00, 0x02, 0x10,
        ],
    ),
    InitOp::Cmd(cmd::NORON, &[]),
    InitOp::DelayMs(10),
];

/// Adafruit's ST7735 rotation mapping (`setRotation`) plus `extra` (RGB/BGR).
const fn st7735_madctl(extra: Madctl) -> [Madctl; 4] {
    [
        extra.union(Madctl::MX).union(Madctl::MY),
        extra.union(Madctl::MY).union(Madctl::MV),
        extra,
        extra.union(Madctl::MX).union(Madctl::MV),
    ]
}

/// ST7735R "red tab" 1.8" 128 × 160: controller configured for 128 × 160, BGR.
pub static ST7735R_REDTAB: PanelSpec = PanelSpec {
    name: "ST7735R red tab",
    native_w: 128,
    native_h: 160,
    ram_w: 128,
    ram_h: 160,
    offset_x: 0,
    offset_y: 0,
    madctl: st7735_madctl(Madctl::BGR),
    colmod: 0x05,
    invert: false,
    init: &ST7735R_INIT,
};

/// ST7735R "black tab" 1.8" 128 × 160: as the red tab with RGB colour order.
pub static ST7735R_BLACKTAB: PanelSpec = ST7735R_REDTAB.with_bgr(false).with_name("ST7735R black tab");

/// ST7735R "green tab" 1.8" 128 × 160 inside 132 × 162 memory at (2, 1), BGR.
pub static ST7735R_GREENTAB: PanelSpec = PanelSpec {
    name: "ST7735R green tab",
    ram_w: 132,
    ram_h: 162,
    offset_x: 2,
    offset_y: 1,
    ..ST7735R_REDTAB
};

/// ST7735S 0.96" 80 × 160 IPS mini module (Adafruit "mini 160x80 plugin"): window at (26, 1),
/// BGR, inverted.
pub static ST7735_80X160: PanelSpec = PanelSpec {
    name: "ST7735 80x160",
    native_w: 80,
    native_h: 160,
    offset_x: 26,
    offset_y: 1,
    invert: true,
    ..ST7735R_GREENTAB
};

crate::panel_macros::spi_panel!(St7735, AsyncSt7735, "ST7735", spec);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{BusOp, Recorder, block_on};
    use crate::panel_macros::test_util::{expected_cmds, init, origin_windows, win10};
    use twine_core::ColorFormat;
    use twine_hal::{DisplayDriver, Rotation};

    /// Adafruit `Rcmd1` (without SWRESET/SLPOUT/INVOFF/MADCTL/COLMOD) and `Rcmd3`.
    const RCMD: [(u8, &[u8]); 13] = [
        (0xB1, &[0x01, 0x2C, 0x2D]),
        (0xB2, &[0x01, 0x2C, 0x2D]),
        (0xB3, &[0x01, 0x2C, 0x2D, 0x01, 0x2C, 0x2D]),
        (0xB4, &[0x07]),
        (0xC0, &[0xA2, 0x02, 0x84]),
        (0xC1, &[0xC5]),
        (0xC2, &[0x0A, 0x00]),
        (0xC3, &[0x8A, 0x2A]),
        (0xC4, &[0x8A, 0xEE]),
        (0xC5, &[0x0E]),
        (
            0xE0,
            &[
                0x02, 0x1c, 0x07, 0x12, 0x37, 0x32, 0x29, 0x2d, 0x29, 0x25, 0x2B, 0x39, 0x00, 0x01, 0x03,
                0x10,
            ],
        ),
        (
            0xE1,
            &[
                0x03, 0x1d, 0x07, 0x06, 0x2E, 0x2C, 0x29, 0x2D, 0x2E, 0x2E, 0x37, 0x3F, 0x00, 0x00, 0x02,
                0x10,
            ],
        ),
        (0x13, &[]),
    ];

    #[test]
    fn st7735_init_sequence_matches_datasheet() {
        for spec in [
            &ST7735R_REDTAB,
            &ST7735R_BLACKTAB,
            &ST7735R_GREENTAB,
            &ST7735_80X160,
        ] {
            let (rec, d) = init(spec, Rotation::Deg0);
            let mut expected = expected_cmds(spec, Rotation::Deg0, &RCMD);
            // NORON is followed by 10 ms (Adafruit `Rcmd3`).
            let pos = expected.iter().position(|o| *o == BusOp::Cmd(0x13)).unwrap();
            expected.insert(pos + 1, BusOp::DelayUs(10_000));
            assert_eq!(rec.ops(), expected, "{}", spec.name);
            assert_eq!(d.info().format, ColorFormat::Rgb565Swapped);
        }
        // Colour order and inversion per variant (Adafruit `setRotation` / `initR`).
        assert_eq!(ST7735R_GREENTAB.madctl[0].bits(), 0xC8);
        assert_eq!(ST7735R_BLACKTAB.madctl[0].bits(), 0xC0);
        assert!(ST7735_80X160.invert && !ST7735R_REDTAB.invert);
    }

    #[test]
    fn st7735_rotation_offsets_all_four() {
        // Green tab: Adafruit colstart 2 / rowstart 1, swapped for rotations 1 and 3.
        assert_eq!(
            origin_windows(&ST7735R_GREENTAB),
            [win10(2, 1), win10(1, 2), win10(2, 1), win10(1, 2)]
        );
        assert_eq!(origin_windows(&ST7735R_REDTAB), [win10(0, 0); 4]);
        assert_eq!(origin_windows(&ST7735R_BLACKTAB), [win10(0, 0); 4]);
        // Mini 80 × 160 plugin: colstart 26 / rowstart 1.
        assert_eq!(
            origin_windows(&ST7735_80X160),
            [win10(26, 1), win10(1, 26), win10(26, 1), win10(1, 26)]
        );
    }

    #[test]
    fn st7735_async_same_bytes() {
        let (a, _) = init(&ST7735_80X160, Rotation::Deg90);
        let b = Recorder::new();
        let d = block_on(new_async(
            b.spi(),
            b.quiet_pin("dc"),
            Some(b.pin("rst")),
            &ST7735_80X160,
            Rotation::Deg90,
            &mut b.delay(),
        ))
        .unwrap();
        assert_eq!(a.ops(), b.ops());
        let info = twine_hal::AsyncDisplayDriver::info(&d);
        assert_eq!((info.width, info.height), (160, 80));
    }
}
