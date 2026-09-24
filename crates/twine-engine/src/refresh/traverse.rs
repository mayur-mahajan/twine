//! Draw traversal: [`find_top_cover`], drawing a dirty area back to front with clipping,
//! opacity groups, transforms and blend modes (layers).

use twine_core::{Color, Opa, Rect};
use twine_render::{LayerDsc, Mask, Painter};
use twine_style::{BlendMode, Part, PropId};

use crate::draw_cx::AuxRes;
use crate::widget::effective_radius;
use crate::{DrawCx, Engine, MeasureCx, NodeId, ObjFlags};

/// Color of the layout-bounds overlay (`Engine::set_bounds_overlay`).
const BOUNDS_COLOR: Color = Color::new(0xFF, 0x00, 0xFF);

/// The deepest, topmost node of `id`'s subtree that fully covers `area` with opaque pixels
/// (LVGL `lv_refr_get_top_obj`). Nodes that need a layer or mask their children's corners are
/// not descended into.
pub(crate) fn find_top_cover(engine: &Engine, id: NodeId, area: Rect) -> Option<NodeId> {
    let n = engine.tree.node(id)?;
    if n.is_hidden() || !n.coords().contains_rect(&area) || engine.needs_layer(id) {
        return None;
    }
    if clips_corners(engine, id) {
        let r = effective_radius(n.coords(), engine.cached_main(id).radius);
        let c = n.coords();
        let inner_h = c.inset(twine_core::Insets::new(r, 0, r, 0));
        let inner_v = c.inset(twine_core::Insets::new(0, r, 0, r));
        if !inner_h.contains_rect(&area) && !inner_v.contains_rect(&area) {
            return None;
        }
    }
    for c in engine.tree.children_rev(id) {
        if let Some(f) = find_top_cover(engine, c, area) {
            return Some(f);
        }
    }
    // A wrapper draws nothing of its own, so it never covers anything.
    (!n.flags().contains(ObjFlags::LAYOUT_PASSTHROUGH)
        && n.widget().covers(&MeasureCx::new(engine, id), area))
    .then_some(id)
}

/// Whether a node with `flags` clips its children to its coordinates (not with
/// `OVERFLOW_VISIBLE`, and never for `LAYOUT_PASSTHROUGH` wrappers).
fn clips_children(flags: ObjFlags) -> bool {
    !flags.intersects(ObjFlags::OVERFLOW_VISIBLE.union(ObjFlags::LAYOUT_PASSTHROUGH))
}

/// Whether `id` clips its children to its rounded corners (`ClipCorner` with a radius).
fn clips_corners(engine: &Engine, id: NodeId) -> bool {
    engine.cached_main(id).radius > 0
        && engine
            .style_prop(id, Part::Main, PropId::ClipCorner)
            .as_bool()
            .unwrap_or(false)
}

/// The clip for `id`'s children inside `area`: `area` clipped by `id` and every ancestor that
/// does not have `OVERFLOW_VISIBLE`.
fn children_clip(engine: &Engine, id: NodeId, area: Rect) -> Option<Rect> {
    let mut clip = area;
    let mut cur = Some(id);
    while let Some(c) = cur {
        let n = engine.tree.node(c)?;
        if clips_children(n.flags()) {
            clip = clip.intersection(&n.coords())?;
        }
        cur = n.parent();
    }
    Some(clip)
}

/// The effective opacity of `id` (its `Opa` times its ancestors').
fn opa_chain(engine: &Engine, id: Option<NodeId>) -> Opa {
    let mut opa = Opa::COVER;
    let mut cur = id;
    while let Some(c) = cur {
        opa = opa.mul(engine.cached_main(c).opa);
        cur = engine.tree.parent(c);
    }
    opa
}

/// Draws everything visible in `area` of display `d`. The painter's clip must be `area`.
///
/// Roots bottom to top: bottom layer, the screens (during a screen load animation the previous
/// screen below the active one, or above it for "out" animations), top layer, system layer.
/// Drawing starts at the topmost node that covers `area` opaquely (LVGL `refr_area_part`).
pub(crate) fn draw_area(engine: &Engine, p: &mut Painter<'_>, aux: &mut AuxRes, d: usize, area: Rect) {
    let disp = &engine.displays[d];
    let mut roots: heapless::Vec<NodeId, 5> = heapless::Vec::new();
    let _ = roots.push(disp.bottom_layer);
    match disp.prev_screen {
        Some(prev) if disp.draw_prev_over_act => {
            let _ = roots.push(disp.active_screen);
            let _ = roots.push(prev);
        }
        Some(prev) => {
            let _ = roots.push(prev);
            let _ = roots.push(disp.active_screen);
        }
        None => {
            let _ = roots.push(disp.active_screen);
        }
    }
    let _ = roots.push(disp.top_layer);
    let _ = roots.push(disp.sys_layer);
    // Topmost cover: from the system layer down to the lowest screen.
    let start = (1..roots.len())
        .rev()
        .find_map(|i| find_top_cover(engine, roots[i], area).map(|t| (i, t)));
    if let Some((root_idx, top)) = start {
        draw_from(engine, p, aux, top, area);
        for &r in &roots[root_idx + 1..] {
            draw_node(engine, p, aux, r, area, Opa::COVER);
        }
    } else {
        // Nothing opaque covers the area: start from a defined background.
        p.fill(area, Color::WHITE, Opa::COVER);
        for &r in &roots {
            draw_node(engine, p, aux, r, area, Opa::COVER);
        }
    }
    if engine.bounds_overlay {
        for &r in &roots {
            draw_bounds(engine, p, r);
        }
    }
}

/// Draws `start`, then for each ancestor the siblings after the branch and the ancestor's
/// post-drawing (LVGL `refr_obj_and_children`).
fn draw_from(engine: &Engine, p: &mut Painter<'_>, aux: &mut AuxRes, start: NodeId, area: Rect) {
    let parent = engine.tree.parent(start);
    let clip = parent.map_or(Some(area), |par| children_clip(engine, par, area));
    if let Some(clip) = clip {
        draw_node(engine, p, aux, start, clip, opa_chain(engine, parent));
    }
    let mut cur = start;
    while let Some(par) = engine.tree.parent(cur) {
        let par_opa = opa_chain(engine, Some(par));
        if let Some(clip) = children_clip(engine, par, area) {
            let mut sib = engine.tree.node(cur).and_then(crate::Node::next_sibling);
            while let Some(s) = sib {
                draw_node(engine, p, aux, s, clip, par_opa);
                sib = engine.tree.node(s).and_then(crate::Node::next_sibling);
            }
        }
        let own_clip = match engine.tree.parent(par) {
            Some(gp) => children_clip(engine, gp, area),
            None => Some(area),
        };
        if let Some(c) = own_clip {
            p.with_clip(c, |p| draw_post(engine, p, aux, par, par_opa));
        }
        cur = par;
    }
}

/// Draws `id` and its subtree inside `clip` (`parent_opa` = the parent's effective opacity).
pub(crate) fn draw_node(
    engine: &Engine,
    p: &mut Painter<'_>,
    aux: &mut AuxRes,
    id: NodeId,
    clip: Rect,
    parent_opa: Opa,
) {
    let Some(n) = engine.tree.node(id) else {
        return;
    };
    if n.is_hidden() {
        return;
    }
    let ext_area = n.coords().expand(i32::from(n.ext_draw()));
    let visible = ext_area.intersection(&clip).is_some();
    if !visible {
        // Children of an `OVERFLOW_VISIBLE` node may still reach into the clip (LVGL
        // `lv_obj_redraw` keeps walking the children).
        if !clips_children(n.flags()) && !engine.needs_layer(id) {
            let opa = parent_opa.mul(engine.cached_main(id).opa);
            draw_children(engine, p, aux, id, clip, opa);
        }
        return;
    }
    engine.nodes_drawn.set(engine.nodes_drawn.get().saturating_add(1));
    n.rendered.set(true);
    let opa = parent_opa.mul(engine.cached_main(id).opa);
    if engine.needs_layer(id) {
        let t = engine.layer_transform(id);
        let base_ext = if t.is_some() {
            engine.ext_draw_untransformed(id)
        } else {
            n.ext_draw()
        };
        let layer_area = n.coords().expand(i32::from(base_ext));
        let dsc = LayerDsc {
            opa: engine.style_opa(id, Part::Main, PropId::OpaLayered),
            blend_mode: engine
                .style_prop(id, Part::Main, PropId::BlendMode)
                .get::<BlendMode>()
                .unwrap_or(BlendMode::Normal),
            transform: t.map(|mut t| {
                // The pivot is relative to the node; the layer starts `base_ext` further out.
                t.pivot.x += i32::from(base_ext);
                t.pivot.y += i32::from(base_ext);
                t
            }),
        };
        p.with_clip(clip, |p| {
            let mask = bitmap_mask(engine, id).map(|m| p.push_mask(m));
            p.layer(layer_area, &dsc, |lp| {
                let c = lp.clip();
                draw_content(engine, lp, aux, id, c, opa);
            });
            if let Some(m) = mask {
                p.pop_mask(m);
            }
        });
    } else {
        draw_content(engine, p, aux, id, clip, opa);
    }
}

static WARN_MASK: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// The `BitmapMaskSrc` of `id` as a mask centered on the node: an uncompressed static `A8` or
/// `L8` image (other sources are not supported as masks and are ignored with a warning).
fn bitmap_mask(engine: &Engine, id: NodeId) -> Option<Mask<'static>> {
    use core::sync::atomic::Ordering;
    let src = engine
        .style_prop(id, Part::Main, PropId::BitmapMaskSrc)
        .get::<&'static twine_image::ImageSource>()?;
    let mask = match src {
        twine_image::ImageSource::Static(img)
            if matches!(
                img.header.format,
                twine_core::ColorFormat::A8 | twine_core::ColorFormat::L8
            ) && usize::from(img.header.stride.max(img.header.w)) == usize::from(img.header.w) =>
        {
            img.bytes().map(|alpha| {
                let c = engine.coords(id);
                let (w, h) = (i32::from(img.header.w), i32::from(img.header.h));
                let area = Rect::from_xywh(c.x0 + (c.width() - w) / 2, c.y0 + (c.height() - h) / 2, w, h);
                Mask::Map { area, alpha }
            })
        }
        _ => None,
    };
    if mask.is_none() && !WARN_MASK.load(Ordering::Relaxed) {
        WARN_MASK.store(true, Ordering::Relaxed);
        twine_core::warn!(target: "twine::engine", "bitmap mask must be an uncompressed static A8/L8 image; ignored");
    }
    mask
}

/// The node's own drawing, its children and its post drawing.
fn draw_content(engine: &Engine, p: &mut Painter<'_>, aux: &mut AuxRes, id: NodeId, clip: Rect, opa: Opa) {
    let Some(n) = engine.tree.node(id) else {
        return;
    };
    if n.flags().contains(ObjFlags::LAYOUT_PASSTHROUGH) {
        // A wrapper draws nothing of its own.
        draw_children(engine, p, aux, id, clip, opa);
        return;
    }
    p.with_clip(clip, |p| {
        let mut cx = DrawCx::new(p, engine, aux, id, opa);
        n.widget().draw(&mut cx);
    });
    draw_children(engine, p, aux, id, clip, opa);
    p.with_clip(clip, |p| draw_post(engine, p, aux, id, opa));
}

/// The children of `id` (clipped to it unless `OVERFLOW_VISIBLE`, with its corner mask).
fn draw_children(engine: &Engine, p: &mut Painter<'_>, aux: &mut AuxRes, id: NodeId, clip: Rect, opa: Opa) {
    let Some(n) = engine.tree.node(id) else {
        return;
    };
    let child_clip = if clips_children(n.flags()) {
        clip.intersection(&n.coords())
    } else {
        Some(clip)
    };
    if let (Some(cc), Some(_)) = (child_clip, n.first_child()) {
        let mask = clips_corners(engine, id).then(|| {
            p.push_mask(Mask::Radius {
                area: n.coords(),
                radius: engine.cached_main(id).radius,
                outer: false,
            })
        });
        let mut c = n.first_child();
        while let Some(ch) = c {
            draw_node(engine, p, aux, ch, cc, opa);
            c = engine.tree.node(ch).and_then(crate::Node::next_sibling);
        }
        if let Some(m) = mask {
            p.pop_mask(m);
        }
    }
}

/// `Widget::draw_post`, the scrollbars and the post border of `id`.
fn draw_post(engine: &Engine, p: &mut Painter<'_>, aux: &mut AuxRes, id: NodeId, opa: Opa) {
    let Some(n) = engine.tree.node(id) else {
        return;
    };
    if n.flags().contains(ObjFlags::LAYOUT_PASSTHROUGH) {
        return;
    }
    let mut cx = DrawCx::new(p, engine, aux, id, opa);
    n.widget().draw_post(&mut cx);
    // Then the scrollbars and the post border (LVGL `lv_obj` `DRAW_POST`).
    crate::scrollbar::draw_scrollbars(engine, p, aux, id, opa);
    DrawCx::new(p, engine, aux, id, opa).draw_border_post(Part::Main);
}

/// Outlines the coordinates of every visible node of `root`'s tree (layout bounds overlay).
fn draw_bounds(engine: &Engine, p: &mut Painter<'_>, root: NodeId) {
    let mut cur = Some(root);
    while let Some(c) = cur {
        let Some(n) = engine.tree.node(c) else {
            return;
        };
        if n.is_hidden() {
            // Skip the hidden subtree.
            cur = skip_subtree(engine, root, c);
            continue;
        }
        let r = n.coords();
        if !r.is_empty() {
            let (x0, y0, x1, y1) = (r.x0, r.y0, r.x1, r.y1);
            for e in [
                Rect::new(x0, y0, x1, y0 + 1),
                Rect::new(x0, y1 - 1, x1, y1),
                Rect::new(x0, y0, x0 + 1, y1),
                Rect::new(x1 - 1, y0, x1, y1),
            ] {
                p.fill(e, BOUNDS_COLOR, Opa::COVER);
            }
        }
        cur = engine.next_in_subtree(root, c);
    }
}

/// The pre-order successor of `id` within `root` that is not inside `id`'s subtree.
fn skip_subtree(engine: &Engine, root: NodeId, id: NodeId) -> Option<NodeId> {
    let mut x = id;
    loop {
        if x == root {
            return None;
        }
        let n = engine.tree.node(x)?;
        if let Some(s) = n.next_sibling() {
            return Some(s);
        }
        x = n.parent()?;
    }
}
