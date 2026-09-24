//! Keyframe evaluation: linear, hold, bezier easing (per axis), spatial tangents, layer time
//! mapping, and allocation-free sequential evaluation.
#![allow(clippy::float_cmp, clippy::cast_precision_loss)] // exact float values are expected; small ints become floats

use twine_lottie::eval::cubic_bezier_ease;
use twine_lottie::load;
use twine_lottie::model::{Animatable, Ease, Keyframe, Keyframes, PathData, Position};
use twine_testing::alloc::{CountingAllocator, count_allocs};

#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator;

fn kf<T>(t: f32, s: T) -> Keyframe<T> {
    Keyframe {
        t,
        s,
        e: None,
        hold: false,
        ease: None,
        to: None,
        ti: None,
    }
}

fn near(a: f32, b: f32, eps: f32) -> bool {
    (a - b).abs() <= eps
}

#[test]
fn linear_keyframes_midpoint() {
    let a = Animatable::Keyframed(Keyframes::new(vec![
        kf(0.0, [0.0, 100.0]),
        kf(20.0, [50.0, 0.0]),
        kf(40.0, [50.0, 50.0]),
    ]));
    assert_eq!(a.value(10.0), [25.0, 50.0]);
    assert_eq!(a.value(30.0), [50.0, 25.0]);
    // Before the first / after the last keyframe: clamped.
    assert_eq!(a.value(-5.0), [0.0, 100.0]);
    assert_eq!(a.value(99.0), [50.0, 50.0]);
    // Exactly on a keyframe.
    assert_eq!(a.value(20.0), [50.0, 0.0]);
}

#[test]
fn hold_keyframe_step() {
    let mut k0 = kf(0.0, 1.0f32);
    k0.hold = true;
    let a = Animatable::Keyframed(Keyframes::new(vec![k0, kf(10.0, 5.0), kf(20.0, 9.0)]));
    assert_eq!(a.value(0.0), 1.0);
    assert_eq!(a.value(9.99), 1.0);
    assert_eq!(a.value(10.0), 5.0);
    assert_eq!(a.value(15.0), 7.0); // the second keyframe interpolates linearly
}

/// Values of lottie-web's `BezierEasing` for the After Effects default ease (0.33, 0, 0.67, 1)
/// and an ease-out (0, 0, 0.2, 1), computed in double precision by bisection.
#[test]
fn bezier_ease_matches_reference_values() {
    let ae = [
        (0.1, 0.028_39),
        (0.25, 0.157_3),
        (0.5, 0.5),
        (0.75, 0.842_7),
        (0.9, 0.971_61),
    ];
    for (x, y) in ae {
        let v = cubic_bezier_ease(0.33, 0.0, 0.67, 1.0, x);
        assert!(near(v, y, 0.001), "{x}: {v} vs {y}");
    }
    let out = [
        (0.1, 0.303_85),
        (0.25, 0.577_57),
        (0.5, 0.839_25),
        (0.75, 0.964_22),
        (0.9, 0.994_6),
    ];
    for (x, y) in out {
        let v = cubic_bezier_ease(0.0, 0.0, 0.2, 1.0, x);
        assert!(near(v, y, 0.001), "{x}: {v} vs {y}");
    }
    // Through a keyframe's easing.
    let mut k0 = kf(0.0, 0.0f32);
    k0.ease = Some(Ease {
        out: [[0.33, 0.0]; 4],
        inn: [[0.67, 1.0]; 4],
        axes: 1,
    });
    let a = Animatable::Keyframed(Keyframes::new(vec![k0, kf(100.0, 100.0)]));
    assert!(near(a.value(25.0), 15.73, 0.1));
}

#[test]
fn per_axis_easing() {
    let mut k0 = kf(0.0, [0.0f32, 0.0]);
    // x: linear, y: ease-in (0.5, 0) → (1, 1).
    k0.ease = Some(Ease {
        out: [[0.0, 0.0], [0.5, 0.0], [0.5, 0.0], [0.5, 0.0]],
        inn: [[1.0, 1.0], [1.0, 1.0], [1.0, 1.0], [1.0, 1.0]],
        axes: 2,
    });
    let a = Animatable::Keyframed(Keyframes::new(vec![k0, kf(10.0, [100.0, 100.0])]));
    let v = a.value(5.0);
    assert!(near(v[0], 50.0, 1e-3), "{v:?}");
    let y = cubic_bezier_ease(0.5, 0.0, 1.0, 1.0, 0.5) * 100.0;
    assert!(near(v[1], y, 1e-3) && v[1] < 40.0, "{v:?}");
    // Loaded from JSON with per-axis arrays.
    let json = br#"{"fr":30,"ip":0,"op":30,"w":10,"h":10,"layers":[{"ty":4,"ks":{"s":{"a":1,"k":[
        {"t":0,"s":[0,0],"o":{"x":[0,0.5],"y":[0,0]},"i":{"x":[1,1],"y":[1,1]}},{"t":10,"s":[100,100]}]}}}]}"#;
    let comp = load(json).unwrap();
    assert_eq!(comp.layers[0].ks.scale.value(5.0), v);
}

#[test]
fn position_spatial_tangent_curve() {
    let mut k0 = kf(0.0, [0.0f32, 0.0]);
    k0.to = Some([0.0, -40.0]);
    k0.ti = Some([0.0, -40.0]);
    let a = Animatable::Keyframed(Keyframes::new(vec![k0, kf(10.0, [100.0, 0.0])]));
    // Cubic (0,0) (0,-40) (100,-40) (100,0) at t = 0.5: (50, -30).
    let mid = a.value(5.0);
    assert!(near(mid[0], 50.0, 1e-3) && near(mid[1], -30.0, 1e-3), "{mid:?}");
    let q = a.value(2.5);
    // B(0.25) = 0.421875·p1 + 0.140625·p2 + 0.015625·p3
    assert!(near(q[0], 15.625, 1e-3) && near(q[1], -22.5, 1e-3), "{q:?}");
}

#[test]
fn layer_outside_ip_op_hidden() {
    let json = br#"{"fr":30,"ip":0,"op":100,"w":10,"h":10,"layers":[
        {"ty":4,"ip":10,"op":20,"st":5,"sr":2,"ks":{}},
        {"ty":4,"ip":0,"op":100,"hd":true,"ks":{}}]}"#;
    let comp = load(json).unwrap();
    let l = &comp.layers[0];
    assert!(!l.is_visible_at(9.9));
    assert!(l.is_visible_at(10.0));
    assert!(l.is_visible_at(19.9));
    assert!(!l.is_visible_at(20.0));
    assert_eq!(l.local_frame(15.0), 5.0); // (15 − st 5) / sr 2
    assert!(!comp.layers[1].is_visible_at(50.0));
}

#[test]
fn path_keyframes_lerp_and_mismatch_hold() {
    let sq = |s: f32| PathData {
        closed: true,
        v: vec![[0.0, 0.0], [s, 0.0], [s, s]],
        i: vec![[0.0, 0.0]; 3],
        o: vec![[0.0, 0.0]; 3],
    };
    let a = Animatable::Keyframed(Keyframes::new(vec![kf(0.0, sq(10.0)), kf(10.0, sq(20.0))]));
    let mut out = PathData::default();
    a.eval_into(5.0, &mut out);
    assert_eq!(out.v, [[0.0, 0.0], [15.0, 0.0], [15.0, 15.0]]);
    let mut other = sq(30.0);
    other.v.push([1.0, 1.0]);
    other.i.push([0.0, 0.0]);
    other.o.push([0.0, 0.0]);
    let b = Animatable::Keyframed(Keyframes::new(vec![kf(0.0, sq(10.0)), kf(10.0, other)]));
    b.eval_into(5.0, &mut out);
    assert_eq!(out, sq(10.0)); // different vertex counts: hold the start
}

#[test]
fn sequential_evaluation_allocates_nothing() {
    let comp =
        load(&std::fs::read(format!("{}/tests/data/masked.json", env!("CARGO_MANIFEST_DIR"))).unwrap())
            .unwrap();
    let mask = &comp.layers[1].masks[0].pt;
    let Position::Combined(_) = comp.layers[0].ks.position else {
        panic!()
    };
    let mut out = PathData::default();
    mask.eval_into(0.0, &mut out); // warm up the output buffers
    let ((), stats) = count_allocs(|| {
        for f in 0..600 {
            let frame = f as f32 * 0.1;
            mask.eval_into(frame, &mut out);
            let _ = comp.layers[0].ks.rotation.value(frame);
        }
    });
    assert_eq!(stats.allocs, 0, "{stats:?}");
}
