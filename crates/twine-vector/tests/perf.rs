//! Performance smoke test (release builds only; debug builds skip it): filling a 100 × 100
//! circle into a 320 × 240 RGB565 buffer with warm caches stays below 150 µs on the host.

#[cfg(not(debug_assertions))]
#[test]
fn fill_circle_100_under_150_us() {
    use std::time::Instant;

    use twine_core::{Color, ColorFormat, Fx, Rect};
    use twine_render::{DrawBuf, Painter, RenderCaches};
    use twine_vector::{FxPoint, PainterVectorExt, Path, VectorDsc};

    let mut path = Path::new();
    path.circle(FxPoint::from_int(160, 120), Fx::from_int(50));
    let dsc = VectorDsc::fill(Color::BLUE);
    let mut px = vec![0u8; 320 * 240 * 2];
    let mut caches = RenderCaches::default();
    let area = Rect::from_xywh(0, 0, 320, 240);
    let mut draw = || {
        let buf = DrawBuf::new_packed(&mut px, ColorFormat::Rgb565, area).unwrap();
        Painter::new(buf, &mut caches).vector(&path, &dsc);
    };
    for _ in 0..20 {
        draw();
    }
    const N: u32 = 200;
    let t = Instant::now();
    for _ in 0..N {
        draw();
    }
    let per = t.elapsed() / N;
    println!("fill_circle_100: {per:?} per draw");
    assert!(per.as_micros() < 150, "{per:?} per draw");
}
