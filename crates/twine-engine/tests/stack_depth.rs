//! Stack cost per tree level of every recursive engine path (R5.S05).
//!
//! Drawing, the opaque-cover search, hit testing and layout recurse over the widget tree, so
//! their stack use grows with the tree depth, which `Limits::max_tree_depth` bounds. Each
//! scene here is a chain of nested nodes ending in a probe widget that records the deepest
//! stack address it is called at (the address of a local: safe Rust, no stack painting). The
//! difference between a chain of `N` and one of `2N` levels, divided by `N`, is the stack one
//! level costs on that path — exact, because every level runs the same code. The constant
//! part (from the caller's frame to the first level) is reported as `base`.
//!
//! The numbers are those of the compiling target and profile: `cargo xtask stack-budget`
//! runs this test in release mode and checks the worst case for the configured depth against
//! the budget (`docs/safety/profile.md` § 10). In a debug build the test only checks that the
//! measurement is consistent (linear in the depth).
//!
//! req: SAF-013

use std::cell::Cell;

use twine_core::{ColorFormat, Instant, Opa, Point, Rect, Size};
use twine_engine::{
    BufferMode, BufferSpec, DrawCx, Engine, EngineConfig, MeasureCx, NodeId, Obj, ObjFlags, Widget,
    WidgetClass,
};
use twine_hal::DisplayInfo;
use twine_style::{FlexFlow, GridTrack, LayoutKind, Length, Selector, StyleProp};
use twine_testing::{HARNESS_LIMITS, MemoryDisplay};

/// The phases a probe records separately.
#[derive(Clone, Copy)]
enum Phase {
    Draw = 0,
    Cover = 1,
    Hit = 2,
    Measure = 3,
}

thread_local! {
    /// The deepest (lowest) stack address each phase reached.
    static DEEPEST: [Cell<usize>; 4] = const { [const { Cell::new(usize::MAX) }; 4] };
}

/// The current stack address (of a local of this frame).
#[inline(never)]
fn stack_here() -> usize {
    let marker = 0u8;
    std::ptr::from_ref(std::hint::black_box(&marker)).addr()
}

fn record(phase: Phase) {
    let sp = stack_here();
    DEEPEST.with(|d| {
        let c = &d[phase as usize];
        c.set(c.get().min(sp));
    });
}

fn reset() {
    DEEPEST.with(|d| d.iter().for_each(|c| c.set(usize::MAX)));
}

fn deepest(phase: Phase) -> usize {
    DEEPEST.with(|d| d[phase as usize].get())
}

/// The leaf of every chain: records where each recursive path reaches it.
struct Probe;
static PROBE_CLASS: WidgetClass = WidgetClass::new("stack_probe");

impl Widget for Probe {
    fn class(&self) -> &'static WidgetClass {
        &PROBE_CLASS
    }
    fn draw(&self, _: &mut DrawCx<'_, '_>) {
        record(Phase::Draw);
    }
    fn content_size(&self, _: &MeasureCx<'_>) -> Size {
        record(Phase::Measure);
        Size::new(4, 4)
    }
    fn hit_test(&self, _: &MeasureCx<'_>, _: Point) -> bool {
        record(Phase::Hit);
        true
    }
    fn covers(&self, _: &MeasureCx<'_>, _: Rect) -> bool {
        record(Phase::Cover);
        false
    }
}

/// How the levels of a chain are styled.
#[derive(Clone, Copy, Debug)]
enum Scene {
    /// Fixed-size, transparent nodes without a layout (the plain draw, cover, hit-test and
    /// arrange recursion).
    Plain,
    /// [`Plain`](Self::Plain) with a full-size leaf too: the opaque-cover search reaches the
    /// leaf only when it contains the whole area.
    Full,
    /// Every level a translucent group (`Opacity` < cover): drawing goes through the layer
    /// path at every level.
    Layers,
    /// Every level content-sized: measuring the outermost node measures the whole chain.
    Content,
    /// Every level a content-sized flex container.
    Flex,
    /// Every level a grid container with one content-sized track per axis.
    Grid,
}

static CONTENT_TRACK: [GridTrack; 1] = [GridTrack::Content];

const W: u16 = 64;
const H: u16 = 48;

/// An engine with one partial-buffer display and a chain of `depth` levels (the last one
/// the probe) on its active screen.
fn chain(depth: usize, scene: Scene) -> (Engine, twine_engine::DisplayId) {
    let mut e = Engine::new(EngineConfig::default()).unwrap();
    e.set_limits(&HARNESS_LIMITS).unwrap();
    let panel = MemoryDisplay::new(DisplayInfo::new(W, H, ColorFormat::Rgb565));
    let d = e
        .add_display(
            panel,
            BufferMode::alloc(BufferSpec::PartialSingle { rows: H }),
            None,
        )
        .unwrap();
    let mut parent = e.active_screen(d).unwrap();
    for level in 1..=depth {
        let widget: Box<dyn Widget> = if level == depth {
            Box::new(Probe)
        } else {
            Box::new(Obj)
        };
        let n = e.create(parent, widget).unwrap();
        style_level(&mut e, n, scene, level == depth);
        parent = n;
    }
    (e, d)
}

fn style_level(e: &mut Engine, n: NodeId, scene: Scene, leaf: bool) {
    // No padding or border: every level has the screen's area.
    for p in [
        StyleProp::PaddingLeft(Length::Px(0).into()),
        StyleProp::PaddingRight(Length::Px(0).into()),
        StyleProp::PaddingTop(Length::Px(0).into()),
        StyleProp::PaddingBottom(Length::Px(0).into()),
    ] {
        e.set_local_prop(n, Selector::MAIN, p);
    }
    e.set_flag(n, ObjFlags::CLICKABLE, true);
    match scene {
        Scene::Plain | Scene::Full | Scene::Layers => {
            if leaf && !matches!(scene, Scene::Full) {
                // Content-sized, so the parent's arrange measures it.
                e.set_size(n, Length::Content, Length::Content);
            } else {
                e.set_size(n, Length::pct(100), Length::pct(100));
            }
            if matches!(scene, Scene::Layers) {
                e.set_local_prop(n, Selector::MAIN, StyleProp::Opacity(Opa::from_raw(200).into()));
            }
        }
        Scene::Content => e.set_size(n, Length::Content, Length::Content),
        Scene::Flex => {
            e.set_size(n, Length::Content, Length::Content);
            e.set_layout(n, LayoutKind::Flex);
            e.set_flex_flow(n, FlexFlow::ROW);
        }
        Scene::Grid => {
            e.set_size(n, Length::Content, Length::Content);
            e.set_layout(n, LayoutKind::Grid);
            e.set_grid_tracks(n, &CONTENT_TRACK, &CONTENT_TRACK);
        }
    }
}

/// The deepest stack address of `phase` while `op` runs on a fresh chain of `depth` levels,
/// and the stack address of the caller of `op`.
fn measure(
    depth: usize,
    scene: Scene,
    phase: Phase,
    op: fn(&mut Engine, twine_engine::DisplayId),
) -> (usize, usize) {
    let (mut e, d) = chain(depth, scene);
    reset();
    let entry = stack_here();
    op(&mut e, d);
    let deep = deepest(phase);
    assert_ne!(
        deep,
        usize::MAX,
        "{scene:?}: the probe was not reached at depth {depth}"
    );
    (entry, deep)
}

fn step(e: &mut Engine, _: twine_engine::DisplayId) {
    let _ = e.step(Instant::from_millis(0));
}

fn hit(e: &mut Engine, d: twine_engine::DisplayId) {
    let _ = e.step(Instant::from_millis(0));
    reset();
    let _ = e.hit_test(d, Point::new(2, 2));
}

/// One measured path: bytes per level and the constant part.
struct PathCost {
    name: &'static str,
    per_level: usize,
    base: usize,
}

/// Measures `name` on chains of `n`, `2n` and `3n` levels.
fn path(
    name: &'static str,
    n: usize,
    scene: Scene,
    phase: Phase,
    op: fn(&mut Engine, twine_engine::DisplayId),
) -> PathCost {
    let (entry, d1) = measure(n, scene, phase, op);
    let (_, d2) = measure(2 * n, scene, phase, op);
    let (_, d3) = measure(3 * n, scene, phase, op);
    // The stack grows downwards: deeper chains reach lower addresses.
    assert!(d2 < d1 && d3 < d2, "{name}: no growth with the depth");
    let a = (d1 - d2) / n;
    let b = (d2 - d3) / n;
    // Every level runs the same code, so the cost per level does not depend on the depth.
    assert_eq!(a, b, "{name}: per-level cost not constant ({a} vs {b} bytes)");
    let base = (entry - d1).saturating_sub(a * n);
    PathCost {
        name,
        per_level: b,
        base,
    }
}

/// Every recursive engine path. `cargo xtask stack-budget` parses the `twine-stack` lines.
#[test]
fn stack_cost_per_level_of_every_recursive_path() {
    let paths = [
        path("draw", 16, Scene::Plain, Phase::Draw, step),
        path("draw_layers", 16, Scene::Layers, Phase::Draw, step),
        path("top_cover", 16, Scene::Full, Phase::Cover, step),
        path("hit_test", 16, Scene::Plain, Phase::Hit, hit),
        path("layout_arrange", 16, Scene::Plain, Phase::Measure, step),
        path("layout_content", 16, Scene::Content, Phase::Measure, step),
        path("layout_flex", 16, Scene::Flex, Phase::Measure, step),
        path("layout_grid", 16, Scene::Grid, Phase::Measure, step),
    ];
    for p in &paths {
        println!(
            "twine-stack: path={} per_level={} base={} profile={}",
            p.name,
            p.per_level,
            p.base,
            if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            }
        );
        assert!(p.per_level > 0, "{}: no stack per level", p.name);
    }
}

/// A tree deeper than `Limits::max_tree_depth` cannot be built, so the measured cost per
/// level times the limit bounds every recursive path (also `Engine::set_limits` refuses a
/// depth limit the tree already exceeds).
#[test]
fn recursion_depth_is_bounded_by_the_tree_depth_limit() {
    let mut e = Engine::new(EngineConfig::default()).unwrap();
    e.set_limits(&HARNESS_LIMITS.max_tree_depth(8)).unwrap();
    let panel = MemoryDisplay::new(DisplayInfo::new(W, H, ColorFormat::Rgb565));
    let d = e
        .add_display(
            panel,
            BufferMode::alloc(BufferSpec::PartialSingle { rows: H }),
            None,
        )
        .unwrap();
    let mut parent = e.active_screen(d).unwrap();
    for _ in 0..8 {
        parent = e.create(parent, Box::new(Obj)).unwrap();
    }
    assert!(e.create(parent, Box::new(Obj)).is_err(), "level 9 refused");
    // A lower depth limit than the tree already has is refused: the bound would not hold.
    assert!(e.set_limits(&HARNESS_LIMITS.max_tree_depth(7)).is_err());
    assert_eq!(e.limits().max_tree_depth, 8);
    assert!(e.set_limits(&HARNESS_LIMITS.max_tree_depth(8)).is_ok());
}
