//! R2.S03: theme modes for regulated displays. Every built-in theme lists its modes, defines
//! every standard element in each of them and meets each mode's minimum WCAG contrast ratios
//! (computed here independently in floating point and with the library's integer
//! `check_contrast`); the default theme's night palette keeps its luminance and blue limits;
//! an unsupported mode falls back to the current one with a warning.
#![allow(clippy::float_arithmetic)] // the test recomputes the WCAG formulas in f64

use std::fmt::Write as _;

use twine_core::{Color, Size};
use twine_engine::ThemeHook;
use twine_style::ThemeMode;
use twine_style::design::{self, ColorElement, ContrastPair, ElementTable, ElementType};
use twine_testing::{EngineHarness, capture_logs};
use twine_theme::{DisplaySize, Lvgl, Simple, lvgl, mono, simple};

const SIZE: Size = Size::new(320, 240);

/// The LVGL theme on large displays.
static LVGL_LARGE: Lvgl = Lvgl::new().display_size(DisplaySize::Large);

/// The built-in themes (LVGL in two display classes) and the modes they must support.
fn themes() -> Vec<(&'static str, &'static dyn ThemeHook, &'static [ThemeMode])> {
    vec![
        ("lvgl", &lvgl::THEME, &ThemeMode::ALL),
        ("lvgl large", &LVGL_LARGE, &ThemeMode::ALL),
        ("mono", &mono::THEME, &ThemeMode::ALL),
        ("simple", &simple::THEME, &ThemeMode::ALL),
    ]
}

/// What a display at 130 DPI of `SIZE` resolves with in `mode`: the mode's `'static` table
/// over the display's metrics table.
fn resolved(t: &dyn ThemeHook, mode: ThemeMode) -> Option<ElementTable> {
    let table = t.design(mode)?;
    let mut all = t.metrics(130, SIZE).unwrap_or_default();
    all.overlay(table);
    Some(all)
}

/// WCAG 2.x relative luminance in f64 (independent of `Color::relative_luminance`).
fn luminance(c: Color) -> f64 {
    let lin = |v: u8| {
        let c = f64::from(v) / 255.0;
        if c <= 0.040_45 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
}

fn ratio(a: Color, b: Color) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

#[test]
fn every_theme_defines_every_standard_element_in_every_mode() {
    for (name, t, modes) in themes() {
        assert_eq!(t.modes(), modes, "{name}");
        for mode in ThemeMode::ALL {
            let a = t.design(mode);
            if !modes.contains(&mode) {
                assert!(a.is_none(), "{name} {mode:?} is not supported");
                continue;
            }
            let a = a.unwrap_or_else(|| panic!("{name} {mode:?}"));
            let b = t.design(mode).unwrap();
            assert!(core::ptr::eq(a, b), "{name} {mode:?}: one static table");
            let all = resolved(t, mode).unwrap();
            assert_eq!(all.missing_standard(), None, "{name} {mode:?}");
        }
    }
}

/// The contrast requirements of every mode, for every built-in theme. Prints the table of
/// ratios (`cargo test -p twine-theme --test theme_modes -- --nocapture`).
#[test]
fn every_mode_meets_its_minimum_contrast_ratios() {
    let mut report = String::new();
    for (name, t, modes) in themes() {
        for &mode in modes {
            let table = resolved(t, mode).unwrap();
            let _ = write!(report, "{name:<14} {:<13}", format!("{mode:?}"));
            for pair in ContrastPair::ALL {
                let (fg, bg) = pair
                    .colors(&table)
                    .unwrap_or_else(|| panic!("{name} {mode:?} {}", pair.name()));
                let exact = ratio(fg, bg);
                let fixed = pair.ratio(&table).unwrap();
                // The integer ratio is the exact one rounded down to hundredths.
                let hundredths = f64::from(fixed.hundredths());
                assert!(
                    hundredths <= exact * 100.0 + 1e-6 && exact * 100.0 - hundredths < 1.1,
                    "{name} {mode:?} {}: {fixed} vs {exact}",
                    pair.name()
                );
                let _ = write!(report, " {exact:>6.2}");
                if let Some(min) = mode.min_contrast(pair) {
                    assert!(
                        exact * 100.0 >= f64::from(min.hundredths()),
                        "{name} {mode:?} {}: {exact:.2}:1 < {min}",
                        pair.name()
                    );
                }
            }
            report.push('\n');
            assert_eq!(table.check_contrast(mode), Ok(()), "{name} {mode:?}");
        }
    }
    println!(
        "theme          mode          {}\n{report}",
        ContrastPair::ALL.map(ContrastPair::name).join(" | ")
    );
}

/// Night: every color of the LVGL and simple themes is dim (≤ 30 % relative luminance) and
/// warm (blue at most half of red), and much darker than the dark mode's text.
#[test]
fn night_is_dim_and_warm() {
    for t in [&lvgl::THEME as &dyn ThemeHook, &simple::THEME] {
        let night = t.design(ThemeMode::Night).unwrap();
        for e in (0..<Color as ElementType>::STANDARD).map(id) {
            let c = night.get(e).unwrap();
            assert!(luminance(c) <= 0.30, "{} {e:?} = {c}: {}", t.name(), luminance(c));
            assert!(c.b <= c.r / 2, "{} {e:?} = {c}: too blue", t.name());
        }
        let dark = t.design(ThemeMode::Dark).unwrap();
        let text = |t: &ElementTable| t.get(design::ON_SURFACE).unwrap().relative_luminance();
        // LVGL: a third of its dark text at most; simple (brighter dark text): a half.
        let factor = if t.name() == "lvgl" { 3 } else { 2 };
        assert!(
            text(night) < text(dark) / factor,
            "{}: night text is 1/{factor} of dark text at most",
            t.name()
        );
    }
}

/// The standard color element with `id`.
fn id(id: u16) -> ColorElement {
    let all = [
        design::BACKGROUND,
        design::SURFACE,
        design::ON_SURFACE,
        design::ON_SURFACE_MUTED,
        design::SURFACE_VARIANT,
        design::OUTLINE,
        design::PRIMARY,
        design::ON_PRIMARY,
        design::SECONDARY,
        design::ON_SECONDARY,
        design::DANGER,
        design::WARNING,
        design::OK,
        design::DISABLED,
        design::FOCUS_RING,
        design::SCROLLBAR,
        design::SHADOW,
        design::SCRIM,
        design::PLACEHOLDER,
        design::NEUTRAL,
        design::ACCENT_TINT,
        design::SEPARATOR,
        design::GROUPED_BACKGROUND,
        design::FILL_1,
        design::FILL_2,
        design::FILL_3,
    ];
    all[usize::from(id)]
}

/// An application that overrides a standard element can verify its table the same way.
#[test]
fn application_overrides_are_checked_too() {
    static HC: ElementTable = Lvgl::SET
        .table(ThemeMode::HighContrast)
        .color(design::PRIMARY, Color::hex(0x0015_65C0));
    let v = HC.check_contrast(ThemeMode::HighContrast).unwrap_err();
    assert_eq!(v.pair, ContrastPair::OnPrimary); // black on dark blue
}

/// An unsupported mode: the display keeps its mode and table (documented fallback), a warning
/// is logged, nothing panics; supported modes switch.
#[test]
fn unsupported_mode_keeps_the_current_mode_and_warns() {
    /// The simple theme listing light and high contrast only.
    static TWO: Simple = Simple::new().modes(&[
        (ThemeMode::Light, &simple::LIGHT),
        (ThemeMode::HighContrast, &simple::HIGH_CONTRAST),
    ]);
    let mut h = EngineHarness::new(64, 48).theme(&TWO);
    h.run_until_idle();
    let d = h.display();
    let epoch = h.engine().design_epoch(d);
    for mode in [ThemeMode::Night, ThemeMode::Dark] {
        let ((), logs) = capture_logs(|| h.engine_mut().set_theme_mode(d, mode));
        assert_eq!(h.engine().theme_mode(d), ThemeMode::Light, "{mode:?}");
        assert_eq!(h.engine().design_epoch(d), epoch, "{mode:?}: table kept");
        assert!(
            logs.iter()
                .any(|l| l.level == log::Level::Warn && l.message.contains(&format!("has no {mode:?} mode"))),
            "{mode:?}: {logs:?}"
        );
        h.assert_idle();
    }
    let next = h.engine().theme_mode(d).next_in(h.engine().theme_modes(d));
    assert_eq!(next, ThemeMode::HighContrast);
    h.engine_mut().set_theme_mode(d, next);
    assert_eq!(h.engine().theme_mode(d), ThemeMode::HighContrast);
    assert_eq!(h.engine().design_value(d, design::ON_SURFACE), Some(Color::BLACK));
}
