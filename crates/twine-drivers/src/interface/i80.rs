//! Intel 8080 ("i80", MCU 8080) parallel interfaces bit-banged over GPIO:
//! [`I80Interface8`] and [`I80Interface16`].
//!
//! The controller latches the data lines on the **rising edge of WR** (MIPI DBI type B). DC
//! (also `RS` / `D/CX`) selects command (low) or data (high). Chip select and `RD` are not
//! driven: tie `CS` low and `RD` high (or drive them yourself around the driver's calls).
//!
//! Bit-banging is slow: roughly 1–3 MB/s depending on the MCU's GPIO speed (a 320×240 RGB565
//! frame is 150 KiB, so ≥ 50 ms). Only data lines whose level changes are written. Prefer SPI
//! with DMA, or a HAL's parallel peripheral (RP PIO, ESP32 `LCD_CAM`, STM32 FMC) wrapped in your
//! own [`DcsInterface`] implementation for speed.

use embedded_hal::digital::OutputPin;

use super::DcsInterface;

/// Shared bit-bang core over `N` data lines.
#[derive(Debug)]
struct Bus<D, WR, DC, const N: usize> {
    data: [D; N],
    wr: WR,
    dc: DC,
    /// Levels currently on the data lines (`None`: unknown, all lines are written).
    last: Option<u16>,
}

impl<D, WR, DC, const N: usize> Bus<D, WR, DC, N>
where
    D: OutputPin,
    WR: OutputPin<Error = D::Error>,
    DC: OutputPin<Error = D::Error>,
{
    fn strobe(&mut self, value: u16) -> Result<(), D::Error> {
        let changed = self.last.map_or(u16::MAX, |l| l ^ value);
        for (i, pin) in self.data.iter_mut().enumerate() {
            let bit = 1u16 << i;
            if changed & bit != 0 {
                pin.set_state((value & bit != 0).into())?;
            }
        }
        self.last = Some(value);
        self.wr.set_low()?;
        self.wr.set_high()
    }

    fn command(&mut self, cmd: u8, params: &[u8]) -> Result<(), D::Error> {
        self.dc.set_low()?;
        self.strobe(u16::from(cmd))?;
        if !params.is_empty() {
            self.dc.set_high()?;
            for &p in params {
                self.strobe(u16::from(p))?;
            }
        }
        Ok(())
    }
}

/// 8-bit i80 interface: `D0`…`D7` plus `WR` and `DC`; one WR strobe per byte.
///
/// All lines use GPIOs with a common error type (HALs usually offer a type-erased pin, e.g.
/// `embassy_rp::gpio::Output` or `esp_hal::gpio::Output`).
///
/// # Wiring (example: RP2040)
///
/// | Panel | GPIO |
/// |-------|------|
/// | `D0`…`D7` | GP0…GP7 → `data[0..8]` |
/// | `WR` | GP8 |
/// | `DC`/`RS` | GP9 |
/// | `CS` | GND (or a GPIO held low) |
/// | `RD` | 3V3 |
///
/// ```
/// use twine_drivers::interface::{DcsInterface, I80Interface8};
/// use twine_drivers::testkit::{BusOp, Recorder};
///
/// let rec = Recorder::new();
/// let data = ["d0", "d1", "d2", "d3", "d4", "d5", "d6", "d7"].map(|n| rec.quiet_pin(n));
/// let mut iface = I80Interface8::new(data, rec.quiet_pin("wr"), rec.quiet_pin("dc"));
/// iface.command(0x3A, &[0x55]).unwrap();
/// assert_eq!(
///     rec.ops(),
///     [BusOp::Strobe { dc: false, value: 0x3A }, BusOp::Strobe { dc: true, value: 0x55 }]
/// );
/// ```
#[derive(Debug)]
pub struct I80Interface8<D, WR, DC>(Bus<D, WR, DC, 8>);

impl<D: OutputPin, WR, DC> I80Interface8<D, WR, DC>
where
    WR: OutputPin<Error = D::Error>,
    DC: OutputPin<Error = D::Error>,
{
    /// Creates the interface; `data[i]` drives `Di`. WR is driven high (idle) on first use.
    #[must_use]
    pub fn new(data: [D; 8], wr: WR, dc: DC) -> Self {
        Self(Bus {
            data,
            wr,
            dc,
            last: None,
        })
    }

    /// Returns the pins.
    #[must_use]
    pub fn release(self) -> ([D; 8], WR, DC) {
        (self.0.data, self.0.wr, self.0.dc)
    }
}

impl<D: OutputPin, WR, DC> DcsInterface for I80Interface8<D, WR, DC>
where
    WR: OutputPin<Error = D::Error>,
    DC: OutputPin<Error = D::Error>,
{
    type Error = D::Error;

    fn command(&mut self, cmd: u8, params: &[u8]) -> Result<(), D::Error> {
        self.0.command(cmd, params)
    }

    fn write_pixels(&mut self, data: &[u8]) -> Result<(), D::Error> {
        self.0.dc.set_high()?;
        for &b in data {
            self.0.strobe(u16::from(b))?;
        }
        Ok(())
    }
}

#[cfg(feature = "async")]
impl<D: OutputPin, WR, DC> super::AsyncDcsInterface for I80Interface8<D, WR, DC>
where
    WR: OutputPin<Error = D::Error>,
    DC: OutputPin<Error = D::Error>,
{
    type Error = D::Error;

    /// GPIO writes cannot be awaited: completes synchronously.
    fn command(&mut self, cmd: u8, params: &[u8]) -> impl Future<Output = Result<(), D::Error>> {
        core::future::ready(DcsInterface::command(self, cmd, params))
    }

    fn write_pixels(&mut self, data: &[u8]) -> impl Future<Output = Result<(), D::Error>> {
        core::future::ready(DcsInterface::write_pixels(self, data))
    }
}

/// 16-bit i80 interface: `D0`…`D15` plus `WR` and `DC`.
///
/// Commands and parameters use `D0`…`D7` (upper lines low); pixel data sends **one RGB565 pixel
/// per strobe**: the big-endian byte pair `[hi, lo]` of an `Rgb565Swapped` buffer becomes
/// `D15…D8 = hi`, `D7…D0 = lo`. A trailing odd byte is sent on `D7…D0`.
///
/// Wiring as [`I80Interface8`] with 16 data lines (`data[i]` → `Di`).
#[derive(Debug)]
pub struct I80Interface16<D, WR, DC>(Bus<D, WR, DC, 16>);

impl<D: OutputPin, WR, DC> I80Interface16<D, WR, DC>
where
    WR: OutputPin<Error = D::Error>,
    DC: OutputPin<Error = D::Error>,
{
    /// Creates the interface; `data[i]` drives `Di`.
    #[must_use]
    pub fn new(data: [D; 16], wr: WR, dc: DC) -> Self {
        Self(Bus {
            data,
            wr,
            dc,
            last: None,
        })
    }

    /// Returns the pins.
    #[must_use]
    pub fn release(self) -> ([D; 16], WR, DC) {
        (self.0.data, self.0.wr, self.0.dc)
    }
}

impl<D: OutputPin, WR, DC> DcsInterface for I80Interface16<D, WR, DC>
where
    WR: OutputPin<Error = D::Error>,
    DC: OutputPin<Error = D::Error>,
{
    type Error = D::Error;

    fn command(&mut self, cmd: u8, params: &[u8]) -> Result<(), D::Error> {
        self.0.command(cmd, params)
    }

    fn write_pixels(&mut self, data: &[u8]) -> Result<(), D::Error> {
        self.0.dc.set_high()?;
        let mut pairs = data.chunks_exact(2);
        for p in &mut pairs {
            self.0.strobe(u16::from_be_bytes([p[0], p[1]]))?;
        }
        if let [b] = pairs.remainder() {
            self.0.strobe(u16::from(*b))?;
        }
        Ok(())
    }
}

#[cfg(feature = "async")]
impl<D: OutputPin, WR, DC> super::AsyncDcsInterface for I80Interface16<D, WR, DC>
where
    WR: OutputPin<Error = D::Error>,
    DC: OutputPin<Error = D::Error>,
{
    type Error = D::Error;

    fn command(&mut self, cmd: u8, params: &[u8]) -> impl Future<Output = Result<(), D::Error>> {
        core::future::ready(DcsInterface::command(self, cmd, params))
    }

    fn write_pixels(&mut self, data: &[u8]) -> impl Future<Output = Result<(), D::Error>> {
        core::future::ready(DcsInterface::write_pixels(self, data))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{BusOp, Recorder, RecordingPin, block_on};
    use alloc::vec::Vec;

    const NAMES: [&str; 16] = [
        "d0", "d1", "d2", "d3", "d4", "d5", "d6", "d7", "d8", "d9", "d10", "d11", "d12", "d13", "d14", "d15",
    ];

    fn pins<const N: usize>(rec: &Recorder) -> [RecordingPin; N] {
        core::array::from_fn(|i| rec.quiet_pin(NAMES[i]))
    }

    fn strobes(ops: &[BusOp]) -> Vec<(bool, u16)> {
        ops.iter()
            .filter_map(|o| match o {
                BusOp::Strobe { dc, value } => Some((*dc, *value)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn i80_8bit_strobes_wr_per_byte() {
        let rec = Recorder::new();
        let mut i = I80Interface8::new(pins(&rec), rec.pin("wr"), rec.quiet_pin("dc"));
        i.write_pixels(&[0xA5, 0x5A, 0xFF]).unwrap();
        let ops = rec.ops();
        // Every byte: WR low, WR high (latch).
        let wr: Vec<_> = ops.iter().filter(|o| matches!(o, BusOp::Pin("wr", _))).collect();
        assert_eq!(wr.len(), 6);
        assert_eq!(strobes(&ops), [(true, 0xA5), (true, 0x5A), (true, 0xFF)]);
        let _ = i.release();
    }

    #[test]
    fn i80_16bit_strobe_per_pixel() {
        let rec = Recorder::new();
        let mut i = I80Interface16::new(pins(&rec), rec.quiet_pin("wr"), rec.quiet_pin("dc"));
        i.write_pixels(&[0xF8, 0x00, 0x07, 0xE0, 0x12]).unwrap();
        assert_eq!(
            strobes(&rec.ops()),
            [(true, 0xF800), (true, 0x07E0), (true, 0x0012)]
        );
        let _ = i.release();
    }

    #[test]
    fn i80_dc_low_for_command_high_for_data() {
        let rec = Recorder::new();
        let mut i = I80Interface16::new(pins(&rec), rec.quiet_pin("wr"), rec.quiet_pin("dc"));
        i.command(0x2A, &[0x00, 0xEF]).unwrap();
        assert_eq!(strobes(&rec.ops()), [(false, 0x2A), (true, 0x00), (true, 0xEF)]);
    }

    #[test]
    fn i80_async_same_as_blocking() {
        let a = Recorder::new();
        let mut i = I80Interface8::new(pins(&a), a.quiet_pin("wr"), a.quiet_pin("dc"));
        DcsInterface::command(&mut i, 0x2C, &[]).unwrap();
        DcsInterface::write_pixels(&mut i, &[1, 2]).unwrap();
        let b = Recorder::new();
        block_on(async {
            let mut i = I80Interface8::new(pins(&b), b.quiet_pin("wr"), b.quiet_pin("dc"));
            crate::interface::AsyncDcsInterface::command(&mut i, 0x2C, &[])
                .await
                .unwrap();
            crate::interface::AsyncDcsInterface::write_pixels(&mut i, &[1, 2])
                .await
                .unwrap();
        });
        assert_eq!(a.ops(), b.ops());
        assert_eq!(strobes(&a.ops()), [(false, 0x2C), (true, 1), (true, 2)]);
    }
}
