//! `style_resolve`: a console walk-through of style precedence (no window).
//!
//! Builds a two-node tree — a screen with a theme style and a button with theme, normal, local
//! and transition entries — and prints, for several button states, the resolved value of a few
//! properties and which entry won and why.
//!
//! ```text
//! cargo xtask sim style_resolve
//! RUST_LOG=twine::style=trace cargo xtask sim style_resolve   # also log every examined entry
//! ```

use std::fmt::Write as _;

use twine_assets::fonts::MONTSERRAT_14;
use twine_core::{Color, Opa};
use twine_style::{
    EntryKind, EntryTrace, Part, PropId, Selector, State, Style, StyleDefaults, StyleEntry, StyleSource,
    StyleValue, TraceEvent, resolve, resolve_traced, style,
};

const DARK_BLUE: Color = Color::hex(0x00_0080);

static SCREEN_THEME: Style = style! { bg_color: Color::WHITE, text_color: Color::BLACK };
static BUTTON_THEME: Style = style! { bg_color: Color::BLUE, radius: 8 };
static BUTTON_PRESSED: Style = style! { bg_color: DARK_BLUE };
static BUTTON_CHECKED: Style = style! { bg_color: Color::GREEN, radius: 12 };
static BUTTON_LOCAL: Style = style! { bg_opa: Opa::COVER };
/// The current value of a running `bg_opa` transition.
static BUTTON_TRANSITION: Style = style! { bg_opa: Opa(128) };

struct Node {
    name: &'static str,
    entries: Vec<StyleEntry>,
    state: State,
    parent: Option<usize>,
}

struct Tree(Vec<Node>);

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

fn states(s: State) -> String {
    if s.is_empty() {
        return "DEFAULT".into();
    }
    s.iter_names().map(|(n, _)| n).collect::<Vec<_>>().join("|")
}

fn value(v: &StyleValue) -> String {
    match v {
        StyleValue::Font(f) if std::ptr::eq(*f, std::ptr::addr_of!(MONTSERRAT_14)) => {
            "MONTSERRAT_14 (theme default)".into()
        }
        v => v.to_string(),
    }
}

/// One entry of the compact trace; entries without the property are left out.
fn describe(tree: &Tree, node: usize, e: &EntryTrace, winner: Option<(EntryKind, u16)>) -> Option<String> {
    if !e.had_prop {
        return None;
    }
    let who = if e.depth == 0 {
        String::new()
    } else {
        format!("{}:", tree.0[node].name)
    };
    let sel = if e.kind == EntryKind::Transition {
        format!("{:?}(part {:?})", e.kind, e.selector.part)
    } else {
        format!("{:?}({}, w={:#x})", e.kind, states(e.selector.state), e.weight)
    };
    let why = if e.chosen {
        "chosen".to_string()
    } else if !e.matched {
        "not matched".to_string()
    } else {
        match winner {
            Some((EntryKind::Transition, _)) => "matched, a transition wins first".into(),
            Some((_, w)) if e.weight < w => "matched, lower weight".into(),
            Some(_) => "matched, same weight but lower priority".into(),
            None => "matched".into(),
        }
    };
    Some(format!("{who}entry#{} {sel} {why}", e.entry_index))
}

fn trace_line(tree: &Tree, id: usize, prop: PropId, defaults: StyleDefaults) -> (StyleValue, String) {
    let mut events = Vec::new();
    let v = resolve_traced(tree, id, Part::Main, prop, &defaults, &mut |e| events.push(e));
    assert_eq!(
        v,
        resolve(tree, id, Part::Main, prop, &defaults),
        "traced and plain resolution agree"
    );
    let mut node = id;
    let mut parts = Vec::new();
    // Kind and weight of the chosen entry per node (depth), to explain the losers.
    let winner = |depth: u16| {
        events.iter().find_map(|e| match e {
            TraceEvent::Entry(t) if t.depth == depth && t.chosen => Some((t.kind, t.weight)),
            _ => None,
        })
    };
    for e in &events {
        match e {
            TraceEvent::Entry(t) => {
                if let Some(s) = describe(tree, node, t, winner(t.depth)) {
                    parts.push(s);
                }
            }
            TraceEvent::Inherited { node: n, part, depth } => {
                node = *n;
                parts.push(format!(
                    "inherit → {} {:?} (depth {depth})",
                    tree.0[*n].name, part
                ));
            }
            TraceEvent::Default(_) => parts.push("default".into()),
        }
    }
    (v, parts.join("; "))
}

fn main() {
    env_logger::init();
    let mut tree = Tree(vec![
        Node {
            name: "screen",
            entries: vec![StyleEntry::new(Selector::MAIN, &SCREEN_THEME, EntryKind::Theme)],
            state: State::DEFAULT,
            parent: None,
        },
        Node {
            name: "button",
            // Priority order: transitions, local, normal (latest added first), theme.
            entries: vec![
                StyleEntry::new(Selector::MAIN, &BUTTON_TRANSITION, EntryKind::Transition),
                StyleEntry::new(Selector::MAIN, &BUTTON_LOCAL, EntryKind::Local),
                StyleEntry::new(
                    Selector::state(State::PRESSED),
                    &BUTTON_PRESSED,
                    EntryKind::Normal,
                ),
                StyleEntry::new(
                    Selector::state(State::CHECKED),
                    &BUTTON_CHECKED,
                    EntryKind::Normal,
                ),
                StyleEntry::new(Selector::MAIN, &BUTTON_THEME, EntryKind::Theme),
            ],
            state: State::DEFAULT,
            parent: Some(0),
        },
    ]);
    let defaults = StyleDefaults { font: &MONTSERRAT_14 };
    let button = 1;

    println!("button entries (highest priority first):");
    for (i, e) in tree.0[button].entries.iter().enumerate() {
        let props: Vec<_> = PropId::ALL
            .iter()
            .filter(|p| e.style.get(**p).is_some())
            .map(|p| p.name())
            .collect();
        println!(
            "  #{i} {:?} {} → {}",
            e.kind,
            states(e.selector.state),
            props.join(", ")
        );
    }
    println!("screen entries: #0 Theme DEFAULT → BgColor, TextColor");

    let props = [
        PropId::BgColor,
        PropId::BgOpa,
        PropId::Radius,
        PropId::TextColor,
        PropId::TextFont,
    ];
    let mut summary = String::new();
    for state in [
        State::DEFAULT,
        State::PRESSED,
        State::PRESSED | State::CHECKED,
        State::DISABLED,
    ] {
        tree.0[button].state = state;
        println!("\n=== button @{} ===", states(state));
        for prop in props {
            let (v, line) = trace_line(&tree, button, prop, defaults);
            println!("  {:<10} = {}", prop.name(), value(&v));
            println!("      {} @{}: {line}", prop.name(), states(state));
            if prop == PropId::BgColor {
                let _ = write!(summary, "{}={} ", states(state), v);
            }
        }
    }

    // The documented outcomes (the example doubles as a smoke test).
    tree.0[button].state = State::PRESSED | State::CHECKED;
    let bg = resolve(&tree, button, Part::Main, PropId::BgColor, &defaults);
    assert_eq!(
        bg,
        StyleValue::Color(DARK_BLUE),
        "PRESSED (0x80) outweighs CHECKED (0x04)"
    );
    assert_eq!(
        resolve(&tree, button, Part::Main, PropId::Radius, &defaults),
        StyleValue::Int(12)
    );
    assert_eq!(
        resolve(&tree, button, Part::Main, PropId::BgOpa, &defaults),
        StyleValue::Opa(Opa(128))
    );
    tree.0[button].state = State::DISABLED;
    assert_eq!(
        resolve(&tree, button, Part::Main, PropId::BgColor, &defaults),
        StyleValue::Color(Color::BLUE)
    );
    println!("\nBgColor per state: {}", summary.trim_end());
}
