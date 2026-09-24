//! ILI9488 320 × 480 TFT controller over SPI (18-bit colour only).
//!
//! | | |
//! |-|-|
//! | Datasheet | ILITEK ILI9488 V1.00 |
//! | Max SPI clock | 20 MHz write per datasheet (t<sub>scycw</sub> = 50 ns); modules often run at 40 MHz |
//! | Format | `COLMOD 0x66` (RGB666, 3 bytes/pixel) → [`ColorFormat::Rgb888`](twine_core::ColorFormat) |
//! | Quirks | **slow**: the SPI interface has no 16-bit mode, so every pixel costs 3 bytes (1.5× RGB565) |
//!
//! # Colour format
//!
//! The panel takes 3 bytes per pixel and uses the upper 6 bits of each. The engine renders
//! `Rgb888`, whose bytes are `B, G, R` in memory, and the driver sends them unchanged; the
//! panel's `MADCTL.BGR` bit swaps red and blue, so the spec **clears** it for the common modules
//! (whose glass needs `BGR` for `R, G, B` data). If red and blue appear swapped, use
//! `ILI9488.with_bgr(true)`.
//!
//! The application must enable `Rgb888` rendering (the `color-rgb888` feature of the `twine`
//! crate); the engine rejects a display whose format it cannot render.
//!
//! Init as mipidsi's `ili948x::init_common`: display function control `0xB6 [02 02 3B]`, `NORON`,
//! plus the generic sequence. Rotation uses [`standard_madctl`].
//!
//! ```
//! use twine_core::ColorFormat;
//! use twine_drivers::ili9488;
//! use twine_drivers::testkit::Recorder;
//! use twine_hal::{DisplayDriver, Rotation};
//!
//! let rec = Recorder::new();
//! let lcd = ili9488::new(rec.spi(), rec.quiet_pin("dc"), Some(rec.pin("rst")), Rotation::Deg90, &mut rec.delay()).unwrap();
//! assert_eq!(lcd.info().format, ColorFormat::Rgb888);
//! assert_eq!((lcd.info().width, lcd.info().height), (480, 320));
//! ```

use crate::mipi_dcs::{InitOp, Madctl, PanelSpec, cmd, standard_madctl};

static ILI9488_INIT: [InitOp; 2] = [
    InitOp::Cmd(0xB6, &[0x02, 0x02, 0x3B]), // Display function control
    InitOp::Cmd(cmd::NORON, &[]),
];

/// ILI9488 320 × 480, RGB666 over SPI, `BGR` bit clear (see the module docs).
pub static ILI9488: PanelSpec = PanelSpec {
    name: "ILI9488",
    native_w: 320,
    native_h: 480,
    ram_w: 320,
    ram_h: 480,
    offset_x: 0,
    offset_y: 0,
    madctl: standard_madctl(Madctl::EMPTY),
    colmod: 0x66,
    invert: false,
    init: &ILI9488_INIT,
};

crate::panel_macros::spi_panel!(Ili9488, AsyncIli9488, "ILI9488", ILI9488);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{BusOp, Recorder, block_on};
    use crate::panel_macros::test_util::{expected_cmds, init, origin_windows, win10};
    use alloc::vec;
    use alloc::vec::Vec;
    use twine_core::Rect;
    use twine_hal::{DisplayDriver, DrawBufferMem, Rotation};

    #[test]
    fn ili9488_uses_colmod_0x66() {
        let (rec, d) = init(&ILI9488, Rotation::Deg0);
        assert_eq!(
            rec.ops(),
            expected_cmds(
                &ILI9488,
                Rotation::Deg0,
                &[(0xB6, &[0x02, 0x02, 0x3B]), (0x13, &[])]
            )
        );
        assert!(
            rec.ops()
                .windows(2)
                .any(|w| w == [BusOp::Cmd(0x3A), BusOp::Data(vec![0x66])])
        );
        assert_eq!(d.info().format, twine_core::ColorFormat::Rgb888);
        assert_eq!(ILI9488.bytes_per_pixel(), 3);
        assert_eq!(origin_windows(&ILI9488), [win10(0, 0); 4]);
    }

    #[test]
    fn ili9488_pixel_bytes_are_rgb888() {
        let (rec, mut d) = init(&ILI9488, Rotation::Deg0);
        let _ = rec.take_ops();
        let px: Vec<u8> = (0u8..18).collect();
        let buf = DrawBufferMem::new(alloc::boxed::Box::leak(px.clone().into_boxed_slice()));
        d.begin_flush(Rect::from_xywh(0, 0, 3, 2), buf).unwrap();
        assert_eq!(rec.ops().last(), Some(&BusOp::Pixels(18)));
        assert_eq!(rec.pixel_bytes(), px);
        // A 2-byte-per-pixel sized buffer is too short.
        let buf = DrawBufferMem::new(alloc::boxed::Box::leak(vec![0u8; 12].into_boxed_slice()));
        assert!(d.begin_flush(Rect::from_xywh(0, 0, 3, 2), buf).is_err());
    }

    #[test]
    fn ili9488_async_same_bytes() {
        let (a, _) = init(&ILI9488, Rotation::Deg90);
        let b = Recorder::new();
        let _d = block_on(new_async(
            b.spi(),
            b.quiet_pin("dc"),
            Some(b.pin("rst")),
            Rotation::Deg90,
            &mut b.delay(),
        ))
        .unwrap();
        assert_eq!(a.ops(), b.ops());
    }
}
