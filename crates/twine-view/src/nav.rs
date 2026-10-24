//! Modals ([`ModalHandle`], [`ScopeExt::show_modal`](crate::ScopeExt::show_modal)) and the
//! focus-group switching they share with the [`navigator`](crate::navigator()).

use alloc::boxed::Box;
use alloc::vec::Vec;

use twine_core::{Color, Opa};
use twine_engine::{Engine, GroupId, InputId, NodeId, Obj, ObjFlags, fmt_node_id};
use twine_reactive::{Scope, Signal, StoredValue};
use twine_style::{Align, Length, Radius, Selector, StyleProp};

use crate::access::EngineAccess;
use crate::build::BuildCx;
use crate::flow::dispose_with;
use crate::hooks::{StyleAnchor, display_of};
use crate::view::View;

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
        self.state.try_with(|s| s.node).ok().flatten()
    }

    /// Whether the modal is open.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open.try_get().ok().unwrap_or(false)
    }
}

/// Makes `to` the default group and the group of the inputs attached to `from`.
pub(crate) fn switch_group(e: &mut Engine, from: Option<GroupId>, to: Option<GroupId>) {
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
    let rt = cx.runtime();
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
        if let Ok((node, grp)) = state.try_with_mut(ModalState::take) {
            if EngineAccess::with(rt, |e| close_modal(e, node, grp)).is_none() {
                crate::engine_queue::defer(cx, crate::engine_queue::EngineCmd::close_modal(node, grp));
            }
        }
    });
    cx.effect_with_cx(move |_| {
        let is_open = open.get();
        if !crate::access::engine_ready(rt) {
            return;
        }
        if is_open {
            let Some(v) = view.take() else { return };
            let v = rt.untrack(|| v(content, handle));
            EngineAccess::with(rt, |e| {
                let Some(display) = display_of(content, e) else {
                    return;
                };
                let Some(top) = e.top_layer(display) else { return };
                let anchor = cx.use_context::<StyleAnchor>().ok().and_then(|a| a.get());
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
                    StyleProp::ShadowWidth(twine_style::Length::Px(0).into()),
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
            EngineAccess::with(rt, |e| {
                dispose_with(e, content);
                close_modal(e, node, grp);
            });
            rt.dispose_current_effect();
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
