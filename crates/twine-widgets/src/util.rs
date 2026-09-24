//! Helpers shared by the widgets: DPI scaling, per-part extra draw sizes, inclusive-area
//! arithmetic for ports of LVGL geometry code, and input-device queries.

use twine_core::Rect;
use twine_engine::{Engine, InputKind, MeasureCx, NodeId};
use twine_render::{ShadowDsc, shadow_ext_size};
use twine_style::{BaseDir, Length, Part, PropId};

/// LVGL `LV_DPI_DEF`: the DPI class default sizes are written for.
pub(crate) const DPI_DEF: i32 = 130;

/// The DPI of the display showing `id` (the default display's, else [`DPI_DEF`]).
pub(crate) fn display_dpi(e: &Engine, id: NodeId) -> u16 {
    e.display_of(id)
        .or_else(|| e.default_display())
        .and_then(|d| e.display_info(d))
        .map_or(DPI_DEF as u16, |i| i.dpi)
}

/// LVGL `LV_DPX_CALC(dpi, px)` / `lv_dpx`: `px` at 160 DPI scaled to the node's display.
pub(crate) fn dpx(e: &Engine, id: NodeId, px: i32) -> i32 {
    let dpi = i32::from(display_dpi(e, id));
    match px {
        0 => 0,
        p if p > 0 => ((dpi * p + 80) / 160).max(1),
        p => -((dpi * -p + 80) / 160).max(1),
    }
}

/// The transform width and height of `part` in pixels (LVGL
/// `lv_obj_get_style_transform_width/height`; percentages of the node's size).
pub(crate) fn transform_wh(cx: &MeasureCx<'_>, part: Part) -> (i32, i32) {
    let c = cx.coords();
    let px = |p, basis: i32| match cx.style(part, p).as_length() {
        Some(Length::Px(v)) => v,
        Some(Length::Pct(pct)) => (i64::from(basis) * i64::from(pct) / 100) as i32,
        _ => 0,
    };
    (
        px(PropId::TransformWidth, c.width()),
        px(PropId::TransformHeight, c.height()),
    )
}

/// LVGL `lv_obj_calculate_ext_draw_size(obj, part)`: how far `part` draws outside its area
/// (shadow, outline, transform size).
pub(crate) fn part_ext_draw(cx: &MeasureCx<'_>, part: Part) -> i32 {
    let mut ext = 0;
    let sh = ShadowDsc {
        width: cx.style_i32(part, PropId::ShadowWidth),
        ofs_x: cx.style_i32(part, PropId::ShadowOffsetX),
        ofs_y: cx.style_i32(part, PropId::ShadowOffsetY),
        spread: cx.style_i32(part, PropId::ShadowSpread),
        color: twine_core::Color::BLACK,
        opa: cx.engine().style_opa(cx.node(), part, PropId::ShadowOpa),
    };
    if sh.is_visible() {
        ext = ext.max(shadow_ext_size(&sh));
    }
    let ow = cx.style_i32(part, PropId::OutlineWidth);
    if ow > 0
        && !cx
            .engine()
            .style_opa(cx.node(), part, PropId::OutlineOpa)
            .is_transparent()
    {
        ext = ext.max(ow + cx.style_i32(part, PropId::OutlinePad).max(0));
    }
    let (tw, th) = transform_wh(cx, part);
    ext.max(tw).max(th)
}

/// Clamps an extra draw size to the `u16` the engine stores.
pub(crate) fn ext_u16(v: i32) -> u16 {
    u16::try_from(v.max(0)).unwrap_or(u16::MAX)
}

/// Whether the node's `Main` part has a right-to-left base direction.
pub(crate) fn is_rtl(cx: &MeasureCx<'_>) -> bool {
    cx.style(Part::Main, PropId::BaseDir).get::<BaseDir>() == Some(BaseDir::Rtl)
}

/// The kind of the input device being processed, if any.
pub(crate) fn active_input_kind(e: &Engine) -> Option<InputKind> {
    e.active_input().and_then(|i| e.input_kind(i))
}

/// Whether an ancestor of `id` is being scrolled (a press that turned into a scroll).
pub(crate) fn ancestor_scrolling(e: &Engine, id: NodeId) -> bool {
    e.tree().ancestors(id).any(|a| a != id && e.is_scrolling(a))
}

/// An area with **inclusive** corners, as LVGL's `lv_area_t`: ports of LVGL geometry code
/// use it so that every `+ 1` / `- 1` stays exactly where LVGL has it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Area {
    pub x1: i32,
    pub y1: i32,
    pub x2: i32,
    pub y2: i32,
}

impl Area {
    /// The inclusive form of a (half-open) [`Rect`].
    pub(crate) fn from_rect(r: Rect) -> Self {
        Self {
            x1: r.x0,
            y1: r.y0,
            x2: r.x1 - 1,
            y2: r.y1 - 1,
        }
    }

    /// The half-open [`Rect`] (empty when inverted).
    pub(crate) fn to_rect(self) -> Rect {
        if self.x2 < self.x1 || self.y2 < self.y1 {
            return Rect::new(self.x1, self.y1, self.x1, self.y1);
        }
        Rect::new(self.x1, self.y1, self.x2 + 1, self.y2 + 1)
    }

    /// LVGL `lv_area_get_width`.
    pub(crate) fn width(self) -> i32 {
        self.x2 - self.x1 + 1
    }

    /// LVGL `lv_area_get_height`.
    pub(crate) fn height(self) -> i32 {
        self.y2 - self.y1 + 1
    }

    /// The coordinates as `[x1, y1, x2, y2]` (for axis-indexed ports).
    pub(crate) fn to_array(self) -> [i32; 4] {
        [self.x1, self.y1, self.x2, self.y2]
    }

    /// From `[x1, y1, x2, y2]`.
    pub(crate) fn from_array(a: [i32; 4]) -> Self {
        Self {
            x1: a[0],
            y1: a[1],
            x2: a[2],
            y2: a[3],
        }
    }
}

/// Whether the pixel `(x, y)` lies inside `r` with corners rounded by `radius` (LVGL
/// `lv_area_is_point_on` with a radius).
pub(crate) fn point_in_rounded(r: Rect, radius: i32, x: i32, y: i32) -> bool {
    if x < r.x0 || x >= r.x1 || y < r.y0 || y >= r.y1 {
        return false;
    }
    let rad = radius.clamp(0, r.width().min(r.height()) / 2);
    if rad == 0 {
        return true;
    }
    let cx0 = if x < r.x0 + rad {
        r.x0 + rad
    } else if x >= r.x1 - rad {
        r.x1 - rad
    } else {
        return true;
    };
    let cy0 = if y < r.y0 + rad {
        r.y0 + rad
    } else if y >= r.y1 - rad {
        r.y1 - rad
    } else {
        return true;
    };
    let dx = i64::from(2 * x + 1 - 2 * cx0);
    let dy = i64::from(2 * y + 1 - 2 * cy0);
    dx * dx + dy * dy <= 4 * i64::from(rad) * i64::from(rad)
}

/// LVGL `lv_area_is_in(inner, outer, radius)`: `inner` lies inside `outer` whose corners are
/// rounded by `radius`.
pub(crate) fn rect_in_rounded(inner: Rect, outer: Rect, radius: i32) -> bool {
    if !outer.contains_rect(&inner) {
        return false;
    }
    if radius == 0 || inner.is_empty() {
        return true;
    }
    [
        (inner.x0, inner.y0),
        (inner.x1 - 1, inner.y0),
        (inner.x0, inner.y1 - 1),
        (inner.x1 - 1, inner.y1 - 1),
    ]
    .into_iter()
    .all(|(x, y)| point_in_rounded(outer, radius, x, y))
}

/// Logs a value change (`debug!` at `twine::engine`).
pub(crate) fn log_value(class: &str, v: i32) {
    twine_core::debug!(target: "twine::engine", "{} value {}", class, v);
    let _ = (class, v);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn area_round_trip() {
        let r = Rect::new(3, 4, 10, 20);
        let a = Area::from_rect(r);
        assert_eq!((a.width(), a.height()), (7, 16));
        assert_eq!(a.to_rect(), r);
        assert!(Area::from_array([5, 5, 4, 9]).to_rect().is_empty());
    }

    #[test]
    fn rounded_containment() {
        let outer = Rect::new(0, 0, 20, 10);
        assert!(point_in_rounded(outer, 5, 10, 0));
        assert!(!point_in_rounded(outer, 5, 0, 0));
        assert!(rect_in_rounded(Rect::new(5, 0, 15, 10), outer, 5));
        assert!(!rect_in_rounded(Rect::new(0, 0, 15, 10), outer, 5));
    }
}
