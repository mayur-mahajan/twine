//! Heap per theme ≤ its tables (R7.S04): a display created with a built-in theme allocates
//! its merged element array and, for the LVGL theme, the metrics table; mode switches allocate
//! nothing. This test binary installs the counting allocator.

use twine_core::{ColorFormat, Size};
use twine_engine::{DisplayTheme, Engine, EngineConfig, ThemeHook};
use twine_hal::DisplayInfo;
use twine_testing::alloc::{CountingAllocator, count_allocs};
use twine_theme::{ThemeMode, lvgl, mono, simple};

#[global_allocator]
static A: CountingAllocator = CountingAllocator;

const INFO: DisplayInfo = DisplayInfo::new(320, 240, ColorFormat::Rgb565);

/// Live heap bytes added by adding a chunked display with `theme` (after a first display
/// warmed the engine's own storage up), and the engine.
fn display_bytes(theme: Option<&'static dyn ThemeHook>) -> (i64, Engine) {
    let mut e = Engine::new(EngineConfig::default()).unwrap();
    e.add_chunked_display(INFO, 320 * 2 * 8, None).unwrap();
    let (_, stats) = count_allocs(|| {
        e.add_chunked_display(INFO, 320 * 2 * 8, theme.map(DisplayTheme::new))
            .unwrap()
    });
    (stats.live, e)
}

/// What a theme costs a display on the heap is its tables: the display's merged element array
/// (`FlatElements`: every element finished for the display, one indexed lookup) for every
/// theme, plus the LVGL theme's metrics table. Mode switches allocate nothing.
#[test]
fn heap_per_theme_is_at_most_its_tables() {
    let size = Size::new(320, 240);
    let (bare, _) = display_bytes(None);
    // The merged array of a theme: room for the standard elements and the slots of every
    // listed mode (and of the metrics).
    let flat_bytes = |t: &'static dyn ThemeHook, metrics: Option<&twine_style::design::ElementTable>| {
        let mut flat = twine_style::design::FlatElements::new();
        for &m in t.modes() {
            flat.reserve(metrics, t.design(m).unwrap());
        }
        flat.heap_bytes() as i64
    };
    for theme in [&mono::THEME as &'static dyn ThemeHook, &simple::THEME] {
        assert!(theme.metrics(130, size).is_none(), "{}", theme.name());
        let (with, _) = display_bytes(Some(theme));
        let budget = flat_bytes(theme, None) + 128; // + the screen's style entries
        assert!(
            with - bare <= budget,
            "{}: {} > {budget}",
            theme.name(),
            with - bare
        );
    }
    // The LVGL theme: its metrics table (one boxed `ElementTable` with its private slots) and
    // the merged array.
    let (metrics, table) = count_allocs(|| Box::new(lvgl::THEME.metrics(130, size).unwrap()));
    let budget = table.live + flat_bytes(&lvgl::THEME, Some(&metrics)) + 128;
    eprintln!(
        "per display: lvgl {} B (metrics {} B + merged array {} B), mono/simple {} B",
        budget - 128,
        table.live,
        flat_bytes(&lvgl::THEME, Some(&metrics)),
        flat_bytes(&mono::THEME, None)
    );
    drop(metrics);
    let (with_lvgl, mut e) = display_bytes(Some(&lvgl::THEME));
    assert!(
        with_lvgl - bare <= budget,
        "lvgl: {} > {budget}",
        with_lvgl - bare
    );
    // Mode switches refill the merged array in place.
    let d = e.displays().last().unwrap();
    let ((), stats) = count_allocs(|| {
        for mode in [
            ThemeMode::Dark,
            ThemeMode::Night,
            ThemeMode::HighContrast,
            ThemeMode::Light,
        ] {
            e.set_theme_mode(d, mode);
        }
    });
    assert_eq!(stats.allocs, 0, "{stats:?}");
}
