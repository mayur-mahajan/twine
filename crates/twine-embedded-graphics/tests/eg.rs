//! `EgDisplay` against a mock embedded-graphics target, and `PainterTarget` drawing
//! embedded-graphics primitives.

use embedded_graphics::pixelcolor::{BinaryColor, Gray8, Rgb565, Rgb888};
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{Circle, PrimitiveStyle, PrimitiveStyleBuilder, Rectangle};
use twine_core::{Color, ColorFormat, Opa, Rect};
use twine_embedded_graphics::{EgDisplay, EgError, PainterExt};
use twine_hal::{DisplayDriver, DrawBufferMem};
use twine_testing::{RenderHarness, assert_render_snapshot};

/// A `w × h` embedded-graphics target at `origin` that stores pixels and counts calls.
struct MockTarget<C> {
    origin: Point,
    w: u32,
    h: u32,
    px: Vec<Option<C>>,
    fill_calls: Vec<Rectangle>,
    fail: bool,
}

impl<C: PixelColor> MockTarget<C> {
    fn new(w: u32, h: u32) -> Self {
        Self {
            origin: Point::zero(),
            w,
            h,
            px: vec![None; (w * h) as usize],
            fill_calls: Vec::new(),
            fail: false,
        }
    }
    fn at(&self, x: i32, y: i32) -> Option<C> {
        self.px[(y as u32 * self.w + x as u32) as usize]
    }
}

impl<C: PixelColor> Dimensions for MockTarget<C> {
    fn bounding_box(&self) -> Rectangle {
        Rectangle::new(self.origin, Size::new(self.w, self.h))
    }
}

impl<C: PixelColor> DrawTarget for MockTarget<C> {
    type Color = C;
    type Error = &'static str;

    fn draw_iter<I: IntoIterator<Item = Pixel<C>>>(&mut self, pixels: I) -> Result<(), &'static str> {
        if self.fail {
            return Err("bus error");
        }
        for Pixel(p, c) in pixels {
            let q = p - self.origin;
            assert!(
                q.x >= 0 && q.y >= 0 && (q.x as u32) < self.w && (q.y as u32) < self.h,
                "{p:?} outside"
            );
            self.px[(q.y as u32 * self.w + q.x as u32) as usize] = Some(c);
        }
        Ok(())
    }

    fn fill_contiguous<I: IntoIterator<Item = C>>(
        &mut self,
        area: &Rectangle,
        colors: I,
    ) -> Result<(), &'static str> {
        self.fill_calls.push(*area);
        self.draw_iter(area.points().zip(colors).map(|(p, c)| Pixel(p, c)))
    }
}

fn leak(bytes: Vec<u8>) -> DrawBufferMem {
    DrawBufferMem::new(Box::leak(bytes.into_boxed_slice()))
}

#[test]
fn rgb565_roundtrip_through_mock_drawtarget() {
    let mut d = EgDisplay::new(MockTarget::<Rgb565>::new(8, 4));
    let info = d.info();
    assert_eq!(
        (info.width, info.height, info.format),
        (8, 4, ColorFormat::Rgb565)
    );
    // Render a 3×2 area with distinct colours through twine's RGB565 encoding.
    let colors = [0xF800u16, 0x07E0, 0x001F, 0xFFFF, 0x1234, 0x0000];
    let bytes: Vec<u8> = colors.iter().flat_map(|c| c.to_le_bytes()).collect();
    d.begin_flush(Rect::from_xywh(0, 0, 3, 2), leak(bytes)).unwrap();
    let buf = d.poll_flush().expect("blocking: buffer returned immediately");
    assert_eq!(buf.len(), 12);
    assert!(d.poll_flush().is_none());
    let t = d.target();
    for (i, raw) in colors.iter().enumerate() {
        let (x, y) = ((i % 3) as i32, (i / 3) as i32);
        assert_eq!(t.at(x, y).unwrap().into_storage(), *raw, "pixel {x},{y}");
    }
    assert_eq!(t.fill_calls.len(), 1, "one fill_contiguous per flush");
}

#[test]
fn rgb888_and_gray8_formats() {
    let mut d = EgDisplay::new(MockTarget::<Rgb888>::new(2, 1));
    assert_eq!(d.info().format, ColorFormat::Rgb888);
    d.begin_flush(Rect::from_xywh(0, 0, 2, 1), leak(vec![3, 2, 1, 0x30, 0x20, 0x10]))
        .unwrap();
    assert_eq!(d.target().at(0, 0), Some(Rgb888::new(1, 2, 3)));
    assert_eq!(d.target().at(1, 0), Some(Rgb888::new(0x10, 0x20, 0x30)));

    let mut d = EgDisplay::new(MockTarget::<Gray8>::new(2, 1));
    assert_eq!(d.info().format, ColorFormat::L8);
    d.begin_flush(Rect::from_xywh(1, 0, 1, 1), leak(vec![99]))
        .unwrap();
    assert_eq!(d.target().at(1, 0), Some(Gray8::new(99)));
}

#[test]
fn binarycolor_from_i1() {
    let mut d = EgDisplay::new(MockTarget::<BinaryColor>::new(16, 2));
    let info = d.info();
    assert_eq!(info.format, ColorFormat::I1);
    assert_eq!(info.align, 8);
    // 10 pixels wide: 2 bytes per row, MSB first.
    let rows = vec![0b1010_0000, 0b1100_0000, 0b0000_0001, 0b0100_0000];
    d.begin_flush(Rect::from_xywh(0, 0, 10, 2), leak(rows)).unwrap();
    let t = d.target();
    let row = |y| (0..10).map(|x| t.at(x, y).unwrap().is_on()).collect::<Vec<_>>();
    assert_eq!(
        row(0),
        [true, false, true, false, false, false, false, false, true, true]
    );
    assert_eq!(
        row(1),
        [false, false, false, false, false, false, false, true, false, true]
    );
    assert_eq!(t.at(10, 0), None, "outside the flushed area");
}

#[test]
fn flush_area_offsets_respected() {
    let mut target = MockTarget::<Gray8>::new(10, 8);
    target.origin = Point::new(100, 50); // e.g. a sub-region of a larger target
    let mut d = EgDisplay::new(target);
    d.begin_flush(Rect::from_xywh(3, 2, 2, 3), leak((1..=6).collect()))
        .unwrap();
    let t = d.target();
    assert_eq!(
        t.fill_calls,
        [Rectangle::new(Point::new(103, 52), Size::new(2, 3))]
    );
    for y in 0..8 {
        for x in 0..10 {
            let inside = (3..5).contains(&x) && (2..5).contains(&y);
            let expected = inside.then(|| Gray8::new((1 + (y - 2) * 2 + (x - 3)) as u8));
            assert_eq!(t.at(x, y), expected, "{x},{y}");
        }
    }
}

#[test]
fn errors_return_the_buffer() {
    let mut target = MockTarget::<Rgb565>::new(4, 4);
    target.fail = true;
    let mut d = EgDisplay::new(target);
    let r = d.begin_flush(Rect::from_xywh(0, 0, 2, 1), leak(vec![0; 4]));
    assert_eq!(r, Err(EgError::Target("bus error")));
    assert!(d.poll_flush().is_some());

    d.target_mut().fail = false;
    let r = d.begin_flush(Rect::from_xywh(0, 0, 2, 2), leak(vec![0; 4]));
    assert_eq!(r, Err(EgError::BufferTooSmall { needed: 8, got: 4 }));
    assert!(d.poll_flush().is_some());
    assert_eq!(
        d.target().fill_calls.len(),
        1,
        "the short buffer never reaches the target"
    );
}

#[test]
fn with_info_keeps_geometry() {
    let d = EgDisplay::new(MockTarget::<Rgb565>::new(4, 3)).with_info(|i| i.with_dpi(200).with_align(1));
    let i = d.info();
    assert_eq!(
        (i.width, i.height, i.dpi, i.format),
        (4, 3, 200, ColorFormat::Rgb565)
    );
    let _target: MockTarget<Rgb565> = d.into_inner();
}

#[test]
fn draw_buf_target_draws_eg_circle() {
    let mut h = RenderHarness::new(64, 48, ColorFormat::Rgb565);
    h.clear(Color::hex(0x0020_2020));
    h.paint(|p| {
        let mut t = p.as_draw_target::<Rgb565>();
        Circle::new(Point::new(8, 4), 40)
            .into_styled(
                PrimitiveStyleBuilder::new()
                    .fill_color(Rgb565::new(31, 40, 0))
                    .stroke_color(Rgb565::WHITE)
                    .stroke_width(3)
                    .build(),
            )
            .draw(&mut t)
            .unwrap();
        Rectangle::new(Point::new(40, 28), Size::new(30, 30)) // partly outside: clipped
            .into_styled(PrimitiveStyle::with_fill(Rgb565::BLUE))
            .draw(&mut t.with_opa(Opa::P50))
            .unwrap();
    });
    // Centre of the circle filled, outside untouched, blue square blended over the background.
    let px =
        |x: usize, y: usize| u16::from_le_bytes([h.data()[(y * 64 + x) * 2], h.data()[(y * 64 + x) * 2 + 1]]);
    assert_eq!(px(28, 24), Rgb565::new(31, 40, 0).into_storage());
    assert_eq!(px(1, 1), Color::hex(0x0020_2020).to_rgb565());
    assert_ne!(px(60, 44), Color::hex(0x0020_2020).to_rgb565());
    assert_render_snapshot!(h, "eg_circle");
}
