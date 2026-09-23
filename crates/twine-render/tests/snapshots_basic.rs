//! Snapshots of the gallery pages (one source of truth for the gallery drawing code).
#![allow(clippy::manual_assert_eq)] // `assert!(a == b)` avoids dumping whole images on failure

#[path = "../../../examples/src/gallery/mod.rs"]
#[allow(dead_code)]
mod gallery;

use twine_core::{Color, ColorFormat, Opa, Rect};
use twine_testing::{RenderHarness, assert_render_snapshot};

fn page(n: usize, frame: u32) -> RenderHarness {
    let mut h = RenderHarness::new(gallery::W as u16, gallery::H as u16, ColorFormat::Rgb565);
    h.paint(|p| (gallery::PAGES[n].draw)(p, frame));
    h
}

#[test]
fn fill_page() {
    assert_render_snapshot!(page(0, 0), "fill_page");
}

#[test]
fn gallery_pages() {
    let names = [
        "gallery_fills",
        "gallery_rects",
        "gallery_gradients",
        "gallery_shadows",
        "gallery_masks",
        "gallery_layers",
        "gallery_lines",
        "gallery_arcs",
        "gallery_polygons",
        "gallery_transforms",
    ];
    assert_eq!(names.len(), gallery::PAGES.len());
    for (i, name) in names.iter().enumerate() {
        assert_render_snapshot!(page(i, 10), name);
    }
}

#[test]
fn gallery_pages_chunked_identical() {
    for (i, pg) in gallery::PAGES.iter().enumerate() {
        let full = page(i, 10);
        let mut chunked = RenderHarness::new(gallery::W as u16, gallery::H as u16, ColorFormat::Rgb565);
        chunked.paint_chunked(7, |p| (pg.draw)(p, 10));
        if i == 9 {
            // Transformed layers render their content with the layer's own clip; only the
            // composited result is compared (the layer covers the whole strip-independent area).
        }
        assert!(
            full.rgb888() == chunked.rgb888(),
            "page {} ({}) differs when chunked",
            i + 1,
            pg.title
        );
    }
}

#[test]
fn paint_chunked_equals_full_fill_scene() {
    let scene = |p: &mut twine_render::Painter<'_>| {
        p.fill(Rect::from_xywh(3, 5, 40, 30), Color::RED, Opa(200));
        p.fill(Rect::from_xywh(20, 1, 10, 50), Color::BLUE, Opa(90));
    };
    let mut a = RenderHarness::new(64, 48, ColorFormat::Rgb888);
    a.paint(scene);
    let mut b = RenderHarness::new(64, 48, ColorFormat::Rgb888);
    b.paint_chunked(5, scene);
    assert_eq!(a.rgb888(), b.rgb888());
}
