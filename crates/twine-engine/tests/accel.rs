//! `Engine::set_accel` / `clear_accel`: every rendered chunk offers its fills to the attached accelerator, and
//! what the accelerator declines is drawn in software (identical pixels).

mod common;

use std::cell::Cell;
use std::rc::Rc;

use common::{boxed, white_screen};
use twine_core::{Color, ColorFormat, Instant, Opa, Rect};
use twine_engine::{Engine, EngineConfig};
use twine_hal::DisplayInfo;
use twine_render::{AccelResult, DrawAccel, DrawBuf, ImagePixels};

const W: u16 = 64;
const H: u16 = 32;

/// Counts calls and declines everything.
struct Counting {
    fills: Rc<Cell<u32>>,
    waits: Rc<Cell<u32>>,
}

impl DrawAccel for Counting {
    fn fill(&mut self, _: &mut DrawBuf<'_>, _: Rect, _: Color, _: Opa) -> AccelResult {
        self.fills.set(self.fills.get() + 1);
        AccelResult::Unsupported
    }
    fn blit(&mut self, _: &mut DrawBuf<'_>, _: Rect, _: &ImagePixels<'_>, _: Opa) -> AccelResult {
        AccelResult::Unsupported
    }
    fn blend_a8(&mut self, _: &mut DrawBuf<'_>, _: Rect, _: Color, _: &[u8], _: usize) -> AccelResult {
        AccelResult::Unsupported
    }
    fn wait(&mut self) {
        self.waits.set(self.waits.get() + 1);
    }
}

/// Renders the scene through the chunk API; returns the frame.
fn frame(accel: Option<Counting>) -> Vec<u8> {
    let mut e = Engine::new(EngineConfig::default()).unwrap();
    let has = accel.is_some();
    if let Some(a) = accel {
        e.set_accel(a);
    }
    assert_eq!(e.has_accel(), has);
    let stride = usize::from(W) * 2;
    e.add_chunked_display(
        DisplayInfo::new(W, H, ColorFormat::Rgb565),
        stride * usize::from(H),
    )
    .unwrap();
    let s = white_screen(&mut e);
    boxed(&mut e, s, Rect::from_xywh(4, 4, 30, 20), Color::RED);
    let mut fb = vec![0u8; stride * usize::from(H)];
    let mut buf = vec![0u8; stride * usize::from(H)];
    assert!(e.refresh_begin(Instant::from_millis(0)).is_some());
    while let Some(area) = e.render_chunk(&mut buf) {
        let w = area.width() as usize * 2;
        for (row, y) in (area.y0..area.y1).enumerate() {
            let dst = y as usize * stride + area.x0 as usize * 2;
            fb[dst..dst + w].copy_from_slice(&buf[row * w..row * w + w]);
        }
    }
    e.refresh_end();
    fb
}

#[test]
fn accel_receives_fills_and_declined_work_is_drawn_in_software() {
    let (fills, waits) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
    let with = frame(Some(Counting {
        fills: fills.clone(),
        waits: waits.clone(),
    }));
    assert!(fills.get() >= 1, "fills offered to the accelerator");
    assert_eq!(with, frame(None), "software fallback renders identical pixels");
}

#[test]
fn clear_accel_detaches() {
    let mut e = Engine::new(EngineConfig::default()).unwrap();
    assert!(!e.has_accel());
    e.set_accel(Counting {
        fills: Rc::default(),
        waits: Rc::default(),
    });
    assert!(e.has_accel());
    e.clear_accel();
    assert!(!e.has_accel());
}
