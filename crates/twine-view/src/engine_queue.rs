//! The per-`Ui` queue of engine commands issued while the engine is not lent.
//!
//! Scope cleanups (an animation stopped, a timer removed, a modal closed) and handle calls
//! ([`AnimController`] methods, [`ThemeHandle::set`](crate::ThemeHandle::set),
//! [`MotionHandle::set`](crate::MotionHandle::set)) act on the engine.
//! Where the engine is lent ([`EngineAccess`]) they act at once. Where it is not — a scope
//! disposed from the application's main loop, a handle called from outside `Ui::update`, or a
//! call while an enclosing [`EngineAccess::with`] holds the engine — they become an
//! [`EngineCmd`] in the queue of the `Ui` that owns the scope, and [`UiCore::update`] applies the
//! queue at the start of its next run. The queue is bounded (below): a command that does not
//! fit is dropped and reported as a fault, never silently lost.
//!
//! - **Owner:** each [`UiCore`] creates one queue and provides it as a context of its root
//!   scope, so a cleanup or handle finds the queue of *its* `Ui` from its scope (several `Ui`s
//!   may share one reactive runtime). The lookup happens only on the engine-less path.
//! - **Bounded, no allocation:** a ring of [`DEFAULT_ENGINE_QUEUE_CAPACITY`] slots (or
//!   [`UiBuilder::engine_queue_capacity`]) allocated once when the `Ui` is built; queuing never
//!   allocates. A full queue drops the command and the next update raises
//!   [`FaultKind::Capacity`](twine_core::fault::FaultKind::Capacity) with code
//!   [`CapacityFault::EngineQueue`] (one record, the number of dropped commands as occurrences);
//!   the commands already queued are applied. Never a panic.
//! - **Stale targets are ignored:** animation and timer ids are generational, a modal's backdrop
//!   is checked for existence, a controller of a disposed scope does nothing.
//! - **Cost:** one `Cell<bool>` read per update while the queue is empty.
//! - **No `Ui` left:** a scope disposed after its `Ui` was dropped has nothing to clean up (the
//!   root and everything below it were disposed with the `Ui`). A [`UiCore`] dropped without
//!   [`UiCore::dispose`] disposes its scopes without an engine: their engine commands are queued
//!   and then dropped with the queue, which logs `warn!` (the host keeps the engine, so pass it
//!   to `dispose`).
//!
//! [`UiCore`]: crate::UiCore
//! [`UiCore::update`]: crate::UiCore::update
//! [`UiCore::dispose`]: crate::UiCore::dispose
//! [`UiBuilder::engine_queue_capacity`]: crate::UiBuilder::engine_queue_capacity

use alloc::boxed::Box;
use alloc::rc::Rc;
use core::cell::{Cell, RefCell};

use twine_anim::{AnimId, Motion, TimerId};
use twine_engine::{DisplayId, Engine, GroupId, NodeId, ThemeHook, ThemeMode};
use twine_reactive::{Scope, UiWaker};

#[cfg(doc)]
use crate::access::EngineAccess;
use crate::error::CapacityFault;
use crate::hooks::{AnimController, AnimOp, UiContext};

/// Engine commands a `Ui` can hold while the engine is not lent (see the module docs). Sized
/// for scopes disposed outside `Ui::update` with a few animations, timers and modals each.
// NOTE(R5.S03): moves into `Limits` with the other capacity bounds.
pub const DEFAULT_ENGINE_QUEUE_CAPACITY: usize = 16;

/// An engine side effect deferred to the next update of its `Ui`: what to do (a plain `fn`)
/// and what to do it on.
///
/// A `fn` pointer per kind rather than one `match` over every kind: only the kinds an
/// application can queue are linked into its firmware (a counter without modals, animations
/// or theme switches links none of their code here).
pub(crate) struct EngineCmd {
    run: fn(&mut Engine, DisplayId, CmdArg),
    arg: CmdArg,
}

/// What an [`EngineCmd`] acts on.
pub(crate) enum CmdArg {
    /// An animation (stopped).
    Anim(AnimId),
    /// A timer (removed).
    Timer(TimerId),
    /// A modal's backdrop and `(modal group, previous default group)` (deleted, restored).
    Modal(Option<NodeId>, Option<(GroupId, Option<GroupId>)>),
    /// An [`AnimController`] and the method called.
    Ctl(AnimController, AnimOp),
    /// A theme (installed on the `Ui`'s display).
    Theme(Rc<dyn ThemeHook>),
    /// A theme mode (switched on the `Ui`'s display).
    Mode(ThemeMode),
    /// A motion preference (set on the engine).
    Motion(Motion),
}

impl EngineCmd {
    /// Stops animation `id` (scope cleanup of a tween or an animation).
    #[inline]
    pub(crate) fn anim_stop(id: AnimId) -> EngineCmd {
        EngineCmd {
            run: |e, _, arg| {
                if let CmdArg::Anim(id) = arg {
                    e.anim_stop(id);
                }
            },
            arg: CmdArg::Anim(id),
        }
    }

    /// Removes timer `t` (scope cleanup of an interval or a timeout).
    #[inline]
    pub(crate) fn timer_remove(t: TimerId) -> EngineCmd {
        EngineCmd {
            run: |e, _, arg| {
                if let CmdArg::Timer(t) = arg {
                    e.timer_remove(t);
                }
            },
            arg: CmdArg::Timer(t),
        }
    }

    /// Deletes a modal's backdrop and focus group (scope cleanup of a modal).
    #[inline]
    pub(crate) fn close_modal(node: Option<NodeId>, groups: Option<(GroupId, Option<GroupId>)>) -> EngineCmd {
        EngineCmd {
            run: |e, _, arg| {
                if let CmdArg::Modal(node, groups) = arg {
                    crate::nav::close_modal(e, node, groups);
                }
            },
            arg: CmdArg::Modal(node, groups),
        }
    }

    /// Calls an [`AnimController`] method.
    #[inline]
    pub(crate) fn anim(ctl: AnimController, op: AnimOp) -> EngineCmd {
        EngineCmd {
            run: |e, _, arg| {
                if let CmdArg::Ctl(ctl, op) = arg {
                    ctl.apply(e, op);
                }
            },
            arg: CmdArg::Ctl(ctl, op),
        }
    }

    /// Installs `theme` on the `Ui`'s display ([`ThemeHandle::set`](crate::ThemeHandle::set)).
    #[inline]
    pub(crate) fn set_theme(theme: Rc<dyn ThemeHook>) -> EngineCmd {
        EngineCmd {
            run: |e, display, arg| {
                if let CmdArg::Theme(t) = arg {
                    e.set_theme(display, t);
                }
            },
            arg: CmdArg::Theme(theme),
        }
    }

    /// Switches the theme mode of the `Ui`'s display
    /// ([`ThemeHandle::set_mode`](crate::ThemeHandle::set_mode)).
    #[inline]
    pub(crate) fn set_theme_mode(mode: ThemeMode) -> EngineCmd {
        EngineCmd {
            run: |e, display, arg| {
                if let CmdArg::Mode(m) = arg {
                    e.set_theme_mode(display, m);
                }
            },
            arg: CmdArg::Mode(mode),
        }
    }

    /// Sets the engine's motion preference ([`MotionHandle::set`](crate::MotionHandle::set)).
    #[inline]
    pub(crate) fn set_motion(motion: Motion) -> EngineCmd {
        EngineCmd {
            run: |e, _, arg| {
                if let CmdArg::Motion(m) = arg {
                    e.set_motion(m);
                }
            },
            arg: CmdArg::Motion(motion),
        }
    }
}

/// The queue of one `Ui`: a fixed ring (allocated once), the number of commands dropped
/// because it was full, and the `Ui`'s waker (woken on every push, so a sleeping loop runs the
/// update that applies it).
pub(crate) struct EngineQueue {
    slots: RefCell<Box<[Option<EngineCmd>]>>,
    head: Cell<usize>,
    len: Cell<usize>,
    /// Commands dropped since the last update (queue full).
    dropped: Cell<u32>,
    /// Something to do at the next update (commands or dropped commands to report).
    pending: Cell<bool>,
    waker: Cell<Option<&'static UiWaker>>,
}

/// `capacity` empty slots (one allocation).
fn slots(capacity: usize) -> Box<[Option<EngineCmd>]> {
    (0..capacity).map(|_| None).collect()
}

impl EngineQueue {
    /// An empty queue of `capacity` slots.
    pub(crate) fn new(capacity: usize) -> Rc<EngineQueue> {
        Rc::new(EngineQueue {
            slots: RefCell::new(slots(capacity)),
            head: Cell::new(0),
            len: Cell::new(0),
            dropped: Cell::new(0),
            pending: Cell::new(false),
            waker: Cell::new(None),
        })
    }

    /// Whether the next update has something to apply or report. The only steady-state cost.
    #[inline]
    pub(crate) fn is_pending(&self) -> bool {
        self.pending.get()
    }

    /// Sets the waker woken by [`push`](Self::push).
    pub(crate) fn set_waker(&self, w: &'static UiWaker) {
        self.waker.set(Some(w));
    }

    /// Queued commands.
    pub(crate) fn len(&self) -> usize {
        self.len.get()
    }

    /// Resizes the ring to `capacity` slots (an allocation, at configuration time), keeping
    /// the queued commands; commands beyond the new capacity are dropped and reported like a
    /// full queue.
    pub(crate) fn set_capacity(&self, capacity: usize) {
        let mut new = slots(capacity);
        let mut kept = 0;
        while let Some(cmd) = self.pop() {
            if let Some(slot) = new.get_mut(kept) {
                *slot = Some(cmd);
                kept += 1;
            } else {
                self.dropped.set(self.dropped.get().saturating_add(1));
                self.pending.set(true);
            }
        }
        let old = self.slots.replace(new);
        self.head.set(0);
        self.len.set(kept);
        self.pending.set(self.pending.get() || kept > 0);
        drop(old);
    }

    /// Queues `cmd` (dropped and counted when the queue is full) and wakes the `Ui`.
    fn push(&self, cmd: EngineCmd) {
        let rejected = match self.slots.try_borrow_mut() {
            Ok(mut slots) if self.len.get() < slots.len() => {
                let mut i = self.head.get() + self.len.get();
                if i >= slots.len() {
                    i -= slots.len();
                }
                slots[i] = Some(cmd);
                self.len.set(self.len.get() + 1);
                None
            }
            _ => Some(cmd),
        };
        if let Some(cmd) = rejected {
            self.dropped.set(self.dropped.get().saturating_add(1));
            twine_core::error!(
                target: "twine::view",
                "engine command queue full ({} commands): command dropped (raise UiBuilder::engine_queue_capacity)",
                self.len.get()
            );
            drop(cmd); // outside the borrow: a theme's `Drop` is user code
        }
        self.pending.set(true);
        if let Some(w) = self.waker.get() {
            w.wake();
        }
    }

    /// Takes the oldest command (the ring is not borrowed when it is returned).
    fn pop(&self) -> Option<EngineCmd> {
        if self.len.get() == 0 {
            return None;
        }
        let mut slots = self.slots.try_borrow_mut().ok()?;
        let head = self.head.get();
        let cmd = slots.get_mut(head).and_then(Option::take);
        let next = head + 1;
        self.head.set(if next >= slots.len() { 0 } else { next });
        self.len.set(self.len.get() - 1);
        cmd
    }

    /// Applies every queued command in order, then reports dropped commands as one
    /// [`FaultKind::Capacity`](twine_core::fault::FaultKind::Capacity) record. Commands queued
    /// while applying (none in practice: the engine is at hand) are applied too.
    #[cold]
    #[inline(never)]
    pub(crate) fn apply(&self, e: &mut Engine, display: DisplayId) {
        // A target that no longer exists is ignored by each command (generational ids, existence
        // checks).
        while let Some(cmd) = self.pop() {
            (cmd.run)(e, display, cmd.arg);
        }
        self.pending.set(false);
        let dropped = self.dropped.replace(0);
        if dropped > 0 {
            e.raise_fault(
                twine_engine::FaultRecord::new(twine_core::fault::FaultKind::Capacity)
                    .display(display)
                    .code(CapacityFault::EngineQueue.code())
                    .occurrences(dropped),
            );
        }
    }
}

/// Queues `cmd` on the `Ui` that owns `cx`: the engine-less path of every engine side effect of
/// the view layer. A scope outside any `Ui` (a bare reactive root) has no engine to apply it to:
/// the command is dropped with a `warn!`.
#[cold]
#[inline(never)]
pub(crate) fn defer(cx: Scope, cmd: EngineCmd) {
    if let Some(ui) = cx.use_context::<UiContext>() {
        ui.queue.push(cmd);
    } else {
        twine_core::warn!(
            target: "twine::view",
            "engine command for a scope outside any Ui: dropped (no engine to apply it to)"
        );
        drop(cmd);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stop() -> EngineCmd {
        EngineCmd::anim_stop(AnimId::DANGLING)
    }

    #[test]
    fn ring_wraps_and_counts_overflow() {
        let q = EngineQueue::new(3);
        assert!(!q.is_pending());
        for _ in 0..4 {
            q.push(stop());
            q.push(stop());
            assert!(q.is_pending());
            assert_eq!(q.len(), 2);
            assert!(q.pop().is_some());
            assert!(q.pop().is_some());
            assert!(q.pop().is_none());
        }
        for _ in 0..5 {
            q.push(stop());
        }
        assert_eq!((q.len(), q.dropped.get()), (3, 2));
    }

    #[test]
    fn set_capacity_keeps_queued_commands() {
        let q = EngineQueue::new(2);
        q.push(stop());
        q.push(stop());
        q.set_capacity(4);
        q.push(stop());
        assert_eq!((q.len(), q.dropped.get()), (3, 0));
        q.set_capacity(1);
        assert_eq!((q.len(), q.dropped.get()), (1, 2));
        assert!(q.is_pending());
    }

    #[test]
    fn zero_capacity_drops_and_reports() {
        let q = EngineQueue::new(0);
        q.push(stop());
        assert_eq!((q.len(), q.dropped.get()), (0, 1));
        assert!(q.is_pending());
    }
}
