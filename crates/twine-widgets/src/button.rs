//! [`Button`]: a clickable container (LVGL `lv_button`).

use alloc::boxed::Box;

use twine_engine::{
    Engine, EngineError, GroupDef, MeasureCx, NodeId, OBJ_FLAGS, ObjFlags, Variant, Widget, WidgetClass,
    WidgetCx,
};
use twine_style::Part;

use crate::log_set;

/// Default flags of [`BUTTON_CLASS`]: the container's, minus `SCROLLABLE` (LVGL
/// `lv_button_constructor`: `lv_obj_set_scrollable(obj, false)`,
/// `lv_obj_set_scroll_on_focus(obj, true)`).
const BUTTON_FLAGS: ObjFlags = OBJ_FLAGS
    .difference(ObjFlags::SCROLLABLE)
    .union(ObjFlags::SCROLL_ON_FOCUS);

/// The class of [`Button`]: `"button"`, parts `Main` and `Scrollbar`, always in the default
/// focus group (LVGL `LV_OBJ_CLASS_GROUP_DEF_TRUE`).
pub static BUTTON_CLASS: WidgetClass = WidgetClass::new("button")
    .parts(&[Part::Main, Part::Scrollbar])
    .default_flags(BUTTON_FLAGS)
    .group_def(GroupDef::True);

/// A theme class derived from [`BUTTON_CLASS`] (only themes read it: parts, flags and
/// behaviour stay the button's).
macro_rules! variant_class {
    ($name:literal) => {
        WidgetClass::new($name)
            .parts(BUTTON_CLASS.parts)
            .default_flags(BUTTON_FLAGS)
            .group_def(GroupDef::True)
            .base(&BUTTON_CLASS)
    };
}

/// The theme class of [`ButtonVariant::Tinted`]: `"button.tinted"`, derived from
/// [`BUTTON_CLASS`].
pub static BUTTON_TINTED_CLASS: WidgetClass = variant_class!("button.tinted");
/// The theme class of [`ButtonVariant::Gray`]: `"button.gray"`, derived from [`BUTTON_CLASS`].
pub static BUTTON_GRAY_CLASS: WidgetClass = variant_class!("button.gray");
/// The theme class of [`ButtonVariant::Plain`]: `"button.plain"`, derived from
/// [`BUTTON_CLASS`].
pub static BUTTON_PLAIN_CLASS: WidgetClass = variant_class!("button.plain");
/// The theme class of [`ButtonVariant::Destructive`]: `"button.destructive"`, derived from
/// [`BUTTON_CLASS`].
pub static BUTTON_DESTRUCTIVE_CLASS: WidgetClass = variant_class!("button.destructive");

/// The look of a button, as a theme class (design `docs/plan/r7-theming-design.md` §2.4):
/// the value of the view's `.variant(..)` (a constant or a signal), which calls
/// [`Engine::set_theme_class`](twine_engine::Engine::set_theme_class). A theme that does not
/// know a variant styles it like a plain button (lineage fallback): the built-in LVGL, Mono
/// and Simple themes show every variant as their button.
///
/// | Variant | Class | Intent |
/// |---------|-------|--------|
/// | `Filled` (default) | [`BUTTON_CLASS`] | the primary action |
/// | `Tinted` | [`BUTTON_TINTED_CLASS`] | a secondary action: accent text on a light accent tint |
/// | `Gray` | [`BUTTON_GRAY_CLASS`] | a neutral action on a gray fill |
/// | `Plain` | [`BUTTON_PLAIN_CLASS`] | text only, no fill |
/// | `Destructive` | [`BUTTON_DESTRUCTIVE_CLASS`] | an action that deletes or cancels something |
/// | `Custom(class)` | `class` | an application variant (a class whose base chain reaches [`BUTTON_CLASS`]) |
///
/// ```
/// use twine_engine::{Variant, WidgetClass};
/// use twine_widgets::button::{BUTTON_CLASS, BUTTON_TINTED_CLASS, ButtonVariant};
///
/// static PILL: WidgetClass = WidgetClass::new("button.pill").base(&BUTTON_CLASS);
/// assert!(ButtonVariant::Filled.class().is(&BUTTON_CLASS));
/// assert!(ButtonVariant::Tinted.class().is(&BUTTON_TINTED_CLASS));
/// assert!(ButtonVariant::Custom(&PILL).class().is_a(&BUTTON_CLASS));
/// assert_eq!(ButtonVariant::default(), ButtonVariant::Filled);
/// ```
#[derive(Clone, Copy, Debug, Default)]
pub enum ButtonVariant {
    /// The theme's button (the primary action).
    #[default]
    Filled,
    /// Accent text on a light accent tint.
    Tinted,
    /// A neutral gray fill.
    Gray,
    /// Text only, no fill.
    Plain,
    /// A destructive action (the theme's danger color).
    Destructive,
    /// An application variant: a class derived from [`BUTTON_CLASS`].
    Custom(&'static WidgetClass),
}

impl PartialEq for ButtonVariant {
    /// Variants are equal when they name the same class (`Custom(&BUTTON_CLASS) == Filled`).
    fn eq(&self, other: &Self) -> bool {
        self.class().is(other.class())
    }
}

impl Eq for ButtonVariant {}

impl Variant for ButtonVariant {
    type Widget = Button;

    fn class(self) -> &'static WidgetClass {
        match self {
            ButtonVariant::Filled => &BUTTON_CLASS,
            ButtonVariant::Tinted => &BUTTON_TINTED_CLASS,
            ButtonVariant::Gray => &BUTTON_GRAY_CLASS,
            ButtonVariant::Plain => &BUTTON_PLAIN_CLASS,
            ButtonVariant::Destructive => &BUTTON_DESTRUCTIVE_CLASS,
            ButtonVariant::Custom(c) => c,
        }
    }
}

/// A button: a container that is clickable, focusable and not scrollable. With the default
/// theme it is a primary-colored rounded rectangle with a shadow (light mode) that darkens
/// while pressed, turns secondary when checked and shows an outline on keyboard focus.
///
/// **Children keep LVGL semantics**: a button has no layout; the theme only sets paddings and
/// centered text alignment. Center a label child with `engine.align(label, Align::Center, 0,
/// 0)` (LVGL `lv_obj_center`); the declarative `button(child)` does this for its child.
///
/// A checkable button toggles `State::CHECKED` when clicked (and with the arrow keys) and
/// sends `ValueChanged` (the engine's base object behaviour, LVGL `lv_obj_event`).
///
/// ```
/// use twine_engine::ObjFlags;
/// use twine_testing::EngineHarness;
/// use twine_widgets::button::{self, Button};
///
/// let mut h = EngineHarness::new(80, 40);
/// let screen = h.screen();
/// let b = button::create(h.engine_mut(), screen).unwrap();
/// h.engine_mut().with_widget_mut(b, |w: &mut Button, cx| w.set_checkable(cx, true));
/// assert!(h.engine().has_flag(b, ObjFlags::CHECKABLE));
/// ```
///
/// Widgets that LVGL derives from the button (a list button, a message box footer button)
/// are buttons with their own class: [`Button::with_class`].
#[derive(Clone, Copy, Debug)]
pub struct Button {
    class: &'static WidgetClass,
}

impl Default for Button {
    fn default() -> Self {
        Self::new()
    }
}

impl PartialEq for Button {
    fn eq(&self, other: &Self) -> bool {
        core::ptr::eq(self.class, other.class)
    }
}

impl Eq for Button {}

impl core::hash::Hash for Button {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        core::ptr::hash(self.class, state);
    }
}

impl Button {
    /// A button widget (insert it with `Engine::create` or use [`create`]).
    #[must_use]
    pub const fn new() -> Self {
        Self::with_class(&BUTTON_CLASS)
    }

    /// A button of another class: an LVGL subclass of the button that only changes the class
    /// (its name for themes and queries, flags, focus group default), e.g. `"list_button"`.
    ///
    /// ```
    /// use twine_engine::{Widget, WidgetClass};
    /// use twine_widgets::button::{BUTTON_CLASS, Button};
    /// static LIST_BUTTON: WidgetClass = WidgetClass::new("list_button")
    ///     .parts(BUTTON_CLASS.parts)
    ///     .default_flags(BUTTON_CLASS.default_flags);
    /// assert_eq!(Button::with_class(&LIST_BUTTON).class().name, "list_button");
    /// ```
    #[must_use]
    pub const fn with_class(class: &'static WidgetClass) -> Self {
        Self { class }
    }

    /// Makes the button toggle `State::CHECKED` on click (the `CHECKABLE` flag). Idempotent.
    pub fn set_checkable(&mut self, cx: &mut WidgetCx<'_>, on: bool) {
        let id = cx.node();
        if cx.engine().has_flag(id, ObjFlags::CHECKABLE) == on {
            return;
        }
        log_set(BUTTON_CLASS.name, id, "checkable");
        cx.engine_mut().set_flag(id, ObjFlags::CHECKABLE, on);
    }

    /// Whether the button is checkable.
    #[must_use]
    pub fn is_checkable(&self, cx: &MeasureCx<'_>) -> bool {
        cx.flags().contains(ObjFlags::CHECKABLE)
    }
}

impl Widget for Button {
    fn class(&self) -> &'static WidgetClass {
        self.class
    }
}

/// Creates a button as the last child of `parent` (LVGL `lv_button_create`). Its size is
/// `Content` until set.
///
/// # Errors
/// [`EngineError::NodeNotFound`] if `parent` does not exist (logged).
pub fn create(engine: &mut Engine, parent: NodeId) -> Result<NodeId, EngineError> {
    engine.create(parent, Box::new(Button::new()))
}
