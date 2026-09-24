//! Draw descriptors of a part resolved as if the node were in another state.
//!
//! LVGL draws each button of a button matrix (and the pressed key of a keyboard popover) by
//! setting `obj->state` temporarily and calling `lv_obj_init_draw_rect_dsc` /
//! `lv_obj_init_draw_label_dsc` with transitions skipped (`skip_trans`). Twine widgets cannot
//! change a node's state while drawing, so [`StateStyles`] resolves the properties with
//! [`resolve_with`] and a state override instead, and builds the descriptors exactly like the
//! engine's `rect_dsc` / `text_dsc` (opacity, recolor of the node and its ancestors).
// NOTE(P18.S01): an engine-side `Engine::rect_dsc_for_state` / `text_dsc_for_state` would let
// this module go (the engine's builders are the same code with the node's own state).

use twine_core::{Color, Opa};
use twine_engine::{Engine, NodeId, RectStyle};
use twine_render::{BorderSide, GradKind, GradStop, Gradient, RectDsc, ShadowDsc};
use twine_style::{
    GradDir, Length, Part, PropId, ResolveOptions, State, StyleDefaults, StyleValue, TextAlign, TextDecor,
    resolve_with,
};
use twine_text::{Font, TextDsc};

/// The styles of one part of one node in a given state (transitions skipped).
#[derive(Clone, Copy)]
pub(crate) struct StateStyles<'a> {
    engine: &'a Engine,
    id: NodeId,
    part: Part,
    state: State,
    defaults: StyleDefaults,
}

/// LVGL `LV_OPA_MIN` / `LV_OPA_MAX`.
const OPA_MIN: u8 = 2;
const OPA_MAX: u8 = 253;

/// A color with an alpha (LVGL `lv_color32_t`) for accumulating recolors.
#[derive(Clone, Copy)]
struct Color32 {
    color: Color,
    alpha: u8,
}

/// LVGL `lv_color_mix32`.
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

/// LVGL `lv_color_over32`.
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

impl<'a> StateStyles<'a> {
    /// The styles of `part` of `id` as if `id` were in `state`.
    pub(crate) fn new(engine: &'a Engine, id: NodeId, part: Part, state: State) -> Self {
        // The default font only matters when nothing in the tree sets one; the node's current
        // font of the part is exactly that default then.
        let defaults = StyleDefaults {
            font: engine.style_font(id, part),
        };
        Self {
            engine,
            id,
            part,
            state,
            defaults,
        }
    }

    fn prop_of(&self, part: Part, p: PropId) -> StyleValue {
        resolve_with(
            self.engine.tree(),
            self.id,
            part,
            p,
            &self.defaults,
            ResolveOptions {
                state: Some(self.state),
                skip_transitions: true,
            },
        )
    }

    /// A property of the part.
    pub(crate) fn prop(&self, p: PropId) -> StyleValue {
        self.prop_of(self.part, p)
    }

    /// An integer property (px lengths resolve; other kinds give 0).
    pub(crate) fn i32(&self, p: PropId) -> i32 {
        let v = self.prop(p);
        v.as_i32()
            .or_else(|| match v.as_length() {
                Some(Length::Px(x)) => Some(x),
                _ => None,
            })
            .unwrap_or(0)
    }

    fn color_of(&self, part: Part, p: PropId) -> Color {
        self.prop_of(part, p).as_color().unwrap_or(Color::BLACK)
    }

    fn opa_of(&self, part: Part, p: PropId) -> Opa {
        self.prop_of(part, p).as_opa().unwrap_or(Opa::COVER)
    }

    /// The font of the part.
    pub(crate) fn font(&self) -> &'static Font {
        self.prop(PropId::TextFont)
            .get::<&'static Font>()
            .unwrap_or(self.defaults.font)
    }

    /// LVGL `lv_obj_get_style_recolor_recursive` with the node itself in the overridden
    /// state: the part's recolor, then the node's `Main` recolor, then every ancestor's.
    fn recolor(&self) -> Color32 {
        let own = Color32 {
            color: self.color_of(self.part, PropId::Recolor),
            alpha: self.opa_of(self.part, PropId::RecolorOpa).0,
        };
        let mut r = own;
        if self.part != Part::Main {
            let main = Color32 {
                color: self.color_of(Part::Main, PropId::Recolor),
                alpha: self.opa_of(Part::Main, PropId::RecolorOpa).0,
            };
            if main.alpha > 0 {
                r = over32(r, main);
            }
        }
        let mut cur = self.engine.tree().parent(self.id);
        while let Some(c) = cur {
            let m = self.engine.cached_main(c);
            if m.recolor_opa.0 > 0 {
                r = over32(
                    r,
                    Color32 {
                        color: m.recolor,
                        alpha: m.recolor_opa.0,
                    },
                );
            }
            cur = self.engine.tree().parent(c);
        }
        r
    }

    /// The rectangle descriptor (like `Engine::rect_dsc`), every opacity multiplied by `opa`.
    /// `border_post` is cleared: the rectangle is drawn in one go (LVGL `lv_draw_rect`).
    pub(crate) fn rect_dsc(&self, opa: Opa) -> RectStyle {
        let rc = self.recolor();
        let recolor = |c: Color| {
            if rc.alpha == 0 {
                c
            } else {
                Color::mix(rc.color, c, Opa(rc.alpha))
            }
        };
        let part = self.part;
        let o = |p| self.opa_of(part, p).mul(opa);
        let bg_color = self.color_of(part, PropId::BgColor);
        let bg_opa = o(PropId::BgOpa);
        let bg_grad = self.prop(PropId::BgGrad).get::<&'static Gradient>();
        let simple_grad = if bg_grad.is_none() && !bg_opa.is_transparent() {
            let kind = match self.prop(PropId::BgGradDir).get::<GradDir>().unwrap_or_default() {
                GradDir::Ver => Some(GradKind::Ver),
                GradDir::Hor => Some(GradKind::Hor),
                _ => None,
            };
            kind.map(|k| {
                let stop = |p| self.i32(p).clamp(0, 255) as u8;
                Gradient::new(
                    k,
                    &[
                        GradStop::with_opa(
                            recolor(bg_color),
                            self.opa_of(part, PropId::BgMainOpa),
                            stop(PropId::BgMainStop),
                        ),
                        GradStop::with_opa(
                            recolor(self.color_of(part, PropId::BgGradColor)),
                            self.opa_of(part, PropId::BgGradOpa),
                            stop(PropId::BgGradStop),
                        ),
                    ],
                )
            })
        } else {
            None
        };
        let base = RectDsc {
            radius: self.i32(PropId::Radius),
            bg_color: recolor(bg_color),
            bg_opa,
            bg_grad,
            border_color: recolor(self.color_of(part, PropId::BorderColor)),
            border_width: self.i32(PropId::BorderWidth),
            border_opa: o(PropId::BorderOpa),
            border_side: self
                .prop(PropId::BorderSide)
                .get::<BorderSide>()
                .unwrap_or(BorderSide::FULL),
            border_post: false,
            outline_color: recolor(self.color_of(part, PropId::OutlineColor)),
            outline_width: self.i32(PropId::OutlineWidth),
            outline_opa: o(PropId::OutlineOpa),
            outline_pad: self.i32(PropId::OutlinePad),
            shadow: ShadowDsc {
                width: self.i32(PropId::ShadowWidth),
                ofs_x: self.i32(PropId::ShadowOffsetX),
                ofs_y: self.i32(PropId::ShadowOffsetY),
                spread: self.i32(PropId::ShadowSpread),
                color: recolor(self.color_of(part, PropId::ShadowColor)),
                opa: o(PropId::ShadowOpa),
            },
        };
        RectStyle { base, simple_grad }
    }

    /// The text descriptor (like `Engine::text_dsc`), opacity multiplied by `opa`.
    pub(crate) fn text_dsc(&self, opa: Opa) -> TextDsc {
        let rc = self.recolor();
        let mut d = TextDsc::new(self.font());
        let c = self.color_of(self.part, PropId::TextColor);
        d.color = if rc.alpha == 0 {
            c
        } else {
            Color::mix(rc.color, c, Opa(rc.alpha))
        };
        d.opa = self.opa_of(self.part, PropId::TextOpa).mul(opa);
        d.align = self
            .prop(PropId::TextAlign)
            .get::<TextAlign>()
            .unwrap_or(TextAlign::Auto);
        d.decor = self
            .prop(PropId::TextDecor)
            .get::<TextDecor>()
            .unwrap_or_default();
        d.letter_space = self.i32(PropId::TextLetterSpace);
        d.line_space = self.i32(PropId::TextLineSpace);
        d
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn over32_rules() {
        let red = Color32 {
            color: Color::RED,
            alpha: 255,
        };
        let half = Color32 {
            color: Color::BLUE,
            alpha: 128,
        };
        assert_eq!(over32(red, half).color, Color::RED);
        assert_eq!(over32(half, half).alpha, 192);
    }
}
