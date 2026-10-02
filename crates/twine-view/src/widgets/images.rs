//! Image-based controls: [`image_button`] and [`animimg`].

use core::cell::Cell;

use twine_anim::Repeat;
use twine_core::Duration;
use twine_engine::{State, WidgetCx};
use twine_image::ImageSource;
use twine_widgets::animimg::AnimImg;
use twine_widgets::image_button::{ImageButton, ImageButtonState};

use crate::build::{WidgetView, widget_view};
use crate::model::{IntoModel, bind_model, on_value_changed};
use crate::prop::{Icon, IntoProp};
use crate::widgets::controls::is_checked;

/// Sets the image (the middle slice) of `state`.
fn set_mid<M>(
    v: WidgetView<ImageButton>,
    state: ImageButtonState,
    src: impl IntoProp<ImageSource, M>,
) -> WidgetView<ImageButton> {
    v.bind(src, move |b: &mut ImageButton, wcx, src| {
        b.set_src(wcx, state, None, Some(src), None);
    })
}

/// Sets slice `slot` (0 = left, 1 = middle, 2 = right) of `state`, keeping the others.
fn set_slice(
    b: &mut ImageButton,
    wcx: &mut WidgetCx<'_>,
    state: ImageButtonState,
    slot: usize,
    src: Option<ImageSource>,
) {
    let mut s = b.src(state).map(Option::<&ImageSource>::cloned);
    if let Some(x) = s.get_mut(slot) {
        *x = src;
    }
    let [l, m, r] = s;
    b.set_src(wcx, state, l, m, r);
}

/// A button drawn with one image per state: `released` and `pressed` (states without an
/// image fall back to `released`, as LVGL).
///
/// ```
/// use twine_view::prelude::*;
///
/// let cx = twine_reactive::create_root();
/// let on = cx.signal(false);
/// let _v = image_button(Symbol::Play, Symbol::Play)
///     .checked_images(Symbol::Pause, Symbol::Pause)
///     .checkable(true)
///     .checked(on);
/// cx.dispose();
/// ```
pub fn image_button<M1, M2>(
    released: impl IntoProp<ImageSource, M1>,
    pressed: impl IntoProp<ImageSource, M2>,
) -> WidgetView<ImageButton> {
    let v = set_mid(
        widget_view(ImageButton::new),
        ImageButtonState::Released,
        released,
    );
    set_mid(v, ImageButtonState::Pressed, pressed)
}

impl WidgetView<ImageButton> {
    /// The images of the checked state (released and pressed).
    #[must_use]
    pub fn checked_images<M1, M2>(
        self,
        released: impl IntoProp<ImageSource, M1>,
        pressed: impl IntoProp<ImageSource, M2>,
    ) -> Self {
        let v = set_mid(self, ImageButtonState::CheckedReleased, released);
        set_mid(v, ImageButtonState::CheckedPressed, pressed)
    }

    /// The image of the disabled state.
    #[must_use]
    pub fn disabled_image<M>(self, src: impl IntoProp<ImageSource, M>) -> Self {
        set_mid(self, ImageButtonState::Disabled, src)
    }

    /// Three-slice images of `state`: `left` and `right` at the ends (optional: `()` for none,
    /// any [`Icon`]), `mid` tiled between them to fill the width.
    /// ```
    /// use twine_view::prelude::*;
    /// // A stretchable button: rounded caps at the ends (any `Icon`), the middle tiled.
    /// let _v = image_button(Symbol::Minus, Symbol::Minus)
    ///     .three_slice(ImageButtonState::Released, Symbol::Left, Symbol::Minus, Symbol::Right)
    ///     .three_slice(ImageButtonState::Pressed, (), Symbol::Minus, ()); // no caps
    /// ```
    #[must_use]
    pub fn three_slice<ML, M, MR>(
        self,
        state: ImageButtonState,
        left: impl IntoProp<Icon, ML>,
        mid: impl IntoProp<ImageSource, M>,
        right: impl IntoProp<Icon, MR>,
    ) -> Self {
        self.bind(mid, move |b: &mut ImageButton, wcx, src| {
            set_slice(b, wcx, state, 1, Some(src));
        })
        .bind(left, move |b: &mut ImageButton, wcx, icon: Icon| {
            set_slice(b, wcx, state, 0, icon.0);
        })
        .bind(right, move |b: &mut ImageButton, wcx, icon: Icon| {
            set_slice(b, wcx, state, 2, icon.0);
        })
    }

    /// The checked state (with [`checkable`](crate::ViewExt::checkable)): a plain value or a signal
    /// kept in sync both ways.
    #[must_use]
    pub fn checked(self, checked: impl IntoModel<bool>) -> Self {
        let model = checked.into_model();
        self.after_children(move |cx, node| {
            bind_model(
                cx,
                node,
                model,
                |e, n, on| {
                    e.with_widget_mut(n, |_: &mut ImageButton, wcx| {
                        if wcx.state().contains(State::CHECKED) != on {
                            if on {
                                wcx.add_state(State::CHECKED);
                            } else {
                                wcx.clear_state(State::CHECKED);
                            }
                            // The images depend on the state, not only the styles.
                            wcx.invalidate();
                        }
                    });
                },
                |e, n, _| is_checked(e, n),
            );
        })
    }

    /// Called with the new checked state whenever the user toggles it.
    #[must_use]
    pub fn on_change(self, f: impl FnMut(bool) + 'static) -> Self {
        self.op(move |cx, node| on_value_changed(cx, node, |e, n, _| Some(is_checked(e, n)), f))
    }
}

/// Settings of an [`animimg`] view read when it is built.
#[derive(Default)]
struct AnimImgCfg {
    /// `.playing(..)` was given (otherwise the animation starts at once).
    controlled: Cell<bool>,
}

/// An image animation showing `frames` one after the other, `period` for all of them. It
/// plays at once (forever unless [`repeat`](WidgetView::repeat) says otherwise) unless
/// [`playing`](WidgetView::playing) controls it.
///
/// ```
/// use twine_view::prelude::*;
///
/// static FRAMES: [ImageSource; 2] = [ImageSource::symbol(Symbol::Play), ImageSource::symbol(Symbol::Pause)];
/// let cx = twine_reactive::create_root();
/// let run = cx.signal(true);
/// let _v = animimg(&FRAMES, Duration::ms(400)).repeat(Repeat::Forever).playing(run);
/// cx.dispose();
/// ```
pub fn animimg<M1, M2>(
    frames: impl IntoProp<&'static [ImageSource], M1>,
    period: impl IntoProp<Duration, M2>,
) -> WidgetView<AnimImg> {
    let mut v = widget_view(AnimImg::new)
        .op(move |cx, node| {
            cx.engine()
                .with_widget_mut(node, |a: &mut AnimImg, wcx| a.set_repeat(wcx, Repeat::Forever));
        })
        .bind(frames, |a: &mut AnimImg, wcx, f| a.set_frames(wcx, f))
        .bind(period, |a: &mut AnimImg, wcx, p| a.set_period(wcx, p));
    let cfg = v.shared::<AnimImgCfg>();
    v.after_children(move |cx, node| {
        if !cfg.controlled.get() {
            cx.engine()
                .with_widget_mut(node, |a: &mut AnimImg, wcx| a.start(wcx));
        }
    })
}

impl WidgetView<AnimImg> {
    /// How often the frames play (default: forever).
    #[must_use]
    pub fn repeat<M>(self, r: impl IntoProp<Repeat, M>) -> Self {
        self.bind(r, |a: &mut AnimImg, cx, r| a.set_repeat(cx, r))
    }

    /// Plays while `true` (from the first frame each time it turns `true`), stops at the
    /// frame shown when `false`.
    #[must_use]
    pub fn playing<M>(mut self, on: impl IntoProp<bool, M>) -> Self {
        self.shared::<AnimImgCfg>().controlled.set(true);
        self.bind_after_children(on, |a: &mut AnimImg, wcx, on| {
            if a.is_playing(&wcx.measure()) != on {
                if on {
                    a.start(wcx);
                } else {
                    a.stop(wcx);
                }
            }
        })
    }
}
