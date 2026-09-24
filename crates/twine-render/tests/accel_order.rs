//! Ordering between queued accelerator work and CPU access to the draw buffer.

use std::cell::RefCell;

use twine_core::{Color, ColorFormat, Opa, Rect};
use twine_render::{AccelResult, DrawAccel, DrawBuf, ImagePixels, Painter, RenderCaches};

/// Queues every fill (writing the pixels immediately) and records calls in order.
struct RecordingAccel<'l> {
    log: &'l RefCell<Vec<&'static str>>,
}

impl DrawAccel for RecordingAccel<'_> {
    fn fill(&mut self, dst: &mut DrawBuf<'_>, area: Rect, color: Color, _: Opa) -> AccelResult {
        self.log.borrow_mut().push("fill");
        for y in area.y0..area.y1 {
            dst.row_mut(y, area.x0, area.x1).fill(color.luminance());
        }
        AccelResult::Queued
    }
    fn blit(&mut self, _: &mut DrawBuf<'_>, _: Rect, _: &ImagePixels<'_>, _: Opa) -> AccelResult {
        AccelResult::Unsupported
    }
    fn blend_a8(&mut self, _: &mut DrawBuf<'_>, _: Rect, _: Color, _: &[u8], _: usize) -> AccelResult {
        AccelResult::Unsupported
    }
    fn wait(&mut self) {
        self.log.borrow_mut().push("wait");
    }
}

#[test]
fn painter_waits_before_cpu_access() {
    let log = RefCell::new(Vec::new());
    let mut accel = RecordingAccel { log: &log };
    let mut caches = RenderCaches::default();
    let mut data = vec![0u8; 64 * 64];
    {
        let buf = DrawBuf::new_packed(&mut data, ColorFormat::L8, Rect::from_xywh(0, 0, 64, 64)).unwrap();
        let mut p = Painter::new(buf, &mut caches).with_accel(&mut accel);
        p.fill(Rect::from_xywh(0, 0, 64, 32), Color::WHITE, Opa::COVER); // queued
        p.fill(Rect::from_xywh(0, 32, 64, 32), Color::WHITE, Opa::COVER); // queued again (no CPU access)
        log.borrow_mut().push("cpu");
        p.fill(Rect::from_xywh(0, 0, 4, 4), Color::BLACK, Opa::COVER); // software: must wait first
        p.fill(Rect::from_xywh(0, 0, 64, 64), Color::WHITE, Opa::COVER); // queued
        log.borrow_mut().push("drop");
    } // dropping the painter (before the buffer goes to a flush) waits
    log.borrow_mut().push("flush");
    assert_eq!(
        *log.borrow(),
        ["fill", "fill", "cpu", "wait", "fill", "drop", "wait", "flush"]
    );
    assert!(data.iter().all(|&v| v == 255));
}

#[test]
fn buf_mut_waits_for_queued_work() {
    let log = RefCell::new(Vec::new());
    let mut accel = RecordingAccel { log: &log };
    let mut caches = RenderCaches::default();
    let mut data = vec![0u8; 64 * 64];
    let buf = DrawBuf::new_packed(&mut data, ColorFormat::L8, Rect::from_xywh(0, 0, 64, 64)).unwrap();
    let mut p = Painter::new(buf, &mut caches).with_accel(&mut accel);
    p.fill(Rect::from_xywh(0, 0, 64, 64), Color::WHITE, Opa::COVER);
    let _ = p.buf_mut();
    assert_eq!(*log.borrow(), ["fill", "wait"]);
    drop(p);
    assert_eq!(log.borrow().len(), 2, "nothing pending: no second wait");
}
