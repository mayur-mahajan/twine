//! [`ImageButton`]: a button drawn from images, one set per state (LVGL `lv_imagebutton`).

use alloc::boxed::Box;

use twine_core::{Rect, Size};
use twine_engine::{
    DrawCx, Engine, EngineError, Event, EventCode, EventCx, EventResult, MeasureCx, NodeId, OBJ_FLAGS, State,
    Widget, WidgetClass, WidgetCx,
};
use twine_image::{ImageHeader, ImageSource, with_pixels};
use twine_style::{Part, TextAlign};
use twine_text::TextLayout;

use crate::image::same_source;
use crate::log_set;
use crate::util;

/// The class of [`ImageButton`]: `"imagebutton"`, part `Main`, the base object's flags
/// (LVGL `lv_imagebutton_class`; make it `CHECKABLE` to toggle).
pub static IMAGE_BUTTON_CLASS: WidgetClass = WidgetClass::new("imagebutton").default_flags(OBJ_FLAGS);

/// The state an [`ImageButton`] picks its images for (LVGL `lv_imagebutton_state_t`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ImageButtonState {
    /// Not pressed, not checked.
    #[default]
    Released,
    /// Pressed.
    Pressed,
    /// Disabled.
    Disabled,
    /// Checked.
    CheckedReleased,
    /// Checked and pressed.
    CheckedPressed,
    /// Checked and disabled.
    CheckedDisabled,
}

impl ImageButtonState {
    /// Every state, in LVGL's order.
    pub const ALL: [ImageButtonState; 6] = [
        ImageButtonState::Released,
        ImageButtonState::Pressed,
        ImageButtonState::Disabled,
        ImageButtonState::CheckedReleased,
        ImageButtonState::CheckedPressed,
        ImageButtonState::CheckedDisabled,
    ];

    /// The image state for the engine states `s` (LVGL `get_state`).
    #[must_use]
    pub fn from_state(s: State) -> Self {
        let checked = s.contains(State::CHECKED);
        if s.contains(State::DISABLED) {
            return if checked {
                ImageButtonState::CheckedDisabled
            } else {
                ImageButtonState::Disabled
            };
        }
        match (checked, s.contains(State::PRESSED)) {
            (true, true) => ImageButtonState::CheckedPressed,
            (true, false) => ImageButtonState::CheckedReleased,
            (false, true) => ImageButtonState::Pressed,
            (false, false) => ImageButtonState::Released,
        }
    }

    /// The engine states of this image state (LVGL `lv_imagebutton_set_state`).
    #[must_use]
    pub fn to_state(self) -> State {
        match self {
            ImageButtonState::Released => State::DEFAULT,
            ImageButtonState::Pressed => State::PRESSED,
            ImageButtonState::Disabled => State::DISABLED,
            ImageButtonState::CheckedReleased => State::CHECKED,
            ImageButtonState::CheckedPressed => State::CHECKED.union(State::PRESSED),
            ImageButtonState::CheckedDisabled => State::CHECKED.union(State::DISABLED),
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// One image of a state: the source and its header (`None` for a symbol, which is measured
/// and drawn as text with the `Main` text style).
#[derive(Clone, Debug, PartialEq, Eq)]
struct Slot {
    src: ImageSource,
    header: Option<ImageHeader>,
}

impl Slot {
    /// The size the image takes: its header, or a symbol's text size in the `Main` font.
    fn size(&self, m: &MeasureCx<'_>) -> Size {
        match (&self.header, &self.src) {
            (Some(h), _) => Size::new(i32::from(h.w), i32::from(h.h)),
            (None, ImageSource::Symbol(t)) => {
                let d = m.text_dsc(Part::Main);
                let mut l = TextLayout::new(t, d.font);
                l.letter_space = d.letter_space;
                l.line_space = d.line_space;
                l.measure()
            }
            (None, _) => Size::ZERO,
        }
    }
}

/// A button drawn from images (LVGL `lv_imagebutton`). Each [`ImageButtonState`] has up to
/// three images: left, middle and right. The middle image is tiled to fill the width between
/// the left and right ones, so one set of images makes buttons of any width ("3-slice").
///
/// A state without a middle image falls back to a related one (LVGL `suggest_state`):
/// pressed → released, checked → released, checked-pressed → checked → pressed → released,
/// disabled → released, checked-disabled → checked → released.
///
/// The size is `Content` by default: the widths of the three images and the middle image's
/// height (with only a middle image, its width, LVGL `GET_SELF_SIZE`). The button behaves
/// like any clickable object: make it `CHECKABLE` to toggle `State::CHECKED`.
///
/// ```
/// use twine_engine::State;
/// use twine_widgets::image_button::{ImageButton, ImageButtonState};
///
/// // Without a pressed image the released one is shown while pressed.
/// let b = ImageButton::new();
/// assert_eq!(ImageButtonState::from_state(State::PRESSED), ImageButtonState::Pressed);
/// assert_eq!(b.shown_state(State::PRESSED), ImageButtonState::Pressed); // nothing set at all
/// ```
#[derive(Debug, Default)]
pub struct ImageButton {
    /// `[state][left, mid, right]`.
    srcs: [[Option<Slot>; 3]; 6],
}

impl ImageButton {
    /// An image button without images.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The `[left, mid, right]` sources of `state`.
    #[must_use]
    pub fn src(&self, state: ImageButtonState) -> [Option<&ImageSource>; 3] {
        let s = &self.srcs[state.index()];
        [0, 1, 2].map(|i| s[i].as_ref().map(|x| &x.src))
    }

    /// Sets the images of `state` (LVGL `lv_imagebutton_set_src`). Idempotent. A source whose
    /// header cannot be read is logged and left unset.
    pub fn set_src(
        &mut self,
        cx: &mut WidgetCx<'_>,
        state: ImageButtonState,
        left: Option<ImageSource>,
        mid: Option<ImageSource>,
        right: Option<ImageSource>,
    ) {
        let new = [left, mid, right];
        let cur = &self.srcs[state.index()];
        let same = cur.iter().zip(new.iter()).all(|(c, n)| match (c, n) {
            (None, None) => true,
            (Some(c), Some(n)) => same_source(&c.src, n),
            _ => false,
        });
        if same {
            return;
        }
        if (new[0].is_some() || new[2].is_some()) && new[1].is_none() {
            twine_core::warn!(
                target: "twine::engine",
                "imagebutton: middle image not set while left and/or right images are"
            );
        }
        log_set(IMAGE_BUTTON_CLASS.name, cx.node(), "src");
        let [l, m, r] = new;
        let slots = [l, m, r].map(|s| {
            s.and_then(|src| {
                if matches!(src, ImageSource::Symbol(_)) {
                    return Some(Slot { src, header: None });
                }
                match cx.engine_mut().image_header(&src) {
                    Ok(header) => Some(Slot {
                        src,
                        header: Some(header),
                    }),
                    Err(e) => {
                        twine_core::warn!(target: "twine::image", "imagebutton: cannot read {}: {:?}", src, e);
                        None
                    }
                }
            })
        });
        self.srcs[state.index()] = slots;
        Self::refresh(cx);
    }

    /// Sets the engine states for an image state (LVGL `lv_imagebutton_set_state`): `CHECKED`,
    /// `PRESSED` and `DISABLED` as the state says.
    pub fn set_state(&mut self, cx: &mut WidgetCx<'_>, state: ImageButtonState) {
        let all = State::CHECKED.union(State::PRESSED).union(State::DISABLED);
        let want = state.to_state();
        if cx.state().intersection(all) == want {
            return;
        }
        log_set(IMAGE_BUTTON_CLASS.name, cx.node(), "state");
        cx.clear_state(all.difference(want));
        cx.add_state(want);
        Self::refresh(cx);
    }

    /// The state whose images are drawn in engine state `s` (LVGL `suggest_state`).
    #[must_use]
    pub fn shown_state(&self, s: State) -> ImageButtonState {
        use ImageButtonState as S;
        let st = S::from_state(s);
        let has = |x: S| self.srcs[x.index()][1].is_some();
        if has(st) {
            return st;
        }
        let order: &[S] = match st {
            S::Pressed | S::CheckedReleased | S::Disabled => &[S::Released],
            S::CheckedPressed => &[S::CheckedReleased, S::Pressed, S::Released],
            S::CheckedDisabled => &[S::CheckedReleased, S::Released],
            S::Released => &[],
        };
        order.iter().copied().find(|x| has(*x)).unwrap_or(st)
    }

    /// LVGL `refr_image`: re-measures and redraws (the content size can differ per state).
    fn refresh(cx: &mut WidgetCx<'_>) {
        cx.mark_layout();
        cx.invalidate_for("imagebutton");
    }

    fn slots(&self, s: State) -> &[Option<Slot>; 3] {
        &self.srcs[self.shown_state(s).index()]
    }
}

/// Creates an image button without images as the last child of `parent` (LVGL
/// `lv_imagebutton_create`).
///
/// # Errors
/// [`EngineError::NodeNotFound`] if `parent` does not exist (logged).
pub fn create(engine: &mut Engine, parent: NodeId) -> Result<NodeId, EngineError> {
    engine.create(parent, Box::new(ImageButton::new()))
}

impl Widget for ImageButton {
    fn class(&self) -> &'static WidgetClass {
        &IMAGE_BUTTON_CLASS
    }

    /// The images' widths (the middle one once) and the middle image's height.
    fn content_size(&self, cx: &MeasureCx<'_>) -> Size {
        let s = self.slots(cx.state());
        let size = |i: usize| s[i].as_ref().map_or(Size::ZERO, |x| x.size(cx));
        Size::new(size(0).w + size(1).w + size(2).w, size(1).h)
    }

    /// LVGL `draw_main`: left and right images at the ends, the middle one tiled between.
    fn draw(&self, cx: &mut DrawCx<'_, '_>) {
        cx.draw_base(Part::Main);
        let m = MeasureCx::new(cx.engine(), cx.node());
        let (tw, th) = util::transform_wh(&m, Part::Main);
        let st = m.state();
        let c = cx.coords();
        let c = Rect::new(c.x0 - tw, c.y0 - th, c.x1 + tw, c.y1 + th);
        let slots = self.slots(st);
        let dsc = cx.image_dsc(Part::Main);
        let draw = |cx: &mut DrawCx<'_, '_>, src: &ImageSource, area: Rect, clip: Rect, tile: bool| {
            if let ImageSource::Symbol(t) = src {
                // Symbols are text in the `Main` font, centered in their area.
                let mut td = cx.text_dsc(Part::Main);
                td.align = TextAlign::Center;
                let _ = cx.with_clip(clip, |cx| cx.draw_text(area, t, &td));
                return;
            }
            let mut d = dsc;
            d.tile = tile;
            let _ = cx.with_clip(clip, |cx| {
                cx.with_images(|p, icx| with_pixels(src, icx, |px| p.image(area, px, &d)))
            });
        };
        let (mut left_w, mut right_w) = (0, 0);
        if let Some(l) = &slots[0] {
            let sz = l.size(&m);
            left_w = sz.w;
            let a = Rect::from_xywh(c.x0, c.y0, left_w, sz.h);
            draw(cx, &l.src, a, a, false);
        }
        if let Some(r) = &slots[2] {
            let sz = r.size(&m);
            right_w = sz.w;
            let a = Rect::from_xywh(c.x1 - right_w, c.y0, right_w, sz.h);
            draw(cx, &r.src, a, a, false);
        }
        if let Some(mid) = &slots[1] {
            let a = Rect::new(c.x0 + left_w, c.y0, c.x1 - right_w, c.y1);
            if !a.is_empty() {
                draw(cx, &mid.src, a, a, true);
            }
        }
    }

    /// LVGL `LV_EVENT_COVER_CHECK`: images may be transparent.
    fn covers(&self, _cx: &MeasureCx<'_>, _area: Rect) -> bool {
        false
    }

    fn event(&mut self, cx: &mut EventCx<'_>, ev: &Event) -> EventResult {
        if ev.target == cx.node()
            && matches!(
                ev.code,
                EventCode::Pressed | EventCode::Released | EventCode::PressLost | EventCode::ValueChanged
            )
        {
            // The shown images depend on the state (LVGL `refr_image`).
            Self::refresh(&mut cx.widget_cx());
        }
        EventResult::Continue
    }
}
