//! `SimDisplay` without a window (P02.S06).

use std::time::{Duration, Instant};

use twine_core::color::{I1, Rgb565Swapped};
use twine_core::{Color, ColorFormat, PixelFormat, Rect, Rotation};
use twine_hal::{DisplayDriver, DrawBufferMem};
use twine_sim::{SimDisplay, SimDisplayError};

fn leak(len: usize) -> DrawBufferMem {
    DrawBufferMem::new(Box::leak(vec![0u8; len].into_boxed_slice()))
}

#[test]
fn rgb565_swapped_is_decoded_correctly() {
    let mut d = SimDisplay::new(4, 4, ColorFormat::Rgb565Swapped, Rotation::Deg0);
    let mut buf = leak(2 * 2 * 2);
    for px in buf.as_mut_slice().chunks_mut(2) {
        Rgb565Swapped::write(px, Rgb565Swapped::from_color(Color::RED));
    }
    assert_eq!(&buf.as_slice()[..2], &[0xF8, 0x00], "big-endian bytes in memory");
    d.begin_flush(Rect::from_xywh(2, 2, 2, 2), buf).unwrap();
    assert!(d.poll_flush().is_some());
    let rgb = d.panel_rgb888();
    let at = |x: usize, y: usize| &rgb[(y * 4 + x) * 3..(y * 4 + x) * 3 + 3];
    assert_eq!(at(3, 3), &[255, 0, 0]);
    assert_eq!(at(1, 1), &[0, 0, 0]);
    assert_eq!(d.take_dirty(), Some(Rect::from_xywh(2, 2, 2, 2)));
    assert!(d.info().hw_rotation);
}

#[test]
fn i1_uses_mono_colors() {
    let ink = Color::hex(0x10_10_10);
    let paper = Color::hex(0xB0_C8_A0);
    let mut d = SimDisplay::new(16, 2, ColorFormat::I1, Rotation::Deg0).with_mono_colors(ink, paper);
    // Row 1, x 3..11: bits 1 0 1 0 1 0 1 0.
    let mut buf = leak(1);
    for x in (0..8).step_by(2) {
        I1::set(buf.as_mut_slice(), x, true);
    }
    d.begin_flush(Rect::from_xywh(3, 1, 8, 1), buf).unwrap();
    d.poll_flush().unwrap();
    let rgb = d.panel_rgb888();
    let px = |x: usize, y: usize| {
        Color::new(
            rgb[(y * 16 + x) * 3],
            rgb[(y * 16 + x) * 3 + 1],
            rgb[(y * 16 + x) * 3 + 2],
        )
    };
    assert_eq!(px(3, 1), paper);
    assert_eq!(px(4, 1), ink);
    assert_eq!(px(9, 1), paper);
    assert_eq!(px(10, 1), ink);
    assert_eq!(px(3, 0), ink, "untouched panel memory is 0 bits");
}

#[test]
fn bus_hz_delays_completion() {
    // 1 kB at 80 kHz = 8192 bits / 80 000 Hz ≈ 102 ms.
    let mut d = SimDisplay::new(512, 1, ColorFormat::Rgb565, Rotation::Deg0).with_bus_hz(Some(80_000));
    let mut buf = leak(1024);
    buf.as_mut_slice().fill(0xFF);
    let start = Instant::now();
    d.begin_flush(Rect::from_xywh(0, 0, 512, 1), buf).unwrap();
    assert!(d.poll_flush().is_none(), "still in flight");
    assert_eq!(
        d.panel()[0],
        0,
        "pixels become visible only when the transfer completes"
    );
    assert_eq!(
        d.begin_flush(Rect::from_xywh(0, 0, 1, 1), leak(2)),
        Err(SimDisplayError::Busy)
    );
    assert!(
        d.poll_flush().is_some(),
        "the rejected buffer is handed back first"
    );
    let back = loop {
        if let Some(b) = d.poll_flush() {
            break b;
        }
        assert!(start.elapsed() < Duration::from_secs(5), "flush never completed");
        std::thread::sleep(Duration::from_millis(2));
    };
    assert!(
        start.elapsed() >= Duration::from_millis(100),
        "{:?}",
        start.elapsed()
    );
    assert_eq!(back.len(), 1024);
    assert_eq!(d.panel()[0], 0xFF);
    assert!(d.busy_until().is_none());
    assert_eq!(d.flush_count(), 1);
}

#[test]
fn area_outside_panel_is_rejected() {
    let mut d = SimDisplay::new(10, 10, ColorFormat::Rgb565, Rotation::Deg0);
    let area = Rect::from_xywh(5, 5, 6, 1);
    assert_eq!(
        d.begin_flush(area, leak(12)),
        Err(SimDisplayError::OutOfBounds(area))
    );
    assert!(d.poll_flush().is_some());
    assert_eq!(
        d.begin_flush(Rect::from_xywh(0, 0, 5, 1), leak(9)),
        Err(SimDisplayError::BufferTooSmall { needed: 10, got: 9 })
    );
    assert!(d.poll_flush().is_some());
    assert_eq!(d.flush_count(), 0);
    assert!(d.take_dirty().is_none());
}
