//! [`Lottie`]: an engine widget playing a Lottie animation (LVGL `lv_lottie`), redrawing only
//! when the frame changes.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::cell::RefCell;

use twine_core::{Color, ColorFormat, Duration, Rect, Scale, Size};
use twine_engine::{
    Anim, AnimId, AnimProp, DrawCx, Engine, EngineError, EventCode, EventParam, MeasureCx, NodeId, OBJ_FLAGS,
    Repeat, Widget, WidgetClass, WidgetCx,
};
use twine_render::{DrawBuf, ImageDsc, ImagePixels, Painter, RectDsc};
use twine_style::Part;

use crate::{LottiePlayer, load};

/// The class of [`Lottie`].
pub static LOTTIE_CLASS: WidgetClass = WidgetClass::new("lottie").default_flags(OBJ_FLAGS);

/// `AnimProp::Custom` id of the playback animation (its values are frame numbers).
pub const ANIM_FRAME: u16 = 0x4C54;

/// The render buffer and the player, touched while drawing (`draw` takes `&self`).
#[derive(Debug, Default)]
struct Canvas {
    player: Option<LottiePlayer>,
    /// `Argb8888` pixels of the content area (transparent background).
    buf: Vec<u8>,
    size: Size,
    /// The frame in `buf` (`None`: re-render before the next blit).
    rendered: Option<u32>,
}

/// Plays a Lottie animation fitted (contain, centered) into its content area.
///
/// Events: `ValueChanged` (parameter: the frame) whenever playback shows another frame,
/// `Ready` when a non-looping animation reaches its last frame.
///
/// The frames are rendered into the widget's own `Argb8888` buffer, allocated once per
/// content size (`w × h × 4` bytes: a 100 × 100 animation costs 40 KB) and re-rendered lazily
/// at draw time, only when the integer frame changed; unchanged frames are just blitted.
/// Playback is an engine animation from the first to the last frame (duration = frames ÷ frame
/// rate ÷ speed), so a paused or finished animation leaves the UI idle.
///
/// ```
/// use twine_core::Duration;
/// use twine_testing::EngineHarness;
/// use twine_lottie::widget::{self, Lottie};
///
/// static SPIN: &[u8] = br#"{"fr":30,"ip":0,"op":30,"w":40,"h":40,"layers":[{"ty":4,
///   "ks":{"r":{"a":1,"k":[{"t":0,"s":[0]},{"t":30,"s":[360]}]},"p":{"k":[20,20]}},
///   "shapes":[{"ty":"rc","p":{"k":[0,0]},"s":{"k":[20,20]},"r":{"k":0}},
///             {"ty":"fl","c":{"k":[1,0,0,1]},"o":{"k":100}}]}]}"#;
///
/// let mut h = EngineHarness::new(64, 64);
/// let screen = h.screen();
/// let l = widget::create(h.engine_mut(), screen).unwrap();
/// h.engine_mut().set_size(l, 40, 40);
/// h.engine_mut().with_widget_mut(l, |w: &mut Lottie, cx| { w.set_src(cx, SPIN); w.play(cx); });
/// h.advance(Duration::ms(500));
/// assert!(h.engine().widget::<Lottie>(l).unwrap().current_frame() >= 14);
/// ```
#[derive(Debug)]
pub struct Lottie {
    canvas: RefCell<Canvas>,
    /// The source could not be loaded: a placeholder is drawn.
    invalid: bool,
    /// The source (identity for idempotence).
    src: Option<&'static [u8]>,
    /// Current frame, `0..total_frames` (relative to the composition's in point).
    frame: u32,
    anim: Option<AnimId>,
    playing: bool,
    looping: bool,
    speed: Scale,
}

impl Default for Lottie {
    fn default() -> Self {
        Self::new()
    }
}

impl Lottie {
    /// A widget without an animation; playback loops by default (like LVGL).
    #[must_use]
    pub fn new() -> Self {
        Self {
            canvas: RefCell::new(Canvas::default()),
            invalid: false,
            src: None,
            frame: 0,
            anim: None,
            playing: false,
            looping: true,
            speed: Scale::ONE,
        }
    }

    /// Loads the Lottie JSON `data` (parsed once; stored without copying). Idempotent for the
    /// same bytes. Invalid data logs a warning and shows a placeholder. The frame goes back to
    /// the first one; playback continues when it was playing.
    pub fn set_src(&mut self, cx: &mut WidgetCx<'_>, data: &'static [u8]) {
        if self.src.is_some_and(|s| core::ptr::eq(s, data)) {
            return;
        }
        twine_core::trace!(target: "twine::engine", "lottie#{:?} set_src", cx.node());
        self.src = Some(data);
        let player = match load(data) {
            Ok(comp) => Some(LottiePlayer::new(comp)),
            Err(e) => {
                twine_core::warn!(target: "twine::lottie", "lottie: cannot load the animation: {}", e);
                None
            }
        };
        self.invalid = player.is_none();
        {
            let c = self.canvas.get_mut();
            c.player = player;
            c.rendered = None;
        }
        self.frame = 0;
        cx.mark_layout();
        cx.invalidate_for("lottie.set_src");
        if self.playing {
            self.start_anim(cx);
        }
    }

    /// Number of frames (`op − ip` of the composition; 0 without a valid source).
    #[must_use]
    pub fn total_frames(&self) -> u32 {
        self.canvas.borrow().player.as_ref().map_or(0, |p| {
            let c = p.composition();
            (c.op - c.ip).max(0.0) as u32
        })
    }

    /// The current frame (`0..total_frames`).
    #[must_use]
    pub fn current_frame(&self) -> u32 {
        self.frame
    }

    /// Whether the animation is playing.
    #[must_use]
    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// Whether playback loops.
    #[must_use]
    pub fn is_looping(&self) -> bool {
        self.looping
    }

    /// The playback speed (256 = 1×).
    #[must_use]
    pub fn speed(&self) -> Scale {
        self.speed
    }

    /// Frames per second of the composition (0 without a valid source).
    #[must_use]
    pub fn frame_rate(&self) -> u32 {
        self.canvas
            .borrow()
            .player
            .as_ref()
            .map_or(0, |p| p.composition().fr.max(0.0) as u32)
    }

    /// Shows frame `f` (clamped to the last frame). Idempotent: the same frame neither
    /// re-renders nor invalidates. While playing, playback continues from `f`.
    pub fn set_frame(&mut self, cx: &mut WidgetCx<'_>, f: u32) {
        let last = self.total_frames().saturating_sub(1);
        let f = f.min(last);
        if f == self.frame {
            return;
        }
        self.show_frame(cx, f);
        self.restart_or_drop(cx);
    }

    /// After a change of frame, loop mode or speed: a playing animation restarts with the new
    /// settings; a paused one is dropped (it would resume with the old ones), so `play`
    /// starts afresh.
    fn restart_or_drop(&mut self, cx: &mut WidgetCx<'_>) {
        if self.playing {
            self.start_anim(cx);
        } else if let Some(id) = self.anim.take() {
            cx.engine_mut().anim_stop(id);
        }
    }

    /// Starts (or resumes) playback. Idempotent. A finished, non-looping animation starts
    /// again from the first frame.
    pub fn play(&mut self, cx: &mut WidgetCx<'_>) {
        if self.playing {
            return;
        }
        self.playing = true;
        if let Some(id) = self.anim {
            if cx.engine_mut().anim_is_paused(id) && cx.engine_mut().anim_resume(id) {
                return;
            }
        }
        if !self.looping && self.frame + 1 >= self.total_frames() {
            self.show_frame(cx, 0);
        }
        self.start_anim(cx);
    }

    /// Pauses playback at the current frame. Idempotent. The UI is idle while paused.
    pub fn pause(&mut self, cx: &mut WidgetCx<'_>) {
        if !self.playing {
            return;
        }
        self.playing = false;
        if let Some(id) = self.anim {
            cx.engine_mut().anim_pause(id);
        }
    }

    /// Sets whether playback loops (default `true`). Idempotent.
    pub fn set_loop(&mut self, cx: &mut WidgetCx<'_>, on: bool) {
        if self.looping == on {
            return;
        }
        self.looping = on;
        self.restart_or_drop(cx);
    }

    /// Sets the playback speed (256 = 1×; 0 is ignored with a warning). Idempotent.
    pub fn set_speed(&mut self, cx: &mut WidgetCx<'_>, speed: Scale) {
        if speed.0 == 0 {
            twine_core::warn!(target: "twine::lottie", "lottie: speed 0 ignored (use pause)");
            return;
        }
        if self.speed == speed {
            return;
        }
        self.speed = speed;
        self.restart_or_drop(cx);
    }

    /// Makes `f` the current frame and invalidates the widget (the buffer re-renders lazily).
    fn show_frame(&mut self, cx: &mut WidgetCx<'_>, f: u32) {
        self.frame = f;
        cx.invalidate_for("lottie.frame");
    }

    /// (Re)starts the playback animation from the current frame.
    fn start_anim(&mut self, cx: &mut WidgetCx<'_>) {
        if let Some(id) = self.anim.take() {
            cx.engine_mut().anim_stop(id);
        }
        let total = self.total_frames();
        let fr = self.frame_rate().max(1);
        if total == 0 {
            self.playing = false;
            return;
        }
        let from = self.frame.min(total - 1);
        // Frames → milliseconds at this speed.
        let ms = |frames: u32| {
            let v = u64::from(frames) * 1000 * 256 / (u64::from(fr) * u64::from(self.speed.0.max(1)));
            Duration::ms(v.max(1))
        };
        let to_i32 = |v: u32| i32::try_from(v).unwrap_or(i32::MAX);
        let anim = if self.looping {
            // One cycle from the current frame round to it again (values taken modulo the
            // frame count), repeated forever: seeking keeps the loop continuous.
            Anim::new(to_i32(from), to_i32(from + total))
                .duration(ms(total))
                .repeat(Repeat::Infinite)
        } else if from + 1 >= total {
            self.playing = false;
            return;
        } else {
            Anim::new(to_i32(from), to_i32(total - 1)).duration(ms(total - 1 - from))
        };
        let node = cx.node();
        self.anim = Some(
            cx.engine_mut()
                .anim_start(node, AnimProp::Custom(ANIM_FRAME), anim),
        );
    }

    /// The placeholder of invalid data: a grey box.
    fn draw_placeholder(cx: &mut DrawCx<'_, '_>, area: Rect) {
        let opa = cx.opa();
        cx.painter().rect(
            area,
            &RectDsc {
                radius: 2,
                bg_color: Color::hex(0x00D0_D0D0),
                bg_opa: opa,
                border_color: Color::hex(0x0090_9090),
                border_width: 1,
                border_opa: opa,
                ..RectDsc::default()
            },
        );
    }
}

/// Renders `frame` of `player` into `buf` (`w × h` `Argb8888`, cleared) with a painter
/// sharing `p`'s render caches.
fn render_into(p: &mut Painter<'_>, player: &mut LottiePlayer, buf: &mut [u8], size: Size, frame: u32) {
    buf.fill(0);
    let dst = Rect::from_xywh(0, 0, size.w, size.h);
    let Ok(db) = DrawBuf::new_packed(buf, ColorFormat::Argb8888, dst) else {
        twine_core::warn!(target: "twine::lottie", "lottie: ARGB8888 rendering not compiled in (color-argb8888)");
        return;
    };
    let ip = player.composition().ip;
    let mut painter = Painter::new(db, p.caches());
    player.render_frame(ip + frame as f32, &mut painter, dst);
}

impl Widget for Lottie {
    fn class(&self) -> &'static WidgetClass {
        &LOTTIE_CLASS
    }

    /// The composition's size.
    fn content_size(&self, _cx: &MeasureCx<'_>) -> Size {
        self.canvas.borrow().player.as_ref().map_or(Size::ZERO, |p| {
            let c = p.composition();
            Size::new(c.w.max(0.0) as i32, c.h.max(0.0) as i32)
        })
    }

    fn draw(&self, cx: &mut DrawCx<'_, '_>) {
        cx.draw_base(Part::Main);
        let area = cx.content_area();
        if area.is_empty() {
            return;
        }
        if self.invalid {
            Self::draw_placeholder(cx, area);
            return;
        }
        let mut c = self.canvas.borrow_mut();
        let Canvas {
            player,
            buf,
            size,
            rendered,
        } = &mut *c;
        let Some(player) = player.as_mut() else {
            return;
        };
        let want = Size::new(area.width(), area.height());
        if *size != want || buf.is_empty() {
            // Once per content size (P6: nothing is allocated per frame).
            let n = (want.w as usize) * (want.h as usize) * 4;
            buf.clear();
            if buf.try_reserve_exact(n).is_err() {
                twine_core::warn!(target: "twine::lottie", "lottie: no memory for a {}x{} buffer", want.w, want.h);
                return;
            }
            buf.resize(n, 0);
            *size = want;
            *rendered = None;
        }
        if *rendered != Some(self.frame) {
            render_into(cx.painter(), player, buf, *size, self.frame);
            *rendered = Some(self.frame);
        }
        let px = ImagePixels {
            format: ColorFormat::Argb8888,
            w: u16::try_from(size.w).unwrap_or(0),
            h: u16::try_from(size.h).unwrap_or(0),
            stride: u16::try_from(size.w * 4).unwrap_or(0),
            data: buf,
            palette: None,
            alpha: None,
            premultiplied: false,
        };
        let dsc: ImageDsc<'_> = cx.image_dsc(Part::Main);
        cx.painter().image(area, &px, &dsc);
    }

    fn anim_custom(&mut self, cx: &mut WidgetCx<'_>, id: u16, v: i32) {
        if id != ANIM_FRAME {
            return;
        }
        let total = self.total_frames();
        if total == 0 {
            return;
        }
        let f = u32::try_from(v.rem_euclid(i32::try_from(total).unwrap_or(i32::MAX))).unwrap_or(0);
        if f != self.frame {
            self.show_frame(cx, f);
            cx.post_event(
                EventCode::ValueChanged,
                EventParam::Value(i32::try_from(f).unwrap_or(0)),
            );
        }
        if !self.looping && f + 1 >= total && self.playing {
            self.playing = false;
            self.anim = None;
            cx.post_event(EventCode::Ready, EventParam::None);
        }
    }
}

/// Creates an empty Lottie widget as the last child of `parent`.
///
/// # Errors
/// [`EngineError::NodeNotFound`] if `parent` does not exist (logged).
pub fn create(engine: &mut Engine, parent: NodeId) -> Result<NodeId, EngineError> {
    engine.create(parent, Box::new(Lottie::new()))
}
