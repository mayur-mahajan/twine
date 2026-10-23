//! The Alarms screen: the active alarms, highest priority first, and the alarm history.
//!
//! Both lists are keyed lists (`for_each`) over the store's signals. When the alarm evaluation
//! (once a second) changes the list, only the rows that appeared, left or changed are touched:
//! a new alarm inserts one row, a value update re-formats one label, a cleared alarm removes
//! its row and the history row gets its end time. The rows of an unchanged list cost nothing.
//!
//! Each active alarm shows its priority as colour, as the IEC 60601-1-8 marker (`!!!` / `!!`)
//! and as text, so the priority never depends on colour alone.

use core::fmt;

use twine::prelude::*;
use twine::style::design::ColorValue;

use crate::components::panel;
use crate::store::{Alarm, LogEntry, Monitor, Priority, hhmm};
use crate::theme::{self, CAPTION};

/// `hh:mm:ss` of clock seconds.
struct Clock(u32);

impl fmt::Display for Clock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (h, m) = hhmm(self.0);
        write!(f, "{h:02}:{m:02}:{:02}", self.0 % 60)
    }
}

/// `m:ss` of a duration in seconds.
struct Span(u32);

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{:02}", self.0 / 60, self.0 % 60)
    }
}

/// How long an alarm lasted (`0:44`), or `active`.
struct Lasted(u32, Option<u32>);

impl fmt::Display for Lasted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.1 {
            Some(end) => Span(end.saturating_sub(self.0)).fmt(f),
            None => f.write_str("active"),
        }
    }
}

/// A plain coloured shape (the priority bar and dot): no card look.
static SWATCH: Style = style! { bg_opacity: Opa::COVER, border_width: 0, shadow_width: 0, padding: 0 };

/// The colour of a priority.
fn priority_color(p: Priority) -> twine::style::design::ColorElement {
    match p {
        Priority::High => theme::ALARM_HIGH,
        Priority::Medium => theme::ALARM_MED,
    }
}

/// One active alarm: a priority bar, the message, the value against its limit, the start.
fn active_row(alarm: ReadSignal<Alarm>) -> impl View {
    let first = alarm.get_untracked();
    let priority = first.cond.priority();
    let (marker, name) = match priority {
        Priority::High => ("!!!", "HIGH"),
        Priority::Medium => ("!!", "MEDIUM"),
    };
    panel((
        container(())
            .style(&SWATCH)
            .size(5, Length::pct(100))
            .bg_color(priority_color(priority))
            .radius(2),
        column((
            label(text!("{marker}  {}", first.cond.message())).font(&fonts::MONTSERRAT_16),
            label(text!("{name} PRIORITY  \u{2022}  since {}", Clock(first.since))).style(&CAPTION),
        ))
        .flex_grow(1)
        .gap(2),
        label(text!(
            "{} {} {}",
            alarm.with(|a| a.value),
            if alarm.with(|a| a.value > a.limit) {
                ">"
            } else {
                "<"
            },
            alarm.with(|a| a.limit)
        ))
        .font(&fonts::MONTSERRAT_22)
        .text_color(priority_color(priority)),
    ))
    .layout(Layout::row())
    .fill_width()
    .height(46)
    .padding_each(Insets::new(6, 6, 12, 6))
    .gap(10)
    .align_items(CrossAlign::Center)
}

/// One history entry: when, what, how bad, how long.
fn log_row(entry: ReadSignal<LogEntry>) -> impl View {
    let e0 = entry.get_untracked();
    row((
        label(text!("{}", Clock(e0.start))).style(&CAPTION).width(70),
        container(())
            .style(&SWATCH)
            .size(8, 8)
            .radius(Radius::Circle)
            .bg_color(priority_color(e0.cond.priority())),
        label(e0.cond.message()).font(&fonts::MONTSERRAT_14).flex_grow(1),
        label(text!("worst {}", entry.with(|e| e.worst)))
            .style(&CAPTION)
            .width(64),
        label(text!("{}", entry.with(|e| Lasted(e.start, e.end))))
            .font(&fonts::MONTSERRAT_12)
            .text_color(move || {
                if entry.with(|e| e.end.is_none()) {
                    ColorValue::from(priority_color(e0.cond.priority()))
                } else {
                    ColorValue::from(design::ON_SURFACE_MUTED)
                }
            })
            .width(48)
            .text_align(TextAlign::Right),
    ))
    .fill_width()
    .padding_each(Insets::hor_ver(4, 3))
    .gap(8)
    .align_items(CrossAlign::Center)
}

/// The Alarms screen.
pub fn page(cx: Scope) -> impl ViewExt {
    let m = cx.expect_context::<Monitor>();
    let none_active = move || m.alarms().with(Vec::is_empty);
    column((
        row((
            label("ACTIVE ALARMS").style(&CAPTION).flex_grow(1),
            label(text!("Audio paused  {}", Span(m.paused().get())))
                .style(&CAPTION)
                .text_color(theme::ALARM_MED)
                .hidden(move || m.paused().get() == 0),
        ))
        .fill_width(),
        column(for_each(m.alarms(), |a| a.cond as u8, |_cx, a| active_row(a)))
            .fill_width()
            .gap(4)
            .hidden(none_active),
        panel((
            label(Symbol::Ok).text_color(theme::HR),
            label("No active alarms").text_color(design::ON_SURFACE_MUTED),
        ))
        .layout(Layout::row())
        .fill_width()
        .height(46)
        .gap(10)
        .padding_x(14)
        .align_items(CrossAlign::Center)
        .hidden(move || !none_active()),
        label("HISTORY").style(&CAPTION).margin_top(4),
        panel(
            scroll_view(
                Axis::Vertical,
                (
                    for_each(m.log(), |e| e.id, |_cx, e| log_row(e)),
                    label("No alarms since admission")
                        .style(&CAPTION)
                        .padding(6)
                        .hidden(move || m.log().with(|l| !l.is_empty())),
                ),
            )
            .fill(),
        )
        .padding(4)
        .fill_width()
        .flex_grow(1),
    ))
    .fill()
    .padding(8)
    .gap(6)
    .flag(ObjFlags::SCROLLABLE, false)
}
