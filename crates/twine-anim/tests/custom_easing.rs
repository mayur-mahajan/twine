//! `Easing::Custom`: an application-defined path (the `anim_gallery` example's smoothstep row)
//! receives the progress on the `0..=1024` scale and its result drives the value.

use twine_anim::Easing;

/// Smoothstep `3t² − 2t³` on the `0..=1024` scale.
fn smoothstep(t: u16) -> i32 {
    let t = i64::from(t);
    (t * t * (3 * 1024 - 2 * t) / (1024 * 1024)) as i32
}

#[test]
fn custom_path_maps_progress_through_the_function() {
    let e = Easing::Custom(smoothstep);
    // Endpoints and the midpoint of smoothstep are fixed points.
    assert_eq!(e.apply(0), 0);
    assert_eq!(e.apply(512), 512);
    assert_eq!(e.apply(1024), 1024);
    // Slow start and end, fast middle.
    assert!(e.apply(128) < 128);
    assert!(e.apply(896) > 896);
    // The eased progress scales the value range.
    assert_eq!(e.value(0, 100, 300), 100);
    assert_eq!(e.value(512, 100, 300), 200);
    assert_eq!(e.value(1024, 100, 300), 300);
}

#[test]
fn custom_path_never_sees_progress_beyond_the_end() {
    fn identity(t: u16) -> i32 {
        i32::from(t)
    }
    assert_eq!(Easing::Custom(identity).apply(5000), 1024);
    assert_eq!(Easing::Custom(smoothstep).value(u16::MAX, 0, 10), 10);
}
