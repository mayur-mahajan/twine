//! [`Checkbox`]: a check box with a text (LVGL `lv_checkbox`).

use alloc::boxed::Box;
use alloc::string::String;

use twine_core::{Rect, Size};
use twine_engine::{
    DrawCx, Engine, EngineError, Event, EventCode, EventCx, EventResult, GroupDef, MeasureCx, NodeId,
    OBJ_FLAGS, ObjFlags, State, Widget, WidgetClass, WidgetCx,
};
use twine_style::{COORD_MAX, Part, PropId};
use twine_text::TextLayout;

use crate::label::LabelText;
use crate::log_set;
use crate::util::{self, Area};

/// LVGL's default checkbox text (`LV_WIDGETS_HAS_DEFAULT_VALUE`).
pub const CHECKBOX_DEFAULT_TEXT: &str = "Check box";

/// Default flags of [`CHECKBOX_CLASS`] (LVGL `lv_checkbox_constructor`:
/// `lv_obj_set_clickable(obj, true)`, `lv_obj_set_checkable(obj, true)`,
/// `lv_obj_set_scroll_on_focus(obj, true)`, `lv_obj_set_scrollable(obj, false)`).
const CHECKBOX_FLAGS: ObjFlags = OBJ_FLAGS
    .difference(ObjFlags::SCROLLABLE)
    .union(ObjFlags::CLICKABLE)
    .union(ObjFlags::CHECKABLE)
    .union(ObjFlags::SCROLL_ON_FOCUS);

/// The class of [`Checkbox`]: `"checkbox"`, parts `Main` (the whole widget and the text) and
/// `Indicator` (the box), always in the default focus group (LVGL `lv_checkbox_class`).
pub static CHECKBOX_CLASS: WidgetClass = WidgetClass::new("checkbox")
    .parts(&[Part::Main, Part::Indicator])
    .default_flags(CHECKBOX_FLAGS)
    .group_def(GroupDef::True);

/// A check box with a text (LVGL `lv_checkbox`).
///
/// The checked state is the engine's `State::CHECKED`: a click anywhere on the widget (box or
/// text) or `Enter` toggles it and sends `ValueChanged`; the arrow keys set it. The box is the
/// `Indicator` part, one line of the font high plus its paddings; the default theme draws the
/// check mark as the indicator's background image (`LV_SYMBOL_OK` with the small font) when
/// checked. The text follows at `pad_column`, vertically centered on the box.
///
/// The size is `Content` by default: box + `pad_column` + text (LVGL `GET_SELF_SIZE`). The
/// text is stored like a [`Label`](crate::label::Label)'s: [`set_text`](Self::set_text)
/// reuses the buffer, [`set_text_static`](Self::set_text_static) does not copy.
///
/// ```
/// use twine_testing::EngineHarness;
/// use twine_widgets::checkbox::{self, Checkbox};
///
/// let mut h = EngineHarness::new(160, 40);
/// let screen = h.screen();
/// let c = checkbox::create(h.engine_mut(), screen).unwrap();
/// h.engine_mut().with_widget_mut(c, |w: &mut Checkbox, cx| w.set_text(cx, "Remember me"));
/// assert_eq!(h.engine().widget::<Checkbox>(c).unwrap().text(), "Remember me");
/// ```
#[derive(Debug)]
pub struct Checkbox {
    text: LabelText,
    /// A released owned buffer, reused by the next [`set_text`](Self::set_text).
    scratch: String,
}

impl Default for Checkbox {
    fn default() -> Self {
        Self::new(CHECKBOX_DEFAULT_TEXT)
    }
}

impl Checkbox {
    /// A checkbox showing `text` (stored without copying).
    #[must_use]
    pub const fn new(text: &'static str) -> Self {
        Self {
            text: LabelText::Static(text),
            scratch: String::new(),
        }
    }

    /// The text.
    #[must_use]
    pub fn text(&self) -> &str {
        self.text.as_str()
    }

    /// Whether the box is checked.
    #[must_use]
    pub fn is_checked(&self, cx: &MeasureCx<'_>) -> bool {
        cx.state().contains(State::CHECKED)
    }

    /// Sets the text, copying it into the checkbox's buffer (no allocation when it fits).
    /// Idempotent; a change re-measures the checkbox.
    pub fn set_text(&mut self, cx: &mut WidgetCx<'_>, s: &str) {
        if self.text() == s {
            return;
        }
        log_set(CHECKBOX_CLASS.name, cx.node(), "text");
        match &mut self.text {
            LabelText::Owned(buf) => {
                buf.clear();
                buf.push_str(s);
            }
            LabelText::Static(_) => {
                let mut buf = core::mem::take(&mut self.scratch);
                buf.clear();
                buf.push_str(s);
                self.text = LabelText::Owned(buf);
            }
        }
        Self::text_changed(cx);
    }

    /// Sets a `'static` text without copying (LVGL `lv_checkbox_set_text_static`).
    /// Idempotent.
    pub fn set_text_static(&mut self, cx: &mut WidgetCx<'_>, s: &'static str) {
        let same = self.text() == s;
        if let LabelText::Owned(buf) = core::mem::replace(&mut self.text, LabelText::Static(s)) {
            self.scratch = buf;
        }
        if same {
            return;
        }
        log_set(CHECKBOX_CLASS.name, cx.node(), "text_static");
        Self::text_changed(cx);
    }

    /// Checks or unchecks the box from code (no `ValueChanged`). Idempotent.
    pub fn set_checked(&mut self, cx: &mut WidgetCx<'_>, on: bool) {
        if cx.state().contains(State::CHECKED) == on {
            return;
        }
        log_set(CHECKBOX_CLASS.name, cx.node(), "checked");
        if on {
            cx.add_state(State::CHECKED);
        } else {
            cx.clear_state(State::CHECKED);
        }
        util::log_value(CHECKBOX_CLASS.name, i32::from(on));
        cx.invalidate_for("checkbox.checked");
    }

    fn text_changed(cx: &mut WidgetCx<'_>) {
        cx.invalidate_for("checkbox.text");
        cx.mark_layout();
    }

    /// The text's size (one layout without width limit, LVGL `lv_text_get_size`).
    fn text_size(&self, m: &MeasureCx<'_>) -> Size {
        let mut l = TextLayout::new(self.text(), m.font(Part::Main));
        l.letter_space = m.style_i32(Part::Main, PropId::TextLetterSpace);
        l.line_space = m.style_i32(Part::Main, PropId::TextLineSpace);
        l.max_width = COORD_MAX;
        l.measure()
    }

    /// The box's area without its transform size (LVGL `lv_checkbox_draw`).
    #[must_use]
    pub fn marker_area(&self, cx: &MeasureCx<'_>) -> Rect {
        Self::marker(cx).to_rect()
    }

    fn marker(m: &MeasureCx<'_>) -> Area {
        let font_h = i32::from(m.font(Part::Main).line_height);
        let c = Area::from_rect(m.coords());
        let border = m.style_i32(Part::Main, PropId::BorderWidth);
        let bg = m.padding(Part::Main);
        let mp = m.padding(Part::Indicator);
        let top = bg.top + border;
        let mut a = Area::default();
        if util::is_rtl(m) {
            // LVGL uses the right padding without the border here.
            a.x2 = c.x2 - bg.right;
            a.x1 = a.x2 - font_h - mp.left - mp.right + 1;
        } else {
            a.x1 = c.x1 + bg.left + border;
            a.x2 = a.x1 + font_h + mp.left + mp.right - 1;
        }
        a.y1 = c.y1 + top;
        a.y2 = a.y1 + font_h + mp.top + mp.bottom - 1;
        a
    }
}

/// Creates a checkbox showing "Check box" as the last child of `parent` (LVGL
/// `lv_checkbox_create`).
///
/// # Errors
/// [`EngineError::NodeNotFound`] if `parent` does not exist (logged).
pub fn create(engine: &mut Engine, parent: NodeId) -> Result<NodeId, EngineError> {
    engine.create(parent, Box::new(Checkbox::default()))
}

/// Creates a checkbox showing `text` (not copied).
///
/// # Errors
/// [`EngineError::NodeNotFound`] if `parent` does not exist (logged).
pub fn create_with(engine: &mut Engine, parent: NodeId, text: &'static str) -> Result<NodeId, EngineError> {
    engine.create(parent, Box::new(Checkbox::new(text)))
}

impl Widget for Checkbox {
    fn class(&self) -> &'static WidgetClass {
        &CHECKBOX_CLASS
    }

    fn init(&mut self, cx: &mut WidgetCx<'_>) {
        let ext = util::ext_u16(util::part_ext_draw(&cx.measure(), Part::Indicator));
        cx.refresh_ext_draw_with(ext);
    }

    /// LVGL `LV_EVENT_GET_SELF_SIZE`: box + `pad_column` + text, the taller of box and text.
    fn content_size(&self, cx: &MeasureCx<'_>) -> Size {
        let font_h = i32::from(cx.font(Part::Main).line_height);
        let txt = self.text_size(cx);
        let col = cx.style_i32(Part::Main, PropId::PadColumn);
        let mp = cx.padding(Part::Indicator);
        let mw = font_h + mp.left + mp.right;
        let mh = font_h + mp.top + mp.bottom;
        Size::new(mw + txt.w + col, mh.max(txt.h))
    }

    fn draw(&self, cx: &mut DrawCx<'_, '_>) {
        cx.draw_base(Part::Main);
        let m = MeasureCx::new(cx.engine(), cx.node());
        let marker = Self::marker(&m);
        let (tw, th) = util::transform_wh(&m, Part::Indicator);
        let mut grown = marker;
        grown.x1 -= tw;
        grown.x2 += tw;
        grown.y1 -= th;
        grown.y2 += th;
        cx.draw_rect_style(grown.to_rect(), Part::Indicator);
        let font_h = i32::from(m.font(Part::Main).line_height);
        let txt = self.text_size(&m);
        let col = m.style_i32(Part::Main, PropId::PadColumn);
        let y_ofs = (marker.height() - font_h) / 2;
        let top = m.padding(Part::Main).top + m.style_i32(Part::Main, PropId::BorderWidth);
        let y1 = m.coords().y0 + top + y_ofs;
        let area = if util::is_rtl(&m) {
            let x2 = marker.x1 - col;
            Rect::new(x2 - txt.w, y1, x2 + 1, y1 + txt.h + 1)
        } else {
            let x1 = marker.x2 + col;
            Rect::new(x1, y1, x1 + txt.w + 1, y1 + txt.h + 1)
        };
        let dsc = cx.text_dsc(Part::Main);
        cx.draw_text(area, self.text(), &dsc);
    }

    fn ext_draw_size(&self, cx: &MeasureCx<'_>) -> u16 {
        util::ext_u16(util::part_ext_draw(cx, Part::Indicator))
    }

    fn text(&self) -> Option<&str> {
        Some(Checkbox::text(self))
    }

    fn event(&mut self, cx: &mut EventCx<'_>, ev: &Event) -> EventResult {
        if ev.target != cx.node() {
            return EventResult::Continue;
        }
        match ev.code {
            // The engine toggled `CHECKED`: redraw even when no style depends on it.
            EventCode::ValueChanged => {
                let mut wcx = cx.widget_cx();
                util::log_value(
                    CHECKBOX_CLASS.name,
                    i32::from(wcx.state().contains(State::CHECKED)),
                );
                wcx.invalidate_for("checkbox.checked");
            }
            EventCode::StyleChanged => {
                let mut wcx = cx.widget_cx();
                let ext = util::ext_u16(util::part_ext_draw(&wcx.measure(), Part::Indicator));
                if wcx.refresh_ext_draw_with(ext) {
                    wcx.invalidate();
                }
            }
            _ => {}
        }
        EventResult::Continue
    }
}
