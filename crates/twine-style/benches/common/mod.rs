//! Toy style trees shared by the benchmarks.

use std::hint::black_box;

use twine_core::{Color, Opa};
use twine_style::{
    EntryKind, Length, Part, PropId, Selector, State, StyleBuf, StyleDefaults, StyleEntry, StyleProp,
    StyleSource, StyleValue, resolve,
};

/// A node of the toy tree.
pub struct Node {
    /// Entries, highest priority first.
    pub entries: Vec<StyleEntry>,
    /// Current state.
    pub state: State,
    /// Parent index.
    pub parent: Option<usize>,
}

/// Nodes indexed by position.
#[derive(Default)]
pub struct Tree(pub Vec<Node>);

impl StyleSource for Tree {
    type Id = usize;
    fn entries(&self, id: usize) -> &[StyleEntry] {
        &self.0[id].entries
    }
    fn state(&self, id: usize) -> State {
        self.0[id].state
    }
    fn parent(&self, id: usize) -> Option<usize> {
        self.0[id].parent
    }
}

fn entry(kind: EntryKind, sel: Selector, props: &[StyleProp]) -> StyleEntry {
    StyleEntry::new(sel, props.iter().copied().collect::<StyleBuf>(), kind)
}

/// Defaults with the empty font.
pub const DEFAULTS: StyleDefaults = StyleDefaults {
    font: &twine_text::EMPTY_FONT,
};

/// A pressed button: local `bg_opa`, a `PRESSED` style and a theme style.
pub fn three_entries() -> Tree {
    Tree(vec![Node {
        entries: vec![
            entry(EntryKind::Local, Selector::MAIN, &[StyleProp::BgOpa(Opa::COVER)]),
            entry(
                EntryKind::Normal,
                Selector::state(State::PRESSED),
                &[StyleProp::BgColor(Color::hex(0x0000_0080))],
            ),
            entry(
                EntryKind::Theme,
                Selector::MAIN,
                &[StyleProp::BgColor(Color::BLUE), StyleProp::Radius(8)],
            ),
        ],
        state: State::PRESSED,
        parent: None,
    }])
}

/// A chain of 6 nodes; only the root sets `text_color`; the others have two unrelated styles.
pub fn depth_5() -> Tree {
    let mut t = Tree::default();
    t.0.push(Node {
        entries: vec![entry(
            EntryKind::Theme,
            Selector::MAIN,
            &[StyleProp::TextColor(Color::RED)],
        )],
        state: State::DEFAULT,
        parent: None,
    });
    for i in 1..6 {
        t.0.push(Node {
            entries: vec![
                entry(
                    EntryKind::Normal,
                    Selector::MAIN,
                    &[StyleProp::BgColor(Color::WHITE), StyleProp::Radius(4)],
                ),
                entry(
                    EntryKind::Theme,
                    Selector::MAIN,
                    &[StyleProp::PadTop(2), StyleProp::Width(Length::Px(10))],
                ),
            ],
            state: State::DEFAULT,
            parent: Some(i - 1),
        });
    }
    t
}

/// One node with 10 styles that only set sizes.
pub fn ten_entries() -> Tree {
    Tree(vec![Node {
        entries: (0..10)
            .map(|i| {
                entry(
                    EntryKind::Normal,
                    Selector::MAIN,
                    &[StyleProp::Width(Length::Px(i)), StyleProp::Height(Length::Px(i))],
                )
            })
            .collect(),
        state: State::DEFAULT,
        parent: None,
    }])
}

/// Resolves `BgColor` of `id`'s main part.
pub fn resolve_bg(t: &Tree, id: usize) -> StyleValue {
    resolve(
        black_box(t),
        black_box(id),
        Part::Main,
        PropId::BgColor,
        &DEFAULTS,
    )
}

/// Resolves `TextColor` of `id`'s main part.
pub fn resolve_text(t: &Tree, id: usize) -> StyleValue {
    resolve(
        black_box(t),
        black_box(id),
        Part::Main,
        PropId::TextColor,
        &DEFAULTS,
    )
}

/// Builds a `StyleBuf` with 20 `set` calls.
pub fn set_20_props() -> StyleBuf {
    let mut s = StyleBuf::new();
    for i in 0..20 {
        let id = PropId::ALL[(i * 5) % PropId::ALL.len()];
        let p = StyleProp::from_value(id, id.meta().default).unwrap_or(StyleProp::Radius(i as i32));
        s.set(black_box(p));
    }
    s
}
