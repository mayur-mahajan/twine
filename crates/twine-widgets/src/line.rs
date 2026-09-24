//! [`Line`]: a polyline (LVGL `lv_line`).

use alloc::boxed::Box;
use alloc::vec::Vec;

use twine_core::{Point, Rect, Size};
use twine_engine::{
    DrawCx, Engine, EngineError, Event, EventCode, EventCx, EventResult, MeasureCx, NodeId, OBJ_FLAGS,
    ObjFlags, Widget, WidgetClass, WidgetCx,
};
use twine_style::{Part, PropId};

use crate::log_set;
use crate::util;

/// The class of [`Line`]: `"line"`, part `Main`, not clickable (LVGL `lv_line_constructor`:
/// `lv_obj_set_clickable(obj, false)`).
pub static LINE_CLASS: WidgetClass =
    WidgetClass::new("line").default_flags(OBJ_FLAGS.difference(ObjFlags::CLICKABLE));

/// Where a line's points live: in flash (no allocation) or in an owned buffer whose capacity
/// is reused by later point sets.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum LinePoints {
    /// A `'static` slice (LVGL `lv_line_set_points`).
    Static(&'static [Point]),
    /// An owned copy.
    Owned(Vec<Point>),
}

impl LinePoints {
    /// The points.
    #[must_use]
    pub fn as_slice(&self) -> &[Point] {
        match self {
            LinePoints::Static(p) => p,
            LinePoints::Owned(p) => p,
        }
    }
}

/// A polyline through points relative to the widget's top-left corner (LVGL `lv_line`),
/// drawn with the `Line*` styles of `Main` (`LineWidth`, `LineColor`, `LineOpa`,
/// `LineRounded`, `LineDashWidth`, `LineDashGap`). Each segment is drawn like LVGL: rounded
/// ends (when `LineRounded`) at the first point and at every segment's end.
///
/// The size is `Content` by default: the largest x and y of the points (LVGL
/// `GET_SELF_SIZE`); the stroke may reach outside by the line width (extra draw size). With
/// [`set_y_invert`](Self::set_y_invert) y grows upwards from the bottom edge.
///
/// Setting points equal to the current ones does nothing; other points invalidate the union
/// of the old and the new bounding box (plus the line width) and re-measure the widget only
/// when its content size changes.
///
/// ```
/// use twine_core::Point;
/// use twine_testing::EngineHarness;
/// use twine_widgets::line::{self, Line};
///
/// static ZIGZAG: [Point; 3] = [Point::new(0, 0), Point::new(20, 30), Point::new(40, 0)];
/// let mut h = EngineHarness::new(60, 40);
/// let screen = h.screen();
/// let l = line::create(h.engine_mut(), screen).unwrap();
/// h.engine_mut().with_widget_mut(l, |w: &mut Line, cx| w.set_points_static(cx, &ZIGZAG));
/// h.run_until_idle();
/// assert_eq!(h.engine().coords(l).size(), twine_core::Size::new(40, 30));
/// ```
#[derive(Debug, Default)]
pub struct Line {
    points: Option<LinePoints>,
    /// A released owned buffer, reused by the next [`set_points`](Self::set_points).
    spare: Vec<Point>,
    y_invert: bool,
}

/// The bounding box of `pts` (relative), `None` when empty.
fn bbox(pts: &[Point]) -> Option<Rect> {
    let first = pts.first()?;
    let mut r = Rect::new(first.x, first.y, first.x + 1, first.y + 1);
    for p in &pts[1..] {
        r = r.union(&Rect::new(p.x, p.y, p.x + 1, p.y + 1));
    }
    Some(r)
}

impl Line {
    /// A line without points.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            points: None,
            spare: Vec::new(),
            y_invert: false,
        }
    }

    /// The points.
    #[must_use]
    pub fn points(&self) -> &[Point] {
        self.points.as_ref().map_or(&[], LinePoints::as_slice)
    }

    /// Where the points are stored, if any are set.
    #[must_use]
    pub fn points_storage(&self) -> Option<&LinePoints> {
        self.points.as_ref()
    }

    /// Whether y grows upwards.
    #[must_use]
    pub fn y_invert(&self) -> bool {
        self.y_invert
    }

    /// Sets the points, copying them into the line's own buffer (no allocation when it is
    /// large enough). Idempotent (slice equality).
    pub fn set_points(&mut self, cx: &mut WidgetCx<'_>, pts: &[Point]) {
        if self.points.is_some() && self.points() == pts {
            return;
        }
        log_set(LINE_CLASS.name, cx.node(), "points");
        let before = self.snapshot(cx);
        let mut buf = match self.points.take() {
            Some(LinePoints::Owned(v)) => v,
            _ => core::mem::take(&mut self.spare),
        };
        buf.clear();
        buf.extend_from_slice(pts);
        self.points = Some(LinePoints::Owned(buf));
        self.changed(cx, before);
    }

    /// Sets `'static` points without copying (LVGL `lv_line_set_points`). Idempotent.
    pub fn set_points_static(&mut self, cx: &mut WidgetCx<'_>, pts: &'static [Point]) {
        if let Some(LinePoints::Static(cur)) = self.points {
            if core::ptr::eq(cur, pts) {
                return;
            }
        }
        let same = self.points.is_some() && self.points() == pts;
        let before = self.snapshot(cx);
        if let Some(LinePoints::Owned(v)) = self.points.replace(LinePoints::Static(pts)) {
            self.spare = v;
        }
        if same {
            return;
        }
        log_set(LINE_CLASS.name, cx.node(), "points_static");
        self.changed(cx, before);
    }

    /// Makes y grow upwards from the bottom edge (LVGL `lv_line_set_y_invert`). Idempotent.
    pub fn set_y_invert(&mut self, cx: &mut WidgetCx<'_>, on: bool) {
        if self.y_invert == on {
            return;
        }
        log_set(LINE_CLASS.name, cx.node(), "y_invert");
        self.y_invert = on;
        cx.invalidate_for("line.y_invert");
    }

    /// The drawn bounding box (absolute, with the stroke) and the content size.
    fn snapshot(&self, cx: &WidgetCx<'_>) -> (Option<Rect>, Size) {
        let m = cx.measure();
        (self.abs_bbox(&m), self.content())
    }

    /// The bounding box of the points on screen, grown by the line width.
    fn abs_bbox(&self, m: &MeasureCx<'_>) -> Option<Rect> {
        let c = m.coords();
        let b = bbox(self.points())?;
        let b = if self.y_invert {
            let h = c.height();
            Rect::new(b.x0, h - (b.y1 - 1), b.x1, h - b.y0 + 1)
        } else {
            b
        };
        let w = m.style_i32(Part::Main, PropId::LineWidth).max(1);
        Some(b.translate(c.x0, c.y0).expand(w / 2 + 1))
    }

    fn content(&self) -> Size {
        let pts = self.points();
        if pts.is_empty() {
            return Size::ZERO;
        }
        let w = pts.iter().map(|p| p.x).fold(0, i32::max);
        let h = pts.iter().map(|p| p.y).fold(0, i32::max);
        Size::new(w, h)
    }

    /// Invalidates the old and new bounding boxes; marks the layout when the content size
    /// changed.
    fn changed(&self, cx: &mut WidgetCx<'_>, before: (Option<Rect>, Size)) {
        let after = self.snapshot(cx);
        if cx.coords().is_empty() {
            cx.invalidate_for("line.points");
        } else {
            for r in [before.0, after.0].into_iter().flatten() {
                cx.invalidate_area(r);
            }
        }
        if before.1 != after.1 {
            cx.mark_layout();
        }
    }

    fn ext_for(m: &MeasureCx<'_>) -> u16 {
        util::ext_u16(m.style_i32(Part::Main, PropId::LineWidth))
    }
}

/// Creates a line without points as the last child of `parent` (LVGL `lv_line_create`).
///
/// # Errors
/// [`EngineError::NodeNotFound`] if `parent` does not exist (logged).
pub fn create(engine: &mut Engine, parent: NodeId) -> Result<NodeId, EngineError> {
    engine.create(parent, Box::new(Line::new()))
}

impl Widget for Line {
    fn class(&self) -> &'static WidgetClass {
        &LINE_CLASS
    }

    fn init(&mut self, cx: &mut WidgetCx<'_>) {
        let ext = Self::ext_for(&cx.measure());
        cx.refresh_ext_draw_with(ext);
    }

    /// LVGL `LV_EVENT_GET_SELF_SIZE`: the largest x and y of the points.
    fn content_size(&self, _cx: &MeasureCx<'_>) -> Size {
        self.content()
    }

    /// LVGL `LV_EVENT_REFR_EXT_DRAW_SIZE`: the line width (the corners of skewed lines).
    fn ext_draw_size(&self, cx: &MeasureCx<'_>) -> u16 {
        Self::ext_for(cx)
    }

    fn draw(&self, cx: &mut DrawCx<'_, '_>) {
        cx.draw_base(Part::Main);
        let pts = self.points();
        if pts.len() < 2 {
            return;
        }
        let c = cx.coords();
        let h = c.height();
        let mut dsc = cx.line_dsc(Part::Main);
        let map = |p: Point| {
            let y = if self.y_invert { h - p.y } else { p.y };
            Point::new(c.x0 + p.x, c.y0 + y)
        };
        for seg in pts.windows(2) {
            cx.painter().line(map(seg[0]), map(seg[1]), &dsc);
            // Only the first segment gets a rounded start (LVGL).
            dsc.round_start = false;
        }
    }

    fn event(&mut self, cx: &mut EventCx<'_>, ev: &Event) -> EventResult {
        if ev.target == cx.node() && ev.code == EventCode::StyleChanged {
            let mut wcx = cx.widget_cx();
            let ext = Self::ext_for(&wcx.measure());
            if wcx.refresh_ext_draw_with(ext) {
                wcx.invalidate();
            }
        }
        EventResult::Continue
    }
}
