//! Display integrity (R5.S06): frame observers, the off-screen re-render and read-back check,
//! safety-critical nodes, the heartbeat and the fail-safe screen.
//!
//! req: SAF-014, SAF-015, SAF-016, SAF-017, SAF-018
#![cfg(feature = "display-integrity")]

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use common::{boxed, style, white_screen};
use twine_core::crc::Crc32;
use twine_core::fault::FaultKind;
use twine_core::{Color, ColorFormat, Duration, Instant, Opa, Rect};
use twine_engine::{
    BufferMode, BufferSpec, DisplayId, Engine, EngineConfig, EngineError, FrameChunk, FrameObserver,
    FrameReport, NodeId, ObjFlags, State,
};
use twine_hal::DisplayInfo;
use twine_style::{
    GradDir, Length, PropId, Props, Radius, Selector, Style, StyleProp, Transition, TransitionRef,
};
use twine_testing::{MemoryDisplay, MockFramebufferDisplay};

/// What an observer saw: every chunk `(display, area, checksum of its rows)` and every frame.
#[derive(Default)]
struct Seen {
    chunks: Vec<(DisplayId, Rect, Crc32)>,
    frames: Vec<FrameReport>,
}

#[derive(Clone, Default)]
struct Recorder(Rc<RefCell<Seen>>);

impl FrameObserver for Recorder {
    fn chunk(&mut self, chunk: &FrameChunk<'_>) {
        self.0
            .borrow_mut()
            .chunks
            .push((chunk.display, chunk.area, chunk.crc()));
    }
    fn frame_done(&mut self, report: &FrameReport) {
        self.0.borrow_mut().frames.push(*report);
    }
}

fn engine_with(info: DisplayInfo, spec: BufferSpec) -> (Engine, DisplayId) {
    let mut e = Engine::new(EngineConfig::default()).unwrap();
    let d = e
        .add_display(MemoryDisplay::new(info), BufferMode::alloc(spec))
        .unwrap();
    (e, d)
}

fn rgb565(w: u16, h: u16) -> (Engine, DisplayId) {
    engine_with(
        DisplayInfo::new(w, h, ColorFormat::Rgb565),
        BufferSpec::PartialDouble { rows: 10 },
    )
}

/// The checksum of `rect` of the panel (packed rows, read back like `Engine::read_back`).
fn panel_crc(e: &mut Engine, d: DisplayId, rect: Rect) -> Crc32 {
    let format = e.display_info(d).unwrap().format;
    let mut buf = vec![0u8; format.stride(rect.width() as u32) as usize * rect.height() as usize];
    e.read_back(d, rect, &mut buf).unwrap();
    Crc32::of(&buf)
}

/// A screen with a rounded gradient box with a shadow and border (circle, gradient and shadow
/// caches) and a plain box above it.
fn scene(e: &mut Engine) -> (NodeId, NodeId) {
    let s = white_screen(e);
    let fancy = boxed(e, s, Rect::from_xywh(10, 8, 40, 30), Color::hex(0x0015_65C0));
    style(
        e,
        fancy,
        &[
            StyleProp::Radius(Radius::Px(9).into()),
            StyleProp::BgGradientColor(Color::hex(0x00FF_CC80).into()),
            StyleProp::BgGradientDir(GradDir::Ver),
            StyleProp::BorderWidth(Length::Px(2).into()),
            StyleProp::BorderColor(Color::BLACK.into()),
            StyleProp::ShadowWidth(8),
            StyleProp::ShadowColor(Color::BLACK.into()),
        ],
    );
    let plain = boxed(e, s, Rect::from_xywh(40, 30, 20, 16), Color::RED);
    (fancy, plain)
}

// ---- Frame observer --------------------------------------------------------------------------

#[test]
fn observer_sees_every_chunk_of_every_frame() {
    let (mut e, d) = rgb565(80, 48);
    let seen = Recorder::default();
    e.set_frame_observer(seen.clone());
    let (fancy, _) = scene(&mut e);
    e.step(Instant::from_millis(0));
    {
        let s = seen.0.borrow();
        let flushes = e.driver::<MemoryDisplay>(d).unwrap().flushes().to_vec();
        assert_eq!(s.chunks.len(), flushes.len(), "one observation per flushed chunk");
        for ((disp, area, _), f) in s.chunks.iter().zip(&flushes) {
            assert_eq!((*disp, *area), (d, f.area));
        }
        assert_eq!(s.frames.len(), 1);
        assert_eq!(s.frames[0].display, d);
        assert_eq!(usize::from(s.frames[0].stats.chunks), s.chunks.len());
    }
    // The chunks hold what reached the panel (they do not overlap within a frame).
    let chunks = seen.0.borrow().chunks.clone();
    for (_, area, crc) in chunks {
        assert_eq!(panel_crc(&mut e, d, area), crc, "chunk {area}");
    }
    // A change: only its chunks, one more frame.
    e.set_local_prop(fancy, Selector::MAIN, StyleProp::BgColor(Color::GREEN.into()));
    seen.0.borrow_mut().chunks.clear();
    e.step(Instant::from_millis(100));
    let s = seen.0.borrow();
    assert_eq!(s.frames.len(), 2);
    assert_eq!(usize::from(s.frames[1].stats.chunks), s.chunks.len());
    assert!(!s.chunks.is_empty());
    assert!(s.chunks.iter().all(|(_, a, _)| a.width() < 80 || a.height() < 48));
}

#[test]
fn observer_sees_framebuffer_areas_and_chunked_frames() {
    // Framebuffer display: the rendered areas of the back buffer.
    let mut e = Engine::new(EngineConfig::default()).unwrap();
    let info = DisplayInfo::new(64, 32, ColorFormat::Rgb565);
    let d = e
        .add_framebuffer_display(MockFramebufferDisplay::new(info, true, 0), BufferMode::full())
        .unwrap();
    let seen = Recorder::default();
    e.set_frame_observer(seen.clone());
    let s = white_screen(&mut e);
    boxed(&mut e, s, Rect::from_xywh(4, 4, 10, 10), Color::BLUE);
    e.step(Instant::from_millis(0));
    let (area, crc) = {
        let s = seen.0.borrow();
        assert_eq!(s.chunks.len(), 1);
        assert_eq!(s.frames.len(), 1);
        (s.chunks[0].1, s.chunks[0].2)
    };
    assert_eq!(area, info.area());
    let fb = e.framebuffer(d, 1).unwrap();
    assert_eq!(crc, Crc32::of(fb), "the whole back buffer was drawn");
    // Read back from the front buffer (the one presented).
    assert_eq!(panel_crc(&mut e, d, area), crc);

    // Chunked display: one observation per `render_chunk`.
    let mut e = Engine::new(EngineConfig::default()).unwrap();
    e.add_chunked_display(DisplayInfo::new(32, 16, ColorFormat::Rgb565), 32 * 2 * 4)
        .unwrap();
    let seen = Recorder::default();
    e.set_frame_observer(seen.clone());
    let now = Instant::from_millis(0);
    e.step(now);
    e.refresh_begin(now).unwrap();
    let mut buf = vec![0u8; 32 * 2 * 4];
    let mut crcs = Vec::new();
    while let Some(a) = e.render_chunk(&mut buf) {
        crcs.push(Crc32::of(&buf[..a.width() as usize * 2 * a.height() as usize]));
    }
    e.refresh_end();
    let s = seen.0.borrow();
    assert_eq!(s.chunks.iter().map(|c| c.2).collect::<Vec<_>>(), crcs);
    assert_eq!(s.frames.len(), 1);
    assert!(e.take_frame_observer().is_some());
}

// ---- Off-screen re-render and read-back ----------------------------------------------------

#[test]
fn render_region_to_matches_the_panel_and_is_stable() {
    let crcs: Vec<(Crc32, Crc32)> = (0..2)
        .map(|_| {
            let (mut e, d) = rgb565(80, 48);
            let (fancy, plain) = scene(&mut e);
            e.step(Instant::from_millis(0));
            let mut out = Vec::new();
            for n in [fancy, plain] {
                let r = e.region(n).unwrap();
                assert_eq!(r.display, d);
                let mut buf = vec![0u8; r.buffer_bytes];
                let expected = e.render_region_to(n, &mut buf).unwrap();
                assert_eq!(expected, Crc32::of(&buf));
                // Same again (fresh mode leaves no trace), and the panel shows the same.
                assert_eq!(e.render_region_to(n, &mut buf).unwrap(), expected);
                assert_eq!(panel_crc(&mut e, d, r.rect), expected, "node region {}", r.rect);
                out.push(expected);
            }
            (out[0], out[1])
        })
        .collect();
    assert_eq!(crcs[0], crcs[1], "the same scene gives the same checksums");
    assert_ne!(crcs[0].0, crcs[0].1);
}

#[test]
fn render_region_to_errors() {
    let (mut e, d) = rgb565(40, 20);
    let s = white_screen(&mut e);
    let n = boxed(&mut e, s, Rect::from_xywh(2, 2, 10, 10), Color::RED);
    e.step(Instant::from_millis(0));
    let mut small = [0u8; 4];
    assert_eq!(
        e.render_region_to(n, &mut small),
        Err(EngineError::BufferTooSmall { needed: 200, got: 4 })
    );
    e.set_flag(n, ObjFlags::HIDDEN, true);
    let mut buf = [0u8; 200];
    assert_eq!(e.render_region_to(n, &mut buf), Err(EngineError::NotVisible(n)));
    e.delete(n).unwrap();
    assert_eq!(e.region(n), Err(EngineError::NodeNotFound(n)));
    // Read-back errors.
    assert_eq!(
        e.read_back(d, Rect::from_xywh(30, 0, 20, 2), &mut buf),
        Err(EngineError::InvalidConfig("read_back: area outside the display"))
    );
    let (mut w, wd) = {
        let mut e = Engine::new(EngineConfig::default()).unwrap();
        let panel = MemoryDisplay::new(DisplayInfo::new(8, 8, ColorFormat::Rgb565)).without_read_back();
        let d = e
            .add_display(panel, BufferMode::alloc(BufferSpec::PartialSingle { rows: 8 }))
            .unwrap();
        (e, d)
    };
    w.step(Instant::from_millis(0));
    assert!(matches!(
        w.read_back(wd, Rect::from_xywh(0, 0, 2, 2), &mut buf),
        Err(EngineError::Unsupported(_))
    ));
}

/// fault: `DisplayIntegrity`
/// req: SAF-031
#[test]
fn check_region_detects_a_corrupted_panel() {
    let (mut e, d) = rgb565(80, 48);
    let (fancy, plain) = scene(&mut e);
    e.step(Instant::from_millis(0));
    let mut buf = vec![0u8; e.region(fancy).unwrap().buffer_bytes];
    let ok = e.check_region(fancy, &mut buf).unwrap();
    assert!(ok.ok());
    assert_eq!(e.fault_counts().get(FaultKind::DisplayIntegrity), 0);

    // A bit flips in the panel's frame memory inside the plain box (in its last column, out
    // of the fancy box's shadow).
    let r = e.region(plain).unwrap().rect;
    assert!(
        !e.region(fancy)
            .unwrap()
            .rect
            .contains(twine_core::Point::new(r.x1 - 1, r.y0))
    );
    let i = (r.y0 as usize * 80 + r.x1 as usize - 1) * 2;
    e.driver_mut::<MemoryDisplay>(d).unwrap().framebuffer_mut()[i] ^= 0x10;
    let mut buf = vec![0u8; e.region(plain).unwrap().buffer_bytes];
    let bad = e.check_region(plain, &mut buf).unwrap();
    assert!(!bad.ok());
    let rec = e.last_fault(FaultKind::DisplayIntegrity).copied().unwrap();
    assert_eq!(
        (rec.display, rec.node, rec.code),
        (Some(d), Some(plain), bad.actual.value())
    );
    // The other region is still fine.
    let mut buf = vec![0u8; e.region(fancy).unwrap().buffer_bytes];
    assert!(e.check_region(fancy, &mut buf).unwrap().ok());

    // A pending change is not a mismatch: the region is busy until it is drawn.
    e.set_local_prop(plain, Selector::MAIN, StyleProp::BgColor(Color::BLUE.into()));
    let mut buf = vec![0u8; e.region(plain).unwrap().buffer_bytes];
    assert_eq!(e.check_region(plain, &mut buf), Err(EngineError::Busy));
    e.step(Instant::from_millis(100));
    assert!(
        e.check_region(plain, &mut buf).unwrap().ok(),
        "redrawn: the panel is right again"
    );
}

#[test]
fn check_region_on_a_mono_panel() {
    let (mut e, _) = engine_with(
        DisplayInfo::new(64, 32, ColorFormat::I1),
        BufferSpec::PartialSingle { rows: 8 },
    );
    let s = white_screen(&mut e);
    let n = boxed(&mut e, s, Rect::from_xywh(3, 5, 21, 9), Color::BLACK);
    e.step(Instant::from_millis(0));
    let r = e.region(n).unwrap();
    assert_eq!(
        r.buffer_bytes,
        21 * 9,
        "one byte per pixel to render, packed in place"
    );
    let mut buf = vec![0u8; r.buffer_bytes];
    let c = e.check_region(n, &mut buf).unwrap();
    assert!(c.ok(), "{c:?}");
    assert_eq!(c.expected, Crc32::of(&[0u8; 3 * 9]), "black: every bit clear");
}

// ---- Safety-critical nodes ---------------------------------------------------------------

/// A parent with `parent_props` and a critical (or not) blue child covering (10, 10)–(30, 30);
/// returns the colors of the child's region as drawn.
fn child_pixels(parent_props: &[StyleProp], critical: bool) -> Vec<Color> {
    let (mut e, d) = rgb565(60, 40);
    let s = white_screen(&mut e);
    let parent = boxed(&mut e, s, Rect::from_xywh(5, 5, 40, 30), Color::RED);
    style(&mut e, parent, parent_props);
    let child = boxed(&mut e, parent, Rect::from_xywh(10, 10, 20, 20), Color::BLUE);
    e.set_flag(child, ObjFlags::SAFETY_CRITICAL, critical);
    e.step(Instant::from_millis(0));
    let panel = e.driver::<MemoryDisplay>(d).unwrap();
    (10..30)
        .flat_map(|y| (10..30).map(move |x| (x, y)))
        .map(|(x, y)| panel.pixel(x, y))
        .collect()
}

#[test]
fn critical_node_is_drawn_without_ancestor_opacity_or_layer() {
    let blue = Color::BLUE;
    for props in [
        &[StyleProp::Opacity(Opa::P50.into())][..], // opacity group (a layer)
        &[StyleProp::PartOpacity(Opa::P50.into())][..], // multiplied into the children
    ] {
        let plain = child_pixels(props, false);
        assert!(
            plain.iter().all(|c| *c != blue),
            "without the flag the child is blended"
        );
        let critical = child_pixels(props, true);
        assert!(critical.iter().all(|c| *c == blue), "{props:?}: drawn opaque");
    }
    // Without any ancestor effect the flag changes no pixel (fresh mode draws the same).
    assert_eq!(child_pixels(&[], true), child_pixels(&[], false));
}

#[test]
fn critical_node_has_no_transitions_and_is_redrawn_whole() {
    const PROPS: Props = Props::from_ids(&[PropId::BgColor]);
    static TR: Transition = Transition::of(PROPS, Duration::ms(200));
    static BASE: Style = Style::new(&[StyleProp::Transition(TransitionRef::Static(&TR))]);
    static PRESSED: Style = Style::new(&[StyleProp::BgColor(twine_style::design::DesignValue::Fixed(
        Color::GREEN,
    ))]);
    let (mut e, d) = rgb565(60, 40);
    let s = white_screen(&mut e);
    let n = boxed(&mut e, s, Rect::from_xywh(10, 10, 30, 20), Color::BLUE);
    e.add_style(n, &BASE, Selector::MAIN);
    e.add_style(n, &PRESSED, Selector::state(State::PRESSED));
    e.set_flag(n, ObjFlags::SAFETY_CRITICAL, true);
    e.step(Instant::from_millis(0));
    e.add_state(n, State::PRESSED);
    assert_eq!(e.transition_count(), 0, "no transition on a critical node");
    assert_eq!(
        e.style_color(n, twine_style::Part::Main, PropId::BgColor),
        Color::GREEN
    );

    // A partial invalidation redraws the whole node.
    e.step(Instant::from_millis(50));
    e.driver_mut::<MemoryDisplay>(d).unwrap().take_flushes();
    e.invalidate_node_area(
        n,
        Rect::from_xywh(12, 12, 2, 2),
        twine_engine::InvalidateReason::Explicit,
    );
    e.step(Instant::from_millis(100));
    let flushed = e
        .driver::<MemoryDisplay>(d)
        .unwrap()
        .flushes()
        .iter()
        .fold(None::<Rect>, |acc, f| {
            Some(acc.map_or(f.area, |a| a.union(&f.area)))
        });
    assert!(flushed.unwrap().contains_rect(&Rect::from_xywh(10, 10, 30, 20)));
}

#[test]
fn critical_nodes_are_listed_and_reported() {
    let (mut e, d) = rgb565(60, 40);
    let s = white_screen(&mut e);
    let a = boxed(&mut e, s, Rect::from_xywh(2, 2, 10, 10), Color::BLUE);
    let b = boxed(&mut e, s, Rect::from_xywh(20, 2, 10, 10), Color::RED);
    e.set_flag(a, ObjFlags::SAFETY_CRITICAL, true);
    e.set_flag(b, ObjFlags::SAFETY_CRITICAL, true);
    e.set_flag(a, ObjFlags::SAFETY_CRITICAL, true); // idempotent
    assert_eq!(e.critical_nodes().collect::<Vec<_>>(), [a, b]);
    assert_eq!(e.critical_report(a).unwrap().drawn_frame, None);
    e.step(Instant::from_millis(0));
    let r = e.critical_report(a).unwrap();
    assert_eq!(r.drawn_frame, Some(e.last_stats(d).frame));
    assert_eq!(r.region.unwrap().rect, Rect::from_xywh(2, 2, 10, 10));
    // An off-screen re-render does not count as drawn.
    let mut buf = vec![0u8; r.region.unwrap().buffer_bytes];
    e.render_region_to(a, &mut buf).unwrap();
    assert_eq!(e.critical_report(a).unwrap().drawn_frame, r.drawn_frame);
    // Deleted nodes leave the list.
    e.delete(b).unwrap();
    assert_eq!(e.critical_nodes().collect::<Vec<_>>(), [a]);
    assert!(e.critical_report(s).is_none());
}

#[test]
fn validity_states_draw_the_standard_marks() {
    let draw = |state: State| {
        let (mut e, d) = rgb565(40, 30);
        let s = white_screen(&mut e);
        let n = boxed(&mut e, s, Rect::from_xywh(5, 5, 30, 20), Color::WHITE);
        e.add_state(n, state);
        e.step(Instant::from_millis(0));
        let panel = e.driver::<MemoryDisplay>(d).unwrap();
        (5..25)
            .flat_map(|y| (5..35).map(move |x| (x, y)))
            .filter(|&(x, y)| panel.pixel(x, y) != Color::WHITE)
            .count()
    };
    let plain = draw(State::DEFAULT);
    let stale = draw(State::STALE);
    let invalid = draw(State::INVALID);
    assert_eq!(plain, 0);
    assert!(stale > 0, "stale: a hatch");
    assert!(
        invalid > stale,
        "invalid: a denser cross-hatch ({invalid} vs {stale})"
    );
}

// ---- Heartbeat and fail-safe screen --------------------------------------------------------

#[test]
fn heartbeat_counts_completed_steps() {
    let (mut e, _) = rgb565(20, 10);
    assert_eq!(e.heartbeat(), 0);
    for i in 1..=5u64 {
        e.step(Instant::from_millis(i * 10));
    }
    assert_eq!(e.heartbeat(), 5);
}

/// fault: `FailSafe`
/// req: SAF-031
#[test]
fn fail_safe_screen_locks_the_display() {
    let (mut e, d) = rgb565(40, 30);
    let normal = e.active_screen(d).unwrap();
    let other = e.create_screen(d).unwrap();
    let safe = e.create_screen(d).unwrap();
    boxed(&mut e, safe, Rect::from_xywh(0, 0, 40, 30), Color::RED);
    let top = e.top_layer(d).unwrap();
    let popup = boxed(&mut e, top, Rect::from_xywh(0, 0, 10, 10), Color::BLUE);
    assert!(
        matches!(e.enter_fail_safe(d, 1), Err(EngineError::InvalidConfig(_))),
        "none registered"
    );
    assert!(e.set_fail_safe_screen(d, normal).is_err(), "the active screen");
    e.set_fail_safe_screen(d, safe).unwrap();
    e.step(Instant::from_millis(0));

    e.enter_fail_safe(d, 42).unwrap();
    assert_eq!((e.active_screen(d), e.fail_safe(d)), (Some(safe), Some(42)));
    let rec = e.last_fault(FaultKind::FailSafe).copied().unwrap();
    assert_eq!((rec.display, rec.code), (Some(d), 42));
    e.enter_fail_safe(d, 7).unwrap(); // idempotent: the first reason stays
    assert_eq!(e.fail_safe(d), Some(42));
    e.load_screen(other);
    assert_eq!(e.active_screen(d), Some(safe), "other screens are refused");
    assert!(e.delete(safe).is_err());
    e.step(Instant::from_millis(100));
    let panel = e.driver::<MemoryDisplay>(d).unwrap();
    assert_eq!(panel.pixel(2, 2), Color::RED, "the popup layer is hidden");
    let _ = popup;

    e.leave_fail_safe(d).unwrap();
    assert_eq!(e.fail_safe(d), None);
    e.load_screen(other);
    assert_eq!(e.active_screen(d), Some(other));
}
