//! [`when`]: one of two branches, rebuilt only when the condition changes.

use alloc::boxed::Box;
use alloc::rc::Rc;
use core::cell::{Cell, RefCell};

use twine_engine::{NodeId, fmt_node_id};
use twine_reactive::{Scope, defer_current_effect, dispose_current_effect, untrack};

use super::{WHEN_CLASS, Wrapper, delete_children, dispose_with};
use crate::access::EngineAccess;
use crate::build::BuildCx;
use crate::view::{AnyView, IntoAnyView, View};

/// Shows `then(scope)` while `cond()` is `true` (and nothing, or the
/// [`otherwise`](When::otherwise) branch, while it is `false`).
///
/// `cond` is a memo: the branch is rebuilt only when its value changes, never when signals it
/// reads change without changing the result. Each branch lives in its own child scope,
/// disposed (with its nodes) when the branch is replaced.
///
/// ```
/// use twine_view::prelude::*;
///
/// fn app(cx: Scope) -> impl View {
///     let logged_in = cx.signal(false);
///     column((
///         when(move || logged_in.get(), |_| label("Welcome back"))
///             .otherwise(|_| label("Please log in")),
///         button(label("Toggle")).on_click(move || logged_in.update(|v| *v = !*v)),
///     ))
/// }
/// # let _ = app;
/// ```
pub fn when<V: View>(cond: impl Fn() -> bool + 'static, then: impl Fn(Scope) -> V + 'static) -> When<V> {
    When {
        cond: Box::new(cond),
        then: Box::new(then),
    }
}

/// The view of [`when`].
#[must_use]
pub struct When<V> {
    cond: Box<dyn Fn() -> bool>,
    then: Box<dyn Fn(Scope) -> V>,
}

impl<V> core::fmt::Debug for When<V> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("When")
    }
}

impl<V: View> When<V> {
    /// The branch shown while the condition is `false`.
    pub fn otherwise<V2: View>(self, f: impl Fn(Scope) -> V2 + 'static) -> WhenElse<V, V2> {
        WhenElse {
            when: self,
            otherwise: Box::new(f),
        }
    }
}

/// The view of [`when`] with an [`otherwise`](When::otherwise) branch.
#[must_use]
pub struct WhenElse<V, V2> {
    when: When<V>,
    otherwise: Box<dyn Fn(Scope) -> V2>,
}

impl<V, V2> core::fmt::Debug for WhenElse<V, V2> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("WhenElse")
    }
}

impl<V: View> View for When<V> {
    fn build(self, cx: &mut BuildCx<'_>) -> NodeId {
        let then = self.then;
        build_switch(cx, self.cond, move |s, on| on.then(|| then(s).into_any()))
    }
}

impl<V: View, V2: View> View for WhenElse<V, V2> {
    fn build(self, cx: &mut BuildCx<'_>) -> NodeId {
        let then = self.when.then;
        let otherwise = self.otherwise;
        build_switch(cx, self.when.cond, move |s, on| {
            Some(if on {
                then(s).into_any()
            } else {
                otherwise(s).into_any()
            })
        })
    }
}

/// The region behind [`When`] / [`WhenElse`]: a wrapper whose content is rebuilt by
/// `make(scope, value)` whenever the memoized condition changes.
fn build_switch(
    cx: &mut BuildCx<'_>,
    cond: Box<dyn Fn() -> bool>,
    make: impl Fn(Scope, bool) -> Option<AnyView> + 'static,
) -> NodeId {
    let wrapper = cx.create(Wrapper(&WHEN_CLASS));
    let scope = cx.scope();
    let memo = scope.memo(cond);
    let content: Rc<RefCell<Option<Scope>>> = Rc::default();
    let c = content.clone();
    cx.on_delete(wrapper, move || {
        if let Some(s) = c.borrow_mut().take() {
            s.dispose();
        }
    });
    let last: Rc<Cell<Option<bool>>> = Rc::default();
    cx.provide(|| {
        scope.effect_with_cx(move |_| {
            let on = memo.get();
            if last.get() == Some(on) {
                return;
            }
            match EngineAccess::with(|e| e.tree().contains(wrapper)) {
                None => return defer_current_effect(),
                Some(false) => return dispose_current_effect(),
                Some(true) => {}
            }
            last.set(Some(on));
            twine_core::debug!(target: "twine::view", "when {}: branch {}", fmt_node_id(wrapper), on);
            untrack(|| {
                let old = content.borrow_mut().take();
                EngineAccess::with(|e| {
                    if let Some(s) = old {
                        dispose_with(e, s);
                    }
                    delete_children(e, wrapper);
                });
                let child = scope.child();
                *content.borrow_mut() = Some(child);
                if let Some(view) = make(child, on) {
                    EngineAccess::with(|e| {
                        let mut bcx = BuildCx::new(e, wrapper, child);
                        view.build(&mut bcx);
                    });
                }
            });
        });
    });
    wrapper
}
