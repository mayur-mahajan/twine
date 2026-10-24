//! R5.S03: the engine's share of `Limits` — each bound refuses with a `Capacity` fault of its
//! own code and never grows the bounded storage; preallocation reserves it up front; the
//! delete scratch is reused; growth after a seal raises `AllocAfterSeal` naming the growth
//! point; the configuration audit lists every field.
//!
//! req: SAF-008

use twine_anim::{Anim, AnimId, TimerId};
use twine_core::alloc_guard::{self, AllocSite};
use twine_core::audit::KeyValues;
use twine_core::fault::{CapacityFault, FaultKind};
use twine_core::{Duration, Instant, Limits};
use twine_engine::{
    AnimProp, Engine, EngineConfig, EngineError, EventCode, EventFilter, EventResult, MAX_GROUPS, NodeId, Obj,
};
use twine_testing::alloc::{CountingAllocator, assert_no_alloc_after_seal, count_allocs};

#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator;

/// An engine with `limits`, preallocated in the safety profile (which refuses any other).
fn engine(limits: Limits) -> Engine {
    let mut e = Engine::new(EngineConfig::default()).unwrap();
    let limits = if twine_core::SAFETY_PROFILE {
        limits.preallocated()
    } else {
        limits
    };
    e.set_limits(&limits).unwrap();
    e
}

/// Small tables besides the bound under test, so preallocating them (safety profile) is
/// cheap.
const SMALL: Limits = Limits::DEFAULT.max_nodes(64).max_timers(8).max_anims(8);

/// The code of the last `Capacity` record, clearing the pending set.
fn capacity_code(e: &mut Engine) -> Option<CapacityFault> {
    let _ = e.take_faults();
    e.last_fault(FaultKind::Capacity)
        .and_then(|r| CapacityFault::from_code(r.code))
}

/// fault: `Capacity`
/// req: SAF-031
#[test]
fn node_limit_refuses_without_growing_the_preallocated_tree() {
    let mut e = engine(Limits::DEFAULT.max_nodes(16).preallocated());
    let reserved = e.memory_report().node_slots;
    let root = e.create_root(Box::new(Obj)).unwrap();
    let mut ids = Vec::new();
    while let Ok(n) = e.create(root, Box::new(Obj)) {
        ids.push(n);
    }
    assert_eq!(e.tree().len(), 16);
    assert_eq!(capacity_code(&mut e), Some(CapacityFault::Nodes));
    assert_eq!(e.memory_report().node_slots, reserved, "no slot was added");
    // Churn within the bound reuses the reserved storage: no allocation by the tree.
    let tree = e.memory_report().tree;
    for &n in &ids {
        e.delete(n).unwrap();
    }
    for _ in 0..ids.len() {
        e.create(root, Box::new(Obj)).unwrap();
    }
    assert_eq!(e.memory_report().tree, tree);
}

#[test]
fn screens_count_against_the_node_limit() {
    let mut e = engine(SMALL.max_nodes(5));
    let d = e
        .add_chunked_display(
            twine_hal::DisplayInfo::new(16, 8, twine_core::ColorFormat::Rgb565),
            16 * 2 * 8,
            None,
        )
        .unwrap();
    // 4 layer and screen nodes: one more screen fits, a second does not.
    e.create_screen(d).unwrap();
    assert_eq!(e.create_screen(d), Err(EngineError::TooManyNodes));
    assert_eq!(capacity_code(&mut e), Some(CapacityFault::Nodes));
}

/// req: SAF-013
#[test]
fn tree_depth_limit_refuses_creation_and_moves() {
    let mut e = engine(SMALL.max_tree_depth(2));
    let root = e.create_root(Box::new(Obj)).unwrap();
    let a = e.create(root, Box::new(Obj)).unwrap(); // level 1
    let b = e.create(a, Box::new(Obj)).unwrap(); // level 2
    assert_eq!(e.create(b, Box::new(Obj)), Err(EngineError::TooDeep));
    assert_eq!(capacity_code(&mut e), Some(CapacityFault::TreeDepth));
    assert_eq!(e.last_fault(FaultKind::Capacity).and_then(|r| r.node), Some(b));
    // Moving `a` (with `b` below it) under its sibling would put `b` at level 3.
    let c = e.create(root, Box::new(Obj)).unwrap();
    assert_eq!(e.move_node(a, c, None), Err(EngineError::TooDeep));
    assert_eq!(e.tree().parent(a), Some(root), "the node stayed");
    // A leaf fits there.
    let leaf = e.create(root, Box::new(Obj)).unwrap();
    e.move_node(leaf, c, None).unwrap();
}

#[test]
fn handler_limit_counts_every_node_and_frees_on_delete() {
    let mut e = engine(SMALL.max_handlers(2));
    let root = e.create_root(Box::new(Obj)).unwrap();
    let a = e.create(root, Box::new(Obj)).unwrap();
    let h = |_: &mut twine_engine::EventCx<'_>, _: &twine_engine::Event| EventResult::Continue;
    let first = e.add_event_handler(root, EventFilter::Code(EventCode::Clicked), h);
    e.add_event_handler(a, EventFilter::All, h);
    e.add_event_handler(a, EventFilter::All, h);
    assert_eq!(e.event_handler_count(a), 1, "the third handler was refused");
    assert_eq!(capacity_code(&mut e), Some(CapacityFault::Handlers));
    // Removing one, or deleting a node with handlers, makes room.
    assert!(e.remove_event_handler(root, first));
    e.add_event_handler(root, EventFilter::All, h);
    e.delete(a).unwrap();
    e.add_event_handler(root, EventFilter::All, h);
    assert_eq!(e.event_handler_count(root), 2);
}

#[test]
fn timer_and_animation_limits_return_dangling_ids_with_a_fault() {
    let mut e = engine(Limits::DEFAULT.max_timers(1).max_anims(1).preallocated());
    let root = e.create_root(Box::new(Obj)).unwrap();
    assert_ne!(e.timer_add(Duration::ms(10), |_, _| {}), TimerId::DANGLING);
    assert_eq!(e.timer_add(Duration::ms(10), |_, _| {}), TimerId::DANGLING);
    assert_eq!(capacity_code(&mut e), Some(CapacityFault::Timers));
    assert_eq!(e.timer_count(), 1);
    let anim = || Anim::new(0, 10).duration(Duration::ms(100));
    assert_ne!(e.anim_start(root, AnimProp::X, anim()), AnimId::DANGLING);
    assert_eq!(e.anim_start(root, AnimProp::Y, anim()), AnimId::DANGLING);
    assert_eq!(capacity_code(&mut e), Some(CapacityFault::Animations));
}

#[test]
fn focus_group_limit_raises_its_code() {
    let mut e = Engine::new(EngineConfig::default()).unwrap();
    for _ in 0..MAX_GROUPS {
        e.create_group().unwrap();
    }
    assert_eq!(e.create_group(), Err(EngineError::TooManyGroups));
    assert_eq!(capacity_code(&mut e), Some(CapacityFault::FocusGroups));
}

#[test]
fn delete_reuses_the_preallocated_scratch() {
    let mut e = engine(Limits::DEFAULT.max_nodes(64).preallocated());
    let root = e.create_root(Box::new(Obj)).unwrap();
    let build = |e: &mut Engine| -> NodeId {
        let sub = e.create(root, Box::new(Obj)).unwrap();
        let mut parent = sub;
        for _ in 0..20 {
            parent = e.create(parent, Box::new(Obj)).unwrap();
        }
        sub
    };
    // Warm-up: the first delete sizes the layout's re-adjust list (a kept scratch).
    let sub = build(&mut e);
    e.delete(sub).unwrap();
    let sub = build(&mut e);
    let ((), stats) = count_allocs(|| e.delete(sub).unwrap());
    assert_eq!(stats.allocs + stats.reallocs, 0, "{stats:?}");
    // And a delete from a `Delete` handler (nested) works on its own segment.
    let sub = build(&mut e);
    let other = build(&mut e);
    e.add_event_handler(sub, EventFilter::Code(EventCode::Delete), move |cx, _| {
        let _ = cx.engine_mut().delete(other);
        EventResult::Continue
    });
    e.delete(sub).unwrap();
    assert!(!e.tree().contains(other));
    assert_eq!(e.tree().len(), 1);
}

/// fault: `AllocAfterSeal`
/// req: SAF-009, SAF-031
#[test]
fn growth_after_seal_raises_alloc_after_seal_with_the_site() {
    let mut e = engine(Limits::DEFAULT.max_timers(4).preallocated());
    let root = e.create_root(Box::new(Obj)).unwrap();
    // Warm-up: the list of timers waiting for their first update (a kept scratch).
    let t = e.timer_add(Duration::ms(5), |_, _| {});
    e.timer_remove(t);
    let site = |e: &mut Engine| {
        e.raise_alloc_after_seal();
        let r = e.last_fault(FaultKind::AllocAfterSeal).copied();
        r.and_then(|r| AllocSite::from_code(r.code))
    };
    alloc_guard::seal(twine_core::Partition::MAIN);
    e.create(root, Box::new(Obj)).unwrap();
    assert_eq!(site(&mut e), Some(AllocSite::Widget));
    e.add_event_handler(root, EventFilter::All, |_, _| EventResult::Continue);
    assert_eq!(site(&mut e), Some(AllocSite::Handler));
    e.create_group().unwrap();
    assert_eq!(site(&mut e), Some(AllocSite::FocusGroup));
    // A timer within the preallocated table is noted for its boxed callback only through
    // the creation (`Timer` table growth is not reached).
    let before = alloc_guard::total(twine_core::Partition::MAIN);
    let ((), stats) = count_allocs(|| {
        e.timer_add(Duration::ms(5), |_, _| {});
    });
    assert!(stats.allocs > 0, "the callback is boxed");
    assert_eq!(
        alloc_guard::total(twine_core::Partition::MAIN),
        before,
        "the reserved table did not grow"
    );
    // Steady state: frames of an idle engine allocate nothing.
    e.run_timers(Instant::ZERO);
    assert_no_alloc_after_seal(|| {
        e.run_anims(Instant::from_millis(1));
    });
    alloc_guard::unseal(twine_core::Partition::MAIN);
    assert_eq!(e.fault_counts().get(FaultKind::AllocAfterSeal), 3);
}

#[test]
fn engine_config_audit_lists_every_field() {
    let text = KeyValues(&EngineConfig::default()).to_string();
    for key in [
        "refr_period_us",
        "max_dirty_areas",
        "flush_timeout_us",
        "flush_policy",
        "layer_buf_bytes",
        "glyph_cache_bytes",
        "image_cache_bytes",
        "circle_cache_entries",
        "gradient_cache_entries",
        "shadow_cache_entries",
        "long_press_time_us",
        "read_period_us",
        "idle_timeout_us",
        "step_budget_chunks",
        "step_budget_time_us",
        "on_chunk",
    ] {
        assert!(text.contains(&format!("engine.{key}=")), "{key} missing:\n{text}");
    }
    assert_eq!(text.lines().count(), 30);
}
