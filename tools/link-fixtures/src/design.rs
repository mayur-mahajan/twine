//! A small design language, shipped like a theme crate ships one (R7.S03; design
//! `docs/plan/r7-theming-design.md` §2.9.3–4): two mode tables and three recipes as separate
//! `static`s, plus [`ALL_RECIPES`] as a convenience. An application's `DesignSystem` lists the
//! ones it uses; the `design_system` fixture lists the light table and two recipes, and must
//! link neither [`DARK`] nor [`SLIDER`].

use twine::engine::{OBJ_CLASS, Recipe};
use twine::prelude::*;
use twine::widgets::button::BUTTON_CLASS;
use twine::widgets::slider::SLIDER_CLASS;

/// The light mode's elements.
pub static LIGHT: design::ElementTable = design::ElementTable::new()
    .color(design::SURFACE, Color::WHITE)
    .color(design::PRIMARY, Color::hex(0x0000_66CC))
    .color(design::ON_PRIMARY, Color::WHITE);

/// The dark mode's elements (not listed by the fixture).
pub static DARK: design::ElementTable = design::ElementTable::new()
    .color(design::SURFACE, Color::hex(0x0011_1111))
    .color(design::PRIMARY, Color::hex(0x0066_AAFF))
    .color(design::ON_PRIMARY, Color::BLACK);

recipe! {
    /// Screens and containers.
    pub static SCREEN for OBJ_CLASS {
        Selector::MAIN => { bg: design::SURFACE },
    }

    /// Buttons (and their variants, by lineage).
    pub static BUTTON for BUTTON_CLASS {
        Selector::MAIN => { bg: design::PRIMARY, text_color: design::ON_PRIMARY, radius: 8, padding: 6 },
        State::PRESSED => { bg_opacity: Opa::pct(80) },
    }

    /// Sliders (not listed by the fixture).
    pub static SLIDER for SLIDER_CLASS {
        Selector::MAIN => { bg: design::SURFACE, radius: Radius::Circle },
        Part::Indicator => { bg: design::PRIMARY },
        Part::Knob => { bg: design::ON_PRIMARY, radius: Radius::Circle },
    }
}

/// Every recipe of the language (a convenience for images that show every widget; a `const`,
/// so it links nothing unless named).
pub const ALL_RECIPES: &[&Recipe] = &[&SCREEN, &BUTTON, &SLIDER];
