//! Many languages on one screen (the idea of LVGL's `demos/multilang`): a scrolling list of
//! people cards in English, German, French, Chinese, Japanese, Hebrew, Arabic and Persian.
//!
//! - Every card sets its own base direction: Hebrew, Arabic and Persian cards are laid out
//!   right to left (avatar on the right, right-aligned text, reordered mixed text) and Arabic
//!   and Persian are shaped (features `bidi`, `arabic-shaping` of the facade).
//! - The header is translated with [`tr!`](twine_extra::tr): the language dropdown (or the
//!   ▶ button) switches every header text and the header's direction without rebuilding.
//! - Fonts are a fallback chain: Montserrat 16 (ASCII) → Montserrat 16 Latin extension
//!   (accents) → `DejaVu` 16 (Hebrew, Arabic, Persian) → Source Han Sans 16 (Chinese, Japanese;
//!   feature `multilang-cjk`, which boards with little flash leave out) → an image font with
//!   three emoji drawn at compile time. The heading is a runtime TrueType font with the
//!   `ttf` feature, else Montserrat 20.
//! - The English avatar is read from a file (`A:/avatars/en.qoi` on a memory file system),
//!   the others are QOI files embedded in flash; all are decoded once into the image cache.
//!
//! Without `multilang-cjk` the Chinese and Japanese cards say that CJK fonts are disabled.

use alloc::vec;

use twine::core::ColorFormat;
use twine::fs::{MemoryFs, Vfs};
use twine::prelude::*;
use twine::render::ImagePixels;
use twine::text::ImageFontProvider;
use twine::view::EngineAccess;
use twine_extra::i18n::{Translations, provide_i18n};
use twine_extra::{tr, translations};

/// Language codes, in the order of [`LANGUAGE_NAMES`] and the translation table.
pub const CODES: [&str; 8] = ["en", "de", "fr", "zh", "ja", "he", "ar", "fa"];

/// The dropdown options: every language in its own script.
pub const LANGUAGE_NAMES: [&str; 8] = if cfg!(feature = "multilang-cjk") {
    [
        "English",
        "Deutsch",
        "Français",
        "中文",
        "日本語",
        "עברית",
        "العربية",
        "فارسی",
    ]
} else {
    [
        "English",
        "Deutsch",
        "Français",
        "Chinese",
        "Japanese",
        "עברית",
        "العربية",
        "فارسی",
    ]
};

/// Base direction per language.
const DIRS: [BaseDir; 8] = [
    BaseDir::Ltr,
    BaseDir::Ltr,
    BaseDir::Ltr,
    BaseDir::Ltr,
    BaseDir::Ltr,
    BaseDir::Rtl,
    BaseDir::Rtl,
    BaseDir::Rtl,
];

/// The header texts.
pub static TEXTS: Translations = translations! {
    langs: ["en", "de", "fr", "zh", "ja", "he", "ar", "fa"],
    "people" => [
        "People", "Menschen", "Personnes", "人們", "人々", "אנשים", "أشخاص", "افراد",
    ],
    "title" => [
        "Twine speaks your language",
        "Twine spricht deine Sprache",
        "Twine parle ta langue",
        "Twine 說你的語言",
        "Twine はあなたの言葉を話す",
        "Twine מדבר בשפה שלך",
        "Twine يتحدث لغتك",
        "Twine به زبان شما حرف میزند",
    ],
};

/// One card.
struct Person {
    name: &'static str,
    about: &'static str,
    avatar: &'static [u8],
}

static PEOPLE: [Person; 8] = [
    Person {
        name: "Emma Johnson",
        about: "Loves hiking and building tiny gadgets 🌍",
        avatar: include_bytes!("../../../assets/images/avatars/en.qoi"),
    },
    Person {
        name: "Lukas Müller",
        about: "Fährt gern Rad und backt sonntags Brötchen.",
        avatar: include_bytes!("../../../assets/images/avatars/de.qoi"),
    },
    Person {
        name: "Chloé Martin",
        about: "Adore la pâtisserie, les échecs et le café ❤",
        avatar: include_bytes!("../../../assets/images/avatars/fr.qoi"),
    },
    Person {
        name: if cfg!(feature = "multilang-cjk") {
            "林明"
        } else {
            "Lin Ming"
        },
        about: if cfg!(feature = "multilang-cjk") {
            "我喜歡看電影，也愛做菜。"
        } else {
            "(CJK fonts disabled on this board)"
        },
        avatar: include_bytes!("../../../assets/images/avatars/zh.qoi"),
    },
    Person {
        name: if cfg!(feature = "multilang-cjk") {
            "高橋 花子"
        } else {
            "Takahashi Hanako"
        },
        about: if cfg!(feature = "multilang-cjk") {
            "音楽と料理が大好きです 😀"
        } else {
            "(CJK fonts disabled on this board)"
        },
        avatar: include_bytes!("../../../assets/images/avatars/ja.qoi"),
    },
    Person {
        name: "נועה כהן",
        about: "אוהבת לצייר ולשחות בים, 3 פעמים בשבוע.",
        avatar: include_bytes!("../../../assets/images/avatars/he.qoi"),
    },
    Person {
        name: "أحمد علي",
        about: "يحب القراءة وكرة القدم (Real Madrid).",
        avatar: include_bytes!("../../../assets/images/avatars/ar.qoi"),
    },
    Person {
        name: "سارا رضایی",
        about: "عاشق موسیقی و کوهنوردی است.",
        avatar: include_bytes!("../../../assets/images/avatars/fa.qoi"),
    },
];

/// The file read through the file system (`A:`).
static FILES: &[(&str, &[u8])] = &[(
    "avatars/en.qoi",
    include_bytes!("../../../assets/images/avatars/en.qoi"),
)];

/// Path of the English avatar.
pub const AVATAR_FILE: &str = "A:/avatars/en.qoi";

/// Image cache budget: the eight 40 × 40 avatars decoded (6.4 KB each) fit.
const IMAGE_CACHE_BYTES: usize = 8 * 40 * 40 * 4;

// ---- Fonts ---------------------------------------------------------------------------------

/// Side of the emoji images.
const EMOJI_PX: usize = 16;

/// One 16 × 16 ARGB8888 emoji drawn at compile time (integer geometry in half pixels from the
/// center): 0 = 😀, 1 = ❤, 2 = 🌍.
const fn emoji(kind: u8) -> [u8; EMOJI_PX * EMOJI_PX * 4] {
    let mut px = [0u8; EMOJI_PX * EMOJI_PX * 4];
    let mut y = 0;
    while y < EMOJI_PX {
        let mut x = 0;
        while x < EMOJI_PX {
            let (dx, dy) = (2 * x as i32 - 15, 2 * y as i32 - 15);
            let d2 = dx * dx + dy * dy;
            let rgb: u32 = match kind {
                0 => {
                    let eye = (dx - 5) * (dx - 5) + (dy + 4) * (dy + 4) <= 6
                        || (dx + 5) * (dx + 5) + (dy + 4) * (dy + 4) <= 6;
                    let mouth = dy > 2 && d2 >= 49 && d2 <= 90;
                    if d2 > 225 {
                        0
                    } else if eye || mouth {
                        0x0063_3A00
                    } else {
                        0x00FF_C928
                    }
                }
                1 => {
                    let lobes = (dx - 6) * (dx - 6) + (dy + 4) * (dy + 4) <= 56
                        || (dx + 6) * (dx + 6) + (dy + 4) * (dy + 4) <= 56;
                    let tip = dy >= -4 && dx.abs() + dy <= 12;
                    if lobes || tip { 0x00E5_3935 } else { 0 }
                }
                _ => {
                    let land = (dx + 4) * (dx + 4) + (dy + 5) * (dy + 5) <= 30
                        || (dx - 6) * (dx - 6) + (dy - 5) * (dy - 5) <= 24
                        || (dx - 8) * (dx - 8) + (dy + 7) * (dy + 7) <= 8;
                    if d2 > 225 {
                        0
                    } else if land {
                        0x0043_A047
                    } else {
                        0x001E_88E5
                    }
                }
            };
            if rgb != 0 {
                let i = (y * EMOJI_PX + x) * 4;
                px[i] = rgb as u8;
                px[i + 1] = (rgb >> 8) as u8;
                px[i + 2] = (rgb >> 16) as u8;
                px[i + 3] = 0xFF;
            }
            x += 1;
        }
        y += 1;
    }
    px
}

static SMILE_PX: [u8; EMOJI_PX * EMOJI_PX * 4] = emoji(0);
static HEART_PX: [u8; EMOJI_PX * EMOJI_PX * 4] = emoji(1);
static GLOBE_PX: [u8; EMOJI_PX * EMOJI_PX * 4] = emoji(2);

const fn emoji_pixels(data: &'static [u8]) -> ImagePixels<'static> {
    ImagePixels {
        format: ColorFormat::Argb8888,
        w: EMOJI_PX as u16,
        h: EMOJI_PX as u16,
        stride: EMOJI_PX as u16 * 4,
        data,
        palette: None,
        alpha: None,
        premultiplied: false,
    }
}

static SMILE: ImagePixels<'static> = emoji_pixels(&SMILE_PX);
static HEART: ImagePixels<'static> = emoji_pixels(&HEART_PX);
static GLOBE: ImagePixels<'static> = emoji_pixels(&GLOBE_PX);

/// The emoji of the image font (drawn 2 px below the baseline, like text descenders).
fn emoji_lookup(cp: char, _next: Option<char>) -> Option<(&'static ImagePixels<'static>, i16)> {
    match cp {
        '😀' => Some((&SMILE, -2)),
        '❤' => Some((&HEART, -2)),
        '🌍' => Some((&GLOBE, -2)),
        _ => None,
    }
}

static EMOJI_PROVIDER: ImageFontProvider = ImageFontProvider {
    lookup: emoji_lookup,
    line_height: 18,
};
static EMOJI: Font = EMOJI_PROVIDER.font(None);

#[cfg(feature = "multilang-cjk")]
static CJK: Font = Font {
    fallback: Some(&EMOJI),
    ..fonts::SOURCE_HAN_SANS_SC_16_CJK
};

/// After the RTL scripts: CJK when enabled, then the emoji.
#[cfg(feature = "multilang-cjk")]
const AFTER_RTL: &Font = &CJK;
#[cfg(not(feature = "multilang-cjk"))]
const AFTER_RTL: &Font = &EMOJI;

static RTL: Font = Font {
    fallback: Some(AFTER_RTL),
    ..fonts::DEJAVU_16_PERSIAN_HEBREW
};

static LATIN: Font = Font {
    fallback: Some(&RTL),
    ..fonts::MONTSERRAT_16_LATIN_EXT
};

/// The text font of the demo: Montserrat 16 and its fallback chain (see the module docs).
pub static TEXT_FONT: Font = Font {
    fallback: Some(&LATIN),
    ..fonts::MONTSERRAT_16
};

/// The heading font without a TTF: Montserrat 20, then the chain of [`TEXT_FONT`].
static HEADING_BITMAP: Font = Font {
    fallback: Some(&LATIN),
    ..fonts::MONTSERRAT_20
};

/// The heading font: Montserrat Medium rendered at run time (feature `ttf`, parsed once),
/// else the Montserrat 20 bitmap font.
fn heading_font() -> &'static Font {
    #[cfg(feature = "ttf")]
    {
        static MONTSERRAT_TTF: &[u8] = include_bytes!("../../../assets/fonts/Montserrat-Medium.ttf");
        match twine::text::TtfFont::new_with_fallback(MONTSERRAT_TTF, 22, Some(&LATIN)) {
            Ok(f) => return f,
            Err(e) => twine::core::warn!(target: "twine::demo", "multilang: TTF heading: {:?}", e),
        }
    }
    &HEADING_BITMAP
}

// ---- Views ---------------------------------------------------------------------------------

/// Mounts the memory file system as `A:` and sizes the image cache (once per engine).
fn setup_engine(cx: Scope) {
    let done = EngineAccess::with(cx, |e| {
        let mut vfs = Vfs::new();
        if let Err(err) = vfs.mount('A', alloc::boxed::Box::new(MemoryFs::new(FILES))) {
            twine::core::warn!(target: "twine::demo", "multilang: mount A: failed: {}", err);
        }
        e.set_vfs(vfs);
        e.set_image_cache_budget(IMAGE_CACHE_BYTES);
    });
    if done.is_none() {
        twine::core::warn!(target: "twine::demo", "multilang: no engine while building (file avatar unavailable)");
    }
}

/// The avatar of card `i`: from the file system for English, from flash for the others.
fn avatar(i: usize) -> ImageSource {
    if i == 0 {
        if let Some(src) = ImageSource::file(AVATAR_FILE) {
            return src;
        }
    }
    ImageSource::Encoded(PEOPLE[i].avatar)
}

/// Height of a card row of the list.
const ROW_H: i32 = 78;

/// Card `i`: avatar, name and description, in the card's language direction.
fn card(i: usize) -> impl View {
    let p = &PEOPLE[i];
    container(
        row((
            image(avatar(i)).size(40, 40),
            column((
                label(p.name).text_color(Color::hex(0x0015_65C0)).fill_width(),
                label(p.about).fill_width(),
            ))
            .flex_grow(1)
            .gap(2),
        ))
        .gap(10)
        .fill_width()
        .align_items(CrossAlign::Center),
    )
    .base_dir(DIRS[i])
    .fill_width()
    .height(ROW_H - 6)
    .padding(8)
    .test_id(CODES[i])
}

/// The multilang demo.
///
/// ```
/// use twine_testing::{TestUi, by_id};
///
/// let mut t = TestUi::new(480, 320).mount(twine_demos::multilang::app);
/// t.run_until_idle();
/// assert_eq!(t.find(by_id("title")).text(), "Twine speaks your language");
/// t.find(by_id("next")).click();
/// t.run_until_idle();
/// assert_eq!(t.find(by_id("title")).text(), "Twine spricht deine Sprache");
/// ```
pub fn app(cx: Scope) -> impl View {
    setup_engine(cx);
    let i18n = provide_i18n(cx, &TEXTS, CODES[0]);
    let lang = cx.signal(0usize);
    let list = cx.node_ref::<twine::engine::Obj>();
    cx.effect(move || {
        let i = lang.get() % CODES.len();
        i18n.set_language(CODES[i]);
        // Show the card of the chosen language (a no-op while the list is being built).
        let y = i32::try_from(i).unwrap_or(0) * ROW_H;
        if list.get_untracked().is_none() {
            return;
        }
        list.with_mut(|_, wcx| {
            let node = wcx.node();
            wcx.engine_mut().scroll_to_y(node, y, false);
        });
    });
    let dir = move || DIRS[lang.get() % DIRS.len()];
    column((
        row((
            label(tr!("title"))
                .font(heading_font())
                .flex_grow(1)
                .test_id("title"),
            dropdown(LANGUAGE_NAMES, lang).width(130).test_id("language"),
            button(label(Symbol::Right))
                .on_click(move || lang.update(|l| *l = (*l + 1) % CODES.len()))
                .test_id("next"),
        ))
        .base_dir(dir)
        .fill_width()
        .gap(8)
        .align_items(CrossAlign::Center),
        label(tr!("people")).base_dir(dir).fill_width().test_id("people"),
        virtual_list(|| PEOPLE.len(), ROW_H, |_cx, i| card(i))
            .node_ref(list)
            .fill_width()
            .flex_grow(1),
    ))
    .font(&TEXT_FONT)
    .fill()
    .padding(8)
    .gap(6)
}

/// The languages as `(code, native name)` pairs, for tests and scripts.
#[must_use]
pub fn languages() -> alloc::vec::Vec<(&'static str, &'static str)> {
    let mut v = vec![];
    for (code, name) in CODES.iter().zip(LANGUAGE_NAMES) {
        v.push((*code, name));
    }
    v
}
