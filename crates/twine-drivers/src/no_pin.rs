//! [`NoPin`]: a placeholder for optional pins that are not wired.

use core::convert::Infallible;

use embedded_hal::digital::{ErrorType, InputPin, OutputPin};

/// A pin that is not connected: writes are ignored, reads return low.
///
/// Use it to name the type of an absent optional pin, e.g. `None::<NoPin>` for a panel whose
/// reset line is tied high.
///
/// ```
/// use embedded_hal::digital::{InputPin, OutputPin};
/// use twine_drivers::NoPin;
///
/// let mut p = NoPin;
/// p.set_high().unwrap();
/// assert!(p.is_low().unwrap());
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct NoPin;

impl ErrorType for NoPin {
    type Error = Infallible;
}

impl OutputPin for NoPin {
    fn set_low(&mut self) -> Result<(), Infallible> {
        Ok(())
    }
    fn set_high(&mut self) -> Result<(), Infallible> {
        Ok(())
    }
}

impl InputPin for NoPin {
    fn is_high(&mut self) -> Result<bool, Infallible> {
        Ok(false)
    }
    fn is_low(&mut self) -> Result<bool, Infallible> {
        Ok(true)
    }
}
