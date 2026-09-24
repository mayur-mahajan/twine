//! The [`Widget`] trait, [`WidgetClass`], and the contexts widgets work with: [`MeasureCx`]
//! (read-only) and [`WidgetCx`] (mutable). Drawing uses [`DrawCx`](crate::DrawCx).

use core::any::Any;

use twine_core::{Color, Insets, Point, Rect, Size};
use twine_render::{ArcDsc, ImageDsc, LineDsc};
use twine_style::{Part, PropId, State, StyleValue};
use twine_text::{Font, TextDsc};

use crate::{
    DrawCx, Engine, Event, EventCx, EventResult, InvalidateReason, LayoutDirty, NodeId, ObjFlags, RectStyle,
};

/// Access to `self` as [`Any`], implemented for every `'static` type. It lets
/// `dyn Widget` be downcast ([`<dyn Widget>::downcast_ref`](trait.Widget.html#method.downcast_ref)).
pub trait AsAny: Any {
    /// `self` as `&dyn Any`.
    fn as_any(&self) -> &dyn Any;
    /// `self` as `&mut dyn Any`.
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl<T: Any> AsAny for T {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// A widget: the behaviour of one node of the tree (LVGL's object class).
///
/// The node itself (coordinates, flags, state, styles) lives in the [`Tree`](crate::Tree);
/// the widget holds only its own data and refers to other nodes by [`NodeId`]. Every method
/// has a default except [`class`](Self::class).
///
/// Animations of [`AnimProp::Value`](twine_anim::AnimProp::Value) and
/// [`AnimProp::Custom`](twine_anim::AnimProp::Custom) reach the widget through
/// [`anim_value`](Self::anim_value) and [`anim_custom`](Self::anim_custom).
///
/// ```
/// use twine_core::Color;
/// use twine_engine::{DrawCx, Widget, WidgetClass};
/// use twine_style::Part;
///
/// struct Swatch;
/// static SWATCH_CLASS: WidgetClass = WidgetClass::new("swatch");
///
/// impl Widget for Swatch {
///     fn class(&self) -> &'static WidgetClass {
///         &SWATCH_CLASS
///     }
///     fn draw(&self, cx: &mut DrawCx<'_, '_>) {
///         cx.draw_base(Part::Main);
///         let area = cx.content_area();
///         cx.painter().fill(area, Color::RED, twine_core::Opa::COVER);
///     }
/// }
/// let w: Box<dyn Widget> = Box::new(Swatch);
/// assert!(w.downcast_ref::<Swatch>().is_some());
/// ```
pub trait Widget: AsAny {
    /// The widget's class (name, parts, default flags…).
    fn class(&self) -> &'static WidgetClass;

    /// Called once right after the node was inserted into the tree (set default flags, local
    /// styles, create child nodes).
    fn init(&mut self, cx: &mut WidgetCx<'_>) {
        let _ = cx;
    }

    /// Size of the content for `Length::Content` sizing, excluding padding. Default: zero.
    fn content_size(&self, cx: &MeasureCx<'_>) -> Size {
        let _ = cx;
        Size::ZERO
    }

    /// Draws the node before its children. Default: [`DrawCx::draw_base`] of `Part::Main`.
    fn draw(&self, cx: &mut DrawCx<'_, '_>) {
        cx.draw_base(Part::Main);
    }

    /// Draws after the children (scrollbars are drawn by the engine afterwards).
    fn draw_post(&self, cx: &mut DrawCx<'_, '_>) {
        let _ = cx;
    }

    /// Extra area outside the node's coordinates the widget draws into, beyond what the
    /// engine derives from the styles (shadow, outline, transforms). Default 0.
    fn ext_draw_size(&self, cx: &MeasureCx<'_>) -> u16 {
        let _ = cx;
        0
    }

    /// Whether the absolute point `p` hits the node. Default: [`default_hit_test`].
    fn hit_test(&self, cx: &MeasureCx<'_>, p: Point) -> bool {
        default_hit_test(cx, p)
    }

    /// Whether the node fully covers `area` with opaque pixels, so nothing below needs to be
    /// drawn. Default: [`default_covers`].
    fn covers(&self, cx: &MeasureCx<'_>, area: Rect) -> bool {
        default_covers(cx, area)
    }

    /// Text content for queries and tree dumps. Default `None`.
    fn text(&self) -> Option<&str> {
        None
    }

    /// Reacts to an event sent to (or bubbling through) the node, after the built-in object
    /// behaviour (pressed / checked / focused states) and before the user handlers. Returning
    /// [`EventResult::Consumed`] skips the user handlers. Default: [`EventResult::Continue`].
    ///
    /// While this runs the widget is taken out of its node: an event sent to the same node
    /// from inside reaches only the built-in behaviour and the user handlers.
    fn event(&mut self, cx: &mut EventCx<'_>, ev: &Event) -> EventResult {
        let _ = (cx, ev);
        EventResult::Continue
    }

    /// Applies a value of an [`AnimProp::Value`](twine_anim::AnimProp::Value) animation
    /// started with [`Engine::anim_start`] (e.g. a bar's value). Default: ignored.
    ///
    /// Like [`event`](Self::event), the widget is taken out of its node while this runs.
    fn anim_value(&mut self, cx: &mut WidgetCx<'_>, v: i32) {
        let _ = (cx, v);
    }

    /// Applies a value of an [`AnimProp::Custom(id)`](twine_anim::AnimProp::Custom) animation
    /// started with [`Engine::anim_start`]. Default: ignored.
    fn anim_custom(&mut self, cx: &mut WidgetCx<'_>, id: u16, v: i32) {
        let _ = (cx, id, v);
    }
}

impl dyn Widget {
    /// The widget as `W`, if it is one.
    #[must_use]
    pub fn downcast_ref<W: Widget>(&self) -> Option<&W> {
        self.as_any().downcast_ref::<W>()
    }

    /// The widget as `&mut W`, if it is one.
    #[must_use]
    pub fn downcast_mut<W: Widget>(&mut self) -> Option<&mut W> {
        self.as_any_mut().downcast_mut::<W>()
    }

    /// Whether the widget is a `W`.
    #[must_use]
    pub fn is<W: Widget>(&self) -> bool {
        self.as_any().is::<W>()
    }
}

/// Whether a class is added to the default focus group automatically (LVGL
/// `lv_obj_class_group_def_t`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum GroupDef {
    /// Decided by the flags (`CLICK_FOCUSABLE`…).
    #[default]
    Default,
    /// Always added.
    True,
    /// Never added.
    False,
}

/// Whether a class has an encoder edit mode (LVGL `lv_obj_class_editable_t`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Editable {
    /// Like the parent class.
    #[default]
    Inherit,
    /// Editable (e.g. slider, roller).
    True,
    /// Not editable.
    False,
}

/// Static description of a widget type (LVGL `lv_obj_class_t`), usually a `static`.
///
/// ```
/// use twine_engine::{Editable, GroupDef, ObjFlags, WidgetClass};
/// use twine_style::Part;
/// static SLIDER: WidgetClass = WidgetClass::new("slider")
///     .parts(&[Part::Main, Part::Indicator, Part::Knob])
///     .default_flags(ObjFlags::CLICKABLE)
///     .group_def(GroupDef::True)
///     .editable(Editable::True)
///     .theme_inheritable(false);
/// assert_eq!(SLIDER.parts.len(), 3);
/// ```
#[derive(Clone, Copy, Debug)]
pub struct WidgetClass {
    /// Class name (`"obj"`, `"button"`…), shown in dumps and used by queries.
    pub name: &'static str,
    /// The parts the widget draws.
    pub parts: &'static [Part],
    /// Flags of a new node of this class.
    pub default_flags: ObjFlags,
    /// Default focus group membership.
    pub group_def: GroupDef,
    /// Encoder edit mode.
    pub editable: Editable,
    /// Whether themes style it like its base class.
    pub theme_inheritable: bool,
}

impl WidgetClass {
    /// A class with parts `[Main]`, no flags, `GroupDef::Default`, `Editable::Inherit`,
    /// theme-inheritable.
    #[must_use]
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            parts: &[Part::Main],
            default_flags: ObjFlags::empty(),
            group_def: GroupDef::Default,
            editable: Editable::Inherit,
            theme_inheritable: true,
        }
    }

    /// With these parts.
    #[must_use]
    pub const fn parts(mut self, parts: &'static [Part]) -> Self {
        self.parts = parts;
        self
    }

    /// With these default flags.
    #[must_use]
    pub const fn default_flags(mut self, flags: ObjFlags) -> Self {
        self.default_flags = flags;
        self
    }

    /// With this group default.
    #[must_use]
    pub const fn group_def(mut self, g: GroupDef) -> Self {
        self.group_def = g;
        self
    }

    /// With this edit mode.
    #[must_use]
    pub const fn editable(mut self, e: Editable) -> Self {
        self.editable = e;
        self
    }

    /// Whether themes style it like its base class.
    #[must_use]
    pub const fn theme_inheritable(mut self, on: bool) -> Self {
        self.theme_inheritable = on;
        self
    }
}

/// Read-only view of one node for measuring, hit testing and cover checks.
#[derive(Clone, Copy)]
pub struct MeasureCx<'a> {
    engine: &'a Engine,
    node: NodeId,
}

impl core::fmt::Debug for MeasureCx<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MeasureCx")
            .field("node", &self.node)
            .finish_non_exhaustive()
    }
}

impl<'a> MeasureCx<'a> {
    /// A view of `node`.
    #[must_use]
    pub fn new(engine: &'a Engine, node: NodeId) -> Self {
        Self { engine, node }
    }

    /// The node.
    #[must_use]
    pub fn node(&self) -> NodeId {
        self.node
    }

    /// The engine.
    #[must_use]
    pub fn engine(&self) -> &'a Engine {
        self.engine
    }

    /// Absolute coordinates of the node.
    #[must_use]
    pub fn coords(&self) -> Rect {
        self.engine.coords(self.node)
    }

    /// The coordinates minus padding and border width of `Part::Main`.
    #[must_use]
    pub fn content_area(&self) -> Rect {
        self.engine.content_area(self.node)
    }

    /// The node's state.
    #[must_use]
    pub fn state(&self) -> State {
        self.engine
            .tree()
            .node(self.node)
            .map_or(State::DEFAULT, crate::Node::state)
    }

    /// The node's flags.
    #[must_use]
    pub fn flags(&self) -> ObjFlags {
        self.engine
            .tree()
            .node(self.node)
            .map_or(ObjFlags::empty(), crate::Node::flags)
    }

    /// A resolved style property.
    #[must_use]
    pub fn style(&self, part: Part, prop: PropId) -> StyleValue {
        self.engine.style_prop(self.node, part, prop)
    }

    /// A resolved integer property (0 if it is not an integer).
    #[must_use]
    pub fn style_i32(&self, part: Part, prop: PropId) -> i32 {
        self.engine.style_i32(self.node, part, prop)
    }

    /// The resolved font of `part`.
    #[must_use]
    pub fn font(&self, part: Part) -> &'static Font {
        self.engine.style_font(self.node, part)
    }

    /// Padding of `part`.
    #[must_use]
    pub fn padding(&self, part: Part) -> Insets {
        Insets::new(
            self.style_i32(part, PropId::PadLeft),
            self.style_i32(part, PropId::PadTop),
            self.style_i32(part, PropId::PadRight),
            self.style_i32(part, PropId::PadBottom),
        )
    }

    /// A resolved color property (black if the property is not a color).
    #[must_use]
    pub fn style_color(&self, part: Part, prop: PropId) -> Color {
        self.engine.style_color(self.node, part, prop)
    }

    /// The rectangle style of `part` (with the node's effective opacity and recolor), as
    /// [`DrawCx::rect_dsc`] would build it.
    #[must_use]
    pub fn rect_dsc(&self, part: Part) -> RectStyle {
        self.engine
            .rect_dsc(self.node, part, self.engine.opa_recursive(self.node))
    }

    /// The text style of `part`, as [`DrawCx::text_dsc`] would build it.
    #[must_use]
    pub fn text_dsc(&self, part: Part) -> TextDsc {
        self.engine
            .text_dsc(self.node, part, self.engine.opa_recursive(self.node))
    }

    /// The image style of `part`, as [`DrawCx::image_dsc`] would build it.
    #[must_use]
    pub fn image_dsc(&self, part: Part) -> ImageDsc<'static> {
        self.engine
            .image_dsc(self.node, part, self.engine.opa_recursive(self.node))
    }

    /// The line style of `part`, as [`DrawCx::line_dsc`] would build it.
    #[must_use]
    pub fn line_dsc(&self, part: Part) -> LineDsc {
        self.engine
            .line_dsc(self.node, part, self.engine.opa_recursive(self.node))
    }

    /// The arc style of `part`, as [`DrawCx::arc_dsc`] would build it.
    #[must_use]
    pub fn arc_dsc(&self, part: Part) -> ArcDsc<'static> {
        self.engine
            .arc_dsc(self.node, part, self.engine.opa_recursive(self.node))
    }
}

/// Mutable access to the engine on behalf of one node (widget `init` and setters).
pub struct WidgetCx<'a> {
    engine: &'a mut Engine,
    node: NodeId,
}

impl core::fmt::Debug for WidgetCx<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WidgetCx")
            .field("node", &self.node)
            .finish_non_exhaustive()
    }
}

impl<'a> WidgetCx<'a> {
    /// A context for `node`.
    pub fn new(engine: &'a mut Engine, node: NodeId) -> Self {
        Self { engine, node }
    }

    /// The node.
    #[must_use]
    pub fn node(&self) -> NodeId {
        self.node
    }

    /// Invalidates the node's area (coords + extra draw size).
    pub fn invalidate(&mut self) {
        self.engine.invalidate(self.node, InvalidateReason::Explicit);
    }

    /// Invalidates the node's area for a setter named `setter` (shown in the invalidation log).
    pub fn invalidate_for(&mut self, setter: &'static str) {
        self.engine
            .invalidate(self.node, InvalidateReason::WidgetSetter(setter));
    }

    /// Invalidates an absolute area on the node's display, clipped to the node's visible area.
    pub fn invalidate_area(&mut self, area: Rect) {
        self.engine
            .invalidate_node_area(self.node, area, InvalidateReason::Explicit);
    }

    /// Marks the node's size/position for the layout pass.
    pub fn mark_layout(&mut self) {
        self.engine.mark_layout(self.node, LayoutDirty::SELF);
    }

    /// The node's state.
    #[must_use]
    pub fn state(&self) -> State {
        self.engine
            .tree()
            .node(self.node)
            .map_or(State::DEFAULT, crate::Node::state)
    }

    /// Adds states (idempotent).
    pub fn add_state(&mut self, s: State) {
        self.engine.add_state(self.node, s);
    }

    /// Removes states (idempotent).
    pub fn clear_state(&mut self, s: State) {
        self.engine.clear_state(self.node, s);
    }

    /// A read-only view of the node (styles, coordinates, draw descriptors).
    #[must_use]
    pub fn measure(&self) -> MeasureCx<'_> {
        MeasureCx::new(self.engine, self.node)
    }

    /// Absolute coordinates of the node.
    #[must_use]
    pub fn coords(&self) -> Rect {
        self.engine.coords(self.node)
    }

    /// The coordinates minus padding and border width of `Part::Main`.
    #[must_use]
    pub fn content_area(&self) -> Rect {
        self.engine.content_area(self.node)
    }

    /// A resolved style property.
    #[must_use]
    pub fn style(&self, part: Part, prop: PropId) -> StyleValue {
        self.engine.style_prop(self.node, part, prop)
    }

    /// A resolved integer property (0 if it is not an integer).
    #[must_use]
    pub fn style_i32(&self, part: Part, prop: PropId) -> i32 {
        self.engine.style_i32(self.node, part, prop)
    }

    /// A resolved color property.
    #[must_use]
    pub fn style_color(&self, part: Part, prop: PropId) -> Color {
        self.engine.style_color(self.node, part, prop)
    }

    /// The resolved font of `part`.
    #[must_use]
    pub fn font(&self, part: Part) -> &'static Font {
        self.engine.style_font(self.node, part)
    }

    /// The rectangle style of `part` (see [`MeasureCx::rect_dsc`]).
    #[must_use]
    pub fn rect_dsc(&self, part: Part) -> RectStyle {
        self.measure().rect_dsc(part)
    }

    /// The text style of `part` (see [`MeasureCx::text_dsc`]).
    #[must_use]
    pub fn text_dsc(&self, part: Part) -> TextDsc {
        self.measure().text_dsc(part)
    }

    /// The image style of `part` (see [`MeasureCx::image_dsc`]).
    #[must_use]
    pub fn image_dsc(&self, part: Part) -> ImageDsc<'static> {
        self.measure().image_dsc(part)
    }

    /// The line style of `part` (see [`MeasureCx::line_dsc`]).
    #[must_use]
    pub fn line_dsc(&self, part: Part) -> LineDsc {
        self.measure().line_dsc(part)
    }

    /// The arc style of `part` (see [`MeasureCx::arc_dsc`]).
    #[must_use]
    pub fn arc_dsc(&self, part: Part) -> ArcDsc<'static> {
        self.measure().arc_dsc(part)
    }

    /// Recomputes the node's extra draw size after a widget property that changes
    /// [`Widget::ext_draw_size`] changed, with `widget_ext` as the value the widget now
    /// reports (the widget is taken out of its node while its setters run, so the engine
    /// cannot ask it). Returns whether the node's extra draw size changed; nothing is
    /// invalidated.
    pub fn refresh_ext_draw_with(&mut self, widget_ext: u16) -> bool {
        self.engine.refresh_ext_draw_with(self.node, widget_ext)
    }

    /// The engine.
    #[must_use]
    pub fn engine(&self) -> &Engine {
        self.engine
    }

    /// The engine, mutably.
    pub fn engine_mut(&mut self) -> &mut Engine {
        self.engine
    }
}

/// The default hit test: `p` inside the coordinates; with [`ObjFlags::ADV_HITTEST`] the
/// rounded corners (`Radius` of `Part::Main`) are excluded.
#[must_use]
pub fn default_hit_test(cx: &MeasureCx<'_>, p: Point) -> bool {
    let c = cx.coords();
    if !c.contains(p) {
        return false;
    }
    if !cx.flags().contains(ObjFlags::ADV_HITTEST) {
        return true;
    }
    let r = effective_radius(c, cx.style_i32(Part::Main, PropId::Radius));
    if r == 0 {
        return true;
    }
    // Distance of the pixel center from the nearest corner circle center, in half pixels.
    let cx0 = if p.x < c.x0 + r {
        c.x0 + r
    } else if p.x >= c.x1 - r {
        c.x1 - r
    } else {
        return true;
    };
    let cy0 = if p.y < c.y0 + r {
        c.y0 + r
    } else if p.y >= c.y1 - r {
        c.y1 - r
    } else {
        return true;
    };
    let dx = i64::from(2 * p.x + 1 - 2 * cx0);
    let dy = i64::from(2 * p.y + 1 - 2 * cy0);
    dx * dx + dy * dy <= 4 * i64::from(r) * i64::from(r)
}

/// The radius actually drawn: at most half the shorter side.
pub(crate) fn effective_radius(area: Rect, radius: i32) -> i32 {
    radius.clamp(0, area.width().min(area.height()).max(0) / 2)
}

/// The default cover check (LVGL `lv_obj` cover check): the node covers `area` when it has no
/// transform, `opa`, `opa_layered` and `bg_opa` are fully opaque, the blend mode is normal, no
/// gradient stop is translucent, and `area` lies inside the coordinates without touching the
/// rounded corners (inside the rectangle minus the `radius × radius` corner squares).
#[must_use]
pub fn default_covers(cx: &MeasureCx<'_>, area: Rect) -> bool {
    let e = cx.engine();
    let id = cx.node();
    if e.needs_layer(id) || e.has_transform(id) {
        return false;
    }
    let m = e.cached_main(id);
    if !m.opa.is_cover() || !m.bg_opa.is_cover() {
        return false;
    }
    if !e.bg_is_opaque(id) {
        return false;
    }
    let c = e.draw_area(id);
    if !c.contains_rect(&area) {
        return false;
    }
    let r = effective_radius(c, m.radius);
    if r == 0 {
        return true;
    }
    let hor_band = c.inset(Insets::new(r, 0, r, 0));
    let ver_band = c.inset(Insets::new(0, r, 0, r));
    hor_band.contains_rect(&area) || ver_band.contains_rect(&area)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{OBJ_CLASS, Obj};
    use alloc::boxed::Box;

    struct Probe;
    static PROBE_CLASS: WidgetClass = WidgetClass::new("probe");
    impl Widget for Probe {
        fn class(&self) -> &'static WidgetClass {
            &PROBE_CLASS
        }
    }

    #[test]
    fn class_builder_is_const() {
        static C: WidgetClass = WidgetClass::new("c")
            .parts(&[Part::Main, Part::Knob])
            .default_flags(ObjFlags::CHECKABLE)
            .group_def(GroupDef::False)
            .editable(Editable::True)
            .theme_inheritable(false);
        assert_eq!(C.name, "c");
        assert_eq!(C.parts, &[Part::Main, Part::Knob]);
        assert_eq!(C.default_flags, ObjFlags::CHECKABLE);
        assert_eq!(C.group_def, GroupDef::False);
        assert_eq!(C.editable, Editable::True);
        assert!(!C.theme_inheritable);
        let d = WidgetClass::new("d");
        assert_eq!(d.parts, &[Part::Main]);
        assert_eq!(d.group_def, GroupDef::Default);
        assert_eq!(d.editable, Editable::Inherit);
    }

    #[test]
    fn downcast_roundtrip() {
        let mut w: Box<dyn Widget> = Box::new(Obj);
        assert!(w.downcast_ref::<Obj>().is_some());
        assert!(w.downcast_ref::<Probe>().is_none());
        assert!(w.downcast_mut::<Obj>().is_some());
        assert!(w.is::<Obj>());
        assert!(core::ptr::eq(w.class(), &raw const OBJ_CLASS));
        let p: Box<dyn Widget> = Box::new(Probe);
        assert!(p.downcast_ref::<Obj>().is_none());
        assert_eq!(p.text(), None);
    }

    #[test]
    fn effective_radius_clamps() {
        let r = Rect::from_xywh(0, 0, 10, 30);
        assert_eq!(effective_radius(r, 100), 5);
        assert_eq!(effective_radius(r, -3), 0);
        assert_eq!(effective_radius(r, 3), 3);
    }
}
