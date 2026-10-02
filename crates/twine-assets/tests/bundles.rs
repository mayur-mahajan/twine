//! Font bundles (R3.S10): every bundle names existing fonts, `fonts-all` really is every font,
//! and the fonts of each bundle are there when it is enabled (the tests build with
//! `fonts-all`, so every bundle's `cfg` is on). That each bundle *alone* compiles for the
//! embedded targets is checked by `cargo xtask nostd` (facade sets with each bundle).

use std::collections::{BTreeMap, BTreeSet};

use twine_assets::fonts;

/// The `[features]` table of this crate's manifest: feature → enabled features.
fn features() -> BTreeMap<String, Vec<String>> {
    let manifest = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml")).unwrap();
    let section = manifest
        .split("\n[")
        .find(|s| s.starts_with("features]"))
        .expect("[features] table");
    let mut out = BTreeMap::new();
    let mut lines = section.lines().skip(1).peekable();
    while let Some(line) = lines.next() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (name, rest) = line.split_once(" = ").expect("feature = [..]");
        let mut list = rest.to_string();
        while !list.contains(']') {
            list.push_str(lines.next().expect("closing bracket"));
        }
        let items = list
            .trim_matches(|c| c == '[' || c == ']')
            .split(',')
            .map(|s| s.trim().trim_matches('"').to_string())
            .filter(|s| !s.is_empty())
            .collect();
        out.insert(name.to_string(), items);
    }
    out
}

/// Every font a feature enables, bundles expanded.
fn fonts_of(all: &BTreeMap<String, Vec<String>>, feature: &str) -> BTreeSet<String> {
    let items = &all[feature];
    if items.is_empty() {
        return BTreeSet::from([feature.to_string()]);
    }
    items.iter().flat_map(|f| fonts_of(all, f)).collect()
}

#[test]
fn bundles_name_existing_fonts_and_fonts_all_is_complete() {
    let all = features();
    let individual: BTreeSet<String> = all
        .iter()
        .filter(|(name, items)| items.is_empty() && *name != "default")
        .map(|(name, _)| name.clone())
        .collect();
    assert!(individual.len() >= 28, "{individual:?}");
    let bundles: Vec<&String> = all.keys().filter(|f| f.starts_with("fonts-")).collect();
    assert_eq!(bundles.len(), 7, "{bundles:?}");
    for bundle in &bundles {
        let fonts = fonts_of(&all, bundle);
        assert!(
            !fonts.is_empty() && fonts.is_subset(&individual),
            "{bundle}: {fonts:?}"
        );
    }
    assert_eq!(
        fonts_of(&all, "fonts-all"),
        individual,
        "`fonts-all` must enable every font"
    );
    for bundle in ["fonts-latin-small", "fonts-latin-medium", "fonts-latin-all"] {
        assert!(
            fonts_of(&all, bundle)
                .iter()
                .all(|f| f.starts_with("montserrat-")),
            "{bundle}"
        );
    }
    // Bundles nest: small ⊂ medium ⊂ all.
    assert!(fonts_of(&all, "fonts-latin-small").is_subset(&fonts_of(&all, "fonts-latin-medium")));
    assert!(fonts_of(&all, "fonts-latin-medium").is_subset(&fonts_of(&all, "fonts-latin-all")));
}

#[test]
fn bundle_fonts_are_compiled_in() {
    #[cfg(feature = "fonts-latin-small")]
    for f in [
        &fonts::MONTSERRAT_12,
        &fonts::MONTSERRAT_14,
        &fonts::MONTSERRAT_16,
    ] {
        assert!(f.line_height > 0);
    }
    #[cfg(feature = "fonts-latin-medium")]
    for f in [&fonts::MONTSERRAT_18, &fonts::MONTSERRAT_24] {
        assert!(f.line_height > 0);
    }
    #[cfg(feature = "fonts-latin-all")]
    for f in [
        &fonts::MONTSERRAT_8,
        &fonts::MONTSERRAT_48,
        &fonts::MONTSERRAT_14_SUBPX,
        &fonts::MONTSERRAT_16_LATIN_EXT,
    ] {
        assert!(f.line_height > 0);
    }
    #[cfg(feature = "fonts-mono")]
    for f in [&fonts::UNSCII_8, &fonts::UNSCII_16] {
        assert!(f.line_height > 0);
    }
    #[cfg(feature = "fonts-rtl")]
    assert!(fonts::DEJAVU_16_PERSIAN_HEBREW.line_height > 0);
    #[cfg(feature = "fonts-cjk")]
    for f in [
        &fonts::SOURCE_HAN_SANS_SC_14_CJK,
        &fonts::SOURCE_HAN_SANS_SC_16_CJK,
    ] {
        assert!(f.line_height > 0);
    }
}
