//! Rendering: tolerant snapshots of shapes, transforms, strokes, parenting, opacity, fill rules,
//! trim paths, repeaters, masks, mattes and precomps, plus pixel checks of the semantics.
//!
//! Snapshots are compared with a tolerance of 1 % of the pixels differing by more than 8 per
//! channel (floating-point geometry may differ in the last bit between platforms). Reference
//! procedure: references are rendered once by this implementation (`cargo xtask snapshots
//! --update`), then each PNG is compared by eye with the same JSON file (`tests/data/`) played
//! in lottie-web (for example the `LottieFiles` web player, paused at the same frame).
#![allow(clippy::float_cmp, clippy::cast_precision_loss)] // exact float values are expected; small ints become floats

use twine_core::{Color, ColorFormat, Rect};
use twine_lottie::{LottiePlayer, load};
use twine_render::RenderConfig;
use twine_testing::{RenderHarness, Tolerance, assert_rgb_snapshot, snapshot_config};

fn data(name: &str) -> Vec<u8> {
    std::fs::read(format!("{}/tests/data/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

fn player(name: &str) -> LottiePlayer {
    LottiePlayer::new(load(&data(name)).unwrap())
}

fn player_json(json: &str) -> LottiePlayer {
    LottiePlayer::new(load(json.as_bytes()).unwrap())
}

/// Renders `frame` of `p` into a `size × size` harness (white background).
fn render(p: &mut LottiePlayer, frame: f32, size: u16) -> RenderHarness {
    let mut h = RenderHarness::new(size, size, ColorFormat::Xrgb8888);
    let area = h.area();
    h.paint(|pt| p.render_frame(frame, pt, area));
    h
}

/// Tolerant snapshot: at most 1 % of the pixels may differ by more than 8.
fn snap(h: &RenderHarness, name: &str) {
    let a = h.area();
    let (w, hh) = (a.width() as u32, a.height() as u32);
    assert_rgb_snapshot(
        &snapshot_config!(),
        name,
        w,
        hh,
        &h.rgb888(),
        Tolerance::new(w * hh / 100, 8),
    );
}

fn px(h: &RenderHarness, x: i32, y: i32) -> [u8; 3] {
    let w = h.area().width();
    let i = ((y * w + x) * 3) as usize;
    let rgb = h.rgb888();
    [rgb[i], rgb[i + 1], rgb[i + 2]]
}

fn is_white(c: [u8; 3]) -> bool {
    c.iter().all(|&v| v >= 250)
}

#[test]
fn shapes_frame0_snapshot() {
    let h = render(&mut player("shapes.json"), 0.0, 150);
    snap(&h, "shapes_frame0");
    // Solid background layer (#e8eaf0) visible between shapes.
    assert_eq!(px(&h, 75, 3), [0xe8, 0xea, 0xf0]);
    // Red rectangle centre, green ellipse centre.
    assert_eq!(px(&h, 37, 37), [230, 51, 51]);
    assert_eq!(px(&h, 37, 112), [51, 179, 77]);
}

#[test]
fn nested_group_transforms_snapshot() {
    let h = render(&mut player("nested.json"), 0.0, 150);
    snap(&h, "nested_group_transforms");
}

#[test]
fn stroke_dash_snapshot() {
    let h = render(&mut player("dash.json"), 0.0, 150);
    snap(&h, "stroke_dash");
}

#[test]
fn parenting_chain_snapshot() {
    let h = render(&mut player("parenting.json"), 0.0, 150);
    snap(&h, "parenting_chain");
    // The child sits at parent (40, 40) + rotate(30°)·(28, 0) = (64.2, 54).
    let c = px(&h, (64.2 * 1.5) as i32, (54.0 * 1.5) as i32);
    assert!(c[1] > 150 && c[0] < 100, "{c:?}");
}

#[test]
fn layer_opacity_snapshot() {
    let h = render(&mut player("opacity.json"), 0.0, 150);
    snap(&h, "layer_opacity");
    // Rendered as one layer: the overlap of the two rectangles is not darker than the red part.
    let red_only = px(&h, 30, 40);
    let overlap = px(&h, 67, 75);
    assert_eq!(red_only, overlap);
    assert!(
        red_only[0] > 230 && red_only[1] > 100 && red_only[1] < 160,
        "{red_only:?}"
    );
    // Blue only (below the red one), half transparent over white.
    let blue = px(&h, 67, 117);
    assert!(blue[2] > 230 && blue[0] < 160 && blue[0] > 100, "{blue:?}");
}

#[test]
fn fill_rule_evenodd_snapshot() {
    let h = render(&mut player("evenodd.json"), 0.0, 150);
    snap(&h, "fill_rule_evenodd");
    assert!(is_white(px(&h, 42, 75)), "even-odd hole");
    assert!(!is_white(px(&h, 111, 75)), "non-zero filled");
    assert!(!is_white(px(&h, 42, 45)), "even-odd ring");
}

#[test]
fn trim_half_path_snapshot() {
    let h = render(&mut player("trim.json"), 0.0, 150);
    snap(&h, "trim_half_path");
    // Ellipses start at the top and run clockwise: 0..50 % is the right half.
    assert!(!is_white(px(&h, 75 + 52, 75)), "right half drawn");
    assert!(is_white(px(&h, 75 - 52, 75)), "left half trimmed");
}

/// A stroked circle (r = 40 around (50, 50)) trimmed to the first quarter with an offset.
fn quarter(offset: f32, mode: u8) -> String {
    format!(
        r#"{{"fr":30,"ip":0,"op":1,"w":100,"h":100,"layers":[{{"ty":4,"ks":{{}},"shapes":[
        {{"ty":"el","p":{{"k":[50,50]}},"s":{{"k":[80,80]}}}},
        {{"ty":"st","c":{{"k":[0,0,0,1]}},"o":{{"k":100}},"w":{{"k":6}},"lc":1}},
        {{"ty":"tm","s":{{"k":0}},"e":{{"k":25}},"o":{{"k":{offset}}},"m":{mode}}}]}}]}}"#
    )
}

#[test]
fn trim_offset_wraps() {
    // No offset: top → right quadrant (upper right arc).
    let h = render(&mut player_json(&quarter(0.0, 1)), 0.0, 100);
    assert!(!is_white(px(&h, 78, 22)));
    assert!(is_white(px(&h, 22, 22)));
    // 270°: starts at 75 % (left), wraps over the end back to the top: upper left arc.
    let h = render(&mut player_json(&quarter(270.0, 1)), 0.0, 100);
    assert!(!is_white(px(&h, 22, 22)));
    assert!(is_white(px(&h, 78, 22)));
    // −90° is the same as 270°.
    let a = render(&mut player_json(&quarter(-90.0, 1)), 0.0, 100);
    assert_eq!(a.rgb888(), h.rgb888());
    // 405° wraps to 45°.
    let a = render(&mut player_json(&quarter(405.0, 1)), 0.0, 100);
    let b = render(&mut player_json(&quarter(45.0, 1)), 0.0, 100);
    assert_eq!(a.rgb888(), b.rgb888());
}

#[test]
fn trim_individual_mode() {
    // Two horizontal lines of equal length; trimmed to 0..50 %.
    let json = |m: u8| {
        format!(
            r#"{{"fr":30,"ip":0,"op":1,"w":100,"h":100,"layers":[{{"ty":4,"ks":{{}},"shapes":[
            {{"ty":"sh","ks":{{"k":{{"c":false,"v":[[10,30],[90,30]],"i":[[0,0],[0,0]],"o":[[0,0],[0,0]]}}}}}},
            {{"ty":"sh","ks":{{"k":{{"c":false,"v":[[10,70],[90,70]],"i":[[0,0],[0,0]],"o":[[0,0],[0,0]]}}}}}},
            {{"ty":"st","c":{{"k":[0,0,0,1]}},"o":{{"k":100}},"w":{{"k":6}},"lc":1}},
            {{"ty":"tm","s":{{"k":0}},"e":{{"k":50}},"o":{{"k":0}},"m":{m}}}]}}]}}"#
        )
    };
    // Simultaneously: the first half of each line.
    let h = render(&mut player_json(&json(1)), 0.0, 100);
    assert!(!is_white(px(&h, 30, 30)) && !is_white(px(&h, 30, 70)));
    assert!(is_white(px(&h, 70, 30)) && is_white(px(&h, 70, 70)));
    // Individually: the paths form one sequence, so the first line is complete and the second
    // one is gone.
    let h = render(&mut player_json(&json(2)), 0.0, 100);
    assert!(!is_white(px(&h, 30, 30)) && !is_white(px(&h, 70, 30)));
    assert!(is_white(px(&h, 30, 70)) && is_white(px(&h, 70, 70)));
}

#[test]
fn repeater_three_copies_snapshot() {
    let h = render(&mut player("repeater.json"), 0.0, 150);
    snap(&h, "repeater_three_copies");
    // Copies at x = 15, 45, 75 with opacity 100 %, 70 %, 40 %.
    let c: Vec<u8> = [15, 45, 75].iter().map(|&x| px(&h, x * 3 / 2, 75)[1]).collect();
    assert!(c[0] < c[1] && c[1] < c[2], "{c:?}");
    assert!(is_white(px(&h, 105 * 3 / 2, 75)), "no fourth copy");
}

#[test]
fn mask_add_snapshot() {
    let h = render(&mut player("mask_add.json"), 0.0, 150);
    snap(&h, "mask_add");
    assert!(!is_white(px(&h, 75, 75)), "inside the circle");
    assert!(is_white(px(&h, 15, 15)), "outside the circle");
}

#[test]
fn mask_subtract_snapshot() {
    let h = render(&mut player("mask_subtract.json"), 0.0, 150);
    snap(&h, "mask_subtract");
    assert!(!is_white(px(&h, 75, 40)), "circle above the subtracted rectangle");
    assert!(is_white(px(&h, 90, 75)), "subtracted");
    assert!(is_white(px(&h, 15, 15)));
}

#[test]
fn mask_inverted() {
    let h = render(&mut player("mask_inverted.json"), 0.0, 150);
    snap(&h, "mask_inverted");
    // Inside the circle: hidden; outside (but inside the blue square): blue at 60 %.
    assert!(is_white(px(&h, 75, 75)));
    let c = px(&h, 15, 15);
    assert!(c[2] > 230 && c[0] > 110 && c[0] < 150, "{c:?}");
}

#[test]
fn alpha_matte_snapshot() {
    let h = render(&mut player("matte.json"), 0.0, 150);
    snap(&h, "alpha_matte");
    // Alpha matte: the red bar shows only inside the circle.
    assert!(!is_white(px(&h, 45, 75)));
    assert!(is_white(px(&h, 45, 20)));
    // Inverted matte: the blue bar shows only outside its circle.
    assert!(is_white(px(&h, 111, 75)));
    assert!(!is_white(px(&h, 111, 20)));
    // Matte source layers are not drawn by themselves (white circles would hide the bars).
    assert_eq!(px(&h, 111, 75), [255, 255, 255]);
    assert!(px(&h, 45, 75)[0] > 200 && px(&h, 45, 75)[1] < 100);
}

#[test]
fn precomp_time_remap_frame() {
    let mut p = player("precomp.json");
    // Square centre x in the asset: 10 + 80 · asset_frame / 80; asset frame = tm(frame) · 30,
    // tm goes 0 → 2 s over comp frames 0 → 40.
    for (frame, expect_x) in [(0.0, 10.0), (10.0, 25.0), (20.0, 40.0), (40.0, 70.0)] {
        let h = render(&mut p, frame, 100);
        let rgb = h.rgb888();
        let xs: Vec<i32> = (0..100)
            .filter(|&x| {
                let i = (50 * 100 + x) as usize * 3;
                rgb[i] > 200 && rgb[i + 1] < 100
            })
            .collect();
        let centre = (xs.first().unwrap() + xs.last().unwrap()) as f32 / 2.0;
        assert!(
            (centre - expect_x).abs() <= 1.0,
            "frame {frame}: {centre} vs {expect_x}"
        );
    }
}

#[test]
fn luma_matte_warns_once() {
    let comp = load(&data("unsupported.json")).unwrap();
    assert_eq!(
        comp.unsupported().iter().filter(|w| *w == "luma matte").count(),
        1
    );
    // Rendering ignores the luma mattes (the targets are drawn unmatted) without panicking.
    let mut p = LottiePlayer::new(comp);
    let h = render(&mut p, 0.0, 100);
    assert!(!is_white(px(&h, 50, 50)));
}

/// The demo samples at a few frames, side by side (for visual review).
#[test]
fn samples_frames_snapshot() {
    for name in ["loader", "check", "heart", "masked"] {
        let mut p = player(&format!("{name}.json"));
        let frames = [0.0, 10.0, 20.0, 30.0, 45.0];
        let mut h = RenderHarness::new(100 * frames.len() as u16, 100, ColorFormat::Xrgb8888);
        h.clear(Color::WHITE);
        h.paint(|pt| {
            for (i, &f) in frames.iter().enumerate() {
                p.render_frame(f, pt, Rect::from_xywh(i as i32 * 100, 0, 100, 100));
            }
        });
        snap(&h, &format!("sample_{name}"));
    }
}

#[test]
fn all_samples_render_every_frame() {
    let dir = format!("{}/tests/data", env!("CARGO_MANIFEST_DIR"));
    for e in std::fs::read_dir(dir).unwrap() {
        let path = e.unwrap().path();
        if path.extension().is_none_or(|x| x != "json") {
            continue;
        }
        let comp = load(&std::fs::read(&path).unwrap()).unwrap();
        let (ip, op) = (comp.ip, comp.op);
        let mut p = LottiePlayer::new(comp);
        let mut h = RenderHarness::new(64, 48, ColorFormat::Rgb565);
        let mut f = ip - 2.0;
        while f < op + 2.0 {
            let area = h.area();
            h.paint(|pt| p.render_frame(f, pt, area));
            f += 0.75;
        }
    }
}

#[test]
fn chunked_and_small_budget_match_full_render() {
    for name in [
        "masked.json",
        "matte.json",
        "opacity.json",
        "mask_subtract.json",
        "heart.json",
    ] {
        let mut p = player(name);
        let full = render(&mut p, 12.0, 120);
        // Partial buffers of 16 rows.
        let mut h = RenderHarness::new(120, 120, ColorFormat::Xrgb8888);
        let area = h.area();
        h.paint_chunked(16, |pt| p.render_frame(12.0, pt, area));
        assert_eq!(h.rgb888(), full.rgb888(), "{name}: chunked");
        // A layer budget of 2 KiB: offscreen layers are rendered in strips of 4 rows.
        let cfg = RenderConfig {
            layer_buf_bytes: 2048,
            ..RenderConfig::default()
        };
        let mut h = RenderHarness::new(120, 120, ColorFormat::Xrgb8888).with_config(cfg);
        h.paint(|pt| p.render_frame(12.0, pt, area));
        assert_eq!(h.rgb888(), full.rgb888(), "{name}: small budget");
    }
}

#[test]
fn render_is_deterministic_and_clipped() {
    let mut p = player("heart.json");
    let a = render(&mut p, 17.3, 90);
    let b = render(&mut p, 17.3, 90);
    assert_eq!(a.rgb888(), b.rgb888());
    // Drawing into a sub-rectangle never touches pixels outside it.
    let mut h = RenderHarness::new(120, 120, ColorFormat::Xrgb8888);
    h.paint(|pt| p.render_frame(8.0, pt, Rect::from_xywh(30, 30, 40, 60)));
    let rgb = h.rgb888();
    for y in 0..120 {
        for x in 0..120 {
            if !(30..70).contains(&x) || !(30..90).contains(&y) {
                let i = (y * 120 + x) * 3;
                assert_eq!(&rgb[i..i + 3], &[255, 255, 255], "({x}, {y})");
            }
        }
    }
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig::with_cases(300))]

    /// Mutated sample files that still load render every frame without panicking.
    #[test]
    fn mutated_samples_render_without_panic(
        idx in 0usize..64,
        flips in proptest::collection::vec((0usize..1_000_000, 0u8..10), 1..6),
        frame in -10.0f32..80.0,
    ) {
        let names = ["loader.json", "check.json", "heart.json", "masked.json", "matte.json", "precomp.json", "repeater.json", "trim.json"];
        let mut bytes = data(names[idx % names.len()]);
        let digits = b"0123456789";
        for (pos, d) in flips {
            // Replace a digit with another digit: keeps the JSON valid, changes the numbers.
            let n = bytes.len();
            let mut i = pos % n;
            while i < n && !bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i < n {
                bytes[i] = digits[usize::from(d)];
            }
        }
        if let Ok(comp) = load(&bytes) {
            let mut p = LottiePlayer::new(comp);
            let mut h = RenderHarness::new(48, 48, ColorFormat::Rgb565);
            let area = h.area();
            h.paint(|pt| p.render_frame(frame, pt, area));
        }
    }
}
