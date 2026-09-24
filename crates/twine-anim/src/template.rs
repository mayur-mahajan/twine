//! [`AnimTemplate`]: animation parameters without a target.

use twine_core::Duration;

use crate::Easing;

/// Animation parameters without start/end values or a target, `const`-constructible so they can
/// live in flash (referenced by the `Anim` style property; LVGL `LV_STYLE_ANIM` points to an
/// `lv_anim_t` used the same way: widgets such as spinners or scrolling labels read its timing
/// and path).
///
/// ```
/// use twine_anim::{AnimTemplate, Easing};
/// use twine_core::Duration;
///
/// static SPIN: AnimTemplate = AnimTemplate::new(Duration::ms(1000), Easing::EaseInOut).repeat(0);
/// assert_eq!(SPIN.repeat, 0); // infinite
/// assert_eq!(SPIN.duration.as_millis(), 1000);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct AnimTemplate {
    /// Duration of one pass.
    pub duration: Duration,
    /// Easing curve.
    pub easing: Easing,
    /// Number of passes; `0` = repeat forever.
    pub repeat: u16,
}

impl AnimTemplate {
    /// A template playing once with `duration` and `easing`.
    #[must_use]
    pub const fn new(duration: Duration, easing: Easing) -> Self {
        Self {
            duration,
            easing,
            repeat: 1,
        }
    }

    /// Sets the number of passes (`0` = infinite).
    #[must_use]
    pub const fn repeat(mut self, count: u16) -> Self {
        self.repeat = count;
        self
    }
}

impl Default for AnimTemplate {
    /// 500 ms, linear, once.
    fn default() -> Self {
        Self::new(Duration::ms(500), Easing::Linear)
    }
}
