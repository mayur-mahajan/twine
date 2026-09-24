//! The chunk-level refresh API (`refresh_begin` / `render_chunk` / `refresh_end`) renders the
//! same pixels as the buffered refresh of a `DisplayDriver`, for every rotation, and reports
//! frames and statistics the same way.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use common::{boxed, style, white_screen};
use twine_core::{Color, ColorFormat, Duration, Instant, Rect, Rotation};
use twine_engine::{BufferMode, Engine, EngineConfig, InvalidateReason, Wake};
use twine_hal::{DisplayDriver, DisplayInfo, DrawBufferMem};
use twine_style::StyleProp;

const W: u16 = 96;
const H: u16 = 64;

type Fb = Rc<RefCell<Vec<u8>>>;

/// Copies a flushed chunk (`area`, rows packed, 2 bytes per pixel) into a `pw`-wide framebuffer.
fn blit(fb: &Fb, pw: usize, area: Rect, buf: &[u8]) {
    let w = area.width() as usize;
    for (row, y) in (area.y0..area.y1).enumerate() {
        let dst = (y as usize * pw + area.x0 as usize) * 2;
        fb.borrow_mut()[dst..dst + w * 2].copy_from_slice(&buf[row * w * 2..(row + 1) * w * 2]);
    }
}

/// A blocking display that copies every flush into a framebuffer.
struct FbDisplay {
    info: DisplayInfo,
    fb: Fb,
    held: Option<DrawBufferMem>,
}

impl DisplayDriver for FbDisplay {
    type Error = ();
    fn info(&self) -> DisplayInfo {
        self.info
    }
    fn begin_flush(&mut self, area: Rect, buf: DrawBufferMem) -> Result<(), ()> {
        blit(&self.fb, physical_width(&self.info), area, buf.as_slice());
        self.held = Some(buf);
        Ok(())
    }
    fn poll_flush(&mut self) -> Option<DrawBufferMem> {
        self.held.take()
    }
}

fn physical_width(info: &DisplayInfo) -> usize {
    usize::from(if info.rotation.swaps_axes() && !info.hw_rotation {
        info.height
    } else {
        info.width
    })
}

/// A `W × H` (logical) RGB565 display rotated in software.
fn info(rotation: Rotation) -> DisplayInfo {
    DisplayInfo::new(W, H, ColorFormat::Rgb565).with_rotation(rotation)
}

fn scene(e: &mut Engine) {
    let s = white_screen(e);
    boxed(e, s, Rect::from_xywh(2, 3, 40, 12), Color::RED);
    boxed(e, s, Rect::from_xywh(60, 40, 30, 20), Color::BLUE);
    let r = boxed(e, s, Rect::from_xywh(20, 22, 30, 30), Color::hex(0x002E_7D32));
    style(e, r, &[StyleProp::Radius(10), StyleProp::BorderWidth(3)]);
}

fn leak(len: usize) -> &'static mut [u8] {
    Box::leak(vec![0u8; len].into_boxed_slice())
}

/// Frame via the blocking driver path.
fn buffered(rotation: Rotation, rows: usize) -> Vec<u8> {
    let fb: Fb = Rc::new(RefCell::new(vec![0; usize::from(W) * usize::from(H) * 2]));
    let mut e = Engine::new(EngineConfig::default()).unwrap();
    let disp = FbDisplay {
        info: info(rotation),
        fb: fb.clone(),
        held: None,
    };
    let len = usize::from(W) * 2 * rows;
    e.add_display(disp, BufferMode::partial_double(leak(len), leak(len)))
        .unwrap();
    scene(&mut e);
    let _ = e.step(Instant::from_millis(0));
    fb.borrow().clone()
}

/// Frame via the chunk API, alternating two caller buffers.
fn chunked(rotation: Rotation, rows: usize) -> (Vec<u8>, u16) {
    let info = info(rotation);
    let pw = physical_width(&info);
    let fb: Fb = Rc::new(RefCell::new(vec![0; usize::from(W) * usize::from(H) * 2]));
    let mut e = Engine::new(EngineConfig::default()).unwrap();
    let len = usize::from(W) * 2 * rows;
    let d = e.add_chunked_display(info, len).unwrap();
    scene(&mut e);
    let now = Instant::from_millis(0);
    let wake = e.step(now);
    assert_eq!(
        wake,
        Wake::Idle,
        "a due external frame is not the engine's to render"
    );
    let frame = e.refresh_begin(now).expect("frame due");
    assert_eq!(frame.display, d);
    let mut bufs = [vec![0u8; len], vec![0u8; len]];
    let mut n = 0;
    while let Some(area) = e.render_chunk(&mut bufs[n % 2]) {
        blit(&fb, pw, area, &bufs[n % 2]);
        n += 1;
    }
    e.refresh_end();
    assert!(e.refresh_begin(now).is_none(), "nothing left to draw");
    assert_eq!(usize::from(e.last_stats(d).chunks), n);
    let out = fb.borrow().clone();
    (out, e.last_stats(d).chunks)
}

#[test]
fn chunk_api_matches_buffered_refresh_pixel_for_pixel() {
    for rotation in [
        Rotation::Deg0,
        Rotation::Deg90,
        Rotation::Deg180,
        Rotation::Deg270,
    ] {
        for rows in [7, 16, 64] {
            let a = buffered(rotation, rows);
            let (b, chunks) = chunked(rotation, rows);
            assert!(
                a == b,
                "{rotation:?} rows {rows}: chunk API differs from buffered refresh"
            );
            assert!(chunks >= 1);
        }
    }
}

#[test]
fn next_frame_respects_refr_period_and_reports_due() {
    let mut e = Engine::new(EngineConfig::default()).unwrap();
    let len = usize::from(W) * 2 * 16;
    let d = e.add_chunked_display(info(Rotation::Deg0), len).unwrap();
    scene(&mut e);
    let t0 = Instant::from_millis(0);
    let _ = e.step(t0);
    e.refresh_begin(t0).unwrap();
    let mut buf = vec![0u8; len];
    while e.render_chunk(&mut buf).is_some() {}
    e.refresh_add_flush_time(120, 30);
    e.refresh_end();
    assert_eq!(
        (e.last_stats(d).flush_us, e.last_stats(d).flush_wait_us),
        (120, 30)
    );
    // A change right after the frame waits for the refresh period.
    e.invalidate_area(d, Rect::from_xywh(5, 5, 4, 4), InvalidateReason::Explicit);
    let t1 = t0 + Duration::ms(1);
    let wake = e.step(t1);
    assert_eq!(wake, Wake::At(t0 + e.config().refr_period));
    assert!(e.refresh_begin(t1).is_none());
    let t2 = t0 + e.config().refr_period;
    let f = e.refresh_begin(t2).unwrap();
    assert_eq!(f.areas, 1);
    let area = e.render_chunk(&mut buf).unwrap();
    assert_eq!(area, Rect::from_xywh(5, 5, 4, 4));
    assert!(e.render_chunk(&mut buf).is_none());
    e.refresh_end();
}

#[test]
fn unfinished_frame_is_redrawn_next_time() {
    let mut e = Engine::new(EngineConfig::default()).unwrap();
    let len = usize::from(W) * 2 * 8;
    let _d = e.add_chunked_display(info(Rotation::Deg0), len).unwrap();
    scene(&mut e);
    let t0 = Instant::from_millis(0);
    let _ = e.step(t0);
    e.refresh_begin(t0).unwrap();
    let mut buf = vec![0u8; len];
    assert!(e.render_chunk(&mut buf).is_some());
    e.refresh_end(); // abandoned after one chunk
    let t1 = t0 + e.config().refr_period;
    let _ = e.step(t1);
    let f = e.refresh_begin(t1).expect("the rest is redrawn");
    assert!(f.px >= u32::from(W) * (u32::from(H) - 8));
    while e.render_chunk(&mut buf).is_some() {}
    e.refresh_end();
}

#[test]
fn chunk_buffer_too_small_is_rejected() {
    let mut e = Engine::new(EngineConfig::default()).unwrap();
    assert!(e.add_chunked_display(info(Rotation::Deg0), 10).is_err());
}
