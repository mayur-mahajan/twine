//! R2.S03: theme modes for regulated displays. Every built-in theme lists its modes, defines
//! every standard element in each of them and meets each mode's minimum WCAG contrast ratios
//! (computed here independently in floating point and with the library's integer
//! `check_contrast`); the default theme's night palette keeps its luminance and blue limits;
//! an unsupported mode falls back to the current one with a warning.
#![allow(clippy::float_arithmetic)] // the test recomputes the WCAG formulas in f64

use std::fmt::Write as _;
use std::rc::Rc;

use twine_core::{Color, Size};
use twine_engine::ThemeHook;
use twine_style::ThemeMode;
use twine_style::design::{self, ColorElement, ContrastPair, ElementTable};
use twine_testing::{EngineHarness, capture_logs};
use twine_theme::{DefaultTheme, DisplaySize, MonoTheme, SimpleTheme};

const SIZE: Size = Size::new(320, 240);

/// The built-in themes (default in two display classes) and the modes they must support.
fn themes() -> Vec<(&'static str, Rc<dyn ThemeHook>, &'static [ThemeMode])> {
    let font = &twine_assets::fonts::MONTSERRAT_14;
    vec![
        ("default", Rc::new(DefaultTheme::light()), &ThemeMode::ALL),
        (
            "default large",
            Rc::new(
                DefaultTheme::builder()
                    .mode(ThemeMode::Dark)
                    .display_size(DisplaySize::Large)
                    .build(),
            ),
            &ThemeMode::ALL,
        ),
        (
            "mono",
            Rc::new(MonoTheme::builder().font(font).build()),
            &ThemeMode::ALL,
        ),
        (
            "simple",
            Rc::new(SimpleTheme::new()),
            &[ThemeMode::Light, ThemeMode::HighContrast],
        ),
    ]
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
        assert!(
            t.modes().contains(&t.mode()),
            "{name}: starts in a supported mode"
        );
        for mode in ThemeMode::ALL {
            let a = t.design(mode, 130, SIZE);
            if !modes.contains(&mode) {
                assert!(a.is_none(), "{name} {mode:?} is not supported");
                continue;
            }
            let a = a.unwrap_or_else(|| panic!("{name} {mode:?}"));
            assert_eq!(a.missing_standard(), None, "{name} {mode:?}");
            let b = t.design(mode, 130, SIZE).unwrap();
            assert!(Rc::ptr_eq(&a, &b), "{name} {mode:?}: built once");
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
            let table = t.design(mode, 130, SIZE).unwrap();
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

/// Night: every color of the default theme is dim (≤ 30 % relative luminance) and warm (blue
/// at most half of red), and darker than the dark mode's text.
#[test]
fn default_night_is_dim_and_warm() {
    let t = DefaultTheme::light();
    let night = t.design(ThemeMode::Night, 130, SIZE).unwrap();
    for e in (0..ColorElement::custom(0).id()).map(id) {
        let c = night.get(e).unwrap();
        assert!(luminance(c) <= 0.30, "{e:?} = {c}: {}", luminance(c));
        assert!(c.b <= c.r / 2, "{e:?} = {c}: too blue");
    }
    let dark = t.design(ThemeMode::Dark, 130, SIZE).unwrap();
    let text = |t: &ElementTable| t.get(design::ON_SURFACE).unwrap().relative_luminance();
    assert!(
        text(&night) < text(&dark) / 3,
        "night text is a third of dark text at most"
    );
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
    ];
    all[usize::from(id)]
}

/// An application that overrides a standard element can verify its table the same way.
#[test]
fn application_overrides_are_checked_too() {
    let t = DefaultTheme::builder()
        .element_in(ThemeMode::HighContrast, design::PRIMARY, Color::hex(0x0015_65C0))
        .build();
    let table: Rc<ElementTable> = t.design(ThemeMode::HighContrast, 130, SIZE).unwrap();
    let v = table.check_contrast(ThemeMode::HighContrast).unwrap_err();
    assert_eq!(v.pair, ContrastPair::OnPrimary); // black on dark blue
}

/// An unsupported mode: the display keeps its mode and table (documented fallback), a warning
/// is logged, nothing panics; supported modes switch.
#[test]
fn unsupported_mode_keeps_the_current_mode_and_warns() {
    let mut h = EngineHarness::new(64, 48).theme(Rc::new(SimpleTheme::new()));
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
