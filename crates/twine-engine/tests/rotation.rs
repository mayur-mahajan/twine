//! Software rotation, vsync, mono (I1) panels and the driver idle hook.

mod common;

use common::{boxed, style, white_screen};
use twine_core::{Color, ColorFormat, Instant, Opa, Rect, Rotation};
use twine_engine::{BufferMode, Engine, EngineConfig, InvalidateReason, Wake};
use twine_hal::{BufferSpec, DisplayDriver, DisplayInfo, DrawBufferMem};
use twine_style::StyleProp;
use twine_testing::{EngineHarness, MemoryDisplay, MemoryDisplayError, leak_buffer};

const W: u16 = 96;
const H: u16 = 64;

/// An asymmetric scene (so every rotation is distinguishable).
fn scene(e: &mut Engine) {
    let s = white_screen(e);
    boxed(e, s, Rect::from_xywh(2, 3, 40, 12), Color::RED);
    boxed(e, s, Rect::from_xywh(60, 40, 30, 20), Color::BLUE);
    let r = boxed(e, s, Rect::from_xywh(20, 22, 30, 30), Color::hex(0x002E_7D32));
    style(e, r, &[StyleProp::Radius(10), StyleProp::BorderWidth(3)]);
}

fn render(rotation: Rotation, spec: BufferSpec) -> EngineHarness {
    let mut h = EngineHarness::new(W, H)
        .no_theme()
        .buffers(spec)
        .rotation(rotation)
        .mount_engine(scene);
    h.run_until_idle();
    h
}

/// Rotates a logical `W × H` RGB888 image with the mapping of `Rect::rotate_in`.
fn rotate_reference(src: &[u8], rotation: Rotation) -> Vec<u8> {
    let (w, h) = (usize::from(W), usize::from(H));
    let (pw, _) = if rotation.swaps_axes() { (h, w) } else { (w, h) };
    let mut out = vec![0u8; src.len()];
    for y in 0..h {
        for x in 0..w {
            let (px, py) = match rotation {
                Rotation::Deg0 => (x, y),
                Rotation::Deg90 => (y, w - 1 - x),
                Rotation::Deg180 => (w - 1 - x, h - 1 - y),
                Rotation::Deg270 => (h - 1 - y, x),
            };
            let s = (y * w + x) * 3;
            let d = (py * pw + px) * 3;
            out[d..d + 3].copy_from_slice(&src[s..s + 3]);
        }
    }
    out
}

#[test]
fn software_rotation_matches_reference() {
    let reference = render(Rotation::Deg0, BufferSpec::PartialDouble { rows: 40 }).panel_rgb888();
    for rotation in [Rotation::Deg90, Rotation::Deg180, Rotation::Deg270] {
        let expect = rotate_reference(&reference, rotation);
        for spec in [
            BufferSpec::PartialSingle { rows: 9 },
            BufferSpec::PartialDouble { rows: 20 },
        ] {
            let got = render(rotation, spec).panel_rgb888();
            assert!(
                got == expect,
                "{rotation:?} {spec:?} differs from the rotated reference"
            );
        }
    }
}

#[test]
fn rotation_flush_areas_are_physical() {
    let mut h = render(Rotation::Deg90, BufferSpec::PartialDouble { rows: 16 });
    let d = h.display();
    let logical = Rect::from_xywh(10, 5, 30, 20);
    h.engine_mut()
        .invalidate_area(d, logical, InvalidateReason::Explicit);
    h.clock().advance(twine_core::Duration::ms(16));
    h.update();
    let flushed: Vec<Rect> = h.flushes().iter().map(|f| f.area).collect();
    // The chunks of the logical area (16 rows), each mapped to the portrait panel.
    let expect: Vec<Rect> = [Rect::new(10, 5, 40, 21), Rect::new(10, 21, 40, 25)]
        .iter()
        .map(|c| c.rotate_in(Rotation::Deg90, i32::from(W), i32::from(H)))
        .collect();
    assert_eq!(flushed, expect);
    let panel = Rect::new(0, 0, i32::from(H), i32::from(W));
    assert!(flushed.iter().all(|r| panel.contains_rect(r)));
}

/// A blocking display counting `wait_vsync` and `idle` calls.
struct Counting {
    inner: MemoryDisplay,
    vsync: u32,
    idle: u32,
}

impl DisplayDriver for Counting {
    type Error = MemoryDisplayError;
    fn info(&self) -> DisplayInfo {
        self.inner.info()
    }
    fn begin_flush(&mut self, area: Rect, buf: DrawBufferMem) -> Result<(), Self::Error> {
        self.inner.begin_flush(area, buf)
    }
    fn poll_flush(&mut self) -> Option<DrawBufferMem> {
        self.inner.poll_flush()
    }
    fn wait_vsync(&mut self) {
        self.vsync += 1;
    }
    fn idle(&mut self) {
        self.idle += 1;
    }
}

fn counting_engine() -> (Engine, twine_engine::DisplayId) {
    let mut e = Engine::new(EngineConfig::default()).unwrap();
    let info = DisplayInfo::new(W, H, ColorFormat::Rgb565);
    let d = e
        .add_display(
            Counting {
                inner: MemoryDisplay::new(info),
                vsync: 0,
                idle: 0,
            },
            BufferMode::Partial {
                a: leak_buffer(usize::from(W) * 2 * 8),
                b: Some(leak_buffer(usize::from(W) * 2 * 8)),
            },
        )
        .unwrap();
    (e, d)
}

#[test]
fn vsync_called_once_per_frame() {
    let (mut e, d) = counting_engine();
    e.set_display_vsync(d, true);
    let mut t = Instant::from_millis(0);
    for i in 1..=3u32 {
        // Full screen: 8 chunks per frame, but one vsync.
        e.invalidate_area(
            d,
            Rect::new(0, 0, i32::from(W), i32::from(H)),
            InvalidateReason::Explicit,
        );
        assert_eq!(e.step(t), Wake::Idle);
        assert_eq!(e.driver::<Counting>(d).unwrap().vsync, i);
        assert_eq!(e.last_stats(d).chunks, 8);
        t += twine_core::Duration::ms(20);
    }
    e.set_display_vsync(d, false);
    e.invalidate_area(d, Rect::new(0, 0, 4, 4), InvalidateReason::Explicit);
    e.step(t);
    assert_eq!(e.driver::<Counting>(d).unwrap().vsync, 3);
}

#[test]
fn idle_called_on_transition_only() {
    let (mut e, d) = counting_engine();
    let mut t = Instant::from_millis(0);
    e.step(t); // first frame (full screen); the last chunk is reclaimed on the next step
    for _ in 0..5 {
        t += twine_core::Duration::ms(20);
        assert_eq!(e.step(t), Wake::Idle);
    }
    assert_eq!(e.driver::<Counting>(d).unwrap().idle, 1);
    e.invalidate_area(d, Rect::new(0, 0, 4, 4), InvalidateReason::Explicit);
    for _ in 0..5 {
        t += twine_core::Duration::ms(20);
        e.step(t);
    }
    assert_eq!(e.driver::<Counting>(d).unwrap().idle, 2);
}

#[test]
fn i1_output_thresholds_and_aligns() {
    let mut h = EngineHarness::new(128, 64)
        .no_theme()
        .format(ColorFormat::I1)
        .align(8)
        .buffers(BufferSpec::PartialDouble { rows: 16 })
        .mount_engine(|e| {
            let s = common::screen(e);
            style(
                e,
                s,
                &[StyleProp::BgColor(Color::BLACK), StyleProp::BgOpa(Opa::COVER)],
            );
            let a = boxed(e, s, Rect::from_xywh(6, 6, 40, 24), Color::WHITE);
            style(e, a, &[StyleProp::Radius(8)]);
            boxed(e, s, Rect::from_xywh(60, 10, 30, 30), Color::hex(0x80_80_80)); // luminance 128: white
            boxed(e, s, Rect::from_xywh(95, 10, 20, 30), Color::hex(0x7E_7E_7E)); // below: black
            let b = boxed(e, s, Rect::from_xywh(20, 38, 90, 20), Color::WHITE);
            style(
                e,
                b,
                &[
                    StyleProp::BorderWidth(4),
                    StyleProp::BorderColor(Color::BLACK),
                    StyleProp::Radius(10),
                ],
            );
        });
    h.run_until_idle();
    assert_eq!(h.pixel(70, 20), Color::WHITE);
    assert_eq!(h.pixel(100, 20), Color::BLACK);
    let d = h.display();
    h.engine_mut()
        .invalidate_area(d, Rect::new(3, 5, 21, 30), InvalidateReason::Explicit);
    h.clock().advance(twine_core::Duration::ms(16));
    h.update();
    assert!(!h.flushes().is_empty());
    for f in h.flushes() {
        let a = f.area;
        assert!(
            a.x0 % 8 == 0 && a.y0 % 8 == 0 && a.x1 % 8 == 0 && a.y1 % 8 == 0,
            "{a}"
        );
    }
    h.assert_snapshot("mono_boxes");
    // Rotated mono panels work too.
    let mut r = EngineHarness::new(128, 64)
        .no_theme()
        .format(ColorFormat::I1)
        .align(8)
        .rotation(Rotation::Deg90)
        .mount_engine(|e| {
            let s = common::screen(e);
            style(
                e,
                s,
                &[StyleProp::BgColor(Color::BLACK), StyleProp::BgOpa(Opa::COVER)],
            );
            boxed(e, s, Rect::from_xywh(8, 8, 16, 8), Color::WHITE);
        });
    r.run_until_idle();
    // Logical (8..24, 8..16) → physical x = y, y = 127 - x.
    assert_eq!(r.pixel(10, 127 - 10), Color::WHITE);
    assert_eq!(r.pixel(20, 127 - 10), Color::BLACK);
}
