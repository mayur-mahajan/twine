//! Screen navigation ([`Navigator`], [`navigator`], [`use_navigator`]) and modals
//! ([`ModalHandle`]).

use alloc::boxed::Box;
use alloc::vec::Vec;

use twine_core::{Color, Opa};
use twine_engine::{Engine, GroupId, InputId, NodeId, Obj, ObjFlags, ScreenLoad, fmt_node_id};
use twine_reactive::{Scope, Signal, StoredValue, defer_current_effect, dispose_current_effect, untrack};
use twine_style::{Align, Length, Radius, Selector, StyleProp};

use crate::access::EngineAccess;
use crate::build::{BuildCx, on_delete};
use crate::flow::{NAVIGATOR_CLASS, Wrapper, dispose_with};
use crate::hooks::{StyleAnchor, display_of};
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
    /// The navigator's anchor node: the screens' views inherit their text style from it (and
    /// so from the views around the navigator), although each is on a screen of its own.
    anchor: Option<NodeId>,
}

/// One screen of the stack.
struct Entry {
    node: NodeId,
    scope: Scope,
    /// The screen's focus group (keypad / encoder), `None` without one.
    group: Option<GroupId>,
}

/// A stack of screens (see [`navigator`]), obtained with [`use_navigator`].
///
/// `Copy`, like a signal: move it into any number of handlers without cloning. Its state is a
/// [`StoredValue`] owned by the navigator's scope; once that scope is disposed every method
/// logs `warn!` and does nothing (`depth` is 0).
///
/// `push`, `pop` and `replace` are queued and applied at the `Ui`'s next effect flush, so they
/// are safe anywhere, e.g. in click handlers. Each screen is its own engine screen with its own
/// child scope, disposed when the screen is deleted.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Navigator {
    state: StoredValue<NavState>,
}

/// Logs a navigator used after its scope was disposed.
#[cold]
#[inline(never)]
fn nav_disposed(what: &str) {
    twine_core::warn!(target: "twine::view", "Navigator::{} after the navigator was disposed; ignored", what);
}

impl core::fmt::Debug for Navigator {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Navigator").field("depth", &self.depth()).finish()
    }
}

impl Navigator {
    fn enqueue(&self, op: NavOp, what: &str) {
        let Some(tick) = self.state.try_with_mut(|s| {
            s.queue.push(op);
            s.tick
        }) else {
            return nav_disposed(what);
        };
        if tick.is_alive() {
            tick.update(|t| *t = t.wrapping_add(1));
        }
    }

    /// Shows `screen` on top of the current one with `anim` (a [`ScreenAnim`](crate::ScreenAnim)
    /// or a [`ScreenLoad`]); the current screen stays alive below.
    ///
    /// `screen` is any `FnOnce(Scope) -> V`: a `fn` item or a closure capturing what the screen
    /// needs (an id, a signal). It is called once, with the new screen's scope, when the push is
    /// applied. It is boxed once, when queued (no allocation for a `fn` item or a closure that
    /// captures nothing).
    pub fn push<V: View>(&self, screen: impl FnOnce(Scope) -> V + 'static, anim: impl Into<ScreenLoad>) {
        self.enqueue(
            NavOp::Push(Box::new(move |s| screen(s).into_any()), anim.into()),
            "push",
        );
    }

    /// Goes back to the previous screen with `anim`; the current one is deleted (and its scope
    /// disposed) when the animation ends. Returns `false` (doing nothing) at the root.
    pub fn pop(&self, anim: impl Into<ScreenLoad>) -> bool {
        if self.depth() <= 1 {
            twine_core::debug!(target: "twine::view", "navigator: pop at the root ignored");
            return false;
        }
        self.enqueue(NavOp::Pop(anim.into()), "pop");
        true
    }

    /// Replaces the current screen with `screen` (the old one is deleted after `anim`).
    /// `screen` is called once, like in [`push`](Self::push).
    pub fn replace<V: View>(&self, screen: impl FnOnce(Scope) -> V + 'static, anim: impl Into<ScreenLoad>) {
        self.enqueue(
            NavOp::Replace(Box::new(move |s| screen(s).into_any()), anim.into()),
            "replace",
        );
    }

    /// Number of screens, counting queued operations (0 once the navigator is disposed).
    #[must_use]
    pub fn depth(&self) -> usize {
        self.state
            .try_with(|s| {
                let mut d = s.stack.len();
                for op in &s.queue {
                    match op {
                        NavOp::Push(..) => d += 1,
                        NavOp::Pop(_) => d = d.saturating_sub(1).max(1),
                        NavOp::Replace(..) => {}
                    }
                }
                d
            })
            .unwrap_or(0)
    }

    /// Applies the queued operations (engine required).
    fn apply(&self, e: &mut Engine) {
        loop {
            // The borrow ends before the screen is built (user code may use the navigator).
            let Some(Some(op)) = self
                .state
                .try_with_mut(|s| (!s.queue.is_empty()).then(|| s.queue.remove(0)))
            else {
                break;
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
                        self.state.with_mut(|s| s.stack.push(entry));
                    }
                }
                NavOp::Pop(load) => {
                    let popped = self.state.with_mut(|s| {
                        if s.stack.len() <= 1 {
                            return None;
                        }
                        let top = s.stack.pop();
                        Some((top, s.stack.last().map(|x| (x.node, x.group))))
                    });
                    let Some((top, prev)) = popped else { continue };
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
                        let old = self.state.with_mut(|s| {
                            let old = s.stack.pop();
                            s.stack.push(Entry {
                                node: entry.node,
                                scope: entry.scope,
                                group: entry.group,
                            });
                            old
                        });
                        if let Some(g) = old.and_then(|o| o.group) {
                            e.delete_group(g);
                        }
                    }
                }
            }
            twine_core::debug!(target: "twine::view", "navigator: depth {}", self.state.with(|s| s.stack.len()));
        }
    }

    /// Builds a new screen from `f` (the screen's scope is disposed when it is deleted). With
    /// `own_group` the screen gets a focus group of its own, which becomes the default group
    /// and the keypads' and encoders' group.
    fn build_screen(&self, e: &mut Engine, f: ScreenFn, own_group: bool) -> Option<Entry> {
        let (parent, anchor) = self.state.with(|s| (s.scope, s.anchor));
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
        let root = {
            let mut bcx = BuildCx::new(e, screen, child);
            view.build(&mut bcx)
        };
        if let Some(a) = anchor.filter(|a| e.tree().contains(*a)) {
            e.set_style_parent(root, Some(a));
        }
        child.provide(StyleAnchor::new(root));
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
/// `initial` is any `FnOnce(Scope) -> V` (a `fn` item or a capturing closure), called once
/// with the screen's scope when the navigator is built.
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
pub fn navigator<V: View>(cx: Scope, initial: impl FnOnce(Scope) -> V + 'static) -> impl View {
    let nav = Navigator {
        state: cx.stored_value(NavState {
            scope: cx,
            stack: Vec::new(),
            queue: Vec::new(),
            tick: cx.signal(0),
            anchor: None,
        }),
    };
    cx.provide(nav);
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
        if anchor == twine_engine::DEAD_NODE {
            return anchor; // not created (reported): build no screens for it
        }
        let nav = self.nav;
        nav.state.with_mut(|s| s.anchor = Some(anchor));
        let e = cx.engine();
        if let Some(entry) = nav.build_screen(e, self.initial, false) {
            e.load_screen(entry.node);
            nav.state.with_mut(|s| s.stack.push(entry));
        }
        cx.on_delete(anchor, move || {
            // The navigator's scope may be gone already (its state with it).
            let stack = nav.state.try_with_mut(|s| core::mem::take(&mut s.stack));
            for entry in stack.into_iter().flatten() {
                entry.scope.dispose();
            }
        });
        let scope = cx.scope();
        let tick = nav.state.with(|s| s.tick);
        cx.provide(|| {
            scope.effect_with_cx(move |_| {
                tick.get();
                if nav.state.try_with(|s| s.queue.is_empty()).unwrap_or(true) {
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
/// ```
/// use twine_view::prelude::*;
///
/// fn details(cx: Scope) -> impl View {
///     let nav = use_navigator(cx); // inside the `navigator`'s pages
///     button(label("Back")).on_click(move || {
///         nav.pop(ScreenAnim::MoveRight(Duration::ms(300)));
///     })
/// }
/// # let _ = details;
/// ```
#[must_use]
pub fn use_navigator(cx: Scope) -> Navigator {
    cx.expect_context::<Navigator>()
}

/// A modal opened with [`ScopeExt::show_modal`](crate::ScopeExt::show_modal), also passed to
/// the modal's view closure (and provided as context in the modal's scope).
///
/// `Copy`, like a signal. Its state is owned by the scope that opened the modal; once that
/// scope is disposed, `close` does nothing, `is_open` is `false` and `node` is `None`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ModalHandle {
    open: Signal<bool>,
    state: StoredValue<ModalState>,
}

/// The engine side of an open modal.
#[derive(Default)]
struct ModalState {
    /// The backdrop node, once shown.
    node: Option<NodeId>,
    /// `(modal group, previous default group)` while open.
    groups: Option<(GroupId, Option<GroupId>)>,
}

impl ModalState {
    /// Takes the node and groups (to close the modal).
    fn take(&mut self) -> (Option<NodeId>, Option<(GroupId, Option<GroupId>)>) {
        (self.node.take(), self.groups.take())
    }
}

impl core::fmt::Debug for ModalHandle {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ModalHandle")
            .field("node", &self.node().map(fmt_node_id))
            .finish()
    }
}

impl ModalHandle {
    /// Closes the modal (disposes its scope, deletes its nodes, restores the focus group) at
    /// the next effect flush. Closing twice, or after the opening scope was disposed, does
    /// nothing.
    pub fn close(&self) {
        if self.open.is_alive() {
            self.open.set_if_changed(false);
        }
    }

    /// The backdrop node (once shown).
    #[must_use]
    pub fn node(&self) -> Option<NodeId> {
        self.state.try_with(|s| s.node).flatten()
    }

    /// Whether the modal is open.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open.try_get().unwrap_or(false)
    }
}

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
pub(crate) fn show_modal<V: View>(
    cx: Scope,
    view: impl FnOnce(Scope, ModalHandle) -> V + 'static,
) -> ModalHandle {
    let content = cx.child();
    let open = cx.signal(true);
    let state = cx.stored_value(ModalState::default());
    let handle = ModalHandle { open, state };
    // The views inside can close their modal (a message box's close button).
    content.provide(handle);
    let mut view = Some(view);
    cx.on_cleanup(move || {
        // Cleanups run before the scope's stored values are dropped.
        // Without the engine (disposed outside `Ui::update`): at the next update.
        if let Some((node, grp)) = state.try_with_mut(ModalState::take) {
            if EngineAccess::with(|e| close_modal(e, node, grp)).is_none() {
                crate::engine_queue::defer(cx, crate::engine_queue::EngineCmd::close_modal(node, grp));
            }
        }
    });
    cx.effect_with_cx(move |_| {
        let is_open = open.get();
        if !crate::access::engine_ready() {
            return;
        }
        if is_open {
            let Some(v) = view.take() else { return };
            let v = untrack(|| v(content, handle));
            EngineAccess::with(|e| {
                let Some(display) = display_of(content, e) else {
                    return;
                };
                let Some(top) = e.top_layer(display) else { return };
                let anchor = cx.use_context::<StyleAnchor>().and_then(|a| a.get());
                let Ok(backdrop) = e.create(top, Box::new(Obj)) else {
                    return;
                };
                for p in [
                    StyleProp::Width(Length::pct(100).into()),
                    StyleProp::Height(Length::pct(100).into()),
                    StyleProp::BgColor(Color::BLACK.into()),
                    StyleProp::BgOpacity(Opa::P50.into()),
                    StyleProp::BorderWidth(Length::Px(0).into()),
                    StyleProp::Radius(Radius::Px(0).into()),
                    StyleProp::PaddingTop(Length::Px(0).into()),
                    StyleProp::PaddingBottom(Length::Px(0).into()),
                    StyleProp::PaddingLeft(Length::Px(0).into()),
                    StyleProp::PaddingRight(Length::Px(0).into()),
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
                    state.with_mut(|s| s.groups = Some((g, prev)));
                }
                let root = {
                    let mut bcx = BuildCx::new(e, backdrop, content);
                    v.build(&mut bcx)
                };
                e.set_align(root, Align::Center);
                // The modal takes the text style of the view that opened it (a nested modal:
                // of the outer modal).
                if let Some(a) = anchor.filter(|a| e.tree().contains(*a)) {
                    e.set_style_parent(root, Some(a));
                }
                content.provide(StyleAnchor::new(root));
                if let Some((_, prev)) = state.with(|s| s.groups) {
                    e.set_default_group(prev);
                }
                state.with_mut(|s| s.node = Some(backdrop));
                twine_core::debug!(target: "twine::view", "modal {} shown", fmt_node_id(backdrop));
            });
        } else {
            let (node, grp) = state.with_mut(ModalState::take);
            EngineAccess::with(|e| {
                dispose_with(e, content);
                close_modal(e, node, grp);
            });
            dispose_current_effect();
        }
    });
    handle
}

/// Deletes the backdrop and the modal's focus group, re-attaching the inputs to the previous
/// group.
pub(crate) fn close_modal(e: &mut Engine, node: Option<NodeId>, groups: Option<(GroupId, Option<GroupId>)>) {
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
