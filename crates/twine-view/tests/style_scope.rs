//! R2.S01: reactive per-part / per-state styling (`part`, `on_state`, `styled` and the
//! `StyleScope` modifiers), and grid templates through the style system (rework F4).

use twine_core::{Color, Point};
use twine_reactive::runtime_stats;
use twine_style::{
    GridTrack, GridTracks, Part, PropId, Selector, SharedTracks, State, Style, StyleBuf, StyleRef,
    StyleValue, style,
};
use twine_testing::{TestUi, by_id};
use twine_view::prelude::*;

const OK: Color = Color::new(0x2E, 0x7D, 0x32);
const DANGER: Color = Color::new(0xC6, 0x28, 0x28);
const PRESSED: Color = Color::new(0x15, 0x65, 0xC0);

fn mount<V: View>(app: impl FnOnce(Scope) -> V) -> TestUi {
    let mut t = TestUi::new(240, 160).mount(app);
    t.run_until_idle();
    t
}

fn node(t: &TestUi, id: &'static str) -> NodeId {
    t.find(by_id(id)).id()
}

fn center(t: &TestUi, id: &'static str) -> Point {
    t.find(by_id(id)).coords().center()
}

fn color(t: &TestUi, id: &'static str, part: Part) -> Color {
    t.engine().style_color(node(t, id), part, PropId::BgColor)
}

#[test]
fn indicator_colour_follows_a_signal() {
    let mut t = mount(|cx| {
        let alarm = cx.signal(false);
        cx.provide(alarm);
        column((
            slider(cx.signal(40))
                .part(Part::Indicator, |s| {
                    s.bg(move || if alarm.get() { DANGER } else { OK })
                })
                .test_id("slider"),
            bar(cx.signal(60))
                .part(Part::Indicator, |s| {
                    s.bg_color(move || if alarm.get() { DANGER } else { OK })
                })
                .test_id("bar"),
        ))
    });
    let alarm = t.root_scope().expect_context::<Signal<bool>>();
    for id in ["slider", "bar"] {
        assert_eq!(color(&t, id, Part::Indicator), OK, "{id}");
        assert_ne!(color(&t, id, Part::Main), OK, "{id}: only the indicator");
    }
    alarm.set(true);
    t.run_until_idle();
    for id in ["slider", "bar"] {
        assert_eq!(color(&t, id, Part::Indicator), DANGER, "{id}");
    }
    alarm.set(false);
    t.run_until_idle();
    assert_eq!(color(&t, "slider", Part::Indicator), OK);
}

#[test]
fn pressed_look_is_applied_and_removed_with_the_state() {
    let mut t = mount(|_| {
        button(label("OK"))
            .size(100, 40)
            .on_state(State::PRESSED, |s| s.bg(PRESSED).transform_scale(Scale::pct(97)))
            .test_id("b")
    });
    let normal = color(&t, "b", Part::Main);
    assert_ne!(normal, PRESSED);
    let p = center(&t, "b");
    t.press(p);
    t.advance(Duration::ms(500)); // the theme's pressed transition
    let b = node(&t, "b");
    assert!(t.find(by_id("b")).state().contains(State::PRESSED));
    assert_eq!(color(&t, "b", Part::Main), PRESSED);
    assert_eq!(
        t.engine().style_prop(b, Part::Main, PropId::TransformScaleX),
        StyleValue::Scale(Scale::pct(97))
    );
    t.release();
    t.run_until_idle();
    assert_eq!(color(&t, "b", Part::Main), normal);
    assert_eq!(
        t.engine().style_prop(b, Part::Main, PropId::TransformScaleX),
        StyleValue::Scale(Scale::ONE)
    );
}

#[test]
fn part_in_a_state_combines_both() {
    let mut t = mount(|cx| {
        let hot = cx.signal(false);
        cx.provide(hot);
        slider(cx.signal(50))
            .size(160, 20)
            .part(Part::Indicator, |s| {
                s.bg(OK).on_state(State::PRESSED, |s| {
                    s.bg(move || if hot.get() { DANGER } else { PRESSED })
                })
            })
            .styled(Selector::part(Part::Knob).with_state(State::PRESSED), |s| {
                s.bg(DANGER)
            })
            .test_id("s")
    });
    assert_eq!(color(&t, "s", Part::Indicator), OK);
    assert_ne!(color(&t, "s", Part::Knob), DANGER);
    t.press(center(&t, "s"));
    t.advance(Duration::ms(500)); // the theme's pressed transition
    assert_eq!(color(&t, "s", Part::Indicator), PRESSED);
    assert_eq!(color(&t, "s", Part::Knob), DANGER);
    assert_ne!(color(&t, "s", Part::Main), PRESSED, "the main part is not styled");
    // The binding inside the nested scope stays reactive.
    let hot = t.root_scope().expect_context::<Signal<bool>>();
    hot.set(true);
    t.update();
    assert_eq!(color(&t, "s", Part::Indicator), DANGER);
    t.release();
    t.run_until_idle();
    assert_eq!(color(&t, "s", Part::Indicator), OK);
    // The scope's selectors are local style entries of the node.
    let e = t.engine();
    let sel = |part, state| Selector::part(part).with_state(state);
    let local = e
        .tree()
        .node(node(&t, "s"))
        .unwrap()
        .styles()
        .entries()
        .iter()
        .filter(|x| x.kind == twine_style::EntryKind::Local);
    let selectors: Vec<Selector> = local.map(|x| x.selector).collect();
    assert!(selectors.contains(&sel(Part::Indicator, State::DEFAULT)));
    assert!(selectors.contains(&sel(Part::Indicator, State::PRESSED)));
    assert!(selectors.contains(&sel(Part::Knob, State::PRESSED)));
}

#[test]
fn part_and_state_nest_in_either_order() {
    let mut t = mount(|cx| {
        column((
            slider(cx.signal(10))
                .part(Part::Knob, |s| s.on_state(State::PRESSED, |s| s.bg(DANGER)))
                .test_id("a"),
            slider(cx.signal(10))
                .on_state(State::PRESSED, |s| s.part(Part::Knob, |s| s.bg(DANGER)))
                .test_id("b"),
            // Repeated selectors: states add up, the innermost part applies, the last value of
            // a selector wins.
            slider(cx.signal(10))
                .on_state(State::CHECKED, |s| {
                    s.on_state(State::PRESSED, |s| {
                        s.part(Part::Indicator, |s| s.part(Part::Knob, |s| s.bg(OK)))
                    })
                })
                .styled(
                    Selector::part(Part::Knob).with_state(State::PRESSED | State::CHECKED),
                    |s| s.bg(DANGER),
                )
                .test_id("c"),
        ))
    });
    let local = |t: &TestUi, id| -> Vec<(Selector, StyleValue)> {
        let e = t.engine();
        e.tree()
            .node(node(t, id))
            .unwrap()
            .styles()
            .entries()
            .iter()
            .filter(|x| x.kind == twine_style::EntryKind::Local)
            .filter_map(|x| Some((x.selector, x.style.get(PropId::BgColor)?)))
            .collect()
    };
    let knob_pressed = Selector::part(Part::Knob).with_state(State::PRESSED);
    assert_eq!(local(&t, "a"), [(knob_pressed, StyleValue::Color(DANGER))]);
    assert_eq!(local(&t, "a"), local(&t, "b"));
    assert_eq!(
        local(&t, "c"),
        [(
            Selector::part(Part::Knob).with_state(State::PRESSED | State::CHECKED),
            StyleValue::Color(DANGER)
        )]
    );
    t.press(center(&t, "b"));
    t.advance(Duration::ms(500));
    assert_eq!(color(&t, "b", Part::Knob), DANGER);
}

#[test]
fn constants_create_no_binding() {
    let nodes = |styled: bool| {
        let before = runtime_stats().nodes;
        let t = mount(move |_| {
            let b = button(label("x")).test_id("b");
            if styled {
                b.part(Part::Main, |s| s.radius(3))
                    .on_state(State::PRESSED, |s| {
                        s.bg(PRESSED).padding(4).layout(Layout::row()).fill_width()
                    })
                    .into_any()
            } else {
                b.into_any()
            }
        });
        let n = runtime_stats().nodes - before;
        if styled {
            let e = t.engine();
            let b = node(&t, "b");
            assert_eq!(
                e.style_prop(b, Part::Main, PropId::Radius),
                StyleValue::Length(Length::Px(3))
            );
        }
        drop(t);
        n
    };
    assert_eq!(
        nodes(true),
        nodes(false),
        "constant scope styles add no reactive node"
    );
}

#[test]
fn idempotent_rerun_does_not_invalidate() {
    let mut t = mount(|cx| {
        let n = cx.signal(1);
        cx.provide(n);
        button(label("x"))
            .on_state(State::CHECKED, |s| {
                s.bg(move || if n.get() > 10 { DANGER } else { OK })
                    .padding(move || if n.get() > 10 { 8 } else { 2 })
            })
            .part(Part::Main, |s| {
                s.bg_opacity(move || if n.get() > 10 { Opa::P50 } else { Opa::COVER })
            })
            .test_id("b")
    });
    let n = t.root_scope().expect_context::<Signal<i32>>();
    let runs = runtime_stats().effect_runs;
    n.set(2); // every binding re-runs with the same value
    t.update();
    assert!(runtime_stats().effect_runs > runs, "the bindings ran");
    assert!(t.invalidations().is_empty(), "{:?}", &*t.invalidations());
    t.assert_idle();
}

// ---- Grid templates through the style system (F4) -----------------------------------------

static THREE: [GridTrack; 3] = [GridTrack::Px(30), GridTrack::Px(30), GridTrack::Px(30)];
static WIDE: Style = style! { grid_column_tracks: &THREE };

fn tracks(t: &TestUi, id: &'static str) -> Option<usize> {
    t.engine().grid_column_tracks(node(t, id)).map(<[GridTrack]>::len)
}

fn cells() -> impl ViewSeq {
    (
        label("a").size(10, 10).grid_col(0).grid_row(0),
        label("b").size(10, 10).grid_col(1).grid_row(0).test_id("cell"),
    )
}

#[test]
fn state_and_theme_styles_override_view_set_grid_tracks() {
    let mut t = mount(|_| {
        container((
            grid(grid_tracks![px(80), px(80)], grid_tracks![content], cells())
                .on_state(State::CHECKED, |s| s.style(&WIDE))
                .size(200, 60)
                .test_id("g"),
            grid(grid_tracks![px(70), px(70)], grid_tracks![content], label("c"))
                .on_state(State::custom::<0>(), |s| s.class_style(&WIDE))
                .test_id("h"),
        ))
    });
    assert_eq!(tracks(&t, "g"), Some(2));
    let x0 = t.find(by_id("cell")).coords().x0;
    let g = node(&t, "g");
    t.engine_mut().set_state(g, State::CHECKED, true);
    t.run_until_idle();
    assert_eq!(
        tracks(&t, "g"),
        Some(3),
        "the CHECKED style wins over the local Main tracks"
    );
    assert_eq!(
        t.find(by_id("cell")).coords().x0,
        x0 - 50,
        "relaid out with 30 px columns"
    );
    t.engine_mut().set_state(g, State::CHECKED, false);
    t.run_until_idle();
    assert_eq!(tracks(&t, "g"), Some(2));
    assert_eq!(t.find(by_id("cell")).coords().x0, x0);
    // A theme-priority (class) state style overrides them as well.
    let h = node(&t, "h");
    assert_eq!(tracks(&t, "h"), Some(2));
    t.engine_mut().set_state(h, State::custom::<0>(), true);
    t.run_until_idle();
    assert_eq!(tracks(&t, "h"), Some(3));
}

#[test]
fn grid_tracks_are_reactive_in_a_state_scope() {
    let mut t = mount(|cx| {
        let n = cx.signal(3usize);
        cx.provide(n);
        grid(&THREE[..2], grid_tracks![content], cells())
            .on_state(State::CHECKED, |s| {
                s.grid_column_tracks(move || vec![GridTrack::Px(20); n.get()])
            })
            .state(State::CHECKED, true)
            .test_id("g")
    });
    let n = t.root_scope().expect_context::<Signal<usize>>();
    assert_eq!(tracks(&t, "g"), Some(3));
    // The second cell starts after one 20 px column (and the gap).
    let checked = t.find(by_id("cell")).coords().x0 - node_x(&t, "g");
    n.set(4);
    t.run_until_idle();
    assert_eq!(tracks(&t, "g"), Some(4));
    // An equal list again: nothing to do.
    let runs = runtime_stats().effect_runs;
    n.set_if_changed(4);
    t.update();
    assert_eq!(runtime_stats().effect_runs, runs);
    // Leaving the state: the static `Main` tracks apply again.
    let g = node(&t, "g");
    t.engine_mut().set_state(g, State::CHECKED, false);
    t.run_until_idle();
    assert_eq!(tracks(&t, "g"), Some(2), "the static Main tracks apply again");
    assert_eq!(
        t.find(by_id("cell")).coords().x0 - node_x(&t, "g"),
        checked + 10,
        "30 px columns"
    );
}

fn node_x(t: &TestUi, id: &'static str) -> i32 {
    t.find(by_id(id)).coords().x0
}

/// Shared lists the tests keep a handle to (to watch who holds them).
fn lists(n: usize) -> Vec<SharedTracks> {
    (1..=n)
        .map(|k| SharedTracks::from(vec![GridTrack::Px(10); k]))
        .collect()
}

/// The holders of each list (the test's own handles included: tests compare changes).
fn counts(l: &[SharedTracks]) -> Vec<usize> {
    l.iter().map(SharedTracks::strong_count).collect()
}

#[test]
fn shared_tracks_are_released_with_values_and_nodes() {
    let all = lists(4);
    let l = all.clone();
    let mut t = mount(move |cx| {
        let show = cx.signal(true);
        let pick = cx.signal(0usize);
        cx.provide((show, pick));
        let l = l.clone();
        container(when(
            move || show.get(),
            move |_| {
                let l = l.clone();
                let rows = l[3].clone();
                grid(
                    move || GridTracks::Shared(l[pick.get()].clone()),
                    GridTracks::Shared(rows.clone()),
                    label("x"),
                )
                .on_state(State::PRESSED, move |s| {
                    s.grid_row_tracks(GridTracks::Shared(rows.clone()))
                })
                .test_id("g")
            },
        ))
    });
    let (show, pick) = t.root_scope().expect_context::<(Signal<bool>, Signal<usize>)>();
    let g = node(&t, "g");
    assert_eq!(t.engine().grid_column_tracks(g), Some(&all[0][..]));
    let c = counts(&all);
    // A new value releases the old list and holds the new one.
    pick.set(1);
    t.run_until_idle();
    assert_eq!(counts(&all), [c[0] - 1, c[1] + 1, c[2], c[3]]);
    pick.set(2);
    t.run_until_idle();
    assert_eq!(counts(&all), [c[0] - 1, c[1], c[2] + 1, c[3]]);
    // Deleting the node releases every list it held (no leak): the Main column local, the
    // rows in the Main and PRESSED locals, and the binding's closure (which captured all).
    show.set(false);
    t.run_until_idle();
    assert_eq!(counts(&all), [c[0] - 2, c[1] - 1, c[2] - 1, c[3] - 3]);
    show.set(true);
    t.run_until_idle();
    assert_eq!(counts(&all), [c[0] - 1, c[1], c[2] + 1, c[3]]);
}

#[test]
fn shared_tracks_live_as_long_as_any_style_holds_them() {
    let l = lists(3);
    let (normal, theme, local) = (l[0].clone(), l[1].clone(), l[2].clone());
    let before = counts(&l);
    let style = StyleRef::from(StyleBuf::new().grid_column_tracks(GridTracks::Shared(normal.clone())));
    let class = StyleRef::from(StyleBuf::new().grid_column_tracks(GridTracks::Shared(theme.clone())));
    let (s2, c2) = (style.clone(), class.clone());
    let mut t = mount(move |_| {
        grid(GridTracks::Shared(local.clone()), grid_tracks![content], cells())
            .on_state(State::CHECKED, |s| s.style(s2.clone()))
            .op(move |cx, n| {
                cx.engine()
                    .add_theme_style(n, c2.clone(), Selector::state(State::custom::<0>()));
            })
            .test_id("g")
    });
    let g = node(&t, "g");
    // The app drops its own references: the styles on the node keep the lists alive.
    drop((style, class, normal, theme));
    let c = counts(&l);
    assert!(
        c[0] >= 2 && c[1] >= 2,
        "held by the node's styles: {c:?} (before: {before:?})"
    );
    // A normal (heap) style and a theme style override the local tracks, by state.
    t.engine_mut().set_state(g, State::CHECKED, true);
    t.run_until_idle();
    assert_eq!(t.engine().grid_column_tracks(g), Some(&l[0][..]));
    t.engine_mut().set_state(g, State::CHECKED, false);
    t.engine_mut().set_state(g, State::custom::<0>(), true);
    t.run_until_idle();
    assert_eq!(t.engine().grid_column_tracks(g), Some(&l[1][..]));
    // Removing the styles releases their lists; the template is still readable meanwhile.
    t.engine_mut().remove_style(g, None, None);
    assert_eq!(t.engine().grid_column_tracks(g), Some(&l[2][..]));
    assert_eq!(
        counts(&l),
        [1, 1, c[2]],
        "only the test holds the removed styles' lists"
    );
}

static TRACK_TRANSITION: Transition = Transition::of(Props::of(PropId::GridColumnTracks), Duration::ms(200));

#[test]
fn a_transition_in_flight_holds_its_tracks() {
    let l = lists(2);
    // The old template comes from a heap style, the new one from a CHECKED local.
    let base = StyleRef::from(StyleBuf::new().grid_column_tracks(GridTracks::Shared(l[0].clone())));
    let b = l[1].clone();
    let base2 = base.clone();
    let mut t = mount(move |_| {
        grid(&THREE[..2], grid_tracks![content], cells())
            .style(base2.clone())
            .transition(&TRACK_TRANSITION)
            .on_state(State::CHECKED, move |s| {
                s.grid_column_tracks(GridTracks::Shared(b.clone()))
            })
            .test_id("g")
    });
    drop(base);
    let g = node(&t, "g");
    // The local Main tracks win over the heap style: make the heap style the shown one.
    t.engine_mut()
        .remove_local_prop(g, PropId::GridColumnTracks, Selector::MAIN);
    t.run_until_idle();
    assert_eq!(t.engine().grid_column_tracks(g), Some(&l[0][..]));
    t.engine_mut().set_state(g, State::CHECKED, true);
    t.advance(Duration::ms(50));
    assert!(t.engine().transition_count() > 0, "the transition runs");
    // Mid-transition the old list is shown (non-interpolable values switch at the end), held
    // by the transition style too: removing the heap style it came from cannot free it.
    let held = l[0].strong_count();
    assert!(held >= 3, "the test, the heap style and the transition: {held}");
    t.engine_mut().remove_style(g, None, Some(Selector::MAIN));
    assert_eq!(l[0].strong_count(), held - 1);
    assert_eq!(
        t.engine().grid_column_tracks(g),
        Some(&l[0][..]),
        "still shown, still alive"
    );
    t.run_until_idle();
    assert_eq!(t.engine().transition_count(), 0);
    assert_eq!(t.engine().grid_column_tracks(g), Some(&l[1][..]));
    assert_eq!(l[0].strong_count(), 1, "released when the transition ended");
}
