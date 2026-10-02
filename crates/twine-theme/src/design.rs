//! The design element tables of the built-in themes: built once per mode (and display class)
//! on first use, with the application's elements and overrides layered on top, then shared.

use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::RefCell;

use twine_style::ThemeMode;
use twine_style::design::{Element, ElementTable, ElementType};

/// The application's design elements of a theme (`.element(..)`, `.element_in(..)`) and the
/// tables built so far, keyed by `K` (what the theme's values depend on besides the mode).
pub(crate) struct DesignTables<K> {
    /// Values for every mode.
    all: ElementTable,
    /// Values for one mode (applied after `all`).
    per_mode: Vec<(ThemeMode, ElementTable)>,
    cache: RefCell<Vec<(K, ThemeMode, Rc<ElementTable>)>>,
}

impl<K: Copy + PartialEq> DesignTables<K> {
    pub(crate) fn new() -> Self {
        Self {
            all: ElementTable::new(),
            per_mode: Vec::new(),
            cache: RefCell::new(Vec::new()),
        }
    }

    /// Sets `element` to `value` in `mode` (`None`: every mode). Drops the built tables.
    pub(crate) fn set<T: ElementType>(&mut self, mode: Option<ThemeMode>, element: Element<T>, value: T) {
        match mode {
            None => self.all.set(element, value),
            Some(m) => {
                if let Some((_, t)) = self.per_mode.iter_mut().find(|(x, _)| *x == m) {
                    t.set(element, value);
                } else {
                    self.per_mode.push((m, ElementTable::new().with(element, value)));
                }
            }
        }
        self.cache.get_mut().clear();
    }

    /// The table for `key` and `mode`: built by `build` (the theme's values) on first use, with
    /// the application's values layered on top, then shared (no allocation afterwards).
    pub(crate) fn get(
        &self,
        key: K,
        mode: ThemeMode,
        build: impl FnOnce() -> ElementTable,
    ) -> Rc<ElementTable> {
        if let Some((_, _, t)) = self
            .cache
            .borrow()
            .iter()
            .find(|(k, m, _)| *k == key && *m == mode)
        {
            return t.clone();
        }
        let mut t = build();
        t.overlay(&self.all);
        if let Some((_, o)) = self.per_mode.iter().find(|(m, _)| *m == mode) {
            t.overlay(o);
        }
        let t = Rc::new(t);
        self.cache.borrow_mut().push((key, mode, t.clone()));
        t
    }
}
