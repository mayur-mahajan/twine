//! [`lottie`]: the declarative view of the [`Lottie`] widget.

use twine_core::Scale;
use twine_engine::{EventCode, EventFilter, EventResult, NodeId};
use twine_view::{
    BuildCx, BuildOp, EngineAccess, IntoModel, IntoProp, View, ViewExt, WidgetView, bind_model, event_value,
    widget_view,
};

use crate::widget::Lottie;

/// A Lottie animation of `w × h` pixels (fitted, centered) from the JSON `src` (parsed once,
/// never copied). It loops and starts paused; bind [`playing`](LottieView::playing),
/// [`looping`](LottieView::looping), [`frame`](LottieView::frame) and
/// [`speed`](LottieView::speed) to control it.
///
/// ```
/// use twine_view::prelude::*;
/// use twine_lottie::view::lottie;
///
/// static LOADER: &[u8] = br#"{"fr":30,"ip":0,"op":60,"w":100,"h":100,"layers":[]}"#;
///
/// fn app(cx: Scope) -> impl View {
///     let playing = cx.signal(true);
///     let frame = cx.signal(0u32);
///     column((
///         lottie(LOADER, 100, 100).playing(playing).frame(frame),
///         label(text!("frame {}", frame.get())),
///         button(label("Pause")).on_click(move || playing.set(false)),
///     ))
/// }
/// # let _ = app;
/// ```
pub fn lottie(src: &'static [u8], w: i32, h: i32) -> LottieView {
    LottieView(
        widget_view(Lottie::new)
            .op(move |cx: &mut BuildCx<'_>, node: NodeId| {
                cx.engine()
                    .with_widget_mut(node, |l: &mut Lottie, wcx| l.set_src(wcx, src));
            })
            .size(w, h),
    )
}

/// The view of [`lottie`].
#[derive(Debug)]
#[must_use]
pub struct LottieView(WidgetView<Lottie>);

impl LottieView {
    /// Plays (`true`) or pauses (`false`); default paused.
    pub fn playing(self, on: impl IntoProp<bool>) -> Self {
        Self(self.0.bind(on, |l: &mut Lottie, cx, on| {
            if on {
                l.play(cx);
            } else {
                l.pause(cx);
            }
        }))
    }

    /// Loops (default) or stops at the last frame. (`loop` is a Rust keyword.)
    pub fn looping(self, on: impl IntoProp<bool>) -> Self {
        Self(self.0.bind(on, |l: &mut Lottie, cx, on| l.set_loop(cx, on)))
    }

    /// The current frame, two-way: a bound signal follows playback and seeks when set.
    pub fn frame(self, frame: impl IntoModel<u32>) -> Self {
        let model = frame.into_model();
        Self(self.0.op(move |cx, node| {
            bind_model(
                cx,
                node,
                model,
                |e, n, f: u32| {
                    e.with_widget_mut(n, |l: &mut Lottie, wcx| l.set_frame(wcx, f));
                },
                |e, n, ev| {
                    event_value(ev)
                        .and_then(|v| u32::try_from(v).ok())
                        .or_else(|| e.widget::<Lottie>(n).map(Lottie::current_frame))
                        .unwrap_or(0)
                },
            );
        }))
    }

    /// The playback speed (256 = 1×).
    pub fn speed(self, s: impl IntoProp<Scale>) -> Self {
        Self(self.0.bind(s, |l: &mut Lottie, cx, s| l.set_speed(cx, s)))
    }

    /// Called when a non-looping animation reaches its last frame (once per playback).
    pub fn on_complete(self, f: impl Fn() + 'static) -> Self {
        Self(self.0.op(move |cx, node| {
            cx.engine()
                .add_event_handler(node, EventFilter::Code(EventCode::Ready), move |ecx, ev| {
                    if ev.target == ecx.node() {
                        EngineAccess::provide(ecx.engine_mut(), &f);
                    }
                    EventResult::Continue
                });
        }))
    }
}

impl View for LottieView {
    fn build(self, cx: &mut BuildCx<'_>) -> NodeId {
        self.0.build(cx)
    }
}

impl ViewExt for LottieView {
    type Widget = Lottie;

    fn push_op(self, op: BuildOp) -> Self {
        LottieView(self.0.push_op(op))
    }
}
