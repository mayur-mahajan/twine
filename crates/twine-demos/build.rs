//! Generates the demos' assets into `OUT_DIR` with `twine-build` (nothing generated is
//! committed):
//!
//! - the images of the `controls` demo, converted from the PNGs in `assets/` (drawn by
//!   `assets/gen_frames.py`; `src/assets.rs` includes them);
//! - with feature `multilang`: the demo's translation catalog from `i18n/multilang/*.ftl`, and
//!   the fonts of its fallback chain subset to the characters the demo can show — its
//!   translations, the numbers they format and the literals of `src/multilang.rs`: the Latin
//!   extension of Montserrat 16, `DejaVu` Sans 16 for Hebrew, Arabic and Persian, and with
//!   `multilang-cjk` Source Han Sans 16 for Chinese and Japanese.

use std::path::Path;

use twine_build::image::{ColorFormat, Image};

/// The images: `(PNG in assets/, static)`, all ARGB8888.
const IMAGES: [(&str, &str); 6] = [
    ("frame0", "FRAME0"),
    ("frame1", "FRAME1"),
    ("frame2", "FRAME2"),
    ("frame3", "FRAME3"),
    ("power_off", "POWER_OFF"),
    ("power_on", "POWER_ON"),
];

fn main() {
    for (file, name) in IMAGES {
        Image::from_file(Path::new("assets").join(format!("{file}.png")))
            .format(ColorFormat::Argb8888)
            .name(name)
            .visibility("pub(crate)")
            .build()
            .and_then(|img| img.write_to_out_dir(&format!("{file}.rs")))
            .unwrap_or_else(|e| panic!("image {file}: {e}"));
    }
    #[cfg(feature = "multilang")]
    multilang::generate();
}

#[cfg(feature = "multilang")]
mod multilang {
    use std::collections::BTreeSet;
    use std::ops::RangeInclusive;

    use twine_build::chars::{CharSource, rust_source_chars};
    use twine_build::font::{BitmapFormat, Bpp, Font};
    use twine_build::i18n::Translations;

    /// The demo's source: its people cards and language names are string literals.
    const SOURCE: &str = "src/multilang.rs";

    /// What each subset font draws in the fallback chain, after Montserrat 16 (from
    /// `twine-assets`: ASCII, `°`, `•` and the symbols). Latin letters with accents, typographic
    /// quotes and dashes, the euro sign.
    const LATIN_EXT: &[RangeInclusive<char>] = &[
        '\u{A0}'..='\u{AC}',
        '\u{AE}'..='\u{17F}',
        '\u{2013}'..='\u{2014}',
        '\u{2018}'..='\u{201A}',
        '\u{201C}'..='\u{201E}',
        '\u{2026}'..='\u{2026}',
        '\u{20AC}'..='\u{20AC}',
    ];
    /// Hebrew, Arabic (Persian letters and both Arabic digit sets included) and the Arabic
    /// presentation forms the shaper draws.
    const RTL: &[RangeInclusive<char>] = &[
        '\u{0590}'..='\u{05FF}',
        '\u{0600}'..='\u{06FF}',
        '\u{FB50}'..='\u{FDFF}',
        '\u{FE70}'..='\u{FEFF}',
    ];
    /// CJK punctuation, kana, the unified ideographs and the full-width forms.
    #[cfg(feature = "multilang-cjk")]
    const CJK: &[RangeInclusive<char>] = &[
        '\u{3000}'..='\u{30FF}',
        '\u{4E00}'..='\u{9FFF}',
        '\u{FF01}'..='\u{FF5E}',
    ];

    /// The characters of `used` in `ranges`.
    fn within(used: &BTreeSet<char>, ranges: &[RangeInclusive<char>]) -> BTreeSet<char> {
        used.iter()
            .copied()
            .filter(|c| ranges.iter().any(|r| r.contains(c)))
            .collect()
    }

    /// A 16 px, 4 bpp, compressed font `name` from the workspace's `assets/fonts/<ttf>`, written
    /// to `OUT_DIR/<file>`. Every glyph is the one of the full font in `twine-assets`.
    fn font(ttf: &str, name: &str, chars: BTreeSet<char>, kerning: bool, file: &str) {
        Font::from_ttf(format!("../../assets/fonts/{ttf}"))
            .size(16)
            .bpp(Bpp::Four)
            .chars_from(chars)
            .format(BitmapFormat::Compressed)
            .kerning(kerning)
            .name(name)
            .visibility("pub(crate)")
            .build()
            .and_then(|f| f.write_to_out_dir(file))
            .unwrap_or_else(|e| panic!("font {name}: {e}"));
    }

    pub fn generate() {
        let catalog = Translations::from_dir("i18n/multilang")
            .default_language("en")
            .languages(["en", "de", "fr", "zh", "ja", "he", "ar", "fa"])
            .strict(true)
            .build()
            .unwrap_or_else(|e| panic!("multilang translations:\n{e}"));
        catalog
            .write_to_out_dir("multilang_i18n.rs")
            .unwrap_or_else(|e| panic!("{e}"));

        // Every character the demo can show: translations, their numbers, the source's texts.
        let mut used = BTreeSet::new();
        catalog.add_chars(&mut used);
        let (literals, files) =
            rust_source_chars(SOURCE.as_ref()).unwrap_or_else(|e| panic!("{SOURCE}: {e}"));
        used.extend(literals);
        for f in files {
            println!("cargo:rerun-if-changed={}", f.display());
        }

        font(
            "Montserrat-Medium.ttf",
            "MONTSERRAT_16_LATIN_EXT",
            within(&used, LATIN_EXT),
            false,
            "multilang_latin_ext.rs",
        );
        font(
            "DejaVuSans.ttf",
            "DEJAVU_16_RTL",
            within(&used, RTL),
            true,
            "multilang_rtl.rs",
        );
        #[cfg(feature = "multilang-cjk")]
        font(
            "SourceHanSansCN-Regular.otf",
            "SOURCE_HAN_SANS_16_CJK",
            within(&used, CJK),
            false,
            "multilang_cjk.rs",
        );
    }
}
