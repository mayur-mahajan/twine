//! Presses outside a node: [`Engine::on_outside_press`].
//!
//! Popups (a dropdown's option list, menus opened on the top layer) close when the user
//! presses anywhere else. LVGL does it per widget (defocus, `LEAVE`); the engine offers one
//! facility instead: a one-shot callback that runs when a pointer press starts outside the
//! watched node's subtree.

use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::{DisplayId, Engine, NodeId, fmt_node_id};

/// The callback of [`Engine::on_outside_press`]: the engine and the node the press went to
/// (`None` when it hit nothing clickable).
pub(crate) type OutsideCb = Box<dyn FnOnce(&mut Engine, Option<NodeId>)>;

impl Engine {
    /// Runs `cb` once, when the next pointer press on `node`'s display starts outside `node`
    /// and its descendants (including presses that hit nothing). `cb` receives the node the
    /// press goes to; it runs before that node receives `Pressed`, so it may delete `node`
    /// (a popup closing itself). Deleting `node` drops the callback unused.
    ///
    /// ```
    /// use core::cell::Cell;
    /// use std::rc::Rc;
    /// use twine_core::Point;
    /// use twine_engine::Obj;
    /// use twine_testing::EngineHarness;
    ///
    /// let mut h = EngineHarness::new(100, 100);
    /// let screen = h.screen();
    /// let popup = h.engine_mut().create(screen, Box::new(Obj)).unwrap();
    /// h.engine_mut().set_size(popup, 20, 20);
    /// h.run_until_idle();
    /// let hits = Rc::new(Cell::new(0));
    /// let c = hits.clone();
    /// h.engine_mut().on_outside_press(popup, move |_e, _pressed| c.set(c.get() + 1));
    /// h.tap(Point::new(5, 5)); // inside: nothing
    /// assert_eq!(hits.get(), 0);
    /// h.tap(Point::new(60, 60)); // outside: runs once
    /// h.tap(Point::new(60, 60));
    /// assert_eq!(hits.get(), 1);
    /// ```
    pub fn on_outside_press(&mut self, node: NodeId, cb: impl FnOnce(&mut Engine, Option<NodeId>) + 'static) {
        if !self.tree.contains(node) {
            twine_core::warn!(target: "twine::input", "on_outside_press: node {} not found", fmt_node_id(node));
            return;
        }
        twine_core::trace!(target: "twine::input", "outside press watch on {}", fmt_node_id(node));
        self.outside_presses.push((node, Box::new(cb)));
    }

    /// The number of pending [`on_outside_press`](Self::on_outside_press) callbacks.
    #[must_use]
    pub fn outside_press_count(&self) -> usize {
        self.outside_presses.len()
    }

    /// A pointer press started on `display` and goes to `pressed`: runs the callbacks of the
    /// watched nodes of that display that `pressed` is not inside.
    pub(crate) fn fire_outside_presses(&mut self, display: DisplayId, pressed: Option<NodeId>) {
        if self.outside_presses.is_empty() {
            return;
        }
        let watches = core::mem::take(&mut self.outside_presses);
        let mut keep: Vec<(NodeId, OutsideCb)> = Vec::new();
        let mut due: Vec<OutsideCb> = Vec::new();
        for (node, cb) in watches {
            if !self.tree.contains(node) {
                continue;
            }
            let inside = pressed.is_some_and(|p| p == node || self.tree.ancestors(p).any(|a| a == node));
            if inside || self.display_of(node) != Some(display) {
                keep.push((node, cb));
            } else {
                twine_core::debug!(target: "twine::input", "press outside {}", fmt_node_id(node));
                due.push(cb);
            }
        }
        self.outside_presses = keep;
        for cb in due {
            cb(self, pressed.filter(|p| self.tree.contains(*p)));
        }
    }

    /// Drops the watches of deleted nodes.
    pub(crate) fn forget_outside_presses(&mut self) {
        if !self.outside_presses.is_empty() {
            let tree = &self.tree;
            self.outside_presses.retain(|(n, _)| tree.contains(*n));
        }
    }
}
