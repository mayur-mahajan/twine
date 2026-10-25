//! The display's text gamma from the configuration (R6.S05): `AppConfig::text_gamma` for the
//! first display, `DisplayBuilder::text_gamma` for any display, `DisplayMut::set_text_gamma` at
//! run time, and its line in the audit report.

use twine_core::ColorFormat;
use twine_hal::DisplayInfo;
use twine_testing::{MemoryDisplay, MockClock};
use twine_text::{TextGamma, gamma};
use twine_view::prelude::*;

fn panel(w: u16, h: u16) -> MemoryDisplay {
    MemoryDisplay::new(DisplayInfo::new(w, h, ColorFormat::Rgb565))
}

fn bufs() -> BufferMode {
    BufferMode::alloc(BufferSpec::default())
}

fn is(g: Option<&'static TextGamma>, want: &'static TextGamma) -> bool {
    g.is_some_and(|g| core::ptr::eq(g, want))
}

#[test]
fn app_config_sets_the_first_display_and_builders_set_their_own() {
    static CUSTOM: TextGamma = TextGamma {
        dark_on_light: gamma::DEFAULT.dark_on_light,
        light_on_dark: gamma::LINEAR.light_on_dark,
    };
    let mut ui = Ui::builder(panel(64, 32))
        .runtime(Runtime::current_thread())
        .clock(MockClock::new())
        .buffers(bufs())
        .app_config(
            AppConfig::new()
                .text_gamma(&gamma::LINEAR)
                .limits(twine_testing::HARNESS_LIMITS),
        )
        .try_build(|_| label("main"))
        .unwrap();
    let main = ui.display();
    assert!(is(ui.engine().text_gamma(main), &gamma::LINEAR));
    let aux = ui
        .mount_on(
            DisplayBuilder::new(panel(32, 16))
                .buffers(bufs())
                .text_gamma(&CUSTOM),
            |_| label("aux"),
        )
        .unwrap();
    assert!(is(ui.engine().text_gamma(aux), &CUSTOM), "the builder's own");
    let plain = ui
        .mount_on(DisplayBuilder::new(panel(32, 16)).buffers(bufs()), |_| {
            label("plain")
        })
        .unwrap();
    assert_eq!(ui.engine().text_gamma(plain), None, "the fonts' own");
    // At run time.
    ui.display_mut(main).unwrap().set_text_gamma(None);
    assert_eq!(ui.display_mut(main).unwrap().text_gamma(), None);
}

#[test]
fn the_audit_report_names_the_text_gamma() {
    static OTHER: TextGamma = TextGamma {
        dark_on_light: gamma::LINEAR.dark_on_light,
        light_on_dark: gamma::LINEAR.light_on_dark,
    };
    let line = |cfg: AppConfig| {
        cfg.audit_report()
            .to_string()
            .lines()
            .find(|l| l.starts_with("view.text_gamma="))
            .map(str::to_owned)
    };
    assert_eq!(line(AppConfig::new()).as_deref(), Some("view.text_gamma=fonts"));
    assert_eq!(
        line(AppConfig::new().text_gamma(&gamma::LINEAR)).as_deref(),
        Some("view.text_gamma=linear")
    );
    assert_eq!(
        line(AppConfig::new().text_gamma(&gamma::DEFAULT)).as_deref(),
        Some("view.text_gamma=default")
    );
    assert_eq!(
        line(AppConfig::new().text_gamma(&OTHER)).as_deref(),
        Some("view.text_gamma=custom"),
        "equal tables in another static are a custom choice"
    );
}
