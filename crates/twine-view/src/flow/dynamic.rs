//! [`dynamic`]: a region rebuilt whenever the signals its function reads change.

use alloc::boxed::Box;
use alloc::rc::Rc;
use core::cell::RefCell;

use twine_engine::{NodeId, fmt_node_id};
use twine_reactive::{Scope, defer_current_effect, dispose_current_effect, untrack};

use super::{DYNAMIC_CLASS, Wrapper, delete_children, dispose_with};
use crate::access::EngineAccess;
use crate::build::BuildCx;
use crate::view::{AnyView, View};

/// A region whose content is `f(scope)`, rebuilt whenever a signal read **by `f` itself**
/// changes. Reads inside the built views (their bindings) do not rebuild the region.
///
/// ```
/// use twine_view::prelude::*;
///
/// fn app(cx: Scope) -> impl View {
///     let mode = cx.signal(0u8);
///     dynamic(move |_cx| match mode.get() {
///         0 => label("idle").into_any(),
///         _ => button(label("busy")).into_any(),
///     })
/// }
/// # let _ = app;
/// ```
pub fn dynamic(f: impl Fn(Scope) -> AnyView + 'static) -> Dynamic {
    Dynamic { f: Box::new(f) }
}

/// The view of [`dynamic`].
#[must_use]
pub struct Dynamic {
    f: Box<dyn Fn(Scope) -> AnyView>,
}

impl core::fmt::Debug for Dynamic {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Dynamic")
    }
}

impl View for Dynamic {
    fn build(self, cx: &mut BuildCx<'_>) -> NodeId {
        let wrapper = cx.create(Wrapper(&DYNAMIC_CLASS));
        let scope = cx.scope();
        let f = self.f;
        let content: Rc<RefCell<Option<Scope>>> = Rc::default();
        let c = content.clone();
        cx.on_delete(wrapper, move || {
            if let Some(s) = c.borrow_mut().take() {
                s.dispose();
            }
        });
        cx.provide(|| {
            scope.effect_with_cx(move |_| {
                match EngineAccess::with(|e| e.tree().contains(wrapper)) {
                    None => return defer_current_effect(),
                    Some(false) => return dispose_current_effect(),
                    Some(true) => {}
                }
                twine_core::debug!(target: "twine::view", "dynamic {}: rebuild", fmt_node_id(wrapper));
                let old = content.borrow_mut().take();
                EngineAccess::with(|e| {
                    if let Some(s) = old {
                        dispose_with(e, s);
                    }
                    delete_children(e, wrapper);
                });
                let child = scope.child();
                *content.borrow_mut() = Some(child);
                // Tracked: the signals `f` reads rebuild the region.
                let view = f(child);
                // Untracked: reads while building belong to the new bindings.
                untrack(|| {
                    EngineAccess::with(|e| {
                        let mut bcx = BuildCx::new(e, wrapper, child);
                        view.build(&mut bcx);
                    });
                });
            });
        });
        wrapper
    }
}
