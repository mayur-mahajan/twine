//! [`Dropdown`]: a button showing the selected option that opens a list of options (LVGL
//! `lv_dropdown`), and its [`DropdownList`] popup on the top layer.

use alloc::boxed::Box;
use alloc::string::String;

use twine_anim::Anim;
use twine_core::{Angle, Duration, Point, Rect, Size};
use twine_engine::{
    AnimProp, DrawCx, Editable, Engine, EngineError, Event, EventCode, EventCx, EventParam, EventResult,
    GroupDef, InputKind, Key, MeasureCx, NodeId, OBJ_FLAGS, ObjFlags, State, Widget, WidgetClass, WidgetCx,
    fmt_node_id,
};
use twine_image::{ImageSource, with_pixels};
use twine_style::{COORD_MAX, Length, Part, PropId, Selector, Side, StyleProp, TextAlign};
use twine_text::{Symbol, TextDrawFlags, TextLayout};
use twine_widgets::label::LabelText;

use crate::options::{self, Options};
use crate::util::{self, log_set};

/// The class of [`Dropdown`]: `"dropdown"`, parts `Main`, `Scrollbar` and `Indicator` (the
/// symbol), the base object's flags, always in the default focus group, editable (LVGL
/// `lv_dropdown_class`).
pub static DROPDOWN_CLASS: WidgetClass = WidgetClass::new("dropdown")
    .parts(&[Part::Main, Part::Scrollbar, Part::Indicator])
    .default_flags(OBJ_FLAGS)
    .group_def(GroupDef::True)
    .editable(Editable::True);

/// The class of [`DropdownList`]: `"dropdown_list"`, parts `Main`, `Scrollbar` and `Selected`
/// (the selected and the pressed option), not click-focusable, no scroll on focus (LVGL
/// `lv_dropdownlist_class` constructor). `Selected` is drawn per option in its own state.
pub static DROPDOWN_LIST_CLASS: WidgetClass = WidgetClass::new("dropdown_list")
    .parts(&[Part::Main, Part::Scrollbar, Part::Selected])
    .default_flags(
        OBJ_FLAGS
            .difference(ObjFlags::CLICK_FOCUSABLE)
            .difference(ObjFlags::SCROLL_ON_FOCUS),
    )
    .group_def(GroupDef::False)
    .item_parts(&[Part::Selected]);

/// LVGL `lv_dropdown_class.width_def`: `LV_DPI_DEF`.
pub const DROPDOWN_DEFAULT_WIDTH: i32 = util::DPI_DEF;

/// The options of a new dropdown (LVGL with `LV_WIDGETS_HAS_DEFAULT_VALUE`).
pub const DROPDOWN_DEFAULT_OPTIONS: &str = "Option 1\nOption 2\nOption 3";

/// The default symbol: `SYMBOL_DOWN` (drawn as `SYMBOL_UP` when the list opens upwards).
pub const DROPDOWN_DEFAULT_SYMBOL: ImageSource = ImageSource::symbol(Symbol::Down);

/// A dropdown (LVGL `lv_dropdown`): shows the selected option (or a fixed
/// [`text`](Self::set_text)) and a symbol; a click opens the options as a
/// [`DropdownList`] on the display's top layer, below the dropdown (or above, left, right:
/// [`set_dir`](Self::set_dir); it flips to the other side when there is more room there).
///
/// - **Pointer**: a click toggles the list; clicking an option selects it, sends
///   `ValueChanged` (with the index as [`EventParam::Value`]) and closes the list; pressing
///   anywhere else closes it unchanged ([`Engine::on_outside_press`]).
/// - **Keypad**: `Up`/`Down` (`Left`/`Right`) open the list, then move the highlighted option;
///   `Enter` confirms, `Esc` closes and restores the selection.
/// - **Encoder**: a click opens the list in edit mode, turning moves, a click confirms.
/// - The options are one `'\n'`-separated string ([`Options`]): static options need no
///   allocation. The list node exists only while open (created by [`open`](Self::open),
///   deleted by [`close`](Self::close)); opening and closing leaves no garbage.
/// - Opening sends `Ready`, closing `Cancel` (LVGL, to style the list).
/// - With a `Main` `anim_duration` on the list (0 in the themes, as LVGL has no animation)
///   the list fades in and out over that time.
///
/// ```
/// use twine_testing::EngineHarness;
/// use twine_widgets_ext::dropdown::{self, Dropdown};
///
/// let mut h = EngineHarness::new(240, 200);
/// let screen = h.screen();
/// let d = dropdown::create(h.engine_mut(), screen).unwrap();
/// h.engine_mut().with_widget_mut(d, |w: &mut Dropdown, cx| {
///     w.set_options(cx, "Red\nGreen\nBlue");
///     w.set_selected(cx, 2);
///     w.open(cx);
/// });
/// let w = h.engine().widget::<Dropdown>(d).unwrap();
/// assert!(w.is_open(h.engine()));
/// let mut s = String::new();
/// w.selected_str(&mut s);
/// assert_eq!(s, "Blue");
/// ```
#[derive(Debug)]
pub struct Dropdown {
    options: Options,
    option_cnt: u16,
    /// The highlighted option (moves with the keys while the list is open).
    sel: u16,
    /// The confirmed selection (shown on the button).
    sel_orig: u16,
    /// The pressed option in the list.
    pr_opt: Option<u16>,
    dir: Side,
    symbol: Option<ImageSource>,
    /// The size of an image symbol (read from its header when set).
    symbol_size: Size,
    text: Option<LabelText>,
    selected_highlight: bool,
    list: Option<NodeId>,
    /// A list fading out after `close`.
    closing: Option<NodeId>,
}

impl Default for Dropdown {
    fn default() -> Self {
        Self::new()
    }
}

impl Dropdown {
    /// A dropdown with LVGL's defaults: three options ("Option 1" … "Option 3"), the first
    /// selected, opening downwards, `SYMBOL_DOWN`, selected highlight on.
    #[must_use]
    pub fn new() -> Self {
        Self {
            options: Options::Static(DROPDOWN_DEFAULT_OPTIONS),
            option_cnt: options::count(DROPDOWN_DEFAULT_OPTIONS),
            sel: 0,
            sel_orig: 0,
            pr_opt: None,
            dir: Side::Bottom,
            symbol: Some(DROPDOWN_DEFAULT_SYMBOL),
            symbol_size: Size::ZERO,
            text: None,
            selected_highlight: true,
            list: None,
            closing: None,
        }
    }

    /// A dropdown showing `options` (`'\n'`-separated, stored without copying).
    #[must_use]
    pub fn with_options(options: &'static str) -> Self {
        Self {
            options: Options::from_static(options),
            option_cnt: options::count(options),
            ..Self::new()
        }
    }

    // ---- Getters --------------------------------------------------------------------------

    /// The options as one `'\n'`-separated string.
    #[must_use]
    pub fn options(&self) -> &str {
        self.options.as_str()
    }

    /// The option storage.
    #[must_use]
    pub fn options_storage(&self) -> &Options {
        &self.options
    }

    /// The number of options.
    #[must_use]
    pub fn option_count(&self) -> u16 {
        self.option_cnt
    }

    /// The index of the selected option (while the list is open with keys moving the
    /// highlight: the highlighted one, LVGL `lv_dropdown_get_selected`).
    #[must_use]
    pub fn selected(&self) -> u16 {
        self.sel
    }

    /// Writes the confirmed selected option into `buf` (cleared first; LVGL
    /// `lv_dropdown_get_selected_str`).
    pub fn selected_str(&self, buf: &mut String) {
        buf.clear();
        if let Some(o) = self.options.get(self.sel_orig) {
            buf.push_str(o);
        }
    }

    /// The index of the option equal to `option` (LVGL `lv_dropdown_get_option_index`).
    #[must_use]
    pub fn option_index(&self, option: &str) -> Option<u16> {
        self.options.index_of(option)
    }

    /// The direction the list opens to.
    #[must_use]
    pub fn dir(&self) -> Side {
        self.dir
    }

    /// The symbol (`None`: no symbol).
    #[must_use]
    pub fn symbol(&self) -> Option<&ImageSource> {
        self.symbol.as_ref()
    }

    /// The fixed text shown instead of the selected option.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        self.text.as_ref().map(LabelText::as_str)
    }

    /// Whether the selected option is highlighted in the list.
    #[must_use]
    pub fn selected_highlight(&self) -> bool {
        self.selected_highlight
    }

    /// The list node while open.
    #[must_use]
    pub fn list(&self) -> Option<NodeId> {
        self.list
    }

    /// Whether the list is open.
    #[must_use]
    pub fn is_open(&self, e: &Engine) -> bool {
        self.list.is_some_and(|l| e.tree().contains(l))
    }

    /// The option pressed in the list, if any.
    #[must_use]
    pub fn pressed_option(&self) -> Option<u16> {
        self.pr_opt
    }

    // ---- Setters --------------------------------------------------------------------------

    /// Sets the options (`'\n'`-separated), copied into the owned buffer (its capacity is
    /// reused); the selection goes to the first option (LVGL `lv_dropdown_set_options`).
    /// Idempotent: the same options change nothing (and keep the selection).
    pub fn set_options(&mut self, cx: &mut WidgetCx<'_>, options: &str) {
        if self.options.same_as(options) {
            return;
        }
        log_set(DROPDOWN_CLASS.name, cx.node(), "options");
        self.options.set_owned(options);
        self.options_changed(cx, true);
    }

    /// Sets `'static` options, stored without copying (LVGL `lv_dropdown_set_options_static`).
    /// Idempotent.
    pub fn set_options_static(&mut self, cx: &mut WidgetCx<'_>, options: &'static str) {
        if self.options.is_static(options) {
            return;
        }
        log_set(DROPDOWN_CLASS.name, cx.node(), "options_static");
        self.options = Options::from_static(options);
        self.options_changed(cx, true);
    }

    /// Inserts `option` before option `pos` (`u32::MAX` or any index past the end: at the
    /// end; LVGL `lv_dropdown_add_option`). The selection index is kept.
    pub fn add_option(&mut self, cx: &mut WidgetCx<'_>, option: &str, pos: u32) {
        if option.contains('\n') {
            twine_core::warn!(target: "twine::engine", "dropdown: an option cannot contain '\\n'");
            return;
        }
        log_set(DROPDOWN_CLASS.name, cx.node(), "add_option");
        self.options.insert(option, pos);
        self.options_changed(cx, false);
    }

    /// Removes every option (LVGL `lv_dropdown_clear_options`). Idempotent.
    pub fn clear_options(&mut self, cx: &mut WidgetCx<'_>) {
        if self.options.is_empty() {
            return;
        }
        log_set(DROPDOWN_CLASS.name, cx.node(), "clear_options");
        self.options = Options::Static("");
        self.options_changed(cx, false);
    }

    fn options_changed(&mut self, cx: &mut WidgetCx<'_>, reset_sel: bool) {
        self.option_cnt = self.options.count();
        if reset_sel {
            self.sel = 0;
            self.sel_orig = 0;
        }
        Self::refresh_size(cx);
        self.refresh_list(cx.engine_mut());
    }

    /// Selects option `idx` (clamped to the last one; LVGL `lv_dropdown_set_selected`, no
    /// `ValueChanged`). Idempotent.
    pub fn set_selected(&mut self, cx: &mut WidgetCx<'_>, idx: u16) {
        if self.sel == idx && self.sel_orig == idx {
            return;
        }
        let idx = idx.min(self.option_cnt.saturating_sub(1));
        if self.sel == idx && self.sel_orig == idx {
            return;
        }
        log_set(DROPDOWN_CLASS.name, cx.node(), "selected");
        self.sel = idx;
        self.sel_orig = idx;
        if self.is_open(cx.engine()) {
            self.position_to_selected(cx.engine_mut(), false);
        }
        Self::refresh_size(cx);
    }

    /// The side the list opens to ([`Side::Bottom`] by default; LVGL `lv_dropdown_set_dir`).
    /// Idempotent.
    pub fn set_dir(&mut self, cx: &mut WidgetCx<'_>, dir: Side) {
        if self.dir == dir {
            return;
        }
        log_set(DROPDOWN_CLASS.name, cx.node(), "dir");
        self.dir = dir;
        cx.invalidate_for("dropdown dir");
    }

    /// The symbol drawn at the side (`None` hides it; LVGL `lv_dropdown_set_symbol`). A
    /// [`ImageSource::Symbol`] is text in the `Indicator` font; other sources are images
    /// (rotated by the `Indicator`'s `transform_rotation`). Idempotent.
    pub fn set_symbol(&mut self, cx: &mut WidgetCx<'_>, symbol: Option<ImageSource>) {
        if self.symbol == symbol {
            return;
        }
        log_set(DROPDOWN_CLASS.name, cx.node(), "symbol");
        self.symbol_size = match &symbol {
            Some(src @ (ImageSource::Static(_) | ImageSource::Encoded(_) | ImageSource::File(_))) => cx
                .engine_mut()
                .image_header(src)
                .map_or(Size::ZERO, |h| Size::new(i32::from(h.w), i32::from(h.h))),
            _ => Size::ZERO,
        };
        self.symbol = symbol;
        Self::refresh_size(cx);
    }

    /// A fixed text shown instead of the selected option (copied; `None`: the selected
    /// option; LVGL `lv_dropdown_set_text`). Idempotent.
    pub fn set_text(&mut self, cx: &mut WidgetCx<'_>, text: Option<&str>) {
        if self.text() == text {
            return;
        }
        log_set(DROPDOWN_CLASS.name, cx.node(), "text");
        self.text = match (text, self.text.take()) {
            (None, _) => None,
            (Some(t), Some(LabelText::Owned(mut s))) => {
                s.clear();
                s.push_str(t);
                Some(LabelText::Owned(s))
            }
            (Some(t), _) => Some(LabelText::Owned(String::from(t))),
        };
        Self::refresh_size(cx);
    }

    /// A fixed `'static` text (stored without copying; LVGL `lv_dropdown_set_text_static`).
    /// Idempotent.
    pub fn set_text_static(&mut self, cx: &mut WidgetCx<'_>, text: Option<&'static str>) {
        if matches!((&self.text, text), (Some(LabelText::Static(a)), Some(b)) if core::ptr::eq(*a, b))
            || (self.text.is_none() && text.is_none())
        {
            return;
        }
        log_set(DROPDOWN_CLASS.name, cx.node(), "text_static");
        self.text = text.map(LabelText::Static);
        Self::refresh_size(cx);
    }

    /// Highlights the selected option in the list (LVGL
    /// `lv_dropdown_set_selected_highlight`). Idempotent.
    pub fn set_selected_highlight(&mut self, cx: &mut WidgetCx<'_>, on: bool) {
        if self.selected_highlight == on {
            return;
        }
        log_set(DROPDOWN_CLASS.name, cx.node(), "selected_highlight");
        self.selected_highlight = on;
        if let Some(l) = self.live_list(cx.engine()) {
            cx.engine_mut().invalidate(
                l,
                twine_engine::InvalidateReason::WidgetSetter("dropdown highlight"),
            );
        }
    }

    /// LVGL `refresh_size`: redraw, and relayout a content-sized dropdown.
    fn refresh_size(cx: &mut WidgetCx<'_>) {
        cx.invalidate_for("dropdown");
        let content_sized = [PropId::Width, PropId::Height]
            .iter()
            .any(|&p| cx.style(Part::Main, p).as_length() == Some(Length::Content));
        if content_sized {
            cx.mark_layout();
        }
    }

    fn live_list(&self, e: &Engine) -> Option<NodeId> {
        self.list.filter(|l| e.tree().contains(*l))
    }

    /// After the options changed while open: the list's content size and a redraw.
    fn refresh_list(&self, e: &mut Engine) {
        let Some(l) = self.live_list(e) else { return };
        let content = self.list_text_size(e, l);
        e.with_widget_mut(l, |w: &mut DropdownList, cx| {
            if w.content != content {
                w.content = content;
                cx.mark_layout();
            }
            cx.invalidate_for("dropdown options");
        });
    }

    /// The size of the options laid out with the list's `Main` text style (LVGL: the size of
    /// the list's content-sized label).
    fn list_text_size(&self, e: &Engine, list: NodeId) -> Size {
        let t = MeasureCx::new(e, list).text_dsc(Part::Main);
        let mut l = TextLayout::new(self.options.as_str(), t.font);
        l.letter_space = t.letter_space;
        l.line_space = t.line_space;
        l.max_width = COORD_MAX;
        l.measure()
    }

    // ---- Open / close ---------------------------------------------------------------------

    /// Opens the list (LVGL `lv_dropdown_open`): creates it on the top layer of the
    /// dropdown's display, sized to the options (at most the space to the screen edge and
    /// the list's `max_height`), placed on the side of [`dir`](Self::dir) (the opposite side
    /// when it has more room), scrolled to the selected option. Adds `CHECKED` to the
    /// dropdown and sends `Ready`. Opening an open list does nothing.
    pub fn open(&mut self, cx: &mut WidgetCx<'_>) {
        if self.is_open(cx.engine()) {
            return;
        }
        let dd = cx.node();
        let e = cx.engine_mut();
        if let Some(old) = self.closing.take().filter(|c| e.tree().contains(*c)) {
            let _ = e.delete(old);
        }
        let Some(layer) = e.display_of(dd).and_then(|d| e.top_layer(d)) else {
            twine_core::warn!(target: "twine::engine", "dropdown {}: not on a display, cannot open", fmt_node_id(dd));
            return;
        };
        let Ok(list) = e.create(
            layer,
            Box::new(DropdownList {
                owner: dd,
                content: Size::ZERO,
            }),
        ) else {
            return;
        };
        self.list = Some(list);
        // The list lives on the top layer but takes the text style (font, color, base
        // direction) of the dropdown and its ancestors.
        e.set_style_parent(list, Some(dd));
        twine_core::debug!(target: "twine::engine", "dropdown {} open: list {}", fmt_node_id(dd), fmt_node_id(list));
        e.add_state(dd, State::CHECKED);
        e.post_event(dd, EventCode::Ready, EventParam::None);
        self.layout_list(e, dd, list, layer);
        watch_outside(e, dd, list);
        let dur = e.style_i32(list, Part::Main, PropId::AnimDuration);
        if dur > 0 {
            e.set_local_prop(
                list,
                Selector::MAIN,
                StyleProp::PartOpacity(twine_core::Opa::TRANSP.into()),
            );
            e.anim_start(
                list,
                AnimProp::Opa,
                Anim::new(0, 255).duration(Duration::ms(u64::from(dur.unsigned_abs()))),
            );
        }
    }

    /// Sizes and places the list (the geometry of LVGL `lv_dropdown_open`).
    fn layout_list(&mut self, e: &mut Engine, dd: NodeId, list: NodeId, layer: NodeId) {
        let text = self.list_text_size(e, list);
        e.with_widget_mut(list, |w: &mut DropdownList, _| w.content = text);
        e.update_layout();
        let c = e.coords(dd);
        let m = MeasureCx::new(e, list);
        let border = m.style_i32(Part::Main, PropId::BorderWidth).max(0);
        let pad = m.padding(Part::Main);
        let vertical = self.dir == Side::Bottom || self.dir == Side::Top;
        let content_w = text.w + pad.left + pad.right + 2 * border;
        let w = if content_w <= c.width() && vertical {
            c.width()
        } else {
            content_w
        };
        let top = pad.top + border;
        let bottom = pad.bottom + border;
        let list_fit_h = text.h + top + bottom;
        let mut list_h = list_fit_h;
        let (_, ver_res) = util::display_res(e, dd);
        // LVGL's inclusive y2.
        let y2 = c.y1 - 1;
        let mut dir = self.dir;
        if self.dir == Side::Bottom {
            if y2 + list_h > ver_res {
                if c.y0 > ver_res - y2 {
                    // More space above: drop up.
                    dir = Side::Top;
                    list_h = c.y0 - 1;
                } else {
                    list_h = ver_res - y2 - 1;
                }
            }
        } else if self.dir == Side::Top && c.y0 - list_h < 0 {
            if c.y0 < ver_res - y2 {
                dir = Side::Bottom;
                list_h = ver_res - y2;
            } else {
                list_h = c.y0;
            }
        }
        list_h = list_h.min(list_fit_h);
        e.set_size(list, w, list_h);
        // The real size: `max_height` may limit it.
        e.update_layout();
        self.position_to_selected(e, false);
        let lc = e.coords(list);
        let (lw, lh) = (lc.width(), lc.height());
        let origin = e.content_area(layer);
        let rtl = util::is_rtl(&MeasureCx::new(e, list));
        let (x, y) = match dir {
            Side::Top | Side::Bottom => {
                let x = if rtl { c.x1 - lw } else { c.x0 };
                let y = if dir == Side::Bottom { c.y1 } else { c.y0 - lh };
                (x, y)
            }
            Side::Left => (c.x0 - lw, c.y0),
            Side::Right => (c.x1, c.y0),
        };
        let mut y = y;
        if (dir == Side::Left || dir == Side::Right) && y + lh > ver_res {
            y -= (y + lh - 1 - ver_res) + 1;
        }
        e.set_pos(list, x - origin.x0, y - origin.y0);
        e.update_layout();
        twine_core::debug!(
            target: "twine::engine",
            "dropdown {} list at ({}, {}) {}x{} dir {:?}",
            fmt_node_id(dd),
            x,
            y,
            lw,
            lh,
            dir
        );
    }

    /// Closes the list (LVGL `lv_dropdown_close`): deletes it (after a fade-out when the list
    /// has an `anim_duration`), removes `CHECKED` from the dropdown and sends `Cancel`. The
    /// selection is left as it is.
    pub fn close(&mut self, cx: &mut WidgetCx<'_>) {
        let dd = cx.node();
        self.pr_opt = None;
        let e = cx.engine_mut();
        e.clear_state(dd, State::CHECKED);
        let Some(list) = self.list.take().filter(|l| e.tree().contains(*l)) else {
            return;
        };
        twine_core::debug!(target: "twine::engine", "dropdown {} close", fmt_node_id(dd));
        let dur = e.style_i32(list, Part::Main, PropId::AnimDuration);
        if dur > 0 {
            e.set_flag(list, ObjFlags::CLICKABLE, false);
            let from = e.style_opa(list, Part::Main, PropId::PartOpacity).raw();
            self.closing = Some(list);
            let a = Anim::new(i32::from(from), 0)
                .duration(Duration::ms(u64::from(dur.unsigned_abs())))
                .on_complete(move |acx| {
                    twine_engine::defer(acx, move |e| {
                        if e.tree().contains(list) {
                            let _ = e.delete(list);
                        }
                    });
                });
            e.anim_start(list, AnimProp::Opa, a);
        } else {
            let _ = e.delete(list);
        }
        e.post_event(dd, EventCode::Cancel, EventParam::None);
    }

    /// LVGL `position_to_selected`: scrolls the list so that the selected option is at the
    /// top (bounded by the content).
    fn position_to_selected(&self, e: &mut Engine, anim: bool) {
        let Some(list) = self.live_list(e) else { return };
        let t = MeasureCx::new(e, list).text_dsc(Part::Main);
        let unit_h = i32::from(t.font.line_height) + t.line_space;
        e.scroll_to_y(list, i32::from(self.sel) * unit_h, anim);
        e.invalidate(
            list,
            twine_engine::InvalidateReason::WidgetSetter("dropdown selected"),
        );
    }

    /// The option at absolute `y` in the list (LVGL `get_id_on_point`).
    fn id_on_point(&self, e: &Engine, list: NodeId, y: i32) -> u16 {
        let t = MeasureCx::new(e, list).text_dsc(Part::Main);
        let label_y = e.content_area(list).y0 - e.scroll_offset(list).y;
        let h = (i32::from(t.font.line_height) + t.line_space).max(1);
        let y = y - label_y + t.line_space / 2;
        let opt = u16::try_from(y.max(0) / h).unwrap_or(u16::MAX);
        opt.min(self.option_cnt.saturating_sub(1))
    }

    /// Moves the highlight by `delta` options while open, clamped (keys, rotary).
    fn move_sel(&mut self, cx: &mut WidgetCx<'_>, delta: i32) {
        let last = i32::from(self.option_cnt.saturating_sub(1));
        let new = (i32::from(self.sel) + delta).clamp(0, last);
        if new != i32::from(self.sel) {
            self.sel = new as u16;
            self.position_to_selected(cx.engine_mut(), true);
        }
    }

    /// LVGL `btn_release_handler`: toggles the list; closing with a moved highlight confirms
    /// it (`ValueChanged`).
    fn btn_release(&mut self, cx: &mut WidgetCx<'_>) {
        let e = cx.engine();
        if util::press_scrolled(e) {
            self.sel = self.sel_orig;
            cx.invalidate();
            return;
        }
        if !self.is_open(e) {
            self.open(cx);
            return;
        }
        let encoder = util::active_input_kind(e) == Some(InputKind::Encoder);
        self.close(cx);
        if self.sel_orig != self.sel {
            self.sel_orig = self.sel;
            util::log_value(DROPDOWN_CLASS.name, cx.node(), i32::from(self.sel));
            cx.post_event(EventCode::ValueChanged, EventParam::Value(i32::from(self.sel)));
            Self::refresh_size(cx);
        }
        if encoder {
            let dd = cx.node();
            if let Some(g) = cx.engine().group_of(dd) {
                cx.engine_mut().set_editing(g, false);
            }
        }
    }

    /// LVGL `list_press_handler`.
    fn list_pressed(&mut self, e: &mut Engine, list: NodeId, p: Option<Point>) {
        if !matches!(
            util::active_input_kind(e),
            Some(InputKind::Pointer | InputKind::Button)
        ) {
            return;
        }
        let Some(p) = p else {
            return;
        };
        self.pr_opt = Some(self.id_on_point(e, list, p.y));
        e.invalidate(
            list,
            twine_engine::InvalidateReason::WidgetSetter("dropdown pressed"),
        );
    }

    /// LVGL `list_release_handler`: selects the released option, closes, `ValueChanged`.
    fn list_released(&mut self, cx: &mut WidgetCx<'_>, list: NodeId, p: Option<Point>) {
        let dd = cx.node();
        let kind = util::active_input_kind(cx.engine());
        if kind == Some(InputKind::Encoder) {
            self.sel_orig = self.sel;
            if let Some(g) = cx.engine().group_of(dd) {
                if cx.engine().group_editing(g) {
                    cx.engine_mut().set_editing(g, false);
                }
            }
        }
        if matches!(kind, Some(InputKind::Pointer | InputKind::Button)) {
            if let Some(p) = p {
                self.sel = self.id_on_point(cx.engine(), list, p.y);
                self.sel_orig = self.sel;
            }
        }
        self.close(cx);
        Self::refresh_size(cx);
        util::log_value(DROPDOWN_CLASS.name, dd, i32::from(self.sel));
        cx.post_event(EventCode::ValueChanged, EventParam::Value(i32::from(self.sel)));
    }

    /// The symbol drawn: the default `SYMBOL_DOWN` turns into `SYMBOL_UP` for `Side::Top`.
    fn shown_symbol(&self) -> Option<ImageSource> {
        match &self.symbol {
            Some(ImageSource::Symbol(s)) if *s == Symbol::Down.as_str() && self.dir == Side::Top => {
                Some(ImageSource::symbol(Symbol::Up))
            }
            s => s.clone(),
        }
    }

    /// The size of the symbol (text in the `Indicator` style, or the image header).
    fn symbol_dims(&self, m: &MeasureCx<'_>, sym: &ImageSource) -> Size {
        if let ImageSource::Symbol(t) = sym {
            let d = m.text_dsc(Part::Indicator);
            let mut l = TextLayout::new(t, d.font);
            l.letter_space = d.letter_space;
            l.line_space = d.line_space;
            l.measure()
        } else {
            self.symbol_size
        }
    }

    /// The text on the button: the fixed text, else the confirmed option.
    fn button_text(&self) -> &str {
        match &self.text {
            Some(t) => t.as_str(),
            None => self.options.get(self.sel_orig).unwrap_or(""),
        }
    }
}

/// Closes `dd`'s list when a press starts outside it; a press on the dropdown itself keeps
/// watching (its release toggles the list).
fn watch_outside(e: &mut Engine, dd: NodeId, list: NodeId) {
    e.on_outside_press(list, move |e, pressed| {
        let on_dd = pressed.is_some_and(|p| p == dd || e.tree().ancestors(p).any(|a| a == dd));
        if on_dd {
            if e.tree().contains(list) {
                watch_outside(e, dd, list);
            }
            return;
        }
        e.with_widget_mut(dd, |d: &mut Dropdown, cx| {
            if d.list == Some(list) {
                twine_core::debug!(target: "twine::engine", "dropdown {}: pressed outside", fmt_node_id(dd));
                d.sel = d.sel_orig;
                d.close(cx);
            }
        });
    });
}

/// Creates a dropdown, 130 px wide and content high, as the last child of `parent` (LVGL
/// `lv_dropdown_create`).
///
/// # Errors
/// [`EngineError::NodeNotFound`] if `parent` does not exist (logged).
pub fn create(engine: &mut Engine, parent: NodeId) -> Result<NodeId, EngineError> {
    engine.create(parent, Box::new(Dropdown::new()))
}

impl Widget for Dropdown {
    fn class(&self) -> &'static WidgetClass {
        &DROPDOWN_CLASS
    }

    fn init(&mut self, cx: &mut WidgetCx<'_>) {
        let id = cx.node();
        cx.engine_mut().set_width(id, Length::Px(DROPDOWN_DEFAULT_WIDTH));
        cx.engine_mut().set_height(id, Length::Content);
    }

    /// LVGL `LV_EVENT_GET_SELF_SIZE`: the symbol, the column gap and the text.
    fn content_size(&self, cx: &MeasureCx<'_>) -> Size {
        let mut size = Size::ZERO;
        if let Some(sym) = self.shown_symbol() {
            size.w += self.symbol_dims(cx, &sym).w + cx.style_i32(Part::Main, PropId::ColumnGap);
        }
        let d = cx.text_dsc(Part::Main);
        let mut l = TextLayout::new(self.button_text(), d.font);
        l.letter_space = d.letter_space;
        l.line_space = d.line_space;
        let t = l.measure();
        size.w += t.w;
        size.h = size.h.max(t.h);
        size
    }

    /// LVGL `draw_main`: the symbol at the side, the text in the rest.
    fn draw(&self, cx: &mut DrawCx<'_, '_>) {
        cx.draw_base(Part::Main);
        let c = cx.coords();
        let m = MeasureCx::new(cx.engine(), cx.node());
        let border = m.style_i32(Part::Main, PropId::BorderWidth).max(0);
        let pad = m.padding(Part::Main);
        let (left, right) = (pad.left + border, pad.right + border);
        let symbol_to_left = self.dir == Side::Left || util::is_rtl(&m);
        let mut symbol_w = -1;
        let sym = self.shown_symbol();
        if let Some(sym) = &sym {
            let s = self.symbol_dims(&m, sym);
            symbol_w = s.w;
            let y = c.y0 + (c.height() - s.h) / 2;
            let x = if symbol_to_left {
                c.x0 + left
            } else {
                c.x1 - right - s.w
            };
            let area = Rect::from_xywh(x, y, s.w, s.h);
            if let ImageSource::Symbol(t) = sym {
                let d = cx.text_dsc(Part::Indicator);
                cx.draw_text(area, t, &d);
            } else if !area.is_empty() {
                let mut d = cx.image_dsc(Part::Indicator);
                d.angle = m
                    .style(Part::Indicator, PropId::TransformRotation)
                    .get::<Angle>()
                    .unwrap_or_default();
                d.pivot = Point::new(s.w / 2, s.h / 2);
                let _ = cx.with_images(|p, icx| with_pixels(sym, icx, |px| p.image(area, px, &d)));
            }
        }
        let mut d = cx.text_dsc(Part::Main);
        d.flags |= TextDrawFlags::EXPAND;
        let text = self.button_text();
        let mut l = TextLayout::new(text, d.font);
        l.letter_space = d.letter_space;
        l.line_space = d.line_space;
        let size = l.measure();
        let y0 = c.y0 + (c.height() - size.h) / 2;
        let mut area = Rect::new(c.x0 + left, y0, c.x1 - right, y0 + size.h);
        if sym.is_none() && d.align == TextAlign::Auto {
            d.align = TextAlign::Center;
        } else {
            let gap = symbol_w + m.style_i32(Part::Main, PropId::ColumnGap);
            if symbol_to_left {
                if d.align == TextAlign::Auto {
                    d.align = TextAlign::Right;
                }
                area.x0 += gap;
            } else {
                if d.align == TextAlign::Auto {
                    d.align = TextAlign::Left;
                }
                area.x1 -= gap;
            }
        }
        if !area.is_empty() {
            let clip = area.intersection(&cx.clip());
            if let Some(clip) = clip {
                let _ = cx.with_clip(clip, |cx| cx.draw_text(area, text, &d));
            }
        }
    }

    fn event(&mut self, cx: &mut EventCx<'_>, ev: &Event) -> EventResult {
        let node = cx.node();
        if ev.target != node {
            return EventResult::Continue;
        }
        match ev.code {
            EventCode::Focused => {
                if util::active_input_kind(cx.engine()) == Some(InputKind::Encoder) {
                    let editing = util::editing(cx.engine(), node);
                    let mut wcx = cx.widget_cx();
                    if editing {
                        self.open(&mut wcx);
                    } else {
                        self.sel = self.sel_orig;
                        self.close(&mut wcx);
                    }
                }
            }
            EventCode::Defocused | EventCode::Leave => {
                let mut wcx = cx.widget_cx();
                if self.is_open(wcx.engine()) {
                    self.close(&mut wcx);
                }
            }
            EventCode::Released => self.btn_release(&mut cx.widget_cx()),
            EventCode::Key => {
                let from_input = cx.input().is_some();
                let mut wcx = cx.widget_cx();
                let open = self.is_open(wcx.engine());
                match ev.key() {
                    Some(Key::Right | Key::Down) => {
                        if open {
                            self.move_sel(&mut wcx, 1);
                        } else {
                            self.open(&mut wcx);
                        }
                    }
                    Some(Key::Left | Key::Up) => {
                        if open {
                            self.move_sel(&mut wcx, -1);
                        } else {
                            self.open(&mut wcx);
                        }
                    }
                    Some(Key::Esc) => {
                        self.sel = self.sel_orig;
                        self.close(&mut wcx);
                    }
                    // An input device's Enter arrives as `Released` too (LVGL handles only
                    // an Enter sent by other code).
                    Some(Key::Enter) if !from_input => self.btn_release(&mut wcx),
                    _ => {}
                }
            }
            EventCode::Rotary => {
                if let EventParam::Rotary(r) = ev.param {
                    let mut wcx = cx.widget_cx();
                    if self.is_open(wcx.engine()) {
                        self.move_sel(&mut wcx, r);
                    } else {
                        self.open(&mut wcx);
                    }
                }
            }
            EventCode::StyleChanged => Self::refresh_size(&mut cx.widget_cx()),
            EventCode::Delete => {
                let e = cx.engine_mut();
                for l in [self.list.take(), self.closing.take()].into_iter().flatten() {
                    if e.tree().contains(l) {
                        let _ = e.delete(l);
                    }
                }
            }
            _ => {}
        }
        EventResult::Continue
    }
}

/// The option list of an open [`Dropdown`] (LVGL `lv_dropdownlist`): a scrollable node on the
/// top layer drawing the owner's options with the `Main` text style and the selected /
/// pressed option with the `Selected` styles in the `CHECKED` / `PRESSED` states.
#[derive(Debug)]
pub struct DropdownList {
    owner: NodeId,
    /// The size of the laid-out options (the list's scrollable content).
    content: Size,
}

impl DropdownList {
    /// The dropdown this list belongs to.
    #[must_use]
    pub fn owner(&self) -> NodeId {
        self.owner
    }

    /// The top of the first option (the content area's top minus the scroll offset).
    fn text_area(&self, e: &Engine, node: NodeId) -> Rect {
        let c = e.content_area(node);
        let s = e.scroll_offset(node);
        Rect::from_xywh(
            c.x0 - s.x,
            c.y0 - s.y,
            c.width().max(self.content.w),
            self.content.h,
        )
    }

    /// LVGL `draw_box` + `draw_box_label` for option `id` in `state`.
    fn draw_option(&self, cx: &mut DrawCx<'_, '_>, text: &str, id: u16, state: State, clip: Rect) {
        let node = cx.node();
        let e = cx.engine();
        let ta = self.text_area(e, node);
        let rect = cx.rect_dsc_for_state(Part::Selected, state);
        let mut t = cx.text_dsc_for_state(Part::Selected, state);
        let font_h = i32::from(t.font.line_height);
        let ls = t.line_space;
        let y1 = ta.y0 + i32::from(id) * (font_h + ls) - ls / 2;
        let c = cx.coords();
        let area = Rect::new(c.x0, y1, c.x1, y1 + font_h + ls);
        let Some(box_clip) = area.intersection(&clip) else {
            return;
        };
        let mut dsc = rect.dsc();
        dsc.border_post = false;
        let _ = cx.with_clip(clip, |cx| cx.painter().rect(area, &dsc));
        t.align = resolve_align(t.align);
        let _ = cx.with_clip(box_clip, |cx| cx.draw_text(ta, text, &t));
    }
}

/// `Auto` text alignment as the list draws it: left.
fn resolve_align(a: TextAlign) -> TextAlign {
    if a == TextAlign::Auto { TextAlign::Left } else { a }
}

impl Widget for DropdownList {
    fn class(&self) -> &'static WidgetClass {
        &DROPDOWN_LIST_CLASS
    }

    fn content_size(&self, _cx: &MeasureCx<'_>) -> Size {
        self.content
    }

    fn draw(&self, cx: &mut DrawCx<'_, '_>) {
        cx.draw_base(Part::Main);
        let e = cx.engine();
        let node = cx.node();
        let Some(dd) = e.widget::<Dropdown>(self.owner) else {
            return;
        };
        let text = dd.options.as_str();
        let ta = self.text_area(e, node);
        let mut t = cx.text_dsc(Part::Main);
        t.align = resolve_align(t.align);
        let Some(clip) = cx.coords().intersection(&cx.clip()) else {
            return;
        };
        let _ = cx.with_clip(clip, |cx| cx.draw_text(ta, text, &t));
        let (pr, sel) = (dd.pr_opt, dd.sel);
        if dd.selected_highlight {
            if pr == Some(sel) {
                self.draw_option(cx, text, sel, State::CHECKED | State::PRESSED, clip);
            } else {
                if let Some(p) = pr {
                    self.draw_option(cx, text, p, State::PRESSED, clip);
                }
                self.draw_option(cx, text, sel, State::CHECKED, clip);
            }
        } else if let Some(p) = pr {
            self.draw_option(cx, text, p, State::PRESSED, clip);
        }
    }

    fn event(&mut self, cx: &mut EventCx<'_>, ev: &Event) -> EventResult {
        let node = cx.node();
        if ev.target != node {
            return EventResult::Continue;
        }
        let owner = self.owner;
        let p = cx.point();
        let e = cx.engine_mut();
        match ev.code {
            EventCode::Pressed => {
                e.with_widget_mut(owner, |d: &mut Dropdown, cx| {
                    d.list_pressed(cx.engine_mut(), node, p);
                });
            }
            EventCode::Released if !util::press_scrolled(e) => {
                e.with_widget_mut(owner, |d: &mut Dropdown, cx| d.list_released(cx, node, p));
            }
            EventCode::ScrollBegin => {
                util::with_ready(e, owner, |d: &mut Dropdown, _| d.pr_opt = None);
                e.invalidate(
                    node,
                    twine_engine::InvalidateReason::WidgetSetter("dropdown scroll"),
                );
            }
            _ => {}
        }
        EventResult::Continue
    }
}
