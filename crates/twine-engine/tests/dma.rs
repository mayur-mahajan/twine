//! Double buffering with DMA pipelining, buffer ownership and cooperative flushing.

mod common;

use std::collections::HashSet;

use proptest::prelude::*;
use twine_core::{Duration, Rect};
use twine_engine::{EngineConfig, InvalidateReason, Wake};
use twine_hal::BufferSpec;
use twine_testing::scenes::engine_boxes;
use twine_testing::{DmaEvent, EngineHarness, clear_dma_log, dma_log};

fn boxes(rows: u16, polls: u32, cfg: EngineConfig, single: bool) -> EngineHarness {
    let spec = if single {
        BufferSpec::PartialSingle { rows }
    } else {
        BufferSpec::PartialDouble { rows }
    };
    EngineHarness::new(320, 240)
        .no_theme()
        .config(cfg)
        .buffers(spec)
        .dma(polls)
        .mount_engine(|e| {
            engine_boxes(e);
        })
}

/// Checks buffer ownership in a DMA log: rendering never targets a buffer in flight.
fn assert_ownership(log: &[DmaEvent]) {
    let mut in_flight = HashSet::new();
    for (i, e) in log.iter().enumerate() {
        match *e {
            DmaEvent::Begin { buf, .. } => {
                assert!(in_flight.insert(buf), "buffer {buf} flushed twice (event {i})");
            }
            DmaEvent::Complete { buf } => {
                assert!(in_flight.remove(&buf), "buffer {buf} completed but not in flight");
            }
            DmaEvent::RenderStart { buf } => {
                assert!(
                    !in_flight.contains(&buf),
                    "rendered into in-flight buffer {buf} at event {i}: {log:?}"
                );
            }
        }
    }
}

#[test]
fn double_buffer_overlaps_render_and_transfer() {
    clear_dma_log();
    let mut h = boxes(40, 3, EngineConfig::default(), false);
    h.run_until_idle();
    let log = dma_log();
    assert_ownership(&log);
    let begin0 = log
        .iter()
        .position(|e| matches!(e, DmaEvent::Begin { buf: 0, .. }))
        .expect("begin 0");
    let render1 = log
        .iter()
        .position(|e| *e == DmaEvent::RenderStart { buf: 1 })
        .expect("render 1");
    let complete0 = log
        .iter()
        .position(|e| *e == DmaEvent::Complete { buf: 0 })
        .expect("complete 0");
    assert!(begin0 < render1 && render1 < complete0, "no overlap: {log:?}");
    // 240 rows in 40-row chunks.
    assert_eq!(
        log.iter().filter(|e| matches!(e, DmaEvent::Begin { .. })).count(),
        6
    );
}

#[test]
fn single_buffer_waits_for_completion() {
    clear_dma_log();
    let mut h = boxes(40, 3, EngineConfig::default(), true);
    h.run_until_idle();
    let log = dma_log();
    assert_ownership(&log);
    for (i, e) in log.iter().enumerate() {
        let ok = match i % 3 {
            0 => *e == DmaEvent::RenderStart { buf: 0 },
            1 => matches!(e, DmaEvent::Begin { buf: 0, .. }),
            _ => *e == DmaEvent::Complete { buf: 0 },
        };
        assert!(ok, "event {i} out of order: {log:?}");
    }
}

#[test]
fn cooperative_flush_returns_now_and_resumes() {
    let reference = {
        let mut h = boxes(40, 3, EngineConfig::default(), false);
        h.run_until_idle();
        h.panel_rgb888()
    };
    clear_dma_log();
    let cfg = EngineConfig {
        cooperative_flush: true,
        ..EngineConfig::default()
    };
    let mut h = boxes(40, 3, cfg, false);
    assert_eq!(
        h.update(),
        Wake::Now,
        "frame must yield while both buffers are in flight"
    );
    assert!(h.engine().flush_pending());
    let mut updates = 1;
    while h.update() != Wake::Idle {
        updates += 1;
        assert!(updates < 1000);
    }
    assert!(updates > 2);
    assert!(!h.engine().flush_pending());
    assert_ownership(&dma_log());
    assert_eq!(h.panel_rgb888(), reference);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]
    #[test]
    fn never_renders_into_in_flight_buffer(
        polls in 1u32..10,
        rows in 1u16..50,
        areas in proptest::collection::vec((0i32..300, 0i32..220, 1i32..120, 1i32..120), 1..6),
        coop in any::<bool>(),
    ) {
        clear_dma_log();
        let cfg = EngineConfig { cooperative_flush: coop, ..EngineConfig::default() };
        let mut h = EngineHarness::new(320, 240).no_theme().config(cfg).buffers(BufferSpec::PartialDouble { rows }).dma(polls);
        h.run_until_idle();
        for (x, y, w, hh) in areas {
            let d = h.display();
            h.engine_mut().invalidate_area(d, Rect::from_xywh(x, y, w, hh), InvalidateReason::Explicit);
            h.advance(Duration::ms(16));
        }
        h.run_until_idle();
        assert_ownership(&dma_log());
    }
}
