//! `cargo xtask theme-size`: the flash each theme occupies in the example firmware, attributed
//! by symbol (R7.S01; design `docs/plan/r7-theming-design.md` §2.9.7).
//!
//! Builds the [`IMAGES`] (`firmware/<example>` with one demo each), reads each image's symbol table and attributes every sized symbol
//! that belongs to a theme to a [`Part`] of it. The table it prints is the R7 baseline that
//! later steps (and the phase gate) compare against.
//!
//! What a symbol belongs to is decided from its demangled name: a symbol of `twine_theme`, or
//! a generic of another crate instantiated by `twine_theme` (its mangled name names the crate),
//! belongs to the theme of its module (`twine_theme::lvgl` → `Lvgl`, `mono`, `simple`;
//! anything else of the crate → `shared`). Font data (`twine_assets::fonts::*`) is reported
//! per image in its own column: today a font is linked by the theme's `FontScale` or by the
//! application, which the symbol table cannot tell apart. The engine's theme mechanism
//! (`twine_engine::theme_hook`, the view's theme state) is reported separately: every themed
//! image has it, whichever theme it uses. Only named symbols are attributed: constants LLVM
//! merged into anonymous symbols, and code inlined into a caller of another crate, count for
//! that symbol instead.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::cmd::firmware::{
    ElfSymbol, Example, build_example, elf_path, elf_symbols, examples, missing_prerequisite,
};
use crate::util::R;

/// `(firmware example, image name, feature set)` of every image measured: the three demos on
/// the default SPI panel with the LVGL theme, and the counter on the mono OLED with the
/// monochrome theme.
pub const IMAGES: &[(&str, &str, &str)] = &[
    ("rp2040", "counter", "panel-ili9341,touch-xpt2046,demo-counter"),
    ("rp2040", "controls", "panel-ili9341,touch-xpt2046,demo-controls"),
    (
        "rp2040",
        "selection",
        "panel-ili9341,touch-xpt2046,demo-selection",
    ),
    ("rp2040", "counter (OLED)", "oled-ssd1306,demo-counter"),
    ("stm32f411", "counter", "panel-ili9341,touch-xpt2046,demo-counter"),
    (
        "stm32f411",
        "controls",
        "panel-ili9341,touch-xpt2046,demo-controls",
    ),
    (
        "stm32f411",
        "selection",
        "panel-ili9341,touch-xpt2046,demo-selection",
    ),
    ("stm32f411", "counter (OLED)", "oled-ssd1306,demo-counter"),
];

/// The part of a theme a symbol is attributed to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Part {
    /// The styles (`static` style data since R7.S04) and the per-display metrics table
    /// (`lvgl::metrics`).
    Styles,
    /// Choosing the styles of a class (`apply`, `recipe`, the per-class recipes and their
    /// functions).
    ClassChain,
    /// Design-element tables and palettes (`design`, colour and palette helpers).
    Tables,
    /// Icons the theme references (the check mark).
    Icons,
    /// Motion: transitions and their curves.
    Curves,
    /// Anything else of the theme (constructors, name, modes, audit, drop glue).
    Other,
}

impl Part {
    /// Every part, in table order.
    pub const ALL: [Part; 6] = [
        Part::Styles,
        Part::ClassChain,
        Part::Tables,
        Part::Icons,
        Part::Curves,
        Part::Other,
    ];

    /// The column title.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Part::Styles => "styles",
            Part::ClassChain => "class chain",
            Part::Tables => "tables",
            Part::Icons => "icons",
            Part::Curves => "curves",
            Part::Other => "other",
        }
    }
}

/// What a symbol is, for the report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Owner {
    /// A part of a theme (`Lvgl`, `Mono`, `Simple`, or `shared` for the crate's common code).
    Theme(&'static str, Part),
    /// Font data or code of a font of `twine_assets::fonts` (the font's module name).
    Font(String),
    /// The engine's and the view's theme mechanism.
    Engine,
}

/// Attributes a symbol (`None`: not theme-related).
#[must_use]
pub fn classify(s: &ElfSymbol) -> Option<Owner> {
    let n = s.name.as_str();
    if let Some(rest) = n.split("twine_assets::fonts::").nth(1) {
        let font = rest.split("::").next().unwrap_or(rest);
        return Some(Owner::Font(font.to_string()));
    }
    // `11twine_theme`: the crate in a v0 or legacy mangled name (path, type argument or
    // instantiating crate).
    if n.contains("twine_theme::") || s.raw.contains("11twine_theme") {
        let theme = if n.contains("twine_theme::lvgl::") {
            "Lvgl"
        } else if n.contains("twine_theme::mono::") {
            "Mono"
        } else if n.contains("twine_theme::simple::") {
            "Simple"
        } else {
            "shared"
        };
        return Some(Owner::Theme(theme, part_of(n)));
    }
    // Methods of `Engine` in its `theme_hook` module demangle as `<…::Engine>::…`: the module
    // is in the mangled path only.
    if n.contains("twine_engine::theme_hook::")
        || s.raw.contains("12twine_engine10theme_hook")
        || n.contains("ThemeState")
        || n.contains("track_theme")
    {
        return Some(Owner::Engine);
    }
    None
}

/// Whether `n` is a function or static directly in a theme module (`twine_theme::lvgl::slider`,
/// `twine_theme::mono::BUTTON`, …): the per-class recipes and their functions.
fn per_class(n: &str) -> bool {
    [
        "twine_theme::lvgl::",
        "twine_theme::mono::",
        "twine_theme::simple::",
    ]
    .iter()
    .filter_map(|m| n.split(m).nth(1))
    .any(|rest| {
        let item = rest.split(['<', ' ', '>']).next().unwrap_or(rest);
        !item.contains("::")
            && !matches!(
                item,
                "LIGHT" | "DARK" | "NIGHT" | "HIGH_CONTRAST" | "MODES" | "THEME" | "CHECK_MARK"
            )
            && !item.starts_with("MOTION")
    })
}

/// The [`Part`] of a theme symbol, from its demangled name.
fn part_of(n: &str) -> Part {
    let has = |p: &[&str]| p.iter().any(|x| n.contains(x));
    if has(&["CHECK_MARK", "::icons::", "Symbol"]) {
        Part::Icons
    } else if has(&["::TRANS_", "Transition", "::motion::", "Spring"]) {
        Part::Curves
    } else if has(&[
        "ThemeHook>::design",
        "::colors::",
        "::palette::",
        "ElementTable",
        "Palette",
        "ModeColors",
        "Set>::table",
        "::LIGHT",
        "::DARK",
        "::NIGHT",
        "::HIGH_CONTRAST",
        "::modes::",
    ]) {
        Part::Tables
    } else if has(&["::styles::", "StyleBuf", "Styles", "::metrics::"]) {
        Part::Styles
    } else if has(&[
        "ThemeHook>::apply",
        "ThemeHook>::recipe",
        "recipe_of",
        "ComputedRecipe",
    ]) || per_class(n)
    {
        Part::ClassChain
    } else {
        Part::Other
    }
}

/// The theme flash of one image.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ThemeSize {
    /// Bytes per `(theme, part)`.
    pub parts: BTreeMap<(&'static str, Part), u64>,
    /// Bytes per font.
    pub fonts: BTreeMap<String, u64>,
    /// Bytes of the engine's theme mechanism.
    pub engine: u64,
}

impl ThemeSize {
    /// Attributes the flash symbols of an image.
    #[must_use]
    pub fn of(symbols: &[ElfSymbol]) -> Self {
        let mut t = ThemeSize::default();
        for s in symbols.iter().filter(|s| s.flash) {
            match classify(s) {
                Some(Owner::Theme(theme, part)) => *t.parts.entry((theme, part)).or_default() += s.size,
                Some(Owner::Font(f)) => *t.fonts.entry(f).or_default() += s.size,
                Some(Owner::Engine) => t.engine += s.size,
                None => {}
            }
        }
        t
    }

    /// Bytes of `part` over every theme.
    #[must_use]
    pub fn part(&self, part: Part) -> u64 {
        self.parts
            .iter()
            .filter(|((_, p), _)| *p == part)
            .map(|(_, b)| b)
            .sum()
    }

    /// Bytes of every theme part.
    #[must_use]
    pub fn total(&self) -> u64 {
        self.parts.values().sum()
    }

    /// The themes present, with their bytes.
    #[must_use]
    pub fn themes(&self) -> BTreeMap<&'static str, u64> {
        let mut m = BTreeMap::new();
        for ((t, _), b) in &self.parts {
            *m.entry(*t).or_default() += b;
        }
        m
    }
}

/// One measured image.
#[derive(Debug, Clone)]
pub struct Row {
    /// `example image` (e.g. `rp2040 counter`).
    pub image: String,
    /// The attribution.
    pub size: ThemeSize,
    /// The image's flash (text + data).
    pub flash: u64,
}

/// The Markdown table of the rows.
#[must_use]
pub fn table(rows: &[Row]) -> String {
    let mut s = String::from("| Image | Themes |");
    for p in Part::ALL {
        let _ = write!(s, " {} |", p.title());
    }
    s.push_str(" theme total | fonts | engine theme support | image flash |\n|---|---|");
    for _ in 0..Part::ALL.len() + 4 {
        s.push_str("---:|");
    }
    s.push('\n');
    for r in rows {
        let themes: Vec<String> = r.size.themes().iter().map(|(t, b)| format!("{t} {b}")).collect();
        let _ = write!(s, "| {} | {} |", r.image, themes.join(", "));
        for p in Part::ALL {
            let _ = write!(s, " {} |", r.size.part(p));
        }
        let fonts: Vec<String> = r.size.fonts.iter().map(|(f, b)| format!("{f} {b}")).collect();
        let _ = writeln!(
            s,
            " {} | {} | {} | {} |",
            r.size.total(),
            if fonts.is_empty() {
                "–".into()
            } else {
                fonts.join(", ")
            },
            r.size.engine,
            r.flash
        );
    }
    s.push_str("\nBytes of named symbols (`cargo xtask theme-size`).\n");
    s
}

/// Builds the images and prints the table.
pub fn run() -> R {
    let all = examples()?;
    let mut rows = Vec::new();
    for (name, image, features) in IMAGES {
        let ex: &Example = all
            .iter()
            .find(|e| e.name == *name)
            .ok_or_else(|| format!("theme-size: no firmware example `{name}`"))?;
        if let Some(reason) = missing_prerequisite(ex, &[]) {
            return Err(format!("theme-size: {name}: {reason}").into());
        }
        let one = Example {
            feature_sets: vec![(*features).to_string()],
            ..ex.clone()
        };
        let built = build_example(&one, &[])?;
        let flash = built.first().map_or(0, |r| r.sizes.flash());
        let path = elf_path(&one);
        let elf = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        rows.push(Row {
            image: format!("{name} {image}"),
            size: ThemeSize::of(&elf_symbols(&elf)?),
            flash,
        });
    }
    println!("{}", table(&rows));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sym(name: &str, raw: &str, size: u64) -> ElfSymbol {
        ElfSymbol {
            name: name.into(),
            raw: raw.into(),
            size,
            flash: true,
        }
    }

    #[test]
    fn classifies_theme_parts_fonts_and_engine() {
        let cases = [
            (
                "twine_theme::lvgl::styles::CARD",
                Some(Owner::Theme("Lvgl", Part::Styles)),
            ),
            (
                "twine_theme::lvgl::metrics::table",
                Some(Owner::Theme("Lvgl", Part::Styles)),
            ),
            (
                "twine_theme::lvgl::slider",
                Some(Owner::Theme("Lvgl", Part::ClassChain)),
            ),
            (
                "twine_theme::lvgl::BUTTON",
                Some(Owner::Theme("Lvgl", Part::ClassChain)),
            ),
            (
                "twine_theme::lvgl::LIGHT",
                Some(Owner::Theme("Lvgl", Part::Tables)),
            ),
            (
                "<twine_theme::mono::Mono as twine_engine::theme_hook::ThemeHook>::apply",
                Some(Owner::Theme("Mono", Part::ClassChain)),
            ),
            (
                "twine_theme::lvgl::styles::CHECK_MARK",
                Some(Owner::Theme("Lvgl", Part::Icons)),
            ),
            (
                "twine_theme::lvgl::styles::TRANS_NORMAL",
                Some(Owner::Theme("Lvgl", Part::Curves)),
            ),
            (
                "twine_assets::fonts::montserrat_14::MONTSERRAT_14_BITMAP",
                Some(Owner::Font("montserrat_14".into())),
            ),
            (
                "<twine_engine::theme_hook::ThemedWalk>::next",
                Some(Owner::Engine),
            ),
            ("<twine_render::painter::Painter>::rect", None),
        ];
        for (name, want) in cases {
            assert_eq!(classify(&sym(name, name, 1)), want, "{name}");
        }
        // A generic of another crate instantiated by the theme crate.
        let generic = sym(
            "<twine_style::style::StyleBuf>::padding::<i32>",
            "_RINvMs2_NtCs9eo7QQOkPts_11twine_style9shorthandNtNtB8_5style8StyleBuf7paddinglECsiLCkRq3apFI_11twine_theme",
            106,
        );
        assert_eq!(classify(&generic), Some(Owner::Theme("shared", Part::Styles)));
        // A method of `Engine` in the engine's `theme_hook` module.
        let engine = sym(REPLACE_THEME.0, REPLACE_THEME.1, 810);
        assert_eq!(classify(&engine), Some(Owner::Engine));
    }

    /// `(demangled, raw)` of `Engine::replace_theme`.
    const REPLACE_THEME: (&str, &str) = (
        "<twine_engine::engine::Engine>::replace_theme",
        "_RNvMs5_NtCs5RVGM0BCmv4_12twine_engine10theme_hookNtNtB7_6engine6Engine13replace_theme",
    );

    #[test]
    fn sums_and_table() {
        let syms = [
            sym("twine_theme::lvgl::styles::CARD", "", 6000),
            sym("twine_theme::lvgl::button", "", 5000),
            sym(
                "twine_assets::fonts::montserrat_14::MONTSERRAT_14_BITMAP",
                "",
                8000,
            ),
            sym(REPLACE_THEME.0, REPLACE_THEME.1, 800),
            ElfSymbol {
                flash: false,
                ..sym("twine_theme::lvgl::RAM_ONLY", "", 99)
            },
        ];
        let t = ThemeSize::of(&syms);
        assert_eq!((t.total(), t.part(Part::Styles), t.engine), (11_000, 6000, 800));
        assert_eq!(t.fonts.get("montserrat_14"), Some(&8000));
        let md = table(&[Row {
            image: "rp2040 counter".into(),
            size: t,
            flash: 100_000,
        }]);
        assert!(
            md.contains("| rp2040 counter | Lvgl 11000 | 6000 | 5000 | 0 | 0 | 0 | 0 | 11000 | montserrat_14 8000 | 800 | 100000 |"),
            "{md}"
        );
    }

    #[test]
    fn images_name_existing_examples_and_demos() {
        let all = examples().unwrap();
        for (name, _, features) in IMAGES {
            let ex = all
                .iter()
                .find(|e| e.name == *name)
                .unwrap_or_else(|| panic!("{name}"));
            let manifest = std::fs::read_to_string(ex.dir.join("Cargo.toml")).unwrap();
            for f in features.split(',') {
                assert!(manifest.contains(&format!("\n{f} =")), "{name}: no feature {f}");
            }
        }
    }
}
