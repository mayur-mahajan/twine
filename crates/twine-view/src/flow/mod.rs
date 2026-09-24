//! Control flow: [`when`], [`dynamic`], [`for_each`], [`virtual_list`].
//!
//! Each region owns one **wrapper node** (a `LAYOUT_PASSTHROUGH` node: its children are laid
//! out as children of the wrapper's parent, it draws nothing and is invisible to hit tests)
//! and a child [`Scope`] per piece of content. Replacing content disposes that scope — with
//! every signal, effect, binding and handler created inside — and deletes its nodes; nothing
//! outside the region is rebuilt.

mod dynamic;
mod for_each;
mod virtual_list;
mod when;

use alloc::vec::Vec;

use twine_engine::{Engine, GroupDef, NodeId, ObjFlags, Widget, WidgetClass};
use twine_reactive::Scope;
use twine_style::Part;

use crate::access::EngineAccess;

pub use dynamic::{Dynamic, dynamic};
pub use for_each::{ForEach, for_each};
pub use virtual_list::{VirtualList, virtual_list};
pub use when::{When, WhenElse, when};

/// Flags of a region wrapper.
const WRAPPER_FLAGS: ObjFlags = ObjFlags::LAYOUT_PASSTHROUGH;

/// Class of the [`when`] wrapper.
pub static WHEN_CLASS: WidgetClass = wrapper_class("when");
/// Class of the [`dynamic`] wrapper.
pub static DYNAMIC_CLASS: WidgetClass = wrapper_class("dynamic");
/// Class of the [`for_each`] wrapper.
pub static FOR_EACH_CLASS: WidgetClass = wrapper_class("for_each");
/// Class of the [`navigator`](crate::navigator) anchor.
pub static NAVIGATOR_CLASS: WidgetClass = wrapper_class("navigator");

const fn wrapper_class(name: &'static str) -> WidgetClass {
    WidgetClass::new(name)
        .parts(&[Part::Main])
        .default_flags(WRAPPER_FLAGS)
        .group_def(GroupDef::False)
}

/// The wrapper node of a control-flow region: no look, no input, transparent to layout.
#[derive(Debug, Clone, Copy)]
pub struct Wrapper(pub &'static WidgetClass);

impl Widget for Wrapper {
    fn class(&self) -> &'static WidgetClass {
        self.0
    }
}

/// Disposes `scope` with the engine lent to its cleanups.
pub(crate) fn dispose_with(e: &mut Engine, scope: Scope) {
    EngineAccess::provide(e, || scope.dispose());
}

/// Deletes every child of `wrapper`.
pub(crate) fn delete_children(e: &mut Engine, wrapper: NodeId) {
    let kids: Vec<NodeId> = e.tree().children(wrapper).collect();
    for k in kids {
        if e.tree().contains(k) {
            let _ = e.delete(k);
        }
    }
}
