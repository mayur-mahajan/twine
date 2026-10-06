//! Components: functions describing a piece of UI, used like built-in widgets.
//!
//! The attribute [`#[component]`](macro@crate::component) (re-exported from `twine-macros`)
//! turns a function into a view with a props builder — required props are arguments, optional
//! ones are methods, modifiers apply to the returned view's root:
//!
//! ```
//! use twine_view::prelude::*;
//!
//! /// A card with a title above its content.
//! #[component]
//! fn titled(
//!     /// The title.
//!     title: TextProp,
//!     /// What the card shows below the title.
//!     content: impl ViewSeq,
//!     /// The title's color.
//!     #[prop(default = design::ON_SURFACE_MUTED)]
//!     title_color: Prop<twine_style::design::ColorValue>,
//! ) -> impl ViewExt<Widget = Card> {
//!     card((label(title).text_color(title_color), content)).gap(6)
//! }
//!
//! let _v = row((
//!     titled("Level", bar(40)).padding(8),
//!     titled("Alarm", led(true)).title_color(design::DANGER).test_id("alarm"),
//! ));
//! ```
//!
//! This module holds what the generated code uses at run time: [`Modifiers`], the modifiers
//! applied to a component, kept until it is built. Without the macro a component is a plain
//! function returning a view (see [`component`](macro@crate::component) § Without the macro).

use alloc::vec::Vec;

use crate::build::BuildOp;
use crate::modifiers::ViewExt;

/// The modifiers (build steps) applied to a component before it is built, handed to the view
/// its body returns when it is built — in order, after the body's own, exactly as if they were
/// written at the end of the body.
///
/// Used by the code [`#[component]`](macro@crate::component) generates; a hand-written
/// component builder implementing [`ViewExt`] can use it the same way:
///
/// ```
/// use twine_view::component::Modifiers;
/// use twine_view::prelude::*;
///
/// /// A hand-written component builder (what `#[component]` generates, in short).
/// struct Badge {
///     text: TextProp,
///     mods: Modifiers,
/// }
///
/// impl View for Badge {
///     fn build(self, cx: &mut BuildCx<'_>) -> NodeId {
///         let body = container(label(self.text)).padding(4).radius(8);
///         self.mods.apply(body).build(cx)
///     }
/// }
///
/// impl ViewExt for Badge {
///     type Widget = Obj;
///     fn push_op(mut self, op: twine_view::BuildOp) -> Self {
///         self.mods.push(op);
///         self
///     }
/// }
///
/// let _v = Badge { text: "new".into_text(), mods: Modifiers::new() }.bg(Color::RED);
/// ```
///
/// Without modifiers it holds an empty `Vec` (nothing allocated); each modifier is one boxed
/// build step, as on any view. Building moves the steps into the returned view (a component
/// with modifiers allocates this one `Vec` more than the same view written inline, once, at
/// build time). Never panics.
#[derive(Default)]
pub struct Modifiers(Vec<BuildOp>);

impl core::fmt::Debug for Modifiers {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("Modifiers").field(&self.0.len()).finish()
    }
}

impl Modifiers {
    /// No modifiers (does not allocate).
    #[must_use]
    #[inline]
    pub const fn new() -> Self {
        Self(Vec::new())
    }

    /// Appends a modifier (a build step run on the root node, see [`ViewExt::push_op`]).
    #[inline]
    pub fn push(&mut self, op: BuildOp) {
        self.0.push(op);
    }

    /// Hands every modifier, in order, to `view` (its [`ViewExt::push_op`]) and returns it.
    #[must_use]
    #[inline]
    pub fn apply<V: ViewExt>(self, view: V) -> V {
        self.0.into_iter().fold(view, ViewExt::push_op)
    }
}
