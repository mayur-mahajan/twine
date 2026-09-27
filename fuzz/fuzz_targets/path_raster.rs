//! Fuzzes path building, flattening, stroking and rasterization: arbitrary bytes become path
//! commands (with extreme coordinates, degenerate curves and arcs), a transform and a stroke,
//! rasterized into a 64 × 64 buffer (whole and in two chunks). Nothing may panic, and a
//! chunked draw must touch only its chunk.
#![no_main]

use libfuzzer_sys::fuzz_target;
use twine_core::{Angle, Color, ColorFormat, Fx, Rect, Transform};
use twine_render::{DrawBuf, FillRule, Painter, RenderCaches};
use twine_vector::{Dash, FxPoint, LineCap, LineJoin, Paint, Path, PainterVectorExt, Stroke, VectorDsc};

/// Reads little-endian values from the input, zero when exhausted.
struct Input<'a>(&'a [u8]);

impl Input<'_> {
    fn u8(&mut self) -> u8 {
        let Some((&b, rest)) = self.0.split_first() else {
            return 0;
        };
        self.0 = rest;
        b
    }
    fn i16(&mut self) -> i16 {
        i16::from_le_bytes([self.u8(), self.u8()])
    }
    /// A coordinate: mostly inside the buffer, sometimes huge (the raw 16.16 value).
    fn fx(&mut self) -> Fx {
        if self.u8() & 0xF0 == 0xF0 {
            Fx(i32::from(self.i16()) << 12)
        } else {
            Fx(i32::from(self.i16()) << 3)
        }
    }
    fn point(&mut self) -> FxPoint {
        FxPoint::new(self.fx(), self.fx())
    }
}

fuzz_target!(|data: &[u8]| {
    let mut inp = Input(data);
    let mut path = Path::new();
    for _ in 0..64 {
        if inp.0.is_empty() {
            break;
        }
        match inp.u8() % 8 {
            0 => {
                path.move_to(inp.point());
            }
            1 | 2 => {
                path.line_to(inp.point());
            }
            3 => {
                path.quad_to(inp.point(), inp.point());
            }
            4 => {
                path.cubic_to(inp.point(), inp.point(), inp.point());
            }
            5 => {
                let (radii, rot, flags) = (inp.point(), Angle(inp.i16().into()), inp.u8());
                path.arc_to(radii, rot, flags & 1 != 0, flags & 2 != 0, inp.point());
            }
            6 => {
                path.close();
            }
            _ => {
                path.circle(inp.point(), inp.fx());
            }
        }
    }
    let t = Transform::rotate(Angle(inp.i16().into()))
        .then(Transform::scale(Fx(i32::from(inp.i16()) << 4), Fx(i32::from(inp.i16()) << 4)))
        .then(Transform::translate(inp.fx(), inp.fx()));
    let flags = inp.u8();
    let stroke = Stroke {
        width: Fx(i32::from(inp.i16()).abs() << 6),
        join: [LineJoin::Miter, LineJoin::Round, LineJoin::Bevel][usize::from(flags % 3)],
        cap: [LineCap::Butt, LineCap::Round, LineCap::Square][usize::from(flags / 3 % 3)],
        miter_limit: Fx(i32::from(inp.u8()) << 14),
        dash: (flags & 0x80 != 0).then(|| Dash::new(&[inp.fx(), inp.fx(), inp.fx()], inp.fx())),
    };
    let rule = if flags & 0x40 != 0 { FillRule::EvenOdd } else { FillRule::NonZero };
    let dsc = VectorDsc {
        transform: t,
        fill: Some((Paint::Solid(Color::RED), rule)),
        stroke: Some((Paint::Solid(Color::BLUE), stroke)),
        ..VectorDsc::default()
    };
    let _ = dsc.bounds(&path);
    let mut caches = RenderCaches::default();
    // Whole buffer.
    let mut whole = vec![0u8; 64 * 64 * 2];
    let area = Rect::from_xywh(0, 0, 64, 64);
    if let Ok(buf) = DrawBuf::new_packed(&mut whole, ColorFormat::Rgb565, area) {
        Painter::new(buf, &mut caches).vector(&path, &dsc);
    }
    // Two chunks of 32 rows: identical to the whole draw.
    for half in 0..2 {
        let chunk = Rect::from_xywh(0, half * 32, 64, 32);
        let mut px = vec![0u8; 64 * 32 * 2];
        if let Ok(buf) = DrawBuf::new_packed(&mut px, ColorFormat::Rgb565, chunk) {
            Painter::new(buf, &mut caches).vector(&path, &dsc);
        }
        let start = (half as usize) * 64 * 32 * 2;
        assert_eq!(px, whole[start..start + px.len()], "chunk {half} differs from the whole draw");
    }
});
