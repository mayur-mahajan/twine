//! Scrollbars (LVGL `lv_obj_get_scrollbar_area`, `draw_scrollbar`): geometry from the scroll
//! extents and the `Part::Scrollbar` styles, drawing after the node's post drawing, and
//! invalidation when their visibility changes.
//!
//! A scrollbar is drawn only when the `Scrollbar` part has a pixel `Width` (its thickness) and
//! a visible background or border; themes provide these styles.

use twine_core::{Opa, Rect};
use twine_render::Painter;
use twine_style::{Dir, Part, PropId, ScrollbarMode};

use crate::draw_cx::AuxRes;
use crate::{DrawCx, Engine, InvalidateReason, NodeId, ObjFlags};

/// LVGL `LV_OPA_MIN`: parts at or below this opacity are not drawn.
const OPA_MIN: u8 = 2;

/// LVGL `LV_DPX_CALC(dpi, n)`: `n` pixels at 160 dpi scaled to `dpi` (at least 1).
fn dpx(dpi: u16, n: i32) -> i32 {
    if n == 0 {
        return 0;
    }
    ((i32::from(dpi) * n + 80) / 160).max(1)
}

/// `v · factor / divisor` in 64 bits (LVGL `mul_div`).
fn mul_div(v: i32, factor: i32, divisor: i32) -> i32 {
    (i64::from(v) * i64::from(factor) / i64::from(divisor)) as i32
}

/// An inclusive LVGL area as a [`Rect`] (`None` when empty).
fn from_incl(x1: i32, y1: i32, x2: i32, y2: i32) -> Option<Rect> {
    let r = Rect::new(x1, y1, x2 + 1, y2 + 1);
    (!r.is_empty()).then_some(r)
}

impl Engine {
    /// The horizontal and the vertical scrollbar of `id` in absolute coordinates, `None` for a
    /// bar that is not shown (LVGL `lv_obj_get_scrollbar_area`).
    ///
    /// Bars are shown on `SCROLLABLE` nodes for the axes of the scroll direction: always with
    /// [`ScrollbarMode::On`], with `Auto` when there is content to scroll to on the axis, with
    /// `Active` while a pointer scrolls the node on that axis, never with `Off`. The thickness
    /// is the `Width` of `Part::Scrollbar`, the paddings of that part are the distances from
    /// the node's edges (vertical bar: `pad_right`, or `pad_left` for right-to-left scrollbars;
    /// horizontal bar: `pad_bottom`) and the insets at the ends (`pad_top`/`pad_bottom`,
    /// `pad_left`/`pad_right`). The bar's length is the visible fraction of the track (or the
    /// part's `Length`), at least 10 px at 160 dpi.
    #[must_use]
    pub fn scrollbar_areas(&self, id: NodeId) -> (Option<Rect>, Option<Rect>) {
        let Some(n) = self.tree.node(id) else {
            return (None, None);
        };
        if !n.flags.contains(ObjFlags::SCROLLABLE) {
            return (None, None);
        }
        let sm = n.scroll_attrs.scrollbar_mode;
        if sm == ScrollbarMode::Off {
            return (None, None);
        }
        let indev_dir = self.indev_scroll_dir(id);
        if sm == ScrollbarMode::Active && indev_dir.is_none() {
            return (None, None);
        }
        // Cheap checks first: without a thickness and a visible background or border nothing
        // is drawn (LVGL computes empty areas then).
        let part = Part::Scrollbar;
        let thickness = self.style_i32(id, part, PropId::Width);
        if thickness <= 0
            || (self.style_opa(id, part, PropId::BgOpa).0 <= OPA_MIN
                && self.style_opa(id, part, PropId::BorderOpa).0 <= OPA_MIN)
        {
            return (None, None);
        }
        let (st, sb) = (self.scroll_top(id), self.scroll_bottom(id));
        let (sl, sr) = (self.scroll_left(id), self.scroll_right(id));
        let dir = n.scroll_attrs.dir;
        let ver_draw = dir.intersects(Dir::VER)
            && (sm == ScrollbarMode::On
                || (sm == ScrollbarMode::Auto && (st > 0 || sb > 0))
                || (sm == ScrollbarMode::Active && indev_dir == Some(Dir::VER)));
        let hor_draw = dir.intersects(Dir::HOR)
            && (sm == ScrollbarMode::On
                || (sm == ScrollbarMode::Auto && (sl > 0 || sr > 0))
                || (sm == ScrollbarMode::Active && indev_dir == Some(Dir::HOR)));
        if !hor_draw && !ver_draw {
            return (None, None);
        }
        let rtl = self.is_rtl(id, part);
        let top_space = self.style_i32(id, part, PropId::PadTop);
        let bottom_space = self.style_i32(id, part, PropId::PadBottom);
        let left_space = self.style_i32(id, part, PropId::PadLeft);
        let right_space = self.style_i32(id, part, PropId::PadRight);
        let length = self.style_i32(id, part, PropId::Length);
        let min_size = dpx(self.display_dpi(id), 10);
        let c = n.coords;
        // Inclusive coordinates as in LVGL.
        let (x1, y1, x2, y2) = (c.x0, c.y0, c.x1 - 1, c.y1 - 1);
        let (obj_w, obj_h) = (c.width(), c.height());
        let ver_req_space = if ver_draw { thickness } else { 0 };
        let hor_req_space = if hor_draw { thickness } else { 0 };

        let mut ver = None;
        let content_h = obj_h + st + sb;
        if ver_draw && content_h != 0 {
            let (vx1, vx2) = if rtl {
                (x1 + left_space, x1 + left_space + thickness - 1)
            } else {
                (x2 - right_space - thickness + 1, x2 - right_space)
            };
            let track = obj_h - top_space - bottom_space - hor_req_space;
            let mut sb_h = mul_div(track, obj_h, content_h);
            sb_h = (if length > 0 { length } else { sb_h }).max(min_size).min(obj_h);
            let rem = track - sb_h;
            let scroll_h = content_h - obj_h;
            let (vy1, vy2) = if scroll_h <= 0 {
                (y1 + top_space, y2 - bottom_space - hor_req_space - 1)
            } else {
                let sb_y = rem - mul_div(rem, sb, scroll_h);
                let mut a = y1 + sb_y + top_space;
                let mut b = a + sb_h - 1;
                if a < y1 + top_space {
                    a = y1 + top_space;
                    if a + min_size > b {
                        b = a + min_size;
                    }
                }
                if b > y2 - hor_req_space - bottom_space {
                    b = y2 - hor_req_space - bottom_space;
                    if b - min_size < a {
                        a = b - min_size;
                    }
                }
                (a, b)
            };
            ver = from_incl(vx1, vy1, vx2, vy2);
        }

        let mut hor = None;
        let content_w = obj_w + sl + sr;
        if hor_draw && content_w != 0 {
            let hy2 = y2 - bottom_space;
            let hy1 = hy2 - thickness + 1;
            let track = obj_w - left_space - right_space - ver_req_space;
            let mut sb_w = mul_div(track, obj_w, content_w);
            sb_w = (if length > 0 { length } else { sb_w }).max(min_size).min(obj_w);
            let rem = track - sb_w;
            let scroll_w = content_w - obj_w;
            let (hx1, hx2) = if scroll_w <= 0 {
                if rtl {
                    (x1 + left_space + ver_req_space - 1, x2 - right_space)
                } else {
                    (x1 + left_space, x2 - right_space - ver_req_space - 1)
                }
            } else {
                let sb_x = rem - mul_div(rem, sr, scroll_w);
                let start = if rtl {
                    x1 + left_space + ver_req_space
                } else {
                    x1 + left_space
                };
                let end = if rtl {
                    x2 - right_space
                } else {
                    x2 - ver_req_space - right_space
                };
                let mut a = x1 + sb_x + left_space + if rtl { ver_req_space } else { 0 };
                let mut b = a + sb_w - 1;
                if a < start {
                    a = start;
                    if a + min_size > b {
                        b = a + min_size;
                    }
                }
                if b > end {
                    b = end;
                    if b - min_size < a {
                        a = b - min_size;
                    }
                }
                (a, b)
            };
            hor = from_incl(hx1, hy1, hx2, hy2);
        }
        (hor, ver)
    }

    /// The dpi of the display showing `id` (130 when it is on none).
    fn display_dpi(&self, id: NodeId) -> u16 {
        self.display_of(id)
            .or(self.default_display)
            .and_then(|d| self.displays.get(d.index()))
            .map_or(130, |d| d.info.dpi)
    }

    /// Invalidates the strips along the edges of `id` where its scrollbars can be (the
    /// vertical and horizontal track, any length), after its content changed size or place:
    /// the bars' old and new geometry both lie there. Nothing when the node cannot show
    /// scrollbars.
    pub(crate) fn scrollbar_invalidate_tracks(&mut self, id: NodeId) {
        let Some(n) = self.tree.node(id) else {
            return;
        };
        if !n.flags.contains(ObjFlags::SCROLLABLE) || n.scroll_attrs.scrollbar_mode == ScrollbarMode::Off {
            return;
        }
        let c = n.coords;
        let part = Part::Scrollbar;
        let thickness = self.style_i32(id, part, PropId::Width);
        if thickness <= 0 {
            return;
        }
        let pad_l = self.style_i32(id, part, PropId::PadLeft).max(0);
        let pad_r = self.style_i32(id, part, PropId::PadRight).max(0);
        let pad_b = self.style_i32(id, part, PropId::PadBottom).max(0);
        let ver = if self.is_rtl(id, part) {
            Rect::new(c.x0, c.y0, c.x0 + pad_l + thickness, c.y1)
        } else {
            Rect::new(c.x1 - pad_r - thickness, c.y0, c.x1, c.y1)
        };
        let hor = Rect::new(c.x0, c.y1 - pad_b - thickness, c.x1, c.y1);
        self.invalidate_rect_of(id, ver, InvalidateReason::Scroll);
        self.invalidate_rect_of(id, hor, InvalidateReason::Scroll);
    }

    /// Invalidates the scrollbars of `id` as they are now (LVGL `lv_obj_scrollbar_invalidate`).
    pub fn scrollbar_invalidate(&mut self, id: NodeId) {
        let (h, v) = self.scrollbar_areas(id);
        for a in [h, v].into_iter().flatten() {
            self.invalidate_rect_of(id, a, InvalidateReason::Scroll);
        }
    }
}

/// Draws the scrollbars of `id` with the `Part::Scrollbar` styles in the node's current state
/// (which includes `SCROLLED` while it is being scrolled).
pub(crate) fn draw_scrollbars(engine: &Engine, p: &mut Painter<'_>, aux: &mut AuxRes, id: NodeId, opa: Opa) {
    let (hor, ver) = engine.scrollbar_areas(id);
    if hor.is_none() && ver.is_none() {
        return;
    }
    let cx = DrawCx::new(p, engine, aux, id, opa);
    let rs = cx.rect_dsc(Part::Scrollbar);
    let part_opa = engine.style_opa(id, Part::Scrollbar, PropId::Opa);
    let mut d = rs.base;
    d.outline_width = 0;
    d.border_post = false;
    if part_opa.0 < Opa::COVER.0 {
        d.bg_opa = d.bg_opa.mul(part_opa);
        d.border_opa = d.border_opa.mul(part_opa);
        d.shadow.opa = d.shadow.opa.mul(part_opa);
    }
    if d.bg_opa.is_transparent()
        && (d.border_opa.is_transparent() || d.border_width <= 0)
        && d.shadow.opa.is_transparent()
    {
        return;
    }
    for a in [hor, ver].into_iter().flatten() {
        p.rect(a, &d);
    }
}
