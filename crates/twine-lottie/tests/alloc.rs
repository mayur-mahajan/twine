//! Steady-state playback allocates nothing: after one pass over the animation (which grows the
//! scratch buffers), rendering every frame again makes no heap allocation.

use twine_core::ColorFormat;
use twine_lottie::{LottiePlayer, load};
use twine_testing::RenderHarness;
use twine_testing::alloc::{CountingAllocator, count_allocs};

#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator;

#[test]
fn no_alloc_per_frame_after_warm_up() {
    for name in [
        "loader.json",
        "check.json",
        "heart.json",
        "masked.json",
        "precomp.json",
    ] {
        let bytes = std::fs::read(format!("{}/tests/data/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap();
        let comp = load(&bytes).unwrap();
        let (ip, op) = (comp.ip, comp.op);
        let mut p = LottiePlayer::new(comp);
        let mut h = RenderHarness::new(100, 100, ColorFormat::Rgb565);
        let area = h.area();
        let pass = |h: &mut RenderHarness, p: &mut LottiePlayer| {
            let mut f = ip;
            while f < op {
                h.paint(|pt| p.render_frame(f, pt, area));
                f += 1.0;
            }
        };
        pass(&mut h, &mut p);
        let before = p.bytes_reserved();
        let ((), stats) = count_allocs(|| pass(&mut h, &mut p));
        assert_eq!(stats.allocs + stats.reallocs, 0, "{name}: {stats:?}");
        assert_eq!(p.bytes_reserved(), before, "{name}");
    }
}
