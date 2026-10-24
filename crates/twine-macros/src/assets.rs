//! `include_font!` and `include_image!` (feature `assets`): the generators of `twine-build`
//! run while the crate compiles, and the macro expands to the generated statics.

use std::path::PathBuf;

use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{ToTokens, quote};
use syn::parse::{Parse, ParseStream};
use syn::spanned::Spanned;
use syn::{Attribute, Expr, ExprLit, ExprRange, Ident, Lit, LitStr, RangeLimits, Token, Visibility};
use twine_build::font::{BitmapFormat, Bpp, Font, Gamma, Hinting, Subpx};
use twine_build::image::{Color, Compression, FORMAT_NAMES, Image};

/// `#[attrs] vis static NAME = "path", key = value, …`
pub(crate) struct AssetInput {
    attrs: Vec<Attribute>,
    vis: Visibility,
    name: Ident,
    path: LitStr,
    options: Vec<(Ident, Expr)>,
}

impl Parse for AssetInput {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let attrs = input.call(Attribute::parse_outer)?;
        let vis: Visibility = input.parse()?;
        input.parse::<Token![static]>()?;
        let name: Ident = input.parse()?;
        input.parse::<Token![=]>()?;
        let path: LitStr = input.parse()?;
        let mut options = Vec::new();
        while input.parse::<Option<Token![,]>>()?.is_some() {
            if input.is_empty() {
                break;
            }
            let key: Ident = input.parse()?;
            input.parse::<Token![=]>()?;
            let value: Expr = input.parse()?;
            options.push((key, value));
        }
        if !input.is_empty() {
            return Err(input.error("expected `, key = value` or the end of the macro input"));
        }
        Ok(AssetInput {
            attrs,
            vis,
            name,
            path,
            options,
        })
    }
}

fn err<T>(span: Span, msg: impl std::fmt::Display) -> syn::Result<T> {
    Err(syn::Error::new(span, msg))
}

fn lit(e: &Expr) -> Option<&Lit> {
    match e {
        Expr::Lit(ExprLit { lit, .. }) => Some(lit),
        Expr::Group(g) => lit(&g.expr),
        _ => None,
    }
}

fn int<T: std::str::FromStr>(e: &Expr) -> syn::Result<T>
where
    T::Err: std::fmt::Display,
{
    match lit(e) {
        Some(Lit::Int(i)) => i.base10_parse(),
        _ => err(e.span(), "expected an integer"),
    }
}

fn boolean(e: &Expr) -> syn::Result<bool> {
    match lit(e) {
        Some(Lit::Bool(b)) => Ok(b.value),
        _ => err(e.span(), "expected `true` or `false`"),
    }
}

fn string(e: &Expr) -> syn::Result<String> {
    match lit(e) {
        Some(Lit::Str(s)) => Ok(s.value()),
        _ => err(e.span(), "expected a string literal"),
    }
}

fn character(e: &Expr) -> syn::Result<char> {
    match lit(e) {
        Some(Lit::Char(c)) => Ok(c.value()),
        _ => err(e.span(), "expected a character literal"),
    }
}

/// A path's last identifier (`Compressed`, `BitmapFormat::Compressed`).
fn variant(e: &Expr) -> syn::Result<String> {
    match e {
        Expr::Path(p) => match p.path.segments.last() {
            Some(s) => Ok(s.ident.to_string()),
            None => err(e.span(), "expected a name"),
        },
        _ => err(e.span(), "expected a name"),
    }
}

/// `gamma = Default | Linear | [dark_on_light, light_on_dark]` (numbers with up to two
/// decimals, such as `[1.3, 2.0]`).
fn gamma(e: &Expr) -> syn::Result<Gamma> {
    if let Expr::Array(a) = e {
        let parts = a
            .elems
            .iter()
            .map(|x| match lit(x) {
                Some(Lit::Float(f)) => Ok(f.base10_digits().to_string()),
                Some(Lit::Int(i)) => Ok(i.base10_digits().to_string()),
                _ => err(x.span(), "expected a number such as `1.8`"),
            })
            .collect::<syn::Result<Vec<_>>>()?;
        if parts.len() != 2 {
            return err(e.span(), "expected `[dark_on_light, light_on_dark]`");
        }
        return parts
            .join(",")
            .parse()
            .map_err(|m: String| syn::Error::new(e.span(), m));
    }
    match variant(e)?.as_str() {
        "Default" => Ok(Gamma::DEFAULT),
        "Linear" => Ok(Gamma::LINEAR),
        _ => err(
            e.span(),
            "expected `Default`, `Linear` or `[dark_on_light, light_on_dark]`",
        ),
    }
}

/// `chars = …`: a character, a string, an inclusive character range, or an array of those.
fn add_chars(mut font: Font, e: &Expr) -> syn::Result<Font> {
    match e {
        Expr::Array(a) => {
            for el in &a.elems {
                font = add_chars(font, el)?;
            }
            Ok(font)
        }
        Expr::Range(ExprRange {
            start: Some(s),
            end: Some(t),
            limits: RangeLimits::Closed(_),
            ..
        }) => {
            let (s, t) = (character(s)?, character(t)?);
            if s > t {
                return err(e.span(), "the range is empty");
            }
            Ok(font.chars_from(s..=t))
        }
        Expr::Range(_) => err(e.span(), "expected an inclusive character range (`'a'..='z'`)"),
        _ => match lit(e) {
            Some(Lit::Char(c)) => Ok(font.chars_from(c.value())),
            Some(Lit::Str(s)) => Ok(font.chars_from(s.value().as_str())),
            _ => err(
                e.span(),
                "expected a character, a string, an inclusive character range or an array of those",
            ),
        },
    }
}

/// The directory relative paths are read from: the package being compiled.
fn manifest_dir() -> PathBuf {
    std::env::var_os("CARGO_MANIFEST_DIR").map_or_else(PathBuf::new, PathBuf::from)
}

/// The path of a Twine crate (`text`, `image`) in the crate being compiled: through the
/// `twine` facade, else the crate itself (`twine-text`).
///
/// Inside the facade's own package, only its library reaches the module as `crate::`; its
/// examples, binaries, tests and doctests (whose crate name differs, or which rustdoc compiles)
/// use the library as `::twine`.
fn crate_path(module: &str, krate: &str) -> String {
    use proc_macro_crate::{FoundCrate, crate_name};
    match crate_name("twine") {
        Ok(FoundCrate::Itself) if compiling_lib_of("twine") => format!("crate::{module}"),
        Ok(FoundCrate::Itself) => format!("::twine::{module}"),
        Ok(FoundCrate::Name(n)) => format!("::{n}::{module}"),
        Err(_) => match crate_name(krate) {
            Ok(FoundCrate::Name(n)) => format!("::{n}"),
            _ => format!("::{}", krate.replace('-', "_")),
        },
    }
}

/// Whether the crate being compiled is the library of package `name` (not one of its
/// examples, binaries or doctests).
fn compiling_lib_of(name: &str) -> bool {
    std::env::var("CARGO_CRATE_NAME").is_ok_and(|c| c == name.replace('-', "_"))
        && std::env::var_os("UNSTABLE_RUSTDOC_TEST_PATH").is_none()
}

/// The generated source as tokens: the caller's attributes are put on the main static (the
/// only item named exactly `NAME`; its data statics are `NAME_…`), and every input file is
/// tracked with `include_bytes!` so that changing it recompiles the crate.
fn expand(source: &str, input: &AssetInput, inputs: &[PathBuf]) -> syn::Result<TokenStream2> {
    let name = input.name.to_string();
    let attrs: TokenStream2 = input.attrs.iter().map(ToTokens::to_token_stream).collect();
    let mut src = source.to_string();
    if let Some(pos) = src.find(&format!("static {name}: ")) {
        let line_start = src[..pos].rfind('\n').map_or(0, |i| i + 1);
        src.insert_str(line_start, &format!("{attrs}\n"));
    }
    let items: TokenStream2 = src
        .parse()
        .map_err(|e| syn::Error::new(input.path.span(), format!("generated code does not parse: {e}")))?;
    let tracked = inputs.iter().filter(|p| p.is_file()).map(|p| {
        let p = p.display().to_string();
        quote!(
            const _: &[u8] = include_bytes!(#p);
        )
    });
    Ok(quote! {
        #items
        #(#tracked)*
    })
}

fn visibility(vis: &Visibility) -> String {
    vis.to_token_stream().to_string().replace(' ', "")
}

/// Expands `include_font!`.
pub(crate) fn include_font(input: &AssetInput) -> syn::Result<TokenStream2> {
    let root = manifest_dir();
    let mut font = Font::from_ttf(input.path.value())
        .root(&root)
        .name(&input.name.to_string())
        .visibility(&visibility(&input.vis))
        .crate_path(&crate_path("text", "twine-text"));
    for (key, value) in &input.options {
        font = match key.to_string().as_str() {
            "size" => font.size(int(value)?),
            "bpp" => {
                let bits: u8 = int(value)?;
                match Bpp::new(bits) {
                    Some(b) => font.bpp(b),
                    None => return err(value.span(), format!("bpp must be 1, 2, 4 or 8 (got {bits})")),
                }
            }
            "chars" => add_chars(font, value)?,
            "chars_file" => font.chars_from_file(string(value)?),
            "chars_from_rust" => font.chars_from_rust_sources(string(value)?),
            "symbol_fonts" => {
                let Expr::Array(a) = value else {
                    return err(value.span(), "expected an array of file paths");
                };
                font.symbol_fonts(a.elems.iter().map(string).collect::<syn::Result<Vec<_>>>()?)
            }
            "format" => font.format(match variant(value)?.as_str() {
                "Plain" => BitmapFormat::Plain,
                "Compressed" => BitmapFormat::Compressed,
                "CompressedNoPrefilter" => BitmapFormat::CompressedNoPrefilter,
                _ => {
                    return err(
                        value.span(),
                        "expected `Plain`, `Compressed` or `CompressedNoPrefilter`",
                    );
                }
            }),
            "kerning" => font.kerning(boolean(value)?),
            "subpx" => font.subpx(match variant(value)?.as_str() {
                "None" => Subpx::None,
                "Hor" => Subpx::Hor,
                "Ver" => Subpx::Ver,
                _ => return err(value.span(), "expected `None`, `Hor` or `Ver`"),
            }),
            "hinting" => font.hinting(match variant(value)?.as_str() {
                "None" => Hinting::None,
                "Light" => Hinting::Light,
                "Full" => Hinting::Full,
                _ => return err(value.span(), "expected `None`, `Light` or `Full`"),
            }),
            "gamma" => font.gamma(gamma(value)?),
            other => {
                return err(
                    key.span(),
                    format!(
                        "unknown option `{other}` (expected size, bpp, chars, chars_file, chars_from_rust, \
                         symbol_fonts, format, kerning, subpx, hinting or gamma)"
                    ),
                );
            }
        };
    }
    let g = font.build().map_err(|e| syn::Error::new(input.path.span(), e))?;
    expand(&g.source, input, &g.inputs)
}

/// Expands `include_image!`.
pub(crate) fn include_image(input: &AssetInput) -> syn::Result<TokenStream2> {
    let root = manifest_dir();
    let mut image = Image::from_file(input.path.value())
        .root(&root)
        .name(&input.name.to_string())
        .visibility(&visibility(&input.vis))
        .crate_path(&crate_path("image", "twine-image"));
    for (key, value) in &input.options {
        image = match key.to_string().as_str() {
            "format" => {
                let v = variant(value)?;
                let Some(f) = FORMAT_NAMES
                    .iter()
                    .map(|&(_, f)| f)
                    .find(|f| format!("{f:?}") == v)
                else {
                    let names: Vec<String> = FORMAT_NAMES.iter().map(|(_, f)| format!("{f:?}")).collect();
                    return err(
                        value.span(),
                        format!("expected a color format: {}", names.join(", ")),
                    );
                };
                image.format(f)
            }
            "compress" => image.compress(match variant(value)?.as_str() {
                "None" => Compression::None,
                "Rle" => Compression::Rle,
                "Lz4" => Compression::Lz4,
                _ => return err(value.span(), "expected `None`, `Rle` or `Lz4`"),
            }),
            "dither" => image.dither(boolean(value)?),
            "premultiply" => image.premultiply(boolean(value)?),
            "background" => image.background(Color::hex(int(value)?)),
            other => {
                return err(
                    key.span(),
                    format!(
                        "unknown option `{other}` (expected format, compress, dither, premultiply or background)"
                    ),
                );
            }
        };
    }
    let g = image.build().map_err(|e| syn::Error::new(input.path.span(), e))?;
    let source = g.to_rust_inline();
    expand(&source, input, g.inputs())
}
