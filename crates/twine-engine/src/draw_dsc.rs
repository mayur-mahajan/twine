//! Draw descriptors built from a node's resolved styles (LVGL `lv_obj_init_draw_*_dsc`):
//! rectangles, text, images, lines and arcs, with the node's effective opacity and the
//! style `Recolor` of the node and its ancestors applied (LVGL 9.3+ `recolor`).
//!
//! [`DrawCx`](crate::DrawCx), [`MeasureCx`](crate::MeasureCx) and
//! [`WidgetCx`](crate::WidgetCx) expose these as `rect_dsc`, `text_dsc`, `image_dsc`,
//! `line_dsc` and `arc_dsc`.

use twine_core::{Color, Opa, Rect};
use twine_render::{
    ArcDsc, BlendMode, BorderSide, GradKind, GradStop, Gradient, ImageDsc, LineDsc, RectDsc, ShadowDsc,
};
use twine_style::{GradDir, Part, PropId, TextAlign, TextDecor};
use twine_text::TextDsc;

use crate::style_list::length_px;
use crate::{Engine, NodeId};

/// The resolved rectangle style of a part: a [`RectDsc`] plus the owned simple gradient
/// (`BgGradDir` + `BgGradColor`) it may refer to. Get the descriptor with [`RectStyle::dsc`].
#[derive(Clone, Copy, Debug)]
pub struct RectStyle {
    /// Everything except a simple gradient.
    pub base: RectDsc<'static>,
    /// The simple gradient built from `BgGradDir`/`BgGradColor`/stops, if any.
    pub simple_grad: Option<Gradient>,
}

impl RectStyle {
    /// The descriptor to pass to [`Painter::rect`](twine_render::Painter::rect).
    #[must_use]
    pub fn dsc(&self) -> RectDsc<'_> {
        let mut d: RectDsc<'_> = self.base;
        if d.bg_grad.is_none() {
            d.bg_grad = self.simple_grad.as_ref();
        }
        d
    }
}

/// A color with its own opacity (LVGL `lv_color32_t`), used to accumulate recolors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Color32 {
    pub color: Color,
    pub alpha: u8,
}

/// LVGL `LV_OPA_MIN` / `LV_OPA_MAX`: below / above counts as fully transparent / opaque.
const OPA_MIN: u8 = 2;
const OPA_MAX: u8 = 253;

/// LVGL `lv_color_mix32`: `fg` over `bg`, keeping `bg`'s alpha.
fn mix32(fg: Color32, bg: Color32) -> Color32 {
    if fg.alpha >= OPA_MAX {
        return Color32 {
            color: fg.color,
            alpha: bg.alpha,
        };
    }
    if fg.alpha <= OPA_MIN {
        return bg;
    }
    Color32 {
        color: Color::mix(fg.color, bg.color, Opa(fg.alpha)),
        alpha: bg.alpha,
    }
}

/// LVGL `lv_color_over32`: `fg` over `bg`, both with their own alpha.
fn over32(fg: Color32, bg: Color32) -> Color32 {
    if fg.alpha >= OPA_MAX || bg.alpha <= OPA_MIN {
        return fg;
    }
    if fg.alpha <= OPA_MIN {
        return bg;
    }
    if bg.alpha == 255 {
        return mix32(fg, bg);
    }
    let inv = |a: u8| 255 - u32::from(a);
    // LV_OPA_MIX2(a, b) = a * b >> 8
    let res_alpha = (255 - ((inv(fg.alpha) * inv(bg.alpha)) >> 8)) as u8;
    let ratio = (u32::from(fg.alpha) * 255 / u32::from(res_alpha.max(1))).min(255) as u8;
    let mut r = mix32(
        Color32 {
            color: fg.color,
            alpha: ratio,
        },
        bg,
    );
    r.alpha = res_alpha;
    r
}

impl Engine {
    /// LVGL `lv_obj_style_apply_recolor`: `c` with `id`'s recolor of `part` under it.
    fn apply_recolor(&self, id: NodeId, part: Part, c: Color32) -> Color32 {
        let (color, opa) = if part == Part::Main {
            let m = self.cached_main(id);
            (m.recolor, m.recolor_opa)
        } else {
            (
                self.style_color(id, part, PropId::Recolor),
                self.style_opa(id, part, PropId::RecolorOpa),
            )
        };
        if opa.0 > 0 {
            over32(c, Color32 { color, alpha: opa.0 })
        } else {
            c
        }
    }

    /// LVGL `lv_obj_get_style_recolor_recursive`: the recolor of `part` of `id` combined with
    /// the `Main` recolors of `id` (for other parts) and of every ancestor.
    pub(crate) fn recolor_recursive(&self, id: NodeId, part: Part) -> Color32 {
        let (color, opa) = if part == Part::Main {
            let m = self.cached_main(id);
            (m.recolor, m.recolor_opa)
        } else {
            (
                self.style_color(id, part, PropId::Recolor),
                self.style_opa(id, part, PropId::RecolorOpa),
            )
        };
        let mut r = Color32 { color, alpha: opa.0 };
        let mut cur = if part == Part::Main {
            self.tree.parent(id)
        } else {
            Some(id)
        };
        while let Some(c) = cur {
            r = self.apply_recolor(c, Part::Main, r);
            cur = self.tree.parent(c);
        }
        r
    }

    /// The effective opacity of `id`: its `Opa` times every ancestor's.
    #[must_use]
    pub fn opa_recursive(&self, id: NodeId) -> Opa {
        let mut opa = Opa::COVER;
        let mut cur = Some(id);
        while let Some(c) = cur {
            opa = opa.mul(self.cached_main(c).opa);
            cur = self.tree.parent(c);
        }
        opa
    }

    /// The extra size `TransformWidth` / `TransformHeight` add to each side of `id` (LVGL
    /// `lv_area_increase(&coords, w, h)` in the object's drawing).
    #[must_use]
    pub fn transform_size(&self, id: NodeId) -> (i32, i32) {
        let c = self.coords(id);
        (
            length_px(self.style_prop(id, Part::Main, PropId::TransformWidth), c.width()),
            length_px(
                self.style_prop(id, Part::Main, PropId::TransformHeight),
                c.height(),
            ),
        )
    }

    /// The area the object's rectangle is drawn in: its coordinates grown by
    /// [`transform_size`](Self::transform_size).
    #[must_use]
    pub fn draw_area(&self, id: NodeId) -> Rect {
        let (w, h) = self.transform_size(id);
        let c = self.coords(id);
        if w == 0 && h == 0 {
            return c;
        }
        Rect::new(c.x0 - w, c.y0 - h, c.x1 + w, c.y1 + h)
    }

    /// The rectangle style of `part` of `id` (LVGL `lv_obj_init_draw_rect_dsc`): background
    /// (color or gradient), border, outline, shadow and radius, every opacity multiplied by
    /// `opa` and every color recolored.
    #[must_use]
    pub fn rect_dsc(&self, id: NodeId, part: Part, opa: Opa) -> RectStyle {
        let (bg_color, bg_opa, radius, border_width, border_color) = if part == Part::Main {
            let m = self.cached_main(id);
            (m.bg_color, m.bg_opa, m.radius, m.border_width, m.border_color)
        } else {
            (
                self.style_color(id, part, PropId::BgColor),
                self.style_opa(id, part, PropId::BgOpa),
                self.style_i32(id, part, PropId::Radius),
                self.style_i32(id, part, PropId::BorderWidth),
                self.style_color(id, part, PropId::BorderColor),
            )
        };
        let rc = self.recolor_recursive(id, part);
        let recolor = |c: Color| {
            if rc.alpha == 0 {
                c
            } else {
                Color::mix(rc.color, c, Opa(rc.alpha))
            }
        };
        let o = |p| self.style_opa(id, part, p).mul(opa);
        let bg_opa = bg_opa.mul(opa);
        let bg_grad = self
            .style_prop(id, part, PropId::BgGrad)
            .get::<&'static Gradient>();
        let simple_grad = if bg_grad.is_none() && !bg_opa.is_transparent() {
            let kind = match self
                .style_prop(id, part, PropId::BgGradDir)
                .get::<GradDir>()
                .unwrap_or_default()
            {
                GradDir::Ver => Some(GradKind::Ver),
                GradDir::Hor => Some(GradKind::Hor),
                _ => None,
            };
            kind.map(|k| {
                let stop = |p| self.style_i32(id, part, p).clamp(0, 255) as u8;
                Gradient::new(
                    k,
                    &[
                        GradStop::with_opa(
                            recolor(bg_color),
                            self.style_opa(id, part, PropId::BgMainOpa),
                            stop(PropId::BgMainStop),
                        ),
                        GradStop::with_opa(
                            recolor(self.style_color(id, part, PropId::BgGradColor)),
                            self.style_opa(id, part, PropId::BgGradOpa),
                            stop(PropId::BgGradStop),
                        ),
                    ],
                )
            })
        } else {
            None
        };
        let border_side = self
            .style_prop(id, part, PropId::BorderSide)
            .get::<BorderSide>()
            .unwrap_or(BorderSide::FULL);
        let base = RectDsc {
            radius,
            bg_color: recolor(bg_color),
            bg_opa,
            bg_grad,
            border_color: recolor(border_color),
            border_width,
            border_opa: o(PropId::BorderOpa),
            border_side,
            border_post: self
                .style_prop(id, part, PropId::BorderPost)
                .as_bool()
                .unwrap_or(false),
            outline_color: recolor(self.style_color(id, part, PropId::OutlineColor)),
            outline_width: self.style_i32(id, part, PropId::OutlineWidth),
            outline_opa: o(PropId::OutlineOpa),
            outline_pad: self.style_i32(id, part, PropId::OutlinePad),
            shadow: ShadowDsc {
                width: self.style_i32(id, part, PropId::ShadowWidth),
                ofs_x: self.style_i32(id, part, PropId::ShadowOffsetX),
                ofs_y: self.style_i32(id, part, PropId::ShadowOffsetY),
                spread: self.style_i32(id, part, PropId::ShadowSpread),
                color: recolor(self.style_color(id, part, PropId::ShadowColor)),
                opa: o(PropId::ShadowOpa),
            },
        };
        RectStyle { base, simple_grad }
    }

    fn blend_of(&self, id: NodeId, part: Part) -> BlendMode {
        self.style_prop(id, part, PropId::BlendMode)
            .get::<BlendMode>()
            .unwrap_or(BlendMode::Normal)
    }

    fn recolored(&self, id: NodeId, part: Part, c: Color) -> Color {
        let rc = self.recolor_recursive(id, part);
        if rc.alpha == 0 {
            c
        } else {
            Color::mix(rc.color, c, Opa(rc.alpha))
        }
    }

    /// The text style of `part` of `id` (LVGL `lv_obj_init_draw_label_dsc`): `Text*`
    /// properties (inherited ones included), opacity multiplied by `opa`, color recolored.
    #[must_use]
    pub fn text_dsc(&self, id: NodeId, part: Part, opa: Opa) -> TextDsc {
        let mut d = TextDsc::new(self.style_font(id, part));
        d.color = self.recolored(id, part, self.style_color(id, part, PropId::TextColor));
        d.opa = self.style_opa(id, part, PropId::TextOpa).mul(opa);
        d.align = self
            .style_prop(id, part, PropId::TextAlign)
            .get::<TextAlign>()
            .unwrap_or(TextAlign::Auto);
        d.decor = self
            .style_prop(id, part, PropId::TextDecor)
            .get::<TextDecor>()
            .unwrap_or_default();
        d.letter_space = self.style_i32(id, part, PropId::TextLetterSpace);
        d.line_space = self.style_i32(id, part, PropId::TextLineSpace);
        d
    }

    /// The image style of `part` of `id` (LVGL `lv_obj_init_draw_image_dsc`): `ImageOpa`
    /// multiplied by `opa`, `ImageRecolor` combined with the style recolor.
    #[must_use]
    pub fn image_dsc(&self, id: NodeId, part: Part, opa: Opa) -> ImageDsc<'static> {
        let color = self.style_color(id, part, PropId::ImageRecolor);
        let copa = self.style_opa(id, part, PropId::ImageRecolorOpa);
        let rc = self.recolor_recursive(id, part);
        // LVGL `image_apply_layer_recolor`.
        let (recolor, recolor_opa) = if copa.0 > 0 && rc.alpha > 0 {
            let r = over32(rc, Color32 { color, alpha: copa.0 });
            (r.color, Opa(r.alpha))
        } else if rc.alpha > 0 {
            (rc.color, Opa(rc.alpha))
        } else {
            (color, copa)
        };
        ImageDsc {
            opa: self.style_opa(id, part, PropId::ImageOpa).mul(opa),
            recolor,
            recolor_opa,
            blend_mode: self.blend_of(id, part),
            ..ImageDsc::default()
        }
    }

    /// The line style of `part` of `id` (`Line*` properties).
    #[must_use]
    pub fn line_dsc(&self, id: NodeId, part: Part, opa: Opa) -> LineDsc {
        let rounded = self
            .style_prop(id, part, PropId::LineRounded)
            .as_bool()
            .unwrap_or(false);
        LineDsc {
            color: self.recolored(id, part, self.style_color(id, part, PropId::LineColor)),
            opa: self.style_opa(id, part, PropId::LineOpa).mul(opa),
            width: self.style_i32(id, part, PropId::LineWidth),
            dash_width: self.style_i32(id, part, PropId::LineDashWidth),
            dash_gap: self.style_i32(id, part, PropId::LineDashGap),
            round_start: rounded,
            round_end: rounded,
            blend_mode: self.blend_of(id, part),
        }
    }

    /// The arc style of `part` of `id` (`Arc*` properties; an arc image source is not
    /// resolved here).
    #[must_use]
    pub fn arc_dsc(&self, id: NodeId, part: Part, opa: Opa) -> ArcDsc<'static> {
        ArcDsc {
            color: self.recolored(id, part, self.style_color(id, part, PropId::ArcColor)),
            opa: self.style_opa(id, part, PropId::ArcOpa).mul(opa),
            width: self.style_i32(id, part, PropId::ArcWidth),
            rounded: self
                .style_prop(id, part, PropId::ArcRounded)
                .as_bool()
                .unwrap_or(false),
            image: None,
            blend_mode: self.blend_of(id, part),
        }
    }

    /// The background-image recolor of `part` of `id` combined with the style recolor.
    pub(crate) fn bg_image_recolor(&self, id: NodeId, part: Part) -> (Color, Opa) {
        let color = self.style_color(id, part, PropId::BgImageRecolor);
        let copa = self.style_opa(id, part, PropId::BgImageRecolorOpa);
        let rc = self.recolor_recursive(id, part);
        if copa.0 > 0 && rc.alpha > 0 {
            let r = over32(rc, Color32 { color, alpha: copa.0 });
            (r.color, Opa(r.alpha))
        } else if rc.alpha > 0 {
            (rc.color, Opa(rc.alpha))
        } else {
            (color, copa)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn over32_matches_lvgl_rules() {
        let red = Color32 {
            color: Color::RED,
            alpha: 255,
        };
        let blue_half = Color32 {
            color: Color::BLUE,
            alpha: 128,
        };
        let none = Color32 {
            color: Color::BLACK,
            alpha: 0,
        };
        assert_eq!(over32(red, blue_half), red);
        assert_eq!(over32(none, blue_half), blue_half);
        assert_eq!(over32(blue_half, none), blue_half);
        // Two half-transparent layers: alpha 255 - (127 * 127 >> 8) = 192.
        let r = over32(blue_half, blue_half);
        assert_eq!(r.alpha, 192);
        assert_eq!(r.color, Color::BLUE);
    }
}
