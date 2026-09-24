//! The label's long modes: `Dots` truncation and the `Scroll` / `ScrollCircular` offset
//! animations (LVGL `lv_label_refr_text`).

use twine_core::{Duration, Point};
use twine_engine::{Anim, AnimId, AnimProp, Easing, Repeat, WidgetCx};
use twine_style::{Part, PropId};
use twine_text::{Font, LongMode, TextFlags, TextLayout};

use super::{LABEL_DOT_NUM, LABEL_WAIT_CHAR_COUNT, Label};

/// `AnimProp::Custom` id of the horizontal text offset (LVGL `set_ofs_x_anim`).
pub const ANIM_OFS_X: u16 = 0;
/// `AnimProp::Custom` id of the vertical text offset (LVGL `set_ofs_y_anim`).
pub const ANIM_OFS_Y: u16 = 1;
/// LVGL `LV_LABEL_DEF_SCROLL_SPEED` = `lv_anim_speed_clamped(40, 300, 10000)`: 40 px/s, the
/// duration clamped to 300 ms … 10 s. Used when the `AnimDuration` style is 0.
pub const LABEL_DEF_SCROLL_SPEED: u32 = 40;
/// LVGL `LV_LABEL_SCROLL_DELAY`: the pause at each end of a `Scroll` animation.
pub const LABEL_SCROLL_DELAY: Duration = Duration::ms(300);

/// `Dots` state: how many lines are drawn and where the visible text ends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct DotState {
    /// Lines drawn (the last one ends with "..."); `None` when the text fits.
    pub lines: Option<u16>,
    /// Byte index of the end of the visible text before "...".
    pub end: Option<usize>,
}

/// One running offset animation: its id and the range it animates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct OfsAnim {
    id: AnimId,
    start: i32,
    end: i32,
    duration: Duration,
}

/// The scroll offset and its animations.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct ScrollState {
    pub ofs: Point,
    x: Option<OfsAnim>,
    y: Option<OfsAnim>,
}

impl ScrollState {
    /// Stops both animations and resets the offset (LVGL `lv_label_set_long_mode`).
    pub fn stop(&mut self, cx: &mut WidgetCx<'_>) {
        for a in [self.x.take(), self.y.take()].into_iter().flatten() {
            cx.engine_mut().anim_stop(a.id);
        }
        if self.ofs != Point::ZERO {
            self.ofs = Point::ZERO;
            cx.invalidate_for("label.scroll");
        }
    }

    /// Applies an animated offset (LVGL `set_ofs_x_anim` / `set_ofs_y_anim`); only the
    /// label's own area is redrawn.
    pub fn apply_anim(&mut self, cx: &mut WidgetCx<'_>, id: u16, v: i32) {
        let changed = match id {
            ANIM_OFS_X if self.ofs.x != v => {
                self.ofs.x = v;
                true
            }
            ANIM_OFS_Y if self.ofs.y != v => {
                self.ofs.y = v;
                true
            }
            _ => false,
        };
        if changed {
            cx.invalidate_for("label.scroll");
        }
    }

    /// Runs `anim` on the axis unless the same one is running already (then the text keeps
    /// scrolling from where it is, like LVGL keeping `act_time`).
    fn ensure(
        slot: &mut Option<OfsAnim>,
        cx: &mut WidgetCx<'_>,
        custom: u16,
        (start, end, duration): (i32, i32, Duration),
        anim: Anim,
    ) {
        if let Some(a) = slot {
            if a.start == start && a.end == end && a.duration == duration && cx.engine().anim_exists(a.id) {
                return;
            }
        }
        let node = cx.node();
        let id = cx.engine_mut().anim_start(node, AnimProp::Custom(custom), anim);
        *slot = Some(OfsAnim {
            id,
            start,
            end,
            duration,
        });
    }

    /// Stops the animation of an axis and resets its offset.
    fn clear(slot: &mut Option<OfsAnim>, ofs: &mut i32, cx: &mut WidgetCx<'_>) {
        if let Some(a) = slot.take() {
            cx.engine_mut().anim_stop(a.id);
        }
        if *ofs != 0 {
            *ofs = 0;
            cx.invalidate_for("label.scroll");
        }
    }
}

/// Width of the gap between the two copies of a circular scroll: `LV_LABEL_WAIT_CHAR_COUNT`
/// space widths.
pub(super) fn gap_width(font: &'static Font) -> i32 {
    font.advance_px(' ', Some(' ')) * LABEL_WAIT_CHAR_COUNT
}

/// The duration of an offset animation over `dist` pixels: the `AnimDuration` style when set,
/// else LVGL's default speed (`lv_anim_resolve_speed` of `LV_LABEL_DEF_SCROLL_SPEED`: 10 px/s
/// resolution, clamped to 300 … 10 000 ms).
fn anim_duration(cx: &WidgetCx<'_>, dist: i32) -> Duration {
    let styled = cx.style(Part::Main, PropId::AnimDuration).as_i32().unwrap_or(0);
    if styled > 0 {
        return Duration::ms(u64::from(styled.unsigned_abs()));
    }
    let speed = u64::from((LABEL_DEF_SCROLL_SPEED + 5) / 10); // in 10 px/s units, as LVGL
    let ms = u64::from(dist.unsigned_abs()) * 100 / speed;
    Duration::ms(ms.clamp(300, 10_000))
}

impl Label {
    /// LVGL `lv_label_refr_text`: measures the text in the content area and updates the long
    /// mode state (offset animations, dots). Invalidates the label.
    pub(super) fn refr_text(&mut self, cx: &mut WidgetCx<'_>) {
        let content = cx.content_area();
        let d = cx.text_dsc(Part::Main);
        let expand = self.expand();
        // Borrow only the text fields: the state fields are updated below.
        let text = match &self.mask {
            Some(m) => m.shown.as_str(),
            None => self.text.as_str(),
        };
        let mut layout = TextLayout::new(text, d.font);
        layout.letter_space = d.letter_space;
        layout.line_space = d.line_space;
        layout.max_width = content.width();
        if expand {
            layout.flags |= TextFlags::EXPAND;
        }
        let size = layout.measure();
        self.text_size = size;
        let (cw, ch) = (content.width(), content.height());
        let font_h = i32::from(d.font.line_height);
        let laid_out = !cx.coords().is_empty();
        match self.long_mode {
            LongMode::Scroll if laid_out => {
                let mut hor = false;
                if size.w > cw {
                    let (start, end) = (0, cw - size.w);
                    let t = anim_duration(cx, end - start);
                    let a = Anim::new(start, end)
                        .duration(t)
                        .easing(Easing::Linear)
                        .playback(t)
                        .playback_delay(LABEL_SCROLL_DELAY)
                        .repeat_delay(LABEL_SCROLL_DELAY)
                        .repeat(Repeat::Infinite);
                    ScrollState::ensure(&mut self.scroll.x, cx, ANIM_OFS_X, (start, end, t), a);
                    hor = true;
                } else {
                    ScrollState::clear(&mut self.scroll.x, &mut self.scroll.ofs.x, cx);
                }
                if size.h > ch && !hor {
                    let (start, end) = (0, ch - size.h - font_h);
                    let t = anim_duration(cx, end - start);
                    let a = Anim::new(start, end)
                        .duration(t)
                        .easing(Easing::Linear)
                        .playback(t)
                        .playback_delay(LABEL_SCROLL_DELAY)
                        .repeat_delay(LABEL_SCROLL_DELAY)
                        .repeat(Repeat::Infinite);
                    ScrollState::ensure(&mut self.scroll.y, cx, ANIM_OFS_Y, (start, end, t), a);
                } else {
                    ScrollState::clear(&mut self.scroll.y, &mut self.scroll.ofs.y, cx);
                }
            }
            LongMode::ScrollCircular if laid_out => {
                let mut hor = false;
                if size.w > cw {
                    let (start, end) = (0, -size.w - gap_width(d.font));
                    let t = anim_duration(cx, end - start);
                    let a = Anim::new(start, end)
                        .duration(t)
                        .easing(Easing::Linear)
                        .repeat(Repeat::Infinite);
                    ScrollState::ensure(&mut self.scroll.x, cx, ANIM_OFS_X, (start, end, t), a);
                    hor = true;
                } else {
                    ScrollState::clear(&mut self.scroll.x, &mut self.scroll.ofs.x, cx);
                }
                if size.h > ch && !hor {
                    let (start, end) = (0, -size.h - font_h);
                    let t = anim_duration(cx, end - start);
                    let a = Anim::new(start, end)
                        .duration(t)
                        .easing(Easing::Linear)
                        .repeat(Repeat::Infinite);
                    ScrollState::ensure(&mut self.scroll.y, cx, ANIM_OFS_Y, (start, end, t), a);
                } else {
                    ScrollState::clear(&mut self.scroll.y, &mut self.scroll.ofs.y, cx);
                }
            }
            LongMode::Dots => {
                self.dot = DotState::default();
                if size.h > ch && size.h > font_h && text.chars().count() > LABEL_DOT_NUM {
                    // LVGL rounds the height down to the last line that fits entirely.
                    let step = font_h + d.line_space;
                    let lines = if step > 0 { (ch + d.line_space) / step } else { 1 }.max(1);
                    let lines = u16::try_from(lines).unwrap_or(u16::MAX);
                    self.dot.lines = Some(lines);
                    self.dot.end = layout.ellipsize(usize::from(lines)).map(|e| e.keep.end);
                }
            }
            // Not laid out yet (scrolling modes), or nothing to do (wrap, clip).
            LongMode::Scroll | LongMode::ScrollCircular | LongMode::Wrap | LongMode::Clip => {}
        }
        cx.invalidate_for("label");
    }
}
