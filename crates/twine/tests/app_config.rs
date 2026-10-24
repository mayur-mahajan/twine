//! One configuration everywhere (R3.S09): the `AppConfig` the firmware ships
//! (`twine_demos::config()`) given to a `Ui`, to the simulator and to `TestUi` yields the same
//! engine settings and the same pixels; every host takes a theme the same way (a `static`).

use twine::core::{ColorFormat, Rotation};
use twine::engine::ThemeHook;
use twine::hal::DisplayInfo;
use twine::prelude::*;
use twine_sim::{Headless, SimConfig};
use twine_testing::{MemoryDisplay, MockClock, TestUi};

const W: u16 = 160;
const H: u16 = 120;

/// The shipped configuration, refined the way a product would (one function, every host).
fn config() -> AppConfig {
    twine_demos::config()
        .motion(Motion::Reduced)
        .limits(
            Limits::DEFAULT
                .messages_per_channel(4)
                .engine_queue(24)
                .max_nodes(300),
        )
        .engine(EngineConfig {
            refr_period: Duration::ms(20),
            ..EngineConfig::default()
        })
}

fn ui_with(config: AppConfig) -> Ui {
    let panel = MemoryDisplay::new(DisplayInfo::new(W, H, ColorFormat::Rgb565));
    Ui::builder(panel)
        .runtime(Runtime::current_thread())
        .clock(MockClock::new())
        .buffers(BufferMode::alloc(BufferSpec::PartialDouble { rows: 16 }))
        .app_config(config)
        .build(twine_demos::counter::app)
}

fn ui_pixels(ui: &Ui) -> Vec<u8> {
    ui.engine()
        .driver::<MemoryDisplay>(ui.display())
        .expect("memory display")
        .to_rgb888()
}

fn sim_pixels(name: &str, config: AppConfig) -> Vec<u8> {
    let out_dir = std::env::temp_dir().join(format!("twine-app-config-{name}-{}", std::process::id()));
    let cfg = SimConfig::new(W, H).app_config(config).headless(Some(Headless {
        frames: 3,
        script: None,
        out_dir: out_dir.clone(),
    }));
    let report = twine_sim::run_headless(cfg, twine_demos::counter::app).expect("headless run");
    let png = twine_testing::png_io::read_rgb_png(report.shots.last().expect("final.png")).unwrap();
    let _ = std::fs::remove_dir_all(out_dir);
    assert_eq!((png.width, png.height), (u32::from(W), u32::from(H)));
    png.data
}

#[test]
fn shared_config_gives_identical_settings_on_ui_and_test_ui() {
    let mut ui = ui_with(config());
    let t = TestUi::new(W, H)
        .app_config(config())
        .mount(twine_demos::counter::app);
    ui.update();
    let (a, b) = (ui.engine(), t.engine());
    for e in [a, &*b] {
        assert_eq!(e.motion(), Motion::Reduced);
        assert_eq!(e.limits().max_nodes, 300);
        assert_eq!(e.config().refr_period, Duration::ms(20));
        let d = e.default_display().unwrap();
        assert_eq!(
            e.theme(d).map(ThemeHook::name),
            twine_demos::config().theme.map(ThemeHook::name)
        );
    }
    assert_eq!(ui.messages_per_channel(), 4);
}

#[test]
fn shared_config_gives_identical_pixels_on_ui_sim_and_test_ui() {
    let mut ui = ui_with(config());
    let _ = ui.update();
    let from_ui = ui_pixels(&ui);

    let mut t = TestUi::new(W, H)
        .app_config(config())
        .mount(twine_demos::counter::app);
    t.run_until_idle();
    let from_test = t.harness_mut().panel_rgb888();

    let from_sim = sim_pixels("pixels", config());

    assert!(from_ui.iter().any(|&p| p != from_ui[0]), "something was drawn");
    assert!(from_ui == from_test, "Ui and TestUi differ");
    assert!(from_ui == from_sim, "Ui and the simulator differ");
}

#[test]
fn a_different_config_changes_the_pixels_everywhere() {
    // Guard against a vacuous comparison: the theme of the configuration reaches the pixels.
    let dark = || config().theme(&lvgl::THEME).mode(ThemeMode::Dark);
    let mut light = ui_with(config());
    let mut night = ui_with(dark());
    light.update();
    night.update();
    assert!(
        ui_pixels(&light) != ui_pixels(&night),
        "the theme reaches the pixels"
    );
    let mut t = TestUi::new(W, H)
        .app_config(dark())
        .mount(twine_demos::counter::app);
    t.run_until_idle();
    assert!(
        t.harness_mut().panel_rgb888() == ui_pixels(&night),
        "TestUi and Ui differ"
    );
}

#[test]
fn a_config_without_theme_runs_unthemed_on_every_host() {
    let bare = || config().no_theme();
    let ui = ui_with(bare());
    assert!(ui.engine().theme(ui.display()).is_none());
    let t = TestUi::new(W, H)
        .app_config(bare())
        .mount(twine_demos::counter::app);
    assert!(t.engine().theme(t.engine().default_display().unwrap()).is_none());
    assert!(SimConfig::new(W, H).app_config(bare()).app.theme.is_none());
}

#[test]
fn configured_rotation_reaches_every_host() {
    let rotated = || config().rotation(Rotation::Deg90);
    // A panel that rotates in hardware (MIPI DCS `MADCTL`): applied before the first frame.
    let panel = MemoryDisplay::new(DisplayInfo::new(W, H, ColorFormat::Rgb565)).with_rotation_control();
    let mut ui = Ui::builder(panel)
        .runtime(Runtime::current_thread())
        .clock(MockClock::new())
        .buffers(BufferMode::alloc(BufferSpec::default()))
        .app_config(rotated())
        .build(twine_demos::counter::app);
    ui.update();
    assert_eq!(ui.display_info().rotation, Rotation::Deg90);
    let t = TestUi::new(W, H)
        .app_config(rotated())
        .mount(twine_demos::counter::app);
    let e = t.engine();
    assert_eq!(
        e.display_info(e.default_display().unwrap()).unwrap().rotation,
        Rotation::Deg90
    );
    let sim = SimConfig::new(W, H).app_config(rotated());
    assert_eq!(sim.rotation, Rotation::Deg90);
    assert!(sim.hw_rotation, "the simulator emulates the rotation in hardware");
}

/// The fault hook of the configuration is installed by every host.
#[test]
fn configured_fault_hook_is_installed_everywhere() {
    use std::sync::atomic::{AtomicU32, Ordering};
    static SEEN: AtomicU32 = AtomicU32::new(0);
    fn count(r: &FaultRecord) {
        SEEN.fetch_add(r.occurrences, Ordering::Relaxed);
    }
    let mut ui = ui_with(config().fault_hook(count));
    ui.engine_mut().raise_fault(FaultRecord::new(FaultKind::Capacity));
    let mut t = TestUi::new(W, H)
        .app_config(config().fault_hook(count))
        .mount(twine_demos::counter::app);
    t.engine_mut().raise_fault(FaultRecord::new(FaultKind::Capacity));
    assert_eq!(SEEN.load(Ordering::Relaxed), 2);
}

// ---- One theme parameter type -------------------------------------------------------------

/// One theme for every host: a `static` (`&'static dyn ThemeHook`) and a start mode, attached
/// when the display is created (R7.S04).
#[test]
fn every_host_takes_a_static_theme() {
    let panel = || MemoryDisplay::new(DisplayInfo::new(W, H, ColorFormat::Rgb565));
    let build = |b: UiBuilder<twine::view::Partial<MemoryDisplay>>| {
        b.runtime(Runtime::current_thread())
            .clock(MockClock::new())
            .buffers(BufferMode::alloc(BufferSpec::default()))
            .build(|_| label("hi"))
    };
    let same =
        |t: Option<&'static dyn ThemeHook>| t.is_some_and(|t| core::ptr::addr_eq(t, &raw const lvgl::THEME));
    let ui = build(Ui::builder(panel()).theme(&lvgl::THEME).mode(ThemeMode::Dark));
    assert!(
        same(ui.engine().theme(ui.display())),
        "the static itself, not a copy"
    );
    assert_eq!(ui.engine().theme_mode(ui.display()), ThemeMode::Dark);

    let t = TestUi::new(W, H)
        .theme(&lvgl::THEME)
        .mode(ThemeMode::Dark)
        .mount(|_| label("hi"));
    let d = t.engine().default_display().unwrap();
    assert!(same(t.engine().theme(d)));
    assert_eq!(t.engine().theme_mode(d), ThemeMode::Dark);

    let cfg = SimConfig::new(W, H).theme(&lvgl::THEME).mode(ThemeMode::Dark);
    assert!(same(cfg.app.theme));
    assert!(same(AppConfig::new().theme(&lvgl::THEME).theme));

    // A further display brings its own theme; each display switches its own mode.
    let aux = DisplayBuilder::new(panel())
        .theme(&lvgl::THEME)
        .mode(ThemeMode::Night);
    let mut ui = ui;
    let d = ui
        .mount_on(aux.buffers(BufferMode::alloc(BufferSpec::default())), |_| {
            label("aux")
        })
        .unwrap();
    assert!(same(ui.engine().theme(d)));
    assert_eq!(ui.engine().theme_mode(d), ThemeMode::Night);
    ui.display_mut(d).unwrap().set_theme_mode(ThemeMode::Light);
    ui.update();
    assert_eq!(ui.engine().theme_mode(d), ThemeMode::Light);
    assert_eq!(ui.engine().theme_mode(ui.display()), ThemeMode::Dark);
}
