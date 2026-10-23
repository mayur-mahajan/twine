//! The frame around every screen: the status bar on top (bed, patient, alarm message, link,
//! battery, clock) and the navigation bar at the bottom (the four screens and the alarm
//! silence key).
//!
//! **The alarm banner** sits in the middle of the status bar, where a clinician looks first.
//! It shows the highest-priority active alarm with its IEC 60601-1-8 priority marker (`!!!`
//! high, `!!` medium) and how many more are active. Its background flashes with the store's
//! flash phase for that priority — red at 2 Hz, yellow at 0.5 Hz; with audio paused it keeps
//! flashing (visual signals are never paused) and shows a muted-bell symbol. Without alarms the
//! space shows the demo disclaimer.
//!
//! **The silence key** pauses alarm audio for two minutes. A thin ring around its bell counts
//! the pause down (an `arc` bound to the remaining seconds, eased so it glides rather than
//! ticks), and its label shows `1:42`. Tapping it again resumes audio.
//!
//! Each piece is bound to the few signals it shows: the clock label re-formats once a second;
//! the banner re-runs on alarm changes and flash toggles; nothing here runs at 60 Hz.

use core::fmt;

use twine::prelude::*;
use twine::style::design::ColorValue;

use crate::store::{Alarm, Monitor, PAUSE_S, Priority, hhmm};
use crate::theme::{self, BARE, CAPTION};

/// The banner text of an alarm: `!!! SpO2 LOW 86 < 90`.
struct AlarmText(Option<Alarm>, usize);

impl fmt::Display for AlarmText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Some(a) = &self.0 else {
            return Ok(());
        };
        let marker = match a.cond.priority() {
            Priority::High => "!!!",
            Priority::Medium => "!!",
        };
        let op = if a.value > a.limit { '>' } else { '<' };
        write!(f, "{marker}  {} {} {op} {}", a.cond.message(), a.value, a.limit)?;
        if self.1 > 1 {
            write!(f, "   +{}", self.1 - 1)?;
        }
        Ok(())
    }
}

/// The silence key's label: `Silence`, or the audio pause left (`1:42`).
struct Countdown(u32);

impl fmt::Display for Countdown {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            0 => f.write_str("Silence"),
            s => write!(f, "{}:{:02}", s / 60, s % 60),
        }
    }
}

/// The status bar.
pub fn status_bar(m: Monitor) -> impl View {
    let top = move || m.top_alarm().map(|a| a.cond.priority());
    let flashing = move || match top() {
        Some(Priority::High) => m.flash_high().get(),
        Some(Priority::Medium) => m.flash_med().get(),
        None => false,
    };
    let banner_bg = move || match top() {
        Some(Priority::High) => theme::ALARM_HIGH,
        Some(Priority::Medium) => theme::ALARM_MED,
        None => theme::CHROME,
    };
    let banner_fg = move || match top() {
        Some(Priority::High) => theme::ON_ALARM_HIGH,
        Some(Priority::Medium) => theme::ON_ALARM_MED,
        None => design::ON_SURFACE_MUTED,
    };

    let banner = row((
        label(Symbol::Mute)
            .font(&fonts::MONTSERRAT_14)
            .text_color(move || ColorValue::from(banner_fg()))
            .hidden(move || m.paused().get() == 0 || top().is_none()),
        label(text!("{}", AlarmText(m.top_alarm(), m.alarms().with(Vec::len))))
            .font(&fonts::MONTSERRAT_14)
            .text_color(move || ColorValue::from(banner_fg()))
            .hidden(move || top().is_none())
            .test_id("banner"),
        label("DEMO  \u{2022}  NOT A MEDICAL DEVICE")
            .style(&CAPTION)
            .font(&fonts::MONTSERRAT_10)
            .hidden(move || top().is_some()),
    ))
    .flex_grow(1)
    .fill_height()
    .gap(6)
    .radius(4)
    .padding_x(8)
    .justify(MainAlign::Center)
    .align_items(CrossAlign::Center)
    .bg_color(move || ColorValue::from(banner_bg()))
    .bg_opacity(move || {
        if flashing() || top().is_none() {
            Opa::COVER
        } else {
            Opa::P30
        }
    });

    row((
        label("BED 12")
            .font(&fonts::MONTSERRAT_10)
            .text_color(design::ON_SURFACE)
            .bg_color(design::SURFACE_VARIANT)
            .bg_opacity(Opa::COVER)
            .radius(3)
            .padding_each(Insets::hor_ver(5, 2)),
        label(m.patient_name())
            .font(&fonts::MONTSERRAT_14)
            .text_color(design::ON_SURFACE)
            .long_mode(LongMode::Dots)
            .max_lines(1)
            .max_width(110)
            .test_id("patient"),
        banner,
        label(text!("{}  {}", Symbol::Wifi, Symbol::Battery3))
            .font(&fonts::MONTSERRAT_12)
            .text_color(design::ON_SURFACE_MUTED),
        label(text!(
            "{:02}:{:02}",
            hhmm(m.clock().get()).0,
            hhmm(m.clock().get()).1
        ))
        .font(&fonts::MONTSERRAT_16)
        .text_color(design::ON_SURFACE)
        .test_id("clock"),
    ))
    .fill_width()
    .height(26)
    .padding_each(Insets::new(8, 2, 8, 2))
    .gap(8)
    .align_items(CrossAlign::Center)
    .bg_color(theme::CHROME)
    .bg_opacity(Opa::COVER)
}

/// The navigation bar: the screens, then the silence key.
pub fn nav_bar(cx: Scope, m: Monitor, page: Signal<usize>) -> impl View {
    static PAGES: [&str; 4] = ["Monitor", "Alarms", "Trends", "Setup"];
    let tabs = PAGES
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let selected = move || page.get() == i;
            let badge = move || if i == 1 { m.alarms().with(Vec::len) } else { 0 };
            button(
                row((
                    label(*name).font(&fonts::MONTSERRAT_14),
                    label(text!("{}", badge()))
                        .font(&fonts::MONTSERRAT_10)
                        .text_color(theme::ON_ALARM_HIGH)
                        .bg_color(theme::ALARM_HIGH)
                        .bg_opacity(Opa::COVER)
                        .radius(Radius::Circle)
                        .padding_each(Insets::hor_ver(5, 1))
                        .hidden(move || badge() == 0),
                ))
                .gap(5)
                .align_items(CrossAlign::Center)
                .clickable(false),
            )
            .style(&NAV_TAB)
            .on_state(State::CHECKED, |s| s.style(&NAV_TAB_SELECTED))
            .on_state(State::PRESSED, |s| s.style(&NAV_TAB_PRESSED))
            .state(State::CHECKED, selected)
            .flex_grow(1)
            .fill_height()
            .on_click(move || page.set(i))
            .test_id(name)
        })
        .collect::<Vec<_>>();

    row((row(tabs).flex_grow(1).fill_height(), silence_key(cx, m)))
        .fill_width()
        .height(36)
        .padding(0)
        .gap(0)
        .bg_color(theme::CHROME)
        .bg_opacity(Opa::COVER)
}

/// The alarm silence (audio paused) key with its countdown ring.
fn silence_key(cx: Scope, m: Monitor) -> impl View {
    let paused = move || m.paused().get() > 0;
    // The ring glides from one second to the next instead of ticking (and fills smoothly
    // when the pause starts).
    let ring = cx.tween(
        move || m.paused().get() as i32 * 10,
        AnimSpec::new(Duration::ms(900)).ease_in_out(),
    );
    // The arc is an editable widget (it takes a two-way `Signal`); this one only displays,
    // so it gets its own signal following the tween.
    let shown = cx.signal(0);
    cx.effect(move || shown.set(ring.get()));
    button(
        row((
            stack((
                arc(shown)
                    .range(0..=PAUSE_S as i32 * 10)
                    .angles(Angle::deg(0), Angle::deg(360))
                    .bg_angles(Angle::deg(0), Angle::deg(360))
                    .rotation(Angle::deg(270))
                    .size(26, 26)
                    .flag(ObjFlags::CLICKABLE, false)
                    .arc_width(2)
                    .part(Part::Indicator, |s| s.arc_width(2).arc_color(theme::ALARM_MED))
                    .knob(false)
                    .arc_color(design::OUTLINE),
                label(move || if paused() { Symbol::Mute } else { Symbol::Bell }).font(&fonts::MONTSERRAT_12),
            ))
            .style(&BARE)
            .size(26, 26),
            label(text!("{}", Countdown(m.paused().get())))
                .font(&fonts::MONTSERRAT_14)
                .width(56),
        ))
        .gap(6)
        .align_items(CrossAlign::Center)
        .clickable(false),
    )
    .style(&NAV_TAB)
    .on_state(State::PRESSED, |s| s.style(&NAV_TAB_PRESSED))
    .text_color(move || {
        ColorValue::from(if paused() {
            theme::ALARM_MED
        } else {
            design::ON_SURFACE
        })
    })
    .width(124)
    .fill_height()
    .on_click(move || m.toggle_silence())
    .test_id("silence")
}

/// A navigation tab: flat, muted text, an accent bar on top when selected.
static NAV_TAB: Style = style! {
    bg_color: theme::CHROME,
    bg_opacity: Opa::COVER,
    text_color: design::ON_SURFACE_MUTED,
    radius: 0,
    shadow_width: 0,
    border_width: 3,
    border_side: BorderSide::TOP,
    border_color: theme::CHROME,
    padding: 0,
    flex_main_align: MainAlign::Center,
};
static NAV_TAB_SELECTED: Style = style! {
    text_color: design::ON_SURFACE,
    border_color: design::PRIMARY,
    bg_color: design::SURFACE,
};
static NAV_TAB_PRESSED: Style = style! { bg_color: design::SURFACE_VARIANT };
