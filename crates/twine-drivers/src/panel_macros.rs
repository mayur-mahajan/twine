//! Helpers shared by the panel modules.

/// Generates `new` (blocking) and `new_async` (feature `async`) SPI constructors plus the
/// `$alias` / `$async_alias` type aliases for a panel module.
///
/// With `spec` the constructors take the panel variant as an argument; otherwise they use the
/// given fixed spec.
macro_rules! spi_panel {
    (@aliases $alias:ident, $async_alias:ident, $model:literal) => {
        #[doc = concat!("A ", $model, " on 4-wire SPI (blocking): [`MipiDcs`](crate::mipi_dcs::MipiDcs) over [`SpiInterface`](crate::interface::SpiInterface).")]
        pub type $alias<SPI, DC, RST> =
            $crate::mipi_dcs::MipiDcs<$crate::interface::SpiInterface<SPI, DC>, RST>;

        #[doc = concat!("A ", $model, " on 4-wire SPI (async, feature `async`).")]
        #[cfg(feature = "async")]
        pub type $async_alias<SPI, DC, RST> =
            $crate::mipi_dcs::AsyncMipiDcs<$crate::interface::SpiInterface<SPI, DC>, RST>;
    };
    ($alias:ident, $async_alias:ident, $model:literal, spec) => {
        $crate::panel_macros::spi_panel!(@aliases $alias, $async_alias, $model);

        #[doc = concat!("Resets and initializes a ", $model, " variant `spec` on 4-wire SPI (blocking).")]
        pub fn new<SPI, DC, RST>(
            spi: SPI,
            dc: DC,
            rst: ::core::option::Option<RST>,
            spec: &'static $crate::mipi_dcs::PanelSpec,
            rotation: ::twine_hal::Rotation,
            delay: &mut impl ::embedded_hal::delay::DelayNs,
        ) -> ::core::result::Result<
            $alias<SPI, DC, RST>,
            $crate::mipi_dcs::DcsError<$crate::interface::InterfaceError<SPI::Error, DC::Error>>,
        >
        where
            SPI: ::embedded_hal::spi::SpiDevice,
            DC: ::embedded_hal::digital::OutputPin,
            RST: ::embedded_hal::digital::OutputPin,
        {
            $crate::mipi_dcs::MipiDcs::new(
                $crate::interface::SpiInterface::new(spi, dc),
                rst,
                spec,
                rotation,
                delay,
            )
        }

        #[doc = concat!("Resets and initializes a ", $model, " variant `spec` on 4-wire SPI (async, feature `async`).")]
        #[cfg(feature = "async")]
        pub async fn new_async<SPI, DC, RST>(
            spi: SPI,
            dc: DC,
            rst: ::core::option::Option<RST>,
            spec: &'static $crate::mipi_dcs::PanelSpec,
            rotation: ::twine_hal::Rotation,
            delay: &mut impl ::embedded_hal_async::delay::DelayNs,
        ) -> ::core::result::Result<
            $async_alias<SPI, DC, RST>,
            $crate::mipi_dcs::DcsError<$crate::interface::InterfaceError<SPI::Error, DC::Error>>,
        >
        where
            SPI: ::embedded_hal_async::spi::SpiDevice,
            DC: ::embedded_hal::digital::OutputPin,
            RST: ::embedded_hal::digital::OutputPin,
        {
            $crate::mipi_dcs::AsyncMipiDcs::new(
                $crate::interface::SpiInterface::new(spi, dc),
                rst,
                spec,
                rotation,
                delay,
            )
            .await
        }
    };
    ($alias:ident, $async_alias:ident, $model:literal, $spec:path) => {
        $crate::panel_macros::spi_panel!(@aliases $alias, $async_alias, $model);

        #[doc = concat!("Resets and initializes a ", $model, " on 4-wire SPI (blocking).")]
        pub fn new<SPI, DC, RST>(
            spi: SPI,
            dc: DC,
            rst: ::core::option::Option<RST>,
            rotation: ::twine_hal::Rotation,
            delay: &mut impl ::embedded_hal::delay::DelayNs,
        ) -> ::core::result::Result<
            $alias<SPI, DC, RST>,
            $crate::mipi_dcs::DcsError<$crate::interface::InterfaceError<SPI::Error, DC::Error>>,
        >
        where
            SPI: ::embedded_hal::spi::SpiDevice,
            DC: ::embedded_hal::digital::OutputPin,
            RST: ::embedded_hal::digital::OutputPin,
        {
            $crate::mipi_dcs::MipiDcs::new(
                $crate::interface::SpiInterface::new(spi, dc),
                rst,
                &$spec,
                rotation,
                delay,
            )
        }

        #[doc = concat!("Resets and initializes a ", $model, " on 4-wire SPI (async, feature `async`).")]
        #[cfg(feature = "async")]
        pub async fn new_async<SPI, DC, RST>(
            spi: SPI,
            dc: DC,
            rst: ::core::option::Option<RST>,
            rotation: ::twine_hal::Rotation,
            delay: &mut impl ::embedded_hal_async::delay::DelayNs,
        ) -> ::core::result::Result<
            $async_alias<SPI, DC, RST>,
            $crate::mipi_dcs::DcsError<$crate::interface::InterfaceError<SPI::Error, DC::Error>>,
        >
        where
            SPI: ::embedded_hal_async::spi::SpiDevice,
            DC: ::embedded_hal::digital::OutputPin,
            RST: ::embedded_hal::digital::OutputPin,
        {
            $crate::mipi_dcs::AsyncMipiDcs::new(
                $crate::interface::SpiInterface::new(spi, dc),
                rst,
                &$spec,
                rotation,
                delay,
            )
            .await
        }
    };
}

pub(crate) use spi_panel;

/// Test helpers for panel modules.
#[cfg(test)]
pub(crate) mod test_util {
    use alloc::vec::Vec;

    use twine_core::Rect;
    use twine_hal::{DisplayDriver, DrawBufferMem, Rotation};

    use crate::interface::SpiInterface;
    use crate::mipi_dcs::{MipiDcs, PanelSpec};
    use crate::mock::{BusOp, Recorder, RecordingPin, RecordingSpi};

    pub type Dut = MipiDcs<SpiInterface<RecordingSpi, RecordingPin>, RecordingPin>;

    /// Initializes `spec` on a recorder (with a reset pin).
    pub fn init(spec: &'static PanelSpec, rot: Rotation) -> (Recorder, Dut) {
        let rec = Recorder::new();
        let d = MipiDcs::new(
            SpiInterface::new(rec.spi(), rec.quiet_pin("dc")),
            Some(rec.pin("rst")),
            spec,
            rot,
            &mut rec.delay(),
        )
        .unwrap();
        (rec, d)
    }

    /// The expected init log for a spec's vendor table (after the hardware reset) followed by
    /// the common tail.
    pub fn expected_init(spec: &PanelSpec, rot: Rotation) -> Vec<BusOp> {
        use crate::mipi_dcs::{InitOp, cmd};
        let mut v = alloc::vec![
            BusOp::Pin("rst", false),
            BusOp::DelayUs(10),
            BusOp::Pin("rst", true),
            BusOp::DelayUs(120_000),
        ];
        for op in spec.init {
            match *op {
                InitOp::Cmd(c, p) => {
                    v.push(BusOp::Cmd(c));
                    if !p.is_empty() {
                        v.push(BusOp::Data(p.to_vec()));
                    }
                }
                InitOp::DelayMs(ms) => v.push(BusOp::DelayUs(ms * 1000)),
            }
        }
        v.extend([
            BusOp::Cmd(cmd::COLMOD),
            BusOp::Data(alloc::vec![spec.colmod]),
            BusOp::Cmd(cmd::MADCTL),
            BusOp::Data(alloc::vec![spec.madctl_for(rot).bits()]),
            BusOp::Cmd(if spec.invert { cmd::INVON } else { cmd::INVOFF }),
            BusOp::Cmd(cmd::SLPOUT),
            BusOp::DelayUs(120_000),
            BusOp::Cmd(cmd::DISPON),
        ]);
        v
    }

    /// The expected init log built from a hand-transcribed vendor command list (independent of
    /// the spec's table, so a transcription error in either shows up).
    pub fn expected_cmds(spec: &PanelSpec, rot: Rotation, cmds: &[(u8, &[u8])]) -> Vec<BusOp> {
        let mut v = expected_init(&PanelSpec { init: &[], ..*spec }, rot);
        let tail = v.split_off(4);
        for (c, p) in cmds {
            v.push(BusOp::Cmd(*c));
            if !p.is_empty() {
                v.push(BusOp::Data(p.to_vec()));
            }
        }
        v.extend(tail);
        v
    }

    /// `(CASET, RASET)` of a 10 × 10 flush at the origin for all four rotations.
    pub fn origin_windows(spec: &'static PanelSpec) -> [([u8; 4], [u8; 4]); 4] {
        [
            Rotation::Deg0,
            Rotation::Deg90,
            Rotation::Deg180,
            Rotation::Deg270,
        ]
        .map(|r| window_at_origin(spec, r, 10, 10))
    }

    /// `(CASET, RASET)` for a 10 × 10 area at logical offset `(ox, oy)`.
    pub fn win10(ox: u16, oy: u16) -> ([u8; 4], [u8; 4]) {
        let e = |s: u16| {
            let a = s.to_be_bytes();
            let b = (s + 9).to_be_bytes();
            [a[0], a[1], b[0], b[1]]
        };
        (e(ox), e(oy))
    }

    /// Flushes a `w × h` area at the origin and returns `(CASET, RASET)` parameters.
    pub fn window_at_origin(spec: &'static PanelSpec, rot: Rotation, w: i32, h: i32) -> ([u8; 4], [u8; 4]) {
        let (rec, mut d) = init(spec, rot);
        let _ = rec.take_ops();
        let len = (w * h) as usize * spec.bytes_per_pixel();
        let buf = DrawBufferMem::new(alloc::boxed::Box::leak(alloc::vec![0u8; len].into_boxed_slice()));
        d.begin_flush(Rect::from_xywh(0, 0, w, h), buf).unwrap();
        assert!(d.poll_flush().is_some());
        let ops = rec.ops();
        assert_eq!(ops.len(), 6, "{ops:?}");
        assert_eq!(ops[4], BusOp::Cmd(0x2C));
        assert_eq!(ops[5], BusOp::Pixels(len));
        let (BusOp::Data(c), BusOp::Data(r)) = (&ops[1], &ops[3]) else {
            panic!("{ops:?}")
        };
        (c.as_slice().try_into().unwrap(), r.as_slice().try_into().unwrap())
    }
}
