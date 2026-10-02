//! [`Container`]: the base object (LVGL `lv_obj`), a styled rectangle holding children.
//!
//! The engine's [`Obj`](twine_engine::Obj) is the container: class `"obj"`, parts `Main`
//! and `Scrollbar`, and LVGL's `lv_obj` default flags (`CLICKABLE`, `CLICK_FOCUSABLE`,
//! `SCROLLABLE`, `SCROLL_ELASTIC`, `SCROLL_MOMENTUM`, `SCROLL_WITH_ARROW`, `SCROLL_CHAIN`,
//! `SCROLL_ON_FOCUS`, `SNAPPABLE`, `PRESS_LOCK`, `GESTURE_BUBBLE`). The default theme draws it
//! as a card (white or dark background, grey border, rounded corners, padding).
//!
//! [`Card`] is a container of its own class, [`CARD_CLASS`]: themes give it their card look
//! explicitly, whatever they do for plain objects (a theme that does not know it styles it
//! like a container, its [`base`](twine_engine::WidgetClass::base)).

use alloc::boxed::Box;

use twine_engine::{Engine, EngineError, NodeId, OBJ_CLASS, OBJ_FLAGS, Widget, WidgetClass};
use twine_style::Part;

pub use twine_engine::{OBJ_CLASS as CONTAINER_CLASS, Obj as Container};

/// The class of [`Card`]: `"card"`, parts `Main` and `Scrollbar`, the base object's flags,
/// base [`CONTAINER_CLASS`] (a theme without a card look styles it like a container).
pub static CARD_CLASS: WidgetClass = WidgetClass::new("card")
    .parts(&[Part::Main, Part::Scrollbar])
    .default_flags(OBJ_FLAGS)
    .base(&OBJ_CLASS);

/// A card: a [`Container`] that themes style with their card look (the default and mono
/// themes' `card` style, the simple theme's white surface), so it looks like a card even
/// where a theme draws plain containers differently. Behaves exactly like a container.
///
/// ```
/// use twine_testing::EngineHarness;
/// use twine_widgets::container::{self, CARD_CLASS};
/// let mut h = EngineHarness::new(64, 48);
/// let screen = h.screen();
/// let c = container::create_card(h.engine_mut(), screen).unwrap();
/// assert!(h.engine().tree().node(c).unwrap().class().is(&CARD_CLASS));
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Card;

impl Widget for Card {
    fn class(&self) -> &'static WidgetClass {
        &CARD_CLASS
    }
}

/// Creates a [`Card`] as the last child of `parent`. Its size is `Content` until set.
///
/// # Errors
/// [`EngineError::NodeNotFound`] if `parent` does not exist (logged).
pub fn create_card(engine: &mut Engine, parent: NodeId) -> Result<NodeId, EngineError> {
    engine.create(parent, Box::new(Card))
}

/// Creates a container as the last child of `parent` (LVGL `lv_obj_create`). Its size is
/// `Content` (it wraps its children) until set.
///
/// ```
/// use twine_testing::EngineHarness;
/// use twine_widgets::container;
/// let mut h = EngineHarness::new(64, 48);
/// let screen = h.screen();
/// let c = container::create(h.engine_mut(), screen).unwrap();
/// assert_eq!(h.engine().tree().node(c).unwrap().class().name, "obj");
/// ```
///
/// # Errors
/// [`EngineError::NodeNotFound`] if `parent` does not exist (logged).
pub fn create(engine: &mut Engine, parent: NodeId) -> Result<NodeId, EngineError> {
    engine.create(parent, Box::new(Container))
}
