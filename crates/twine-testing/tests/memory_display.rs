//! `MemoryDisplay` behaviour.

use twine_core::{Color, ColorFormat, Rect};
use twine_hal::{DisplayDriver, DisplayInfo};
use twine_testing::{MemoryDisplay, MemoryDisplayError, leak_buffer};

fn rgb565(w: u16, h: u16) -> MemoryDisplay {
    MemoryDisplay::new(DisplayInfo::new(w, h, ColorFormat::Rgb565))
}

#[test]
fn blocking_flush_returns_buffer_immediately() {
    let mut d = rgb565(10, 10);
    let buf = leak_buffer(10 * 2);
    let addr = buf.addr();
    d.begin_flush(Rect::from_xywh(0, 0, 10, 1), buf).unwrap();
    let back = d.poll_flush().expect("returned on the first poll");
    assert_eq!(back.addr(), addr);
    assert!(d.poll_flush().is_none());
    let r = d.flushes()[0];
    assert_eq!(
        (r.area, r.bytes, r.buffer_addr),
        (Rect::from_xywh(0, 0, 10, 1), 20, addr)
    );
    assert_eq!((r.begin_at, r.end_at), (0, 1));
}

#[test]
fn latency_flush_returns_after_n_polls() {
    let mut d = rgb565(10, 10).with_latency(3);
    d.begin_flush(Rect::from_xywh(0, 0, 2, 2), leak_buffer(8))
        .unwrap();
    assert!(d.poll_flush().is_none());
    assert!(d.poll_flush().is_none());
    assert_eq!(d.in_flight(), 1);
    assert!(d.poll_flush().is_some());
    assert_eq!(d.in_flight(), 0);
    assert_eq!(d.flushes()[0].end_at - d.flushes()[0].begin_at, 3);
}

#[test]
fn busy_when_in_flight() {
    let mut d = rgb565(10, 10).with_latency(2);
    d.begin_flush(Rect::from_xywh(0, 0, 1, 1), leak_buffer(2))
        .unwrap();
    let second = leak_buffer(2);
    let addr = second.addr();
    assert_eq!(
        d.begin_flush(Rect::from_xywh(0, 0, 1, 1), second),
        Err(MemoryDisplayError::Busy)
    );
    // The rejected buffer comes back first, then the in-flight one after its latency.
    assert_eq!(d.poll_flush().map(|b| b.addr()), Some(addr));
    assert!(d.poll_flush().is_none());
    assert!(d.poll_flush().is_some());
    assert_eq!(d.flushes().len(), 1);

    // Two in flight are fine when allowed.
    let mut d = rgb565(10, 10).with_latency(1).with_max_in_flight(2);
    d.begin_flush(Rect::from_xywh(0, 0, 1, 1), leak_buffer(2))
        .unwrap();
    d.begin_flush(Rect::from_xywh(1, 0, 1, 1), leak_buffer(2))
        .unwrap();
    assert_eq!(d.in_flight(), 2);
}

#[test]
fn flush_writes_pixels_rgb565() {
    let mut d = rgb565(4, 3);
    d.clear(Color::WHITE);
    let mut buf = leak_buffer(2 * 2 * 2);
    for px in buf.as_mut_slice().chunks_mut(2) {
        px.copy_from_slice(&Color::BLUE.to_rgb565().to_le_bytes());
    }
    d.begin_flush(Rect::from_xywh(2, 1, 2, 2), buf).unwrap();
    d.poll_flush().unwrap();
    assert_eq!(d.pixel(2, 1), Color::BLUE);
    assert_eq!(d.pixel(3, 2), Color::BLUE);
    assert_eq!(d.pixel(1, 1), Color::WHITE);
    assert_eq!(d.pixel(2, 0), Color::WHITE);
    // Row 1 of the framebuffer: 2 white px then 2 blue px.
    assert_eq!(
        &d.framebuffer()[8..16],
        &[0xFF, 0xFF, 0xFF, 0xFF, 0x1F, 0x00, 0x1F, 0x00]
    );
    let rgb = d.to_rgb888();
    assert_eq!(rgb.len(), 4 * 3 * 3);
    assert_eq!(&rgb[(4 + 2) * 3..(4 + 3) * 3], &[0, 0, 255]);
}

#[test]
fn out_of_bounds_area_is_rejected() {
    let mut d = rgb565(10, 10);
    let area = Rect::from_xywh(8, 8, 4, 1);
    assert_eq!(
        d.begin_flush(area, leak_buffer(8)),
        Err(MemoryDisplayError::OutOfBounds(area))
    );
    assert!(d.poll_flush().is_some(), "rejected buffer is handed back");
    let empty = Rect::from_xywh(0, 0, 0, 5);
    assert_eq!(
        d.begin_flush(empty, leak_buffer(8)),
        Err(MemoryDisplayError::OutOfBounds(empty))
    );
    d.poll_flush().unwrap();
    assert_eq!(
        d.begin_flush(Rect::from_xywh(0, 0, 4, 1), leak_buffer(7)),
        Err(MemoryDisplayError::BufferTooSmall { needed: 8, got: 7 })
    );
    assert!(d.flushes().is_empty());
}
