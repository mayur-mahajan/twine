//! The thermostat: the measured temperature arrives as messages from a sensor task or
//! interrupt ([`SENSOR`]); the label glides to each new value; `−`/`+` set the target; the
//! state line follows a memo (it changes only when heating starts or stops).
//!
//! Between two sensor messages the UI does no work at all.

use twine::prelude::*;

/// A sensor reading.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SensorMsg {
    /// The measured temperature.
    pub celsius: f32,
}

/// The channel the sensor task (or interrupt) sends readings to:
/// `SENSOR.try_send(SensorMsg { celsius: 21.7 }).ok();` — it never blocks and wakes the UI.
pub static SENSOR: Channel<SensorMsg, 8> = Channel::new();

/// The thermostat application.
#[allow(clippy::cast_precision_loss)] // tenths of a degree: far below 2^23
///
/// ```
/// use twine_demos::thermostat::{SENSOR, SensorMsg, app};
/// use twine_testing::{TestUi, by_id};
///
/// let mut t = TestUi::new(320, 240).mount(app);
/// SENSOR.try_send(SensorMsg { celsius: 23.5 }).unwrap();
/// t.run_until_idle();
/// assert_eq!(t.find(by_id("current")).text(), "23.5 °C");
/// assert_eq!(t.find(by_id("state")).text(), "Idle");
/// ```
pub fn app(cx: Scope) -> impl View {
    let current = cx.signal(200i32); // tenths of a degree
    let target = cx.signal(215i32);
    cx.on_message(&SENSOR, move |m: SensorMsg| {
        current.set((m.celsius * 10.0) as i32);
    });
    let shown = cx.tween(move || current.get(), Duration::ms(400), Easing::EaseOut);
    let heating = cx.memo(move || current.get() < target.get());

    column((
        label(text!("{:.1} °C", shown.get() as f32 / 10.0))
            .font(&fonts::MONTSERRAT_20)
            .test_id("current"),
        row((
            button(label("-")).on_click(move || target.update(|t| *t -= 5)),
            label(text!("Target {:.1} °C", target.get() as f32 / 10.0)).test_id("target"),
            button(label("+")).on_click(move || target.update(|t| *t += 5)),
        ))
        .gap(8)
        .align_items(FlexAlign::Center),
        label(text!("{}", if heating.get() { "Heating" } else { "Idle" }))
            .text_color(move || {
                if heating.get() {
                    Color::hex(0xE5_39_35)
                } else {
                    Color::hex(0x75_75_75)
                }
            })
            .test_id("state"),
    ))
    .gap(12)
    .padding(16)
    .align_items(FlexAlign::Center)
    .size(Length::Pct(100), Length::Pct(100))
}
