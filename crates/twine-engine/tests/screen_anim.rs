//! Screen load animations (LVGL `lv_screen_load_anim`): every variant at 50 %, end state,
//! events, auto-delete, interruption, input blocking and idleness.

mod common;

use twine_core::{Color, Duration, Point, Rect};
use twine_engine::{EventCode, NodeId, ScreenAnim, Wake};
use twine_style::StyleProp;
use twine_testing::EngineHarness;

const T: Duration = Duration::ms(100);

struct Scene {
    h: EngineHarness,
    a: NodeId,
    b: NodeId,
    log: common::EvLog,
}

/// Screen `a` (active, light gray with a red box) and screen `b` (blue with a yellow box) on an
/// 80 × 60 display.
fn scene() -> Scene {
    let mut ids = None;
    let log = common::EvLog::default();
    let l = log.clone();
    let mut h = EngineHarness::new(80, 60).no_theme().mount_engine(|e| {
        let d = e.default_display().unwrap();
        let a = e.active_screen(d).unwrap();
        let b = e.create_screen(d).unwrap();
        common::style(
            e,
            a,
            &[
                StyleProp::BgColor(Color::hex(0xDD_DD_DD)),
                StyleProp::BgOpa(twine_core::Opa::COVER),
            ],
        );
        common::style(
            e,
            b,
            &[
                StyleProp::BgColor(Color::hex(0x20_40_C0)),
                StyleProp::BgOpa(twine_core::Opa::COVER),
            ],
        );
        common::boxed(e, a, Rect::from_xywh(8, 8, 24, 16), Color::RED);
        common::boxed(e, b, Rect::from_xywh(44, 30, 24, 20), Color::hex(0xF0_D0_00));
        for s in [a, b] {
            let l = l.clone();
            e.add_event_handler(s, twine_engine::EventFilter::All, move |cx, ev| {
                if matches!(
                    ev.code,
                    EventCode::ScreenLoadStart
                        | EventCode::ScreenLoaded
                        | EventCode::ScreenUnloadStart
                        | EventCode::ScreenUnloaded
                ) {
                    l.borrow_mut().push((ev.code, cx.node()));
                }
                twine_engine::EventResult::Continue
            });
        }
        ids = Some((a, b));
    });
    h.run_until_idle();
    let (a, b) = ids.unwrap();
    Scene { h, a, b, log }
}

#[test]
fn every_variant_at_50_percent_and_final_state() {
    let mut direct = scene();
    direct.h.engine_mut().load_screen(direct.b);
    direct.h.run_until_idle();
    let expected = direct.h.panel_rgb888();
    for anim in ScreenAnim::all(T) {
        if anim == ScreenAnim::None {
            continue;
        }
        let mut s = scene();
        let d = s.h.display();
        let t0 = s.h.now();
        s.h.engine_mut().load_screen_anim(s.b, anim);
        s.h.update();
        assert_eq!(s.h.engine().active_screen(d), Some(s.b), "{}", anim.name());
        assert_eq!(s.h.engine().prev_screen(d), Some(s.a));
        s.h.clock().set(t0 + Duration::ms(50));
        s.h.update();
        s.h.assert_panel_snapshot(&format!("screen_anim_{}_50", anim.name()));
        s.h.run_until_idle();
        assert!(!s.h.engine().screen_anim_running(d));
        assert_eq!(s.h.engine().prev_screen(d), None);
        assert_eq!(
            s.h.engine().coords(s.a),
            Rect::from_xywh(0, 0, 80, 60),
            "{}",
            anim.name()
        );
        assert_eq!(s.h.engine().coords(s.b), Rect::from_xywh(0, 0, 80, 60));
        assert!(
            s.h.panel_rgb888() == expected,
            "{}: final state differs from a direct load",
            anim.name()
        );
        assert_eq!(
            *s.log.borrow(),
            [
                (EventCode::ScreenUnloadStart, s.a),
                (EventCode::ScreenLoadStart, s.b),
                (EventCode::ScreenLoaded, s.b),
                (EventCode::ScreenUnloaded, s.a),
            ],
            "{}",
            anim.name()
        );
        s.h.assert_idle();
    }
}

#[test]
fn move_left_positions_follow_lvgl() {
    let mut s = scene();
    let t0 = s.h.now();
    s.h.engine_mut().load_screen_anim(s.b, ScreenAnim::MoveLeft(T));
    s.h.update();
    assert_eq!(s.h.engine().coords(s.b).x0, 80);
    assert_eq!(s.h.engine().coords(s.a).x0, 0);
    s.h.clock().set(t0 + Duration::ms(25));
    s.h.update();
    // Linear 25 %: new screen 80 → 0, old screen 0 → −80; children move along.
    assert_eq!(s.h.engine().coords(s.b).x0, 60);
    assert_eq!(s.h.engine().coords(s.a).x0, -20);
    let child = s.h.engine().tree().children(s.b).next().unwrap();
    assert_eq!(s.h.engine().coords(child).x0, 60 + 44);
}

#[test]
fn delay_keeps_old_screen_active() {
    let mut s = scene();
    let d = s.h.display();
    let t0 = s.h.now();
    s.h.engine_mut()
        .load_screen_anim(s.b, ScreenAnim::FadeIn(T).delay(Duration::ms(50)));
    let w = s.h.update();
    assert_eq!(s.h.engine().active_screen(d), Some(s.a));
    assert!(s.h.engine().screen_anim_running(d));
    // Nothing moves during the delay: the next wake-up is its end.
    assert!(matches!(w, Wake::At(t) if t == t0 + Duration::ms(50)), "{w:?}");
    assert_eq!(*s.log.borrow(), [(EventCode::ScreenUnloadStart, s.a)]);
    s.h.clock().set(t0 + Duration::ms(50));
    s.h.update();
    assert_eq!(s.h.engine().active_screen(d), Some(s.b));
}

#[test]
fn auto_delete_deletes_old_screen() {
    let mut s = scene();
    s.h.engine_mut()
        .load_screen_anim(s.b, ScreenAnim::OverLeft(T).auto_delete(true));
    s.h.run_until_idle();
    assert!(!s.h.engine().tree().contains(s.a));
    assert_eq!(s.h.engine().screens(s.h.display()), [s.b]);
    s.h.assert_idle();
}

#[test]
fn instant_load_with_auto_delete() {
    let mut s = scene();
    s.h.engine_mut()
        .load_screen_anim(s.b, ScreenAnim::None.auto_delete(true));
    assert!(!s.h.engine().tree().contains(s.a));
    assert_eq!(s.h.engine().active_screen(s.h.display()), Some(s.b));
}

#[test]
fn second_load_finishes_first() {
    let mut s = scene();
    let d = s.h.display();
    let c = s.h.engine_mut().create_screen(d).unwrap();
    let t0 = s.h.now();
    s.h.engine_mut()
        .load_screen_anim(s.b, ScreenAnim::MoveTop(T).auto_delete(true));
    s.h.update();
    s.h.clock().set(t0 + Duration::ms(30));
    s.h.update();
    s.h.engine_mut().load_screen_anim(c, ScreenAnim::FadeOut(T));
    // The first load completed at once: `b` loaded and in place, `a` deleted.
    assert!(!s.h.engine().tree().contains(s.a));
    assert_eq!(s.h.engine().coords(s.b), Rect::from_xywh(0, 0, 80, 60));
    assert_eq!(
        s.log.borrow()[..4],
        [
            (EventCode::ScreenUnloadStart, s.a),
            (EventCode::ScreenLoadStart, s.b),
            (EventCode::ScreenLoaded, s.b),
            (EventCode::ScreenUnloaded, s.a),
        ]
    );
    assert_eq!(s.log.borrow()[4], (EventCode::ScreenUnloadStart, s.b));
    s.h.run_until_idle();
    assert_eq!(s.h.engine().active_screen(d), Some(c));
    assert!(s.h.engine().tree().contains(s.b));
    assert_eq!(s.h.engine().anim_count(), 0);
}

#[test]
fn input_blocked_during_anim() {
    let mut s = scene();
    let clicks = common::EvLog::default();
    let target = s.h.engine().tree().children(s.b).next().unwrap();
    common::record(s.h.engine_mut(), target, &clicks);
    s.h.engine_mut().load_screen_anim(s.b, ScreenAnim::FadeIn(T));
    s.h.update();
    s.h.tap(Point::new(50, 35));
    assert!(
        !clicks.borrow().iter().any(|(c, _)| *c == EventCode::Clicked),
        "input reached a screen during the animation"
    );
    s.h.run_until_idle();
    s.h.tap(Point::new(50, 35));
    assert!(clicks.borrow().iter().any(|(c, _)| *c == EventCode::Clicked));
}

#[test]
fn idle_after_screen_anim() {
    let mut s = scene();
    s.h.engine_mut().load_screen_anim(s.b, ScreenAnim::MoveBottom(T));
    let dur = s.h.run_until_idle();
    assert!(dur >= T && dur <= T + Duration::ms(40), "{dur}");
    assert_eq!(s.h.engine().anim_count(), 0);
    s.h.assert_idle();
}

#[test]
fn deleting_screen_being_loaded_cancels() {
    let mut s = scene();
    let d = s.h.display();
    s.h.engine_mut()
        .load_screen_anim(s.b, ScreenAnim::MoveLeft(T).delay(Duration::ms(50)));
    s.h.update();
    s.h.engine_mut().delete(s.b).unwrap();
    assert!(!s.h.engine().screen_anim_running(d));
    assert_eq!(s.h.engine().anim_count(), 0);
    s.h.run_until_idle();
    assert_eq!(s.h.engine().active_screen(d), Some(s.a));
}
