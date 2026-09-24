//! [`AnimImg`]: an image that plays a sequence of frames (LVGL `lv_animimg`).

use alloc::boxed::Box;

use twine_core::{Duration, Point, Rect, Size};
use twine_engine::{
    Anim, AnimId, AnimProp, DrawCx, Engine, EngineError, Event, EventCx, EventResult, MeasureCx, NodeId,
    OBJ_FLAGS, ObjFlags, Repeat, Widget, WidgetClass, WidgetCx,
};
use twine_image::ImageSource;

use crate::image::Image;
use crate::log_set;

/// `AnimProp::Custom` id of the frame animation (distinct from the image's own ids).
pub const ANIM_FRAME: u16 = 0x7F10;

/// LVGL's default frame animation duration (`lv_animimg_constructor`: 30 ms).
pub const ANIMIMG_DEFAULT_PERIOD: Duration = Duration::ms(30);

/// The class of [`AnimImg`]: `"animimg"`, an image's part and flags (LVGL `lv_animimg_class`,
/// derived from `lv_image_class`).
pub static ANIMIMG_CLASS: WidgetClass = WidgetClass::new("animimg")
    .parts(&[twine_style::Part::Main])
    .default_flags(
        OBJ_FLAGS
            .difference(ObjFlags::CLICKABLE)
            .union(ObjFlags::ADV_HITTEST),
    );

/// A GIF played frame by frame with its own delays.
#[cfg(feature = "gif")]
#[derive(Debug)]
struct GifState {
    player: twine_image::decoders::gif::GifPlayer<'static>,
    timer: Option<twine_engine::TimerId>,
}

/// An animated image (LVGL `lv_animimg`): an [`Image`] whose source steps through `frames`.
///
/// One animation runs the frame index from the first to the last frame over
/// [`period`](Self::period) (linear, like LVGL), repeated per [`Repeat`] (LVGL's default is
/// forever). Every step sets the next frame through the image's idempotent `set_src`, so only
/// the image's area is redrawn when the frame changes. After the last repetition the last
/// frame stays and the engine goes idle. Nothing plays until [`start`](Self::start) (LVGL
/// `lv_animimg_start`).
///
/// With the `gif` feature, `AnimImg::from_gif` plays an animated GIF with each frame's own
/// delay (and the GIF's loop count).
///
/// ```
/// use twine_core::Duration;
/// use twine_image::ImageSource;
/// use twine_testing::EngineHarness;
/// use twine_widgets::animimg::{self, AnimImg};
///
/// static FRAMES: [ImageSource; 2] = [ImageSource::Symbol("\u{F00C}"), ImageSource::Symbol("\u{F00D}")];
/// let mut h = EngineHarness::new(60, 40);
/// let screen = h.screen();
/// let a = animimg::create(h.engine_mut(), screen).unwrap();
/// h.engine_mut().with_widget_mut(a, |w: &mut AnimImg, cx| {
///     w.set_frames(cx, &FRAMES);
///     w.set_period(cx, Duration::ms(400));
///     w.start(cx);
/// });
/// h.advance(Duration::ms(250));
/// assert_eq!(h.engine().widget::<AnimImg>(a).unwrap().frame_index(), 1);
/// ```
#[derive(Debug)]
pub struct AnimImg {
    image: Image,
    frames: &'static [ImageSource],
    cur: usize,
    period: Duration,
    repeat: Repeat,
    anim: Option<AnimId>,
    #[cfg(feature = "gif")]
    gif: Option<GifState>,
}

impl Default for AnimImg {
    fn default() -> Self {
        Self::new()
    }
}

impl AnimImg {
    /// An animated image without frames (30 ms, repeating forever, stopped).
    #[must_use]
    pub const fn new() -> Self {
        Self {
            image: Image::new(),
            frames: &[],
            cur: 0,
            period: ANIMIMG_DEFAULT_PERIOD,
            repeat: Repeat::Infinite,
            anim: None,
            #[cfg(feature = "gif")]
            gif: None,
        }
    }

    /// The image showing the frames (rotation, scale, alignment…).
    #[must_use]
    pub fn image(&self) -> &Image {
        &self.image
    }

    /// The image, mutably (its setters change how every frame is drawn).
    pub fn image_mut(&mut self) -> &mut Image {
        &mut self.image
    }

    /// The frames.
    #[must_use]
    pub fn frames(&self) -> &'static [ImageSource] {
        self.frames
    }

    /// The index of the frame shown.
    #[must_use]
    pub fn frame_index(&self) -> usize {
        #[cfg(feature = "gif")]
        if let Some(g) = &self.gif {
            return g.player.frame_index();
        }
        self.cur
    }

    /// The duration of one pass over the frames.
    #[must_use]
    pub fn period(&self) -> Duration {
        self.period
    }

    /// How often the frames are played.
    #[must_use]
    pub fn repeat(&self) -> Repeat {
        self.repeat
    }

    /// Whether the animation is running.
    #[must_use]
    pub fn is_playing(&self, cx: &MeasureCx<'_>) -> bool {
        #[cfg(feature = "gif")]
        if let Some(g) = &self.gif {
            return g.timer.is_some_and(|t| cx.engine().timer_exists(t));
        }
        self.anim.is_some_and(|id| cx.engine().anim_exists(id))
    }

    /// Sets the frames (LVGL `lv_animimg_set_src`) and shows the first one. A running
    /// animation restarts. Idempotent (same slice).
    pub fn set_frames(&mut self, cx: &mut WidgetCx<'_>, frames: &'static [ImageSource]) {
        if core::ptr::eq(self.frames, frames) {
            return;
        }
        log_set(ANIMIMG_CLASS.name, cx.node(), "frames");
        self.frames = frames;
        self.cur = 0;
        if let Some(f) = frames.first() {
            self.image.set_src(cx, f.clone());
        }
        self.restart_if_running(cx);
    }

    /// Sets the duration of one pass (LVGL `lv_animimg_set_duration`). Idempotent; a running
    /// animation restarts.
    pub fn set_period(&mut self, cx: &mut WidgetCx<'_>, period: Duration) {
        if self.period == period {
            return;
        }
        log_set(ANIMIMG_CLASS.name, cx.node(), "period");
        self.period = period;
        self.restart_if_running(cx);
    }

    /// Sets how often the frames play (LVGL `lv_animimg_set_repeat_count`). Idempotent; a
    /// running animation restarts.
    pub fn set_repeat(&mut self, cx: &mut WidgetCx<'_>, repeat: Repeat) {
        if self.repeat == repeat {
            return;
        }
        log_set(ANIMIMG_CLASS.name, cx.node(), "repeat");
        self.repeat = repeat;
        self.restart_if_running(cx);
    }

    fn restart_if_running(&mut self, cx: &mut WidgetCx<'_>) {
        if self.anim.is_some_and(|id| cx.engine().anim_exists(id)) {
            self.anim = None;
            self.start(cx);
        }
    }

    /// Plays the frames from the first one (LVGL `lv_animimg_start`).
    pub fn start(&mut self, cx: &mut WidgetCx<'_>) {
        #[cfg(feature = "gif")]
        if self.gif.is_some() {
            self.gif_start(cx);
            return;
        }
        self.stop(cx);
        let n = self.frames.len();
        if n == 0 {
            twine_core::warn!(target: "twine::engine", "animimg: start without frames");
            return;
        }
        log_set(ANIMIMG_CLASS.name, cx.node(), "start");
        let node = cx.node();
        let a = Anim::new(0, i32::try_from(n).unwrap_or(i32::MAX))
            .duration(self.period)
            .repeat(self.repeat);
        self.anim = Some(cx.engine_mut().anim_start(node, AnimProp::Custom(ANIM_FRAME), a));
        self.show(cx, 0);
    }

    /// Stops at the frame shown (LVGL `lv_animimg_delete`).
    pub fn stop(&mut self, cx: &mut WidgetCx<'_>) {
        #[cfg(feature = "gif")]
        if let Some(t) = self.gif.as_mut().and_then(|g| g.timer.take()) {
            cx.engine_mut().timer_remove(t);
        }
        if let Some(id) = self.anim.take() {
            cx.engine_mut().anim_stop(id);
        }
    }

    /// Shows frame `idx` (LVGL `index_change`: clamped to the last frame).
    fn show(&mut self, cx: &mut WidgetCx<'_>, idx: usize) {
        let n = self.frames.len();
        if n == 0 {
            return;
        }
        let idx = idx.min(n - 1);
        if idx == self.cur && self.image.src().is_some() {
            return;
        }
        self.cur = idx;
        self.image.set_src(cx, self.frames[idx].clone());
    }
}

#[cfg(feature = "gif")]
impl AnimImg {
    /// An animated image playing a GIF (feature `gif`): every frame is composited by the
    /// GIF player and shown for its own delay; the GIF's loop count applies. Call
    /// [`start`](Self::start) to play it.
    ///
    /// # Errors
    /// The GIF cannot be parsed.
    pub fn from_gif(bytes: &'static [u8]) -> Result<Self, twine_image::Error> {
        let player = twine_image::decoders::gif::GifPlayer::new(bytes)?;
        let mut s = Self::new();
        s.gif = Some(GifState { player, timer: None });
        Ok(s)
    }

    fn gif_start(&mut self, cx: &mut WidgetCx<'_>) {
        self.stop(cx);
        let Some(g) = self.gif.as_mut() else {
            return;
        };
        g.player.reset();
        let delay = g.player.advance();
        let node = cx.node();
        let t = cx.engine_mut().timer_add(delay, move |e, id| {
            let alive = e.with_widget_mut(node, |w: &mut AnimImg, cx| w.gif_step(cx, id));
            if alive.is_none() {
                e.timer_remove(id);
            }
        });
        g.timer = Some(t);
        cx.mark_layout();
        cx.invalidate_for("animimg.gif");
    }

    /// One GIF frame: composite the next one and wait its delay.
    fn gif_step(&mut self, cx: &mut WidgetCx<'_>, id: twine_engine::TimerId) {
        let Some(g) = self.gif.as_mut() else {
            cx.engine_mut().timer_remove(id);
            return;
        };
        let delay = g.player.advance();
        if g.player.is_finished() {
            g.timer = None;
            cx.engine_mut().timer_remove(id);
            return;
        }
        cx.engine_mut().timer_set_period(id, delay);
        cx.invalidate_for("animimg.gif");
    }
}

/// Creates an animated image without frames as the last child of `parent` (LVGL
/// `lv_animimg_create`).
///
/// # Errors
/// [`EngineError::NodeNotFound`] if `parent` does not exist (logged).
pub fn create(engine: &mut Engine, parent: NodeId) -> Result<NodeId, EngineError> {
    engine.create(parent, Box::new(AnimImg::new()))
}

impl Widget for AnimImg {
    fn class(&self) -> &'static WidgetClass {
        &ANIMIMG_CLASS
    }

    fn init(&mut self, cx: &mut WidgetCx<'_>) {
        self.image.init(cx);
    }

    fn content_size(&self, cx: &MeasureCx<'_>) -> Size {
        #[cfg(feature = "gif")]
        if let Some(g) = &self.gif {
            let h = g.player.header();
            return Size::new(i32::from(h.w), i32::from(h.h));
        }
        self.image.content_size(cx)
    }

    fn draw(&self, cx: &mut DrawCx<'_, '_>) {
        #[cfg(feature = "gif")]
        if let Some(g) = &self.gif {
            cx.draw_base(twine_style::Part::Main);
            let px = g.player.pixels();
            let c = cx.coords();
            let area = Rect::from_xywh(c.x0, c.y0, i32::from(px.w), i32::from(px.h));
            let dsc = cx.image_dsc(twine_style::Part::Main);
            cx.painter().image(area, &px, &dsc);
            return;
        }
        self.image.draw(cx);
    }

    fn ext_draw_size(&self, cx: &MeasureCx<'_>) -> u16 {
        self.image.ext_draw_size(cx)
    }

    fn covers(&self, cx: &MeasureCx<'_>, area: Rect) -> bool {
        #[cfg(feature = "gif")]
        if self.gif.is_some() {
            return twine_engine::default_covers(cx, area);
        }
        self.image.covers(cx, area)
    }

    fn hit_test(&self, cx: &MeasureCx<'_>, p: Point) -> bool {
        self.image.hit_test(cx, p)
    }

    fn event(&mut self, cx: &mut EventCx<'_>, ev: &Event) -> EventResult {
        self.image.event(cx, ev)
    }

    fn anim_value(&mut self, cx: &mut WidgetCx<'_>, v: i32) {
        self.image.anim_value(cx, v);
    }

    fn anim_custom(&mut self, cx: &mut WidgetCx<'_>, id: u16, v: i32) {
        if id == ANIM_FRAME {
            self.show(cx, usize::try_from(v).unwrap_or(0));
        } else {
            self.image.anim_custom(cx, id, v);
        }
    }
}
