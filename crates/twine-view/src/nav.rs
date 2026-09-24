//! Screen navigation ([`Navigator`], [`navigator`], [`use_navigator`]) and modals
//! ([`ModalHandle`]).

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};

use twine_core::{Color, Opa};
use twine_engine::{Engine, GroupId, InputId, NodeId, Obj, ObjFlags, ScreenLoad, fmt_node_id};
use twine_reactive::{Scope, Signal, defer_current_effect, dispose_current_effect, untrack};
use twine_style::{Align, Length, Selector, StyleProp};

use crate::access::EngineAccess;
use crate::build::{BuildCx, on_delete};
use crate::flow::{NAVIGATOR_CLASS, Wrapper, dispose_with};
use crate::hooks::display_of;
use crate::view::{AnyView, IntoAnyView, View};

/// A screen constructor queued for the next flush.
type ScreenFn = Box<dyn FnOnce(Scope) -> AnyView>;

/// A queued navigation.
enum NavOp {
    Push(ScreenFn, ScreenLoad),
    Pop(ScreenLoad),
    Replace(ScreenFn, ScreenLoad),
}

struct NavState {
    /// The scope owning the screens' scopes.
    scope: Scope,
    /// The screens, root first.
    stack: Vec<Entry>,
    /// Operations not applied yet (run at the next effect flush).
    queue: Vec<NavOp>,
    /// Bumped to wake the navigation effect.
    tick: Signal<u32>,
}

/// One screen of the stack.
struct Entry {
    node: NodeId,
    scope: Scope,
    /// The screen's focus group (keypad / encoder), `None` without one.
    group: Option<GroupId>,
}

/// A stack of screens (see [`navigator`]). `Clone`; obtained with [`use_navigator`].
///
/// `push`, `pop` and `replace` are queued and applied at the `Ui`'s next effect flush, so they
/// are safe anywhere, e.g. in click handlers. Each screen is its own engine screen with its own
/// child scope, disposed when the screen is deleted.
#[derive(Clone)]
pub struct Navigator {
    inner: Rc<RefCell<NavState>>,
}

impl core::fmt::Debug for Navigator {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Navigator").field("depth", &self.depth()).finish()
    }
}

impl Navigator {
    fn enqueue(&self, op: NavOp) {
        let tick = {
            let mut s = self.inner.borrow_mut();
            s.queue.push(op);
            s.tick
        };
        if tick.is_alive() {
            tick.update(|t| *t = t.wrapping_add(1));
        }
    }

    /// Shows `screen` on top of the current one with `anim` (a [`ScreenAnim`](crate::ScreenAnim)
    /// or a [`ScreenLoad`]); the current screen stays alive below.
    pub fn push<V: View>(&self, screen: fn(Scope) -> V, anim: impl Into<ScreenLoad>) {
        self.enqueue(NavOp::Push(Box::new(move |s| screen(s).into_any()), anim.into()));
    }

    /// Goes back to the previous screen with `anim`; the current one is deleted (and its scope
    /// disposed) when the animation ends. Returns `false` (doing nothing) at the root.
    pub fn pop(&self, anim: impl Into<ScreenLoad>) -> bool {
        if self.depth() <= 1 {
            twine_core::debug!(target: "twine::view", "navigator: pop at the root ignored");
            return false;
        }
        self.enqueue(NavOp::Pop(anim.into()));
        true
    }

    /// Replaces the current screen with `screen` (the old one is deleted after `anim`).
    pub fn replace<V: View>(&self, screen: fn(Scope) -> V, anim: impl Into<ScreenLoad>) {
        self.enqueue(NavOp::Replace(
            Box::new(move |s| screen(s).into_any()),
            anim.into(),
        ));
    }

    /// Number of screens, counting queued operations.
    #[must_use]
    pub fn depth(&self) -> usize {
        let s = self.inner.borrow();
        let mut d = s.stack.len();
        for op in &s.queue {
            match op {
                NavOp::Push(..) => d += 1,
                NavOp::Pop(_) => d = d.saturating_sub(1).max(1),
                NavOp::Replace(..) => {}
            }
        }
        d
    }

    /// Applies the queued operations (engine required).
    fn apply(&self, e: &mut Engine) {
        loop {
            let op = {
                let mut s = self.inner.borrow_mut();
                if s.queue.is_empty() {
                    break;
                }
                s.queue.remove(0)
            };
            match op {
                NavOp::Push(f, load) => {
                    if let Some(entry) = self.build_screen(e, f, true) {
                        e.load_screen_anim(
                            entry.node,
                            ScreenLoad {
                                auto_delete: false,
                                ..load
                            },
                        );
                        self.inner.borrow_mut().stack.push(entry);
                    }
                }
                NavOp::Pop(load) => {
                    let (top, prev) = {
                        let mut s = self.inner.borrow_mut();
                        if s.stack.len() <= 1 {
                            continue;
                        }
                        let top = s.stack.pop();
                        (top, s.stack.last().map(|x| (x.node, x.group)))
                    };
                    if let (Some(top), Some((prev, prev_group))) = (top, prev) {
                        switch_group(e, top.group, prev_group);
                        if let Some(g) = top.group.filter(|g| Some(*g) != prev_group) {
                            e.delete_group(g);
                        }
                        e.load_screen_anim(
                            prev,
                            ScreenLoad {
                                auto_delete: true,
                                ..load
                            },
                        );
                    }
                }
                NavOp::Replace(f, load) => {
                    if let Some(entry) = self.build_screen(e, f, true) {
                        e.load_screen_anim(
                            entry.node,
                            ScreenLoad {
                                auto_delete: true,
                                ..load
                            },
                        );
                        let old = {
                            let mut s = self.inner.borrow_mut();
                            let old = s.stack.pop();
                            s.stack.push(Entry {
                                node: entry.node,
                                scope: entry.scope,
                                group: entry.group,
                            });
                            old
                        };
                        if let Some(g) = old.and_then(|o| o.group) {
                            e.delete_group(g);
                        }
                    }
                }
            }
            twine_core::debug!(target: "twine::view", "navigator: depth {}", self.inner.borrow().stack.len());
        }
    }

    /// Builds a new screen from `f` (the screen's scope is disposed when it is deleted). With
    /// `own_group` the screen gets a focus group of its own, which becomes the default group
    /// and the keypads' and encoders' group.
    fn build_screen(&self, e: &mut Engine, f: ScreenFn, own_group: bool) -> Option<Entry> {
        let parent = self.inner.borrow().scope;
        let display = display_of(parent, e)?;
        let Ok(screen) = e.create_screen(display) else {
            twine_core::warn!(target: "twine::view", "navigator: cannot create a screen");
            return None;
        };
        let group = if own_group {
            let prev = e.default_group();
            e.create_group().ok().inspect(|g| switch_group(e, prev, Some(*g)))
        } else {
            e.default_group()
        };
        let child = parent.child();
        let view = EngineAccess::provide(e, || untrack(|| f(child)));
        {
            let mut bcx = BuildCx::new(e, screen, child);
            view.build(&mut bcx);
        }
        on_delete(e, screen, move || child.dispose());
        twine_core::debug!(target: "twine::view", "navigator: screen {} built", fmt_node_id(screen));
        Some(Entry {
            node: screen,
            scope: child,
            group,
        })
    }
}

/// A navigation root: provides a [`Navigator`] in `cx` (see [`use_navigator`]) and shows
/// `initial` as the display's active screen. The view itself is an empty anchor node.
///
/// ```
/// use twine_view::prelude::*;
///
/// fn app(cx: Scope) -> impl View {
///     navigator(cx, home)
/// }
/// fn home(cx: Scope) -> impl View {
///     let nav = use_navigator(cx);
///     button(label("Settings")).on_click(move || nav.push(settings, ScreenAnim::MoveLeft(Duration::ms(300))))
/// }
/// fn settings(cx: Scope) -> impl View {
///     let nav = use_navigator(cx);
///     button(label("Back")).on_click(move || nav.pop(ScreenAnim::MoveRight(Duration::ms(300))))
/// }
/// # let _ = app;
/// ```
pub fn navigator<V: View>(cx: Scope, initial: fn(Scope) -> V) -> impl View {
    let nav = Navigator {
        inner: Rc::new(RefCell::new(NavState {
            scope: cx,
            stack: Vec::new(),
            queue: Vec::new(),
            tick: cx.signal(0),
        })),
    };
    cx.provide(nav.clone());
    NavigatorView {
        nav,
        initial: Box::new(move |s| initial(s).into_any()),
    }
}

/// The view of [`navigator`].
struct NavigatorView {
    nav: Navigator,
    initial: ScreenFn,
}

impl View for NavigatorView {
    fn build(self, cx: &mut BuildCx<'_>) -> NodeId {
        let anchor = cx.create(Wrapper(&NAVIGATOR_CLASS));
        let nav = self.nav;
        let e = cx.engine();
        if let Some(entry) = nav.build_screen(e, self.initial, false) {
            e.load_screen(entry.node);
            nav.inner.borrow_mut().stack.push(entry);
        }
        let n2 = nav.clone();
        cx.on_delete(anchor, move || {
            for entry in n2.inner.borrow_mut().stack.drain(..) {
                entry.scope.dispose();
            }
        });
        let scope = cx.scope();
        let tick = nav.inner.borrow().tick;
        cx.provide(|| {
            scope.effect_with_cx(move |_| {
                tick.get();
                if nav.inner.borrow().queue.is_empty() {
                    return;
                }
                if EngineAccess::with(|e| nav.apply(e)).is_none() {
                    defer_current_effect();
                }
            });
        });
        anchor
    }
}

/// The [`Navigator`] provided by the enclosing [`navigator`].
///
/// # Panics
/// If `cx` is not inside a navigator (with the type name, like `expect_context`).
#[must_use]
pub fn use_navigator(cx: Scope) -> Navigator {
    cx.expect_context::<Navigator>()
}

/// A modal opened with [`ScopeExt::show_modal`](crate::ScopeExt::show_modal). `Clone`.
#[derive(Clone)]
pub struct ModalHandle {
    inner: Rc<ModalState>,
}

struct ModalState {
    open: Signal<bool>,
    node: Cell<Option<NodeId>>,
}

impl core::fmt::Debug for ModalHandle {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ModalHandle")
            .field("node", &self.inner.node.get().map(fmt_node_id))
            .finish()
    }
}

impl ModalHandle {
    /// Closes the modal (disposes its scope, deletes its nodes, restores the focus group) at
    /// the next effect flush. Closing twice does nothing.
    pub fn close(&self) {
        if self.inner.open.is_alive() {
            self.inner.open.set_if_changed(false);
        }
    }

    /// The backdrop node (once shown).
    #[must_use]
    pub fn node(&self) -> Option<NodeId> {
        self.inner.node.get()
    }

    /// Whether the modal is open.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.inner.open.try_get().unwrap_or(false)
    }
}

/// `(modal group, previous default group)` of an open modal.
type ModalGroups = Rc<Cell<Option<(GroupId, Option<GroupId>)>>>;

/// Makes `to` the default group and the group of the inputs attached to `from`.
fn switch_group(e: &mut Engine, from: Option<GroupId>, to: Option<GroupId>) {
    if from == to {
        return;
    }
    move_inputs(e, from, to);
    e.set_default_group(to);
}

/// The inputs attached to group `from`, re-attached to `to`.
fn move_inputs(e: &mut Engine, from: Option<GroupId>, to: Option<GroupId>) {
    let ids: Vec<InputId> = e.inputs().filter(|&i| e.input_group(i) == from).collect();
    for i in ids {
        e.set_input_group(i, to);
    }
}

/// See [`ScopeExt::show_modal`](crate::ScopeExt::show_modal).
pub(crate) fn show_modal<V: View>(cx: Scope, view: impl FnOnce(Scope) -> V + 'static) -> ModalHandle {
    let content = cx.child();
    let open = cx.signal(true);
    let state = Rc::new(ModalState {
        open,
        node: Cell::new(None),
    });
    let mut view = Some(view);
    // `(modal group, previous default group)` while open.
    let groups: ModalGroups = Rc::default();
    let st = state.clone();
    let g2 = groups.clone();
    cx.on_cleanup(move || {
        let node = st.node.take();
        let grp = g2.take();
        EngineAccess::with(|e| close_modal(e, node, grp));
    });
    let st = state.clone();
    cx.effect_with_cx(move |_| {
        let is_open = open.get();
        if !EngineAccess::available() {
            return defer_current_effect();
        }
        if is_open {
            let Some(v) = view.take() else { return };
            let v = untrack(|| v(content));
            EngineAccess::with(|e| {
                let Some(display) = display_of(content, e) else {
                    return;
                };
                let Some(top) = e.top_layer(display) else { return };
                let Ok(backdrop) = e.create(top, Box::new(Obj)) else {
                    return;
                };
                for p in [
                    StyleProp::Width(Length::pct(100)),
                    StyleProp::Height(Length::pct(100)),
                    StyleProp::BgColor(Color::BLACK),
                    StyleProp::BgOpa(Opa::P50),
                    StyleProp::BorderWidth(0),
                    StyleProp::Radius(0),
                    StyleProp::PadTop(0),
                    StyleProp::PadBottom(0),
                    StyleProp::PadLeft(0),
                    StyleProp::PadRight(0),
                    StyleProp::ShadowWidth(0),
                ] {
                    e.set_local_prop(backdrop, Selector::MAIN, p);
                }
                e.set_flag(backdrop, ObjFlags::SCROLLABLE, false);
                e.set_flag(backdrop, ObjFlags::CLICKABLE, true);
                // A focus group of its own (LVGL message boxes).
                let prev = e.default_group();
                if let Ok(g) = e.create_group() {
                    e.set_default_group(Some(g));
                    move_inputs(e, prev, Some(g));
                    groups.set(Some((g, prev)));
                }
                let root = {
                    let mut bcx = BuildCx::new(e, backdrop, content);
                    v.build(&mut bcx)
                };
                e.set_align(root, Align::Center);
                if let Some((_, prev)) = groups.get() {
                    e.set_default_group(prev);
                }
                st.node.set(Some(backdrop));
                twine_core::debug!(target: "twine::view", "modal {} shown", fmt_node_id(backdrop));
            });
        } else {
            let node = st.node.take();
            let grp = groups.take();
            EngineAccess::with(|e| {
                dispose_with(e, content);
                close_modal(e, node, grp);
            });
            dispose_current_effect();
        }
    });
    ModalHandle { inner: state }
}

/// Deletes the backdrop and the modal's focus group, re-attaching the inputs to the previous
/// group.
fn close_modal(e: &mut Engine, node: Option<NodeId>, groups: Option<(GroupId, Option<GroupId>)>) {
    if let Some((g, prev)) = groups {
        move_inputs(e, Some(g), prev);
        if e.default_group() == Some(g) {
            e.set_default_group(prev);
        }
        e.delete_group(g);
    }
    if let Some(n) = node.filter(|n| e.tree().contains(*n)) {
        twine_core::debug!(target: "twine::view", "modal {} closed", fmt_node_id(n));
        let _ = e.delete(n);
    }
}
