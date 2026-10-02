//! The fault stream (R0.S01): every recovered failure is raised, counted, kept as the last
//! record of its kind and reported to the hook — without allocating.

use std::sync::atomic::{AtomicU32, Ordering};

use twine_core::fault::{FaultKind, Faults};
use twine_core::{Color, ColorFormat, Instant, Opa, Rect};
use twine_engine::{BufferMode, Engine, EngineConfig, EngineError, FaultRecord, Obj};
use twine_hal::{DisplayDriver, DisplayInfo, DrawBufferMem, FramebufferDisplay};
use twine_style::{Selector, StyleProp};
use twine_testing::alloc::{CountingAllocator, count_allocs};

#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator;

/// A 64×32 RGB565 panel whose flushes always fail; the buffer comes back through `poll_flush`
/// (the documented driver contract on error).
struct FailingPanel(Option<DrawBufferMem>);

impl DisplayDriver for FailingPanel {
    type Error = &'static str;
    fn info(&self) -> DisplayInfo {
        DisplayInfo::new(64, 32, ColorFormat::Rgb565)
    }
    fn begin_flush(&mut self, _area: Rect, buf: DrawBufferMem) -> Result<(), Self::Error> {
        self.0 = Some(buf);
        Err("bus error")
    }
    fn poll_flush(&mut self) -> Option<DrawBufferMem> {
        self.0.take()
    }
}

/// A 64×32 framebuffer panel whose `present` always fails.
struct FailingFb(Option<(DrawBufferMem, Option<DrawBufferMem>)>);

impl FramebufferDisplay for FailingFb {
    type Error = &'static str;
    fn info(&self) -> DisplayInfo {
        DisplayInfo::new(64, 32, ColorFormat::Rgb565)
    }
    fn framebuffers(&mut self) -> Option<(DrawBufferMem, Option<DrawBufferMem>)> {
        self.0.take()
    }
    fn present(&mut self, _index: u8) -> Result<(), Self::Error> {
        Err("present failed")
    }
    fn present_done(&mut self) -> bool {
        true
    }
}

fn leak(len: usize) -> &'static mut [u8] {
    Box::leak(vec![0u8; len].into_boxed_slice())
}

/// An engine with the failing flush panel showing a coloured screen.
fn flush_engine() -> (Engine, twine_engine::DisplayId) {
    let mut e = Engine::new(EngineConfig::default()).unwrap();
    let d = e
        .add_display(FailingPanel(None), BufferMode::partial_single(leak(64 * 2 * 32)))
        .unwrap();
    let s = e.active_screen(d).unwrap();
    e.set_local_prop(s, Selector::MAIN, StyleProp::BgColor(Color::RED.into()));
    e.set_local_prop(s, Selector::MAIN, StyleProp::BgOpacity(Opa::COVER.into()));
    (e, d)
}

#[test]
fn flush_error_is_raised_with_its_display() {
    let (mut e, d) = flush_engine();
    assert!(e.pending_faults().is_empty());
    let _ = e.step(Instant::from_millis(5));
    assert!(e.pending_faults().contains(FaultKind::FlushError));
    let r = e.last_fault(FaultKind::FlushError).copied().unwrap();
    assert_eq!(r.display, Some(d));
    assert_eq!(r.at, Instant::from_millis(5));
    assert!(e.fault_counts().get(FaultKind::FlushError) >= 1);
}

#[test]
fn present_error_is_raised() {
    let mut e = Engine::new(EngineConfig::default()).unwrap();
    let fb = FailingFb(Some((DrawBufferMem::new(leak(64 * 2 * 32)), None)));
    let d = e.add_framebuffer_display(fb, BufferMode::direct()).unwrap();
    let s = e.active_screen(d).unwrap();
    e.set_local_prop(s, Selector::MAIN, StyleProp::BgColor(Color::BLUE.into()));
    let _ = e.step(Instant::from_millis(1));
    assert_eq!(e.take_faults(), Faults::from(FaultKind::FlushError));
    assert_eq!(
        e.last_fault(FaultKind::FlushError).and_then(|r| r.display),
        Some(d)
    );
}

#[test]
fn capacity_is_raised_when_the_tree_is_full() {
    let cfg = EngineConfig {
        max_nodes: 8,
        ..EngineConfig::default()
    };
    let mut e = Engine::new(cfg).unwrap();
    let root = e.create_root(Box::new(Obj)).unwrap();
    let mut created = 1;
    let full = loop {
        match e.create(root, Box::new(Obj)) {
            Ok(_) => created += 1,
            Err(err) => break err,
        }
    };
    assert_eq!(created, 8);
    assert_eq!(full, EngineError::TooManyNodes);
    assert_eq!(e.take_faults(), Faults::from(FaultKind::Capacity));
    assert_eq!(e.last_fault(FaultKind::Capacity).and_then(|r| r.node), Some(root));
    assert!(e.create_root(Box::new(Obj)).is_err());
    assert!(
        e.last_fault(FaultKind::Capacity).unwrap().node.is_none(),
        "roots have no parent"
    );
    assert_eq!(e.fault_counts().get(FaultKind::Capacity), 2);
}

#[test]
fn dead_node_is_never_handed_out_and_every_call_ignores_it() {
    use twine_engine::DEAD_NODE;
    let cfg = EngineConfig {
        max_nodes: 8,
        ..EngineConfig::default()
    };
    let mut e = Engine::new(cfg).unwrap();
    let root = e.create_root(Box::new(Obj)).unwrap();
    let mut ids = vec![root];
    while let Ok(id) = e.create(root, Box::new(Obj)) {
        ids.push(id);
    }
    assert!(ids.iter().all(|&id| id != DEAD_NODE));
    assert!(!e.tree().contains(DEAD_NODE));
    let len = e.tree().len();
    e.set_local_prop(DEAD_NODE, Selector::MAIN, StyleProp::BgColor(Color::RED.into()));
    assert!(e.widget::<Obj>(DEAD_NODE).is_none());
    assert!(e.delete(DEAD_NODE).is_err());
    assert!(e.create(DEAD_NODE, Box::new(Obj)).is_err());
    assert_eq!(e.tree().len(), len, "no node was created, deleted or aliased");
    assert!(e.tree().children(DEAD_NODE).next().is_none());
}

#[test]
fn zero_max_nodes_is_rejected() {
    let cfg = EngineConfig {
        max_nodes: 0,
        ..EngineConfig::default()
    };
    assert!(matches!(Engine::new(cfg), Err(EngineError::InvalidConfig(_))));
}

#[test]
fn take_clears_the_pending_set_but_keeps_counts_and_records() {
    let mut e = Engine::new(EngineConfig::default()).unwrap();
    e.raise_fault(
        FaultRecord::new(FaultKind::ChannelOverflow)
            .occurrences(4)
            .code(9),
    );
    e.raise_fault(FaultRecord::new(FaultKind::DepthGuard));
    assert_eq!(
        e.take_faults(),
        Faults::from(FaultKind::ChannelOverflow) | FaultKind::DepthGuard
    );
    assert!(e.take_faults().is_empty());
    assert_eq!(e.fault_counts().get(FaultKind::ChannelOverflow), 4);
    assert_eq!(e.fault_counts().total(), 5);
    let r = e.last_fault(FaultKind::ChannelOverflow).unwrap();
    assert_eq!((r.occurrences, r.code), (4, 9));
    assert!(e.last_fault(FaultKind::FlushTimeout).is_none());
}

static HOOK_CALLS: AtomicU32 = AtomicU32::new(0);
static HOOK_OCCURRENCES: AtomicU32 = AtomicU32::new(0);

fn hook(r: &FaultRecord) {
    if r.kind == FaultKind::InputDevice {
        HOOK_CALLS.fetch_add(1, Ordering::Relaxed);
        HOOK_OCCURRENCES.fetch_add(r.occurrences, Ordering::Relaxed);
    }
}

#[test]
fn hook_is_called_once_per_raise() {
    let mut e = Engine::new(EngineConfig::default()).unwrap();
    e.set_fault_hook(Some(hook));
    e.raise_fault(FaultRecord::new(FaultKind::InputDevice));
    e.raise_fault(FaultRecord::new(FaultKind::InputDevice).occurrences(2));
    assert_eq!(HOOK_CALLS.load(Ordering::Relaxed), 2);
    assert_eq!(HOOK_OCCURRENCES.load(Ordering::Relaxed), 3);
    e.set_fault_hook(None);
    e.raise_fault(FaultRecord::new(FaultKind::InputDevice));
    assert_eq!(HOOK_CALLS.load(Ordering::Relaxed), 2);
}

#[test]
fn zero_occurrences_count_as_one() {
    assert_eq!(
        FaultRecord::new(FaultKind::Capacity).occurrences(0).occurrences,
        1
    );
}

fn noop_hook(_: &FaultRecord) {}

#[test]
fn raising_a_fault_does_not_allocate() {
    let mut e = Engine::new(EngineConfig::default()).unwrap();
    e.set_fault_hook(Some(noop_hook));
    let ((), stats) = count_allocs(|| {
        for k in FaultKind::ALL {
            e.raise_fault(FaultRecord::new(k).code(1));
        }
        let _ = e.take_faults();
    });
    assert_eq!(stats.allocs + stats.reallocs, 0, "{stats:?}");
    assert_eq!(e.fault_counts().kinds().len(), FaultKind::COUNT);
}
