//! Allocation audits with a counting global allocator: tree iterators and the steady-state
//! frame allocate nothing (P4).

mod common;

use twine_core::{Color, Duration, Opa, Rect};
use twine_engine::{Engine, EngineConfig, InvalidateReason, Obj};
use twine_style::{GradDir, Length, Radius, StyleProp};
use twine_testing::EngineHarness;
use twine_testing::alloc::{CountingAllocator, count_allocs};
use twine_testing::scenes::styled_box;

#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator;

#[test]
fn iterators_do_not_allocate() {
    let mut e = Engine::new(EngineConfig::default()).unwrap();
    let root = e.create_root(Box::new(Obj)).unwrap();
    let mut parent = root;
    for i in 0..1000 {
        let n = e.create(parent, Box::new(Obj)).unwrap();
        if i % 10 == 9 {
            parent = n;
        }
    }
    let t = e.tree();
    let (sum, stats) = count_allocs(|| {
        let mut n = 0usize;
        for id in t.descendants(root) {
            n += t.children(id).count() + t.children_rev(id).count() + t.ancestors(id).count();
        }
        n
    });
    assert!(sum > 1000);
    assert_eq!(stats.allocs + stats.reallocs, 0, "{stats:?}");
}

#[test]
fn steady_state_frame_allocates_nothing() {
    let mut h = EngineHarness::new(320, 240).no_theme().mount_engine(|e| {
        let s = common::white_screen(e);
        for i in 0..50 {
            let (x, y) = ((i % 10) * 30 + 5, (i / 10) * 45 + 5);
            let mut props = vec![
                StyleProp::BgColor(Color::hex(0x20_40_80 + i as u32 * 0x0003_0107).into()),
                StyleProp::BgOpacity(Opa::COVER.into()),
                StyleProp::Radius(Radius::Px(6).into()),
            ];
            if i % 3 == 0 {
                props.extend([
                    StyleProp::ShadowWidth(10),
                    StyleProp::ShadowOpacity(Opa::P50.into()),
                ]);
            }
            if i % 4 == 1 {
                props.extend([
                    StyleProp::BgGradientColor(Color::WHITE.into()),
                    StyleProp::BgGradientDir(GradDir::Ver),
                ]);
            }
            if i % 5 == 2 {
                props.extend([
                    StyleProp::BorderWidth(Length::Px(2).into()),
                    StyleProp::OutlineWidth(1),
                ]);
            }
            styled_box(e, s, Rect::from_xywh(x, y, 24, 36), &props);
        }
    });
    h.run_until_idle();
    let d = h.display();
    let area = Rect::from_xywh(40, 40, 100, 100);
    // Warm-up frames grow every buffer to its steady size.
    for _ in 0..2 {
        h.engine_mut()
            .invalidate_area(d, area, InvalidateReason::Explicit);
        h.advance(Duration::ms(16));
    }
    let ((), stats) = count_allocs(|| {
        h.engine_mut()
            .invalidate_area(d, area, InvalidateReason::Explicit);
        h.advance(Duration::ms(16));
    });
    assert!(h.last_frame().dirty_px >= 100 * 100);
    assert_eq!(
        (stats.allocs, stats.deallocs, stats.reallocs),
        (0, 0, 0),
        "{stats:?}"
    );
}

#[test]
fn anim_frame_allocates_nothing() {
    use twine_anim::{Anim, Repeat};
    use twine_engine::AnimProp;
    let mut boxes = Vec::new();
    let mut h = EngineHarness::new(320, 240).no_theme().mount_engine(|e| {
        let s = common::white_screen(e);
        for i in 0..20 {
            let r = Rect::from_xywh((i % 5) * 60 + 5, (i / 5) * 55 + 5, 30, 30);
            boxes.push(common::boxed(
                e,
                s,
                r,
                Color::hex(0x30_60_90 + i as u32 * 0x0002_0304),
            ));
        }
    });
    h.run_until_idle();
    for (i, &b) in boxes.iter().enumerate() {
        let (prop, from, to) = if i % 2 == 0 {
            (AnimProp::X, 0, 25)
        } else {
            (AnimProp::Opa, 255, 40)
        };
        h.engine_mut().anim_start(
            b,
            prop,
            Anim::new(from, to)
                .duration(Duration::ms(700))
                .playback(Duration::ms(700))
                .repeat(Repeat::Forever),
        );
    }
    // Warm-up: every queue, dirty set and layout buffer reaches its steady size.
    for _ in 0..10 {
        h.advance(Duration::ms(16));
    }
    let ((), stats) = count_allocs(|| {
        for _ in 0..30 {
            h.advance(Duration::ms(16));
        }
    });
    assert!(h.last_frame().dirty_px > 0);
    assert_eq!(
        (stats.allocs, stats.deallocs, stats.reallocs),
        (0, 0, 0),
        "{stats:?}"
    );
}

#[test]
fn input_reads_and_health_transitions_allocate_nothing() {
    use twine_core::Point;
    use twine_hal::DeviceHealth;
    let mut h = EngineHarness::new(100, 100).no_theme().mount_engine(|e| {
        let s = common::white_screen(e);
        common::clickable(e, s, Rect::from_xywh(10, 10, 40, 40));
    });
    h.run_until_idle();
    let (id, m) = h.pointer_input();
    // Warm-up: a tap and one pass through every health state.
    h.tap(Point::new(20, 20));
    for health in [
        DeviceHealth::Degraded { errors: 1 },
        DeviceHealth::Failed,
        DeviceHealth::Ok,
    ] {
        m.set_health(health);
        h.engine_mut().notify_input(id);
        h.update();
    }
    h.run_until_idle();
    let now = h.now();
    let ((), stats) = count_allocs(|| {
        // Healthy reads, then a press held when the device degrades and fails, then recovery.
        m.press(Point::new(20, 20));
        for health in [
            DeviceHealth::Ok,
            DeviceHealth::Degraded { errors: 1 },
            DeviceHealth::Degraded { errors: 2 },
            DeviceHealth::Failed,
            DeviceHealth::Failed,
            DeviceHealth::Ok,
        ] {
            m.set_health(health);
            h.engine_mut().notify_input(id);
            h.engine_mut().read_inputs(now);
        }
        m.release();
        h.engine_mut().notify_input(id);
        h.engine_mut().read_inputs(now);
    });
    assert_eq!(
        h.engine()
            .fault_counts()
            .get(twine_core::fault::FaultKind::InputDevice),
        4
    );
    assert_eq!(
        (stats.allocs, stats.deallocs, stats.reallocs),
        (0, 0, 0),
        "{stats:?}"
    );
}

/// R2.S06: starting style transitions — an inline (run-time) transition deriving its
/// properties from the state change, and a `static` one with a property set — allocates
/// nothing once the timeline, the transition list and the transition style pool are warm.
#[test]
fn transition_start_allocates_nothing_after_warm_up() {
    use twine_engine::State;
    use twine_style::{Props, Selector, Transition, TransitionRef};
    static GROUPS: Transition = Transition::of(Props::BG.union(Props::TRANSFORM), Duration::ms(40));
    let mut ids = Vec::new();
    let mut h = EngineHarness::new(200, 100).no_theme().mount_engine(|e| {
        let s = common::white_screen(e);
        for i in 0..2 {
            let n = common::boxed(e, s, Rect::from_xywh(10 + i * 60, 10, 40, 40), Color::RED);
            let pressed = Selector::state(State::PRESSED);
            e.set_local_prop(n, pressed, StyleProp::BgColor(Color::BLUE.into()));
            e.set_local_prop(n, pressed, StyleProp::TransformScaleX(twine_core::Scale::pct(90)));
            if i == 0 {
                e.set_local_transition(n, Selector::MAIN, Transition::all(Duration::ms(40)).ease_out());
            } else {
                e.set_local_prop(
                    n,
                    Selector::MAIN,
                    StyleProp::Transition(TransitionRef::Static(&GROUPS)),
                );
            }
            ids.push(n);
        }
    });
    h.run_until_idle();
    let cycle = |h: &mut EngineHarness| {
        for &n in &ids {
            h.engine_mut().add_state(n, State::PRESSED);
        }
        h.advance(Duration::ms(16));
        assert_eq!(h.engine().transition_count(), 4);
        h.run_until_idle();
        for &n in &ids {
            h.engine_mut().clear_state(n, State::PRESSED);
        }
        h.run_until_idle();
    };
    // Warm-up: press and release twice (pool, list, timeline slots, render buffers).
    cycle(&mut h);
    cycle(&mut h);
    let ((), stats) = count_allocs(|| cycle(&mut h));
    assert_eq!((stats.allocs, stats.reallocs), (0, 0), "{stats:?}");
}
