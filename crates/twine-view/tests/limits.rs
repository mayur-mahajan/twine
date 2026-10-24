//! R5.S03: `AppConfig::limits` reaches every bounded structure (engine, runtime, waker pool,
//! `Ui` queues, navigator), refusals are `Capacity` faults with their code, sealing checks
//! the heap budget and detects allocation afterwards, a sealed steady state allocates
//! nothing, and `AppConfig::audit_report` dumps the whole configuration.
//!
//! req: SAF-008

use std::cell::Cell;

use twine_core::alloc_guard::{self, AllocSite};
use twine_reactive::Runtime;
use twine_testing::TestUi;
use twine_testing::alloc::{CountingAllocator, assert_no_alloc_after_seal};
use twine_view::prelude::*;

#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator;

/// A product's limits: every bound explicit and reserved at start-up.
fn product_limits() -> Limits {
    Limits::DEFAULT
        .max_nodes(256)
        .max_reactive_nodes(256)
        .max_scopes(64)
        .max_pending_effects(32)
        .max_text_len(48)
        .max_timers(8)
        .max_anims(8)
        .max_nav_depth(4)
        .preallocated()
}

/// The code of the last record of `kind`.
fn last_code(t: &TestUi, kind: FaultKind) -> Option<u32> {
    t.engine().last_fault(kind).map(|r| r.code)
}

#[test]
fn app_config_limits_reach_the_engine_and_the_runtime() {
    let cfg = AppConfig::new().limits(product_limits().max_heap_wakers(7));
    let t = TestUi::new(64, 32).app_config(cfg).mount(|cx| {
        let n = cx.signal(1);
        label(text!("{}", n.get()))
    });
    // (The waker pool's share is process-wide: `tests/waker_pool.rs` checks it alone.)
    assert_eq!(*t.engine().limits(), product_limits().max_heap_wakers(7));
    // The runtime's bound is in force: a 65th scope is refused.
    let rt = Runtime::current_thread();
    let holder = t.root_scope().child();
    let mut refused = None;
    for _ in 0..64 {
        if let Err(e) = holder.try_child() {
            refused = Some(e);
            break;
        }
    }
    assert!(refused.is_some(), "Limits::max_scopes applied to the runtime");
    holder.dispose();
    let _ = rt;
}

#[test]
fn invalid_limits_fail_the_build() {
    use twine_core::ColorFormat;
    use twine_hal::DisplayInfo;
    use twine_testing::{MemoryDisplay, MockClock};
    let r = Ui::builder(MemoryDisplay::new(DisplayInfo::new(64, 32, ColorFormat::Rgb565)))
        .runtime(Runtime::current_thread())
        .buffers(BufferMode::alloc(BufferSpec::default()))
        .clock(MockClock::new())
        .limits(twine_testing::HARNESS_LIMITS.max_scopes(0))
        .try_build(|_| label("x"));
    assert!(matches!(
        r,
        Err(UiError::Engine(twine_engine::EngineError::InvalidConfig(
            "Limits::max_scopes must be >= 1"
        )))
    ));
}

#[test]
fn pending_effects_bound_is_forwarded_with_its_code() {
    let mut t = TestUi::new(64, 32)
        .app_config(AppConfig::new().limits(twine_testing::HARNESS_LIMITS.max_pending_effects(2)))
        .mount(|cx| {
            let s = cx.signal(0u32);
            cx.provide(s);
            for _ in 0..3 {
                cx.effect(move || {
                    s.get();
                });
            }
            label("x")
        });
    t.run_until_idle();
    let s = t.root_scope().use_context::<Signal<u32>>().unwrap();
    s.set(1);
    t.update();
    assert_eq!(
        last_code(&t, FaultKind::Capacity),
        Some(CapacityFault::PendingEffects.code())
    );
}

#[derive(Clone, PartialEq, Debug)]
struct Page(u32);

thread_local! {
    static NAV: Cell<Option<twine_view::Navigator<Page>>> = const { Cell::new(None) };
}

#[test]
fn navigator_depth_is_bounded_with_a_fault() {
    let mut t = TestUi::new(64, 32)
        .app_config(AppConfig::new().limits(product_limits()))
        .mount(|cx| {
            navigator(cx, Page(0), |cx, p: Page| {
                NAV.with(|n| n.set(use_navigator::<Page>(cx).ok()));
                label(format!("page {}", p.0))
            })
        });
    t.run_until_idle();
    let nav = NAV.with(Cell::get).unwrap();
    for i in 1..4 {
        assert_eq!(nav.push(Page(i), ScreenAnim::None), NavOutcome::Accepted);
    }
    assert_eq!(nav.push(Page(9), ScreenAnim::None), NavOutcome::Full);
    assert_eq!(nav.reset((0..5).map(Page), ScreenAnim::None), NavOutcome::Full);
    assert_eq!(nav.depth(), 4);
    t.run_until_idle();
    let r = *t.engine().last_fault(FaultKind::Capacity).unwrap();
    assert_eq!(r.code, CapacityFault::NavDepth.code());
    assert_eq!(r.occurrences, 2);
    assert_eq!(
        nav.replace(Page(5), ScreenAnim::None),
        NavOutcome::Accepted,
        "same depth"
    );
}

#[test]
fn seal_checks_the_heap_budget() {
    let mut t = TestUi::new(64, 32)
        .app_config(AppConfig::new().limits(twine_testing::HARNESS_LIMITS.heap_budget(Some(64))))
        .mount(|_| label("hi"));
    t.run_until_idle();
    let report = t.seal();
    alloc_guard::unseal(twine_core::Partition::MAIN);
    assert!(report.heap_bytes() > 64);
    assert_eq!(
        last_code(&t, FaultKind::Capacity),
        Some(CapacityFault::HeapBudget.code())
    );
}

/// req: SAF-010
#[test]
fn sealed_steady_state_allocates_nothing() {
    let mut t = TestUi::new(200, 100)
        .app_config(AppConfig::new().limits(product_limits()))
        .mount(|cx| {
            let (a, _ctl) = cx.animation(0, 3600, AnimSpec::new(Duration::ms(1000)).forever());
            let v = cx.tween(move || a.get() / 36, Duration::ms(50));
            column((label(text!("{} %", v.get())), bar(move || a.get() / 36)))
        });
    let period = t.engine().config().refr_period;
    // Start-up: the first frames size the label's buffers, queues and caches.
    for _ in 0..30 {
        t.advance(period);
    }
    let report = t.seal();
    assert!(report.heap_bytes() > 0);
    assert_no_alloc_after_seal(|| {
        for _ in 0..100 {
            t.advance(period);
        }
    });
    assert_eq!(t.engine().fault_counts().get(FaultKind::AllocAfterSeal), 0);
    alloc_guard::unseal(twine_core::Partition::MAIN);
}

/// fault: `AllocAfterSeal`
/// req: SAF-009, SAF-031
#[test]
fn creating_after_seal_raises_alloc_after_seal_at_the_next_update() {
    let mut t = TestUi::new(64, 32).mount(|cx| {
        let show = cx.signal(false);
        cx.provide(show);
        column((when(move || show.get(), |_| label("late")),))
    });
    t.run_until_idle();
    t.seal();
    let show = t.root_scope().use_context::<Signal<bool>>().unwrap();
    show.set(true);
    t.run_until_idle();
    alloc_guard::unseal(twine_core::Partition::MAIN);
    let r = *t.engine().last_fault(FaultKind::AllocAfterSeal).unwrap();
    assert!(r.occurrences > 1, "{r:?}");
    assert!(AllocSite::from_code(r.code).is_some());
}

/// req: SAF-024
#[test]
fn audit_report_dumps_the_whole_configuration() {
    let cfg = AppConfig::new()
        .limits(product_limits().heap_budget(Some(40 * 1024)))
        .theme(&lvgl::THEME)
        .motion(Motion::Reduced)
        .rotation(Rotation::Deg90);
    let text = cfg.audit_report().to_string();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.first(), Some(&"audit.format=1"));
    let safety = format!("build.safety={}", twine_core::SAFETY_PROFILE);
    for expected in [
        safety.as_str(),
        "build.reactive_no_panic_runtime=",
        "build.engine_debug_checks=",
        "limits.max_nodes=256",
        "limits.max_nav_depth=4",
        "limits.heap_budget_bytes=40960",
        "limits.preallocate=true",
        "engine.refr_period_us=16000",
        "engine.glyph_cache_bytes=8192",
        "fixed.max_displays=",
        "fixed.static_wakers=4",
        "view.motion=reduced",
        "view.rotation_deg=90",
        "view.fault_hook=false",
        "theme.set=true",
        "theme.name=",
        "theme.mode=light",
    ] {
        assert!(
            lines.iter().any(|l| l.starts_with(expected)),
            "{expected} missing:\n{text}"
        );
    }
    // Every line is `section.key=value`, and keys are unique.
    let mut keys: Vec<&str> = lines.iter().map(|l| l.split('=').next().unwrap()).collect();
    assert!(keys.iter().all(|k| k.contains('.')));
    let n = keys.len();
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys.len(), n, "duplicate keys:\n{text}");
    // The same configuration gives the same report; the state does not enter it.
    assert_eq!(cfg.clone().audit_report().to_string(), text);
    assert!(
        AppConfig::new()
            .audit_report()
            .to_string()
            .contains("theme.set=false\n")
    );
}
