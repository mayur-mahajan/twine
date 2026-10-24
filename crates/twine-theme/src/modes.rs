//! The modes a built-in theme lists: its `'static` table per [`ThemeMode`], fixed at compile
//! time (`Lvgl::new().modes(..)`, `Mono::new().modes(..)`, `Simple::new().modes(..)`).

use twine_style::ThemeMode;
use twine_style::design::ElementTable;

/// A theme's mode tables and the list of their modes (in order, each once). `Copy`, built in
/// `const` context; reading is a short scan, never while drawing.
#[derive(Clone, Copy)]
pub(crate) struct ModeList {
    tables: &'static [(ThemeMode, &'static ElementTable)],
    list: [ThemeMode; 4],
    len: usize,
}

impl ModeList {
    /// The modes of `tables`, in order; a mode listed again is ignored (its first table
    /// counts). Never panics.
    pub(crate) const fn new(tables: &'static [(ThemeMode, &'static ElementTable)]) -> Self {
        let mut list = [ThemeMode::Light; 4];
        let mut len = 0;
        let mut rest = tables;
        while let [(mode, _), tail @ ..] = rest {
            let mut seen = false;
            if let Some((listed, free)) = list.split_at_mut_checked(len) {
                let mut l: &[ThemeMode] = listed;
                while let [m, more @ ..] = l {
                    seen |= *m as u8 == *mode as u8;
                    l = more;
                }
                // At most four distinct modes: a new one always finds a free slot.
                if let (false, Some(slot)) = (seen, free.first_mut()) {
                    *slot = *mode;
                    len = len.saturating_add(1);
                }
            }
            rest = tail;
        }
        Self { tables, list, len }
    }

    /// The listed modes, in order.
    pub(crate) fn modes(&self) -> &[ThemeMode] {
        self.list.get(..self.len).unwrap_or(&[])
    }

    /// The table of `mode` (`None` if it is not listed).
    pub(crate) fn design(&self, mode: ThemeMode) -> Option<&'static ElementTable> {
        self.tables.iter().find(|&&(m, _)| m == mode).map(|&(_, t)| t)
    }

    /// The listed tables.
    pub(crate) const fn tables(&self) -> &'static [(ThemeMode, &'static ElementTable)] {
        self.tables
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static T: ElementTable = ElementTable::new();

    #[test]
    fn lists_each_mode_once_in_order() {
        static TABLES: [(ThemeMode, &ElementTable); 3] = [
            (ThemeMode::Dark, &T),
            (ThemeMode::Light, &T),
            (ThemeMode::Dark, &T),
        ];
        let l = ModeList::new(&TABLES);
        assert_eq!(l.modes(), &[ThemeMode::Dark, ThemeMode::Light]);
        assert!(l.design(ThemeMode::Night).is_none());
        assert!(l.design(ThemeMode::Light).is_some());
    }
}
