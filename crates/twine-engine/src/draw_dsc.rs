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
use twine_style::{
    GradDir, Length, Part, PropId, ResolveOptions, State, StyleValue, TextAlign, TextDecor, resolve_with,
};
use twine_text::{Font, TextDsc};

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

/// Resolves the properties of one node for a descriptor: in its current state (through the
/// engine's resolver and caches), or as if it were in another state with transitions skipped
/// (LVGL sets `obj->state` and `skip_trans` temporarily, e.g. for button matrix buttons).
#[derive(Clone, Copy)]
struct Props<'a> {
    e: &'a Engine,
    id: NodeId,
    state: Option<State>,
}

impl Props<'_> {
    fn get(&self, part: Part, p: PropId) -> StyleValue {
        match self.state {
            None => self.e.style_prop(self.id, part, p),
            Some(state) => resolve_with(
                &self.e.tree,
                self.id,
                part,
                p,
                &self.e.style_defaults(),
                ResolveOptions {
                    state: Some(state),
                    skip_transitions: true,
                },
            ),
        }
    }

    fn i32(&self, part: Part, p: PropId) -> i32 {
        let v = self.get(part, p);
        v.as_i32()
            .or_else(|| match v.as_length() {
                Some(Length::Px(x)) => Some(x),
                _ => None,
            })
            .unwrap_or(0)
    }

    fn color(&self, part: Part, p: PropId) -> Color {
        self.get(part, p).as_color().unwrap_or(Color::BLACK)
    }

    fn opa(&self, part: Part, p: PropId) -> Opa {
        self.get(part, p).as_opa().unwrap_or(Opa::COVER)
    }

    fn font(&self, part: Part) -> &'static Font {
        self.get(part, PropId::TextFont)
            .get::<&'static Font>()
            .unwrap_or(self.e.style_defaults().font)
    }

    /// The node's own `Main` recolor (cached in the current state).
    fn main_recolor(&self) -> (Color, Opa) {
        if self.state.is_none() {
            let m = self.e.cached_main(self.id);
            (m.recolor, m.recolor_opa)
        } else {
            (
                self.color(Part::Main, PropId::Recolor),
                self.opa(Part::Main, PropId::RecolorOpa),
            )
        }
    }
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
        self.recolor_with(
            Props {
                e: self,
                id,
                state: None,
            },
            part,
        )
    }

    fn recolor_with(&self, props: Props<'_>, part: Part) -> Color32 {
        let id = props.id;
        let (color, opa) = if part == Part::Main {
            props.main_recolor()
        } else {
            (
                props.color(part, PropId::Recolor),
                props.opa(part, PropId::RecolorOpa),
            )
        };
        let mut r = Color32 { color, alpha: opa.0 };
        if part != Part::Main {
            let (color, opa) = props.main_recolor();
            if opa.0 > 0 {
                r = over32(r, Color32 { color, alpha: opa.0 });
            }
        }
        let mut cur = self.tree.parent(id);
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
        self.rect_dsc_with(
            Props {
                e: self,
                id,
                state: None,
            },
            part,
            opa,
        )
    }

    /// The rectangle style of `part` of `id` as if the node were in `state`, with running
    /// transitions ignored (LVGL sets `obj->state` and `skip_trans` around
    /// `lv_obj_init_draw_rect_dsc`): for widgets that draw several items of one part in their
    /// own states, like the buttons of a button matrix. Opacity and recolor are applied as in
    /// [`rect_dsc`](Self::rect_dsc) (ancestors in their current state).
    ///
    /// ```
    /// use twine_core::{Color, Opa};
    /// use twine_engine::{Engine, EngineConfig, Obj};
    /// use twine_style::{Part, Selector, State, StyleProp};
    ///
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// let n = e.create_root(Box::new(Obj)).unwrap();
    /// let pressed = Selector::part(Part::Items).with_state(State::PRESSED);
    /// e.set_local_prop(n, pressed, StyleProp::BgColor(Color::RED));
    /// let d = e.rect_dsc_for_state(n, Part::Items, State::PRESSED, Opa::COVER);
    /// assert_eq!(d.base.bg_color, Color::RED);
    /// assert_ne!(e.rect_dsc(n, Part::Items, Opa::COVER).base.bg_color, Color::RED);
    /// ```
    #[must_use]
    pub fn rect_dsc_for_state(&self, id: NodeId, part: Part, state: State, opa: Opa) -> RectStyle {
        self.rect_dsc_with(
            Props {
                e: self,
                id,
                state: Some(state),
            },
            part,
            opa,
        )
    }

    fn rect_dsc_with(&self, props: Props<'_>, part: Part, opa: Opa) -> RectStyle {
        let (bg_color, bg_opa, radius, border_width, border_color) =
            if part == Part::Main && props.state.is_none() {
                let m = self.cached_main(props.id);
                (m.bg_color, m.bg_opa, m.radius, m.border_width, m.border_color)
            } else {
                (
                    props.color(part, PropId::BgColor),
                    props.opa(part, PropId::BgOpa),
                    props.i32(part, PropId::Radius),
                    props.i32(part, PropId::BorderWidth),
                    props.color(part, PropId::BorderColor),
                )
            };
        let rc = self.recolor_with(props, part);
        let recolor = |c: Color| {
            if rc.alpha == 0 {
                c
            } else {
                Color::mix(rc.color, c, Opa(rc.alpha))
            }
        };
        let o = |p| props.opa(part, p).mul(opa);
        let bg_opa = bg_opa.mul(opa);
        let bg_grad = props.get(part, PropId::BgGrad).get::<&'static Gradient>();
        let simple_grad = if bg_grad.is_none() && !bg_opa.is_transparent() {
            let kind = match props
                .get(part, PropId::BgGradDir)
                .get::<GradDir>()
                .unwrap_or_default()
            {
                GradDir::Ver => Some(GradKind::Ver),
                GradDir::Hor => Some(GradKind::Hor),
                _ => None,
            };
            kind.map(|k| {
                let stop = |p| props.i32(part, p).clamp(0, 255) as u8;
                Gradient::new(
                    k,
                    &[
                        GradStop::with_opa(
                            recolor(bg_color),
                            props.opa(part, PropId::BgMainOpa),
                            stop(PropId::BgMainStop),
                        ),
                        GradStop::with_opa(
                            recolor(props.color(part, PropId::BgGradColor)),
                            props.opa(part, PropId::BgGradOpa),
                            stop(PropId::BgGradStop),
                        ),
                    ],
                )
            })
        } else {
            None
        };
        let border_side = props
            .get(part, PropId::BorderSide)
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
            border_post: props.get(part, PropId::BorderPost).as_bool().unwrap_or(false),
            outline_color: recolor(props.color(part, PropId::OutlineColor)),
            outline_width: props.i32(part, PropId::OutlineWidth),
            outline_opa: o(PropId::OutlineOpa),
            outline_pad: props.i32(part, PropId::OutlinePad),
            shadow: ShadowDsc {
                width: props.i32(part, PropId::ShadowWidth),
                ofs_x: props.i32(part, PropId::ShadowOffsetX),
                ofs_y: props.i32(part, PropId::ShadowOffsetY),
                spread: props.i32(part, PropId::ShadowSpread),
                color: recolor(props.color(part, PropId::ShadowColor)),
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
        self.text_dsc_with(
            Props {
                e: self,
                id,
                state: None,
            },
            part,
            opa,
        )
    }

    /// The text style of `part` of `id` as if the node were in `state`, with transitions
    /// ignored (see [`rect_dsc_for_state`](Self::rect_dsc_for_state)).
    #[must_use]
    pub fn text_dsc_for_state(&self, id: NodeId, part: Part, state: State, opa: Opa) -> TextDsc {
        self.text_dsc_with(
            Props {
                e: self,
                id,
                state: Some(state),
            },
            part,
            opa,
        )
    }

    fn text_dsc_with(&self, props: Props<'_>, part: Part, opa: Opa) -> TextDsc {
        let mut d = TextDsc::new(props.font(part));
        let rc = self.recolor_with(props, part);
        let c = props.color(part, PropId::TextColor);
        d.color = if rc.alpha == 0 {
            c
        } else {
            Color::mix(rc.color, c, Opa(rc.alpha))
        };
        d.opa = props.opa(part, PropId::TextOpa).mul(opa);
        d.align = props
            .get(part, PropId::TextAlign)
            .get::<TextAlign>()
            .unwrap_or(TextAlign::Auto);
        d.decor = props
            .get(part, PropId::TextDecor)
            .get::<TextDecor>()
            .unwrap_or_default();
        d.letter_space = props.i32(part, PropId::TextLetterSpace);
        d.line_space = props.i32(part, PropId::TextLineSpace);
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
