//! AMOLED controllers need even flush areas (`DisplayInfo::align = 2`): the engine must round
//! every dirty area before it reaches the QSPI driver — with hardware rotation (RM67162) and
//! with software rotation (SH8601).

use twine_core::{Instant, Rect, Rotation};
use twine_drivers::mipi_dcs::PanelSpec;
use twine_drivers::testkit::{BusOp, Recorder};
use twine_drivers::{rm67162, sh8601};
use twine_engine::{BufferMode, Engine, EngineConfig, InvalidateReason};

/// Inclusive `(start, end)` of a `CASET`/`RASET` parameter block.
fn range(d: &[u8]) -> (u16, u16) {
    (u16::from_be_bytes([d[0], d[1]]), u16::from_be_bytes([d[2], d[3]]))
}

fn flushed_windows(
    spec: &'static PanelSpec,
    rotation: Rotation,
    dirty: &[Rect],
) -> Vec<((u16, u16), (u16, u16))> {
    let rec = Recorder::new();
    let lcd = if spec.sw_rotation {
        sh8601::new(rec.qspi(), Some(rec.pin("rst")), spec, rotation, &mut rec.delay()).unwrap()
    } else {
        rm67162::new(rec.qspi(), Some(rec.pin("rst")), spec, rotation, &mut rec.delay()).unwrap()
    };
    let rows = 16usize;
    let len = usize::from(spec.native_w.max(spec.native_h)) * 2 * rows;
    let buf: &'static mut [u8] = Box::leak(vec![0u8; len].into_boxed_slice());
    let mut e = Engine::new(EngineConfig::default()).unwrap();
    let d = e.add_display(lcd, BufferMode::partial_single(buf)).unwrap();
    e.step(Instant::from_millis(0));
    let _ = rec.take_ops();
    for a in dirty {
        e.invalidate_area(d, *a, InvalidateReason::Explicit);
    }
    e.step(Instant::from_millis(1000));
    let ops = rec.ops();
    let mut out = Vec::new();
    for w in ops.windows(2) {
        if let [
            BusOp::QspiCmd {
                addr: 0x2A00,
                data: c,
                ..
            },
            BusOp::QspiCmd {
                addr: 0x2B00,
                data: r,
                ..
            },
        ] = w
        {
            out.push((range(c), range(r)));
        }
    }
    assert!(!out.is_empty(), "nothing flushed: {ops:?}");
    out
}

#[test]
fn engine_rounds_flush_areas_to_even() {
    let dirty = [Rect::from_xywh(3, 5, 7, 9), Rect::from_xywh(101, 33, 1, 1)];
    for (spec, rot) in [
        (&sh8601::SH8601_368X448, Rotation::Deg0),
        (&sh8601::SH8601_368X448, Rotation::Deg90),
        (&sh8601::SH8601_368X448, Rotation::Deg270),
        (&rm67162::RM67162_240X536, Rotation::Deg90),
        (&rm67162::RM67162_240X536, Rotation::Deg0),
    ] {
        for ((x0, x1), (y0, y1)) in flushed_windows(spec, rot, &dirty) {
            assert!(
                x0 % 2 == 0 && (x1 + 1) % 2 == 0 && y0 % 2 == 0 && (y1 + 1) % 2 == 0,
                "{} {rot:?}: odd window x {x0}..={x1} y {y0}..={y1}",
                spec.name
            );
        }
    }
}
