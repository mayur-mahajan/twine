//! # twine-macros
//!
//! The procedural macros of Twine. Use them through `twine-view` or the `twine` facade
//! (`use twine::prelude::*;` brings [`component`] into scope); this crate only runs on the host
//! while compiling and never reaches firmware — what firmware links is the code the macros
//! generate.
//!
//! | Macro | Role |
//! |-------|------|
//! | [`#[component]`](macro@component) | a component function becomes a view with a props builder: `gauge(rpm).max(8000).redline(6500).padding(4)` |
//! | `include_font!` (feature `assets`) | a bitmap font generated from a TrueType/OpenType file while the crate compiles: `include_font!(pub static BODY = "assets/Inter.ttf", size = 16, bpp = 4, chars = '\u{20}'..='\u{7E}')` |
//! | `include_image!` (feature `assets`) | an image converted from a PNG/JPEG/BMP/GIF/QOI file while the crate compiles: `include_image!(pub static LOGO = "assets/logo.png", format = Rgb565)` |
//!
//! Projects that avoid procedural macros write the same component as a plain function (see
//! [`component`] § Without the macro); nothing in Twine requires this crate.

#![forbid(unsafe_code)]

#[cfg(feature = "assets")]
mod assets;

use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{ToTokens, format_ident, quote};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::visit::Visit;
use syn::{
    AngleBracketedGenericArguments, Attribute, Expr, FnArg, GenericArgument, GenericParam, Generics, Ident,
    ItemFn, Pat, PathArguments, ReturnType, Token, Type, TypeImplTrait, TypeParamBound, TypePath,
};

/// Generates a bitmap font from a TrueType or OpenType file **while the crate compiles** and
/// expands to the `static` [`twine::text::Font`] (feature `assets`; through the facade,
/// `twine::include_font!` with its feature `include-assets`).
///
/// The same generator as `twine_build::font::Font` in a build script, without one: handy for a
/// few fonts; a build script is better for many (it runs once, not on every compilation of
/// the crate) and for subsetting from translation catalogs.
///
/// ```
/// twine_macros::include_font!(
///     /// Digits and a degree sign for the temperature readout.
///     pub static READOUT = "../../assets/fonts/Montserrat-Medium.ttf",
///     size = 20,
///     bpp = 4,
///     chars = ['0'..='9', "°C-."],
/// );
///
/// assert_eq!(READOUT.line_height, 26);
/// assert!(READOUT.glyph('7', None).is_some());
/// assert!(READOUT.glyph('A', None).is_none(), "only the characters asked for");
///
/// // Vertical-only hinting (unhinted advances) and stronger text gamma.
/// twine_macros::include_font!(
///     static SMALL = "../../assets/fonts/Montserrat-Medium.ttf",
///     size = 12,
///     chars = 'a'..='z',
///     hinting = Light,
///     gamma = [1.3, 2.0],
/// );
/// assert!(SMALL.glyph('q', None).is_some());
/// ```
///
/// # Syntax
///
/// `[attributes] [visibility] static NAME = "path/to/font.ttf", option = value, …` — the path
/// is relative to the package's directory (`CARGO_MANIFEST_DIR`); attributes (doc comments)
/// go on the font static. Options (each at most once, except `chars`):
///
/// | Option | Value | Default |
/// |--------|-------|---------|
/// | `size` | pixel size, 1 to 255 | 14 |
/// | `bpp` | 1, 2, 4 or 8 bits per pixel (checked at compile time) | 4 |
/// | `chars` | `'a'..='z'`, `'°'`, `"äöü"` or an array of those (repeatable) | — (required unless `chars_file` / `chars_from_rust` / `symbol_fonts`) |
/// | `chars_file` | a UTF-8 file of characters | — |
/// | `chars_from_rust` | a `.rs` file or directory whose string and character literals are added | — |
/// | `symbol_fonts` | `["fa-solid-900.ttf", …]`: merge the Font Awesome symbols | none |
/// | `format` | `Plain`, `Compressed`, `CompressedNoPrefilter` | `Plain` |
/// | `kerning` | `true` / `false` | `true` |
/// | `subpx` | `None`, `Hor`, `Ver` | `None` |
/// | `hinting` | `None`, `Light`, `Full` (grid-fitting, see `twine_build::font::Hinting`) | `Full` |
/// | `gamma` | `Default`, `Linear` or `[dark_on_light, light_on_dark]` strengths such as `[1.3, 2.0]` (text gamma, see `twine_build::font::Gamma`) | `Default` (1.2 / 1.8) |
///
/// The generated code names the text crate through the `twine` facade when the crate depends
/// on it (`twine::text`), else through `twine-text`. Every input file is tracked: changing it
/// recompiles the crate.
///
/// # Errors
/// A compile error for an unknown option or value, an unreadable font file, a character
/// set with no glyph, or anything else `twine_build::font::Font::build` refuses.
///
/// [`twine::text::Font`]: https://docs.rs/twine-text/latest/twine_text/struct.Font.html
#[cfg(feature = "assets")]
#[proc_macro]
pub fn include_font(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as assets::AssetInput);
    assets::include_font(&input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Converts an image file (PNG, JPEG, BMP, GIF, QOI) **while the crate compiles** and expands
/// to the `static` [`twine::image::Image`], pixel data included (feature `assets`; through the
/// facade, `twine::include_image!` with its feature `include-assets`).
///
/// The same converter as `twine_build::image::Image` in a build script, without one.
///
/// ```
/// twine_macros::include_image!(
///     /// The logo for the splash screen.
///     pub static LOGO = "../../assets/images/twine_logo.png",
///     format = Rgb565,
///     background = 0xFFFFFF,
///     compress = Rle,
/// );
///
/// assert_eq!((LOGO.header.w, LOGO.header.h), (64, 64));
/// assert_eq!(LOGO.header.format, twine_core::ColorFormat::Rgb565);
/// ```
///
/// # Syntax
///
/// `[attributes] [visibility] static NAME = "path/to/image.png", option = value, …` — the path
/// is relative to the package's directory (`CARGO_MANIFEST_DIR`); attributes (doc comments)
/// go on the image static. Options:
///
/// | Option | Value | Default |
/// |--------|-------|---------|
/// | `format` | a `ColorFormat` variant: `Rgb565`, `Argb8888`, `A4`, `I8`, … | `Argb8888` |
/// | `compress` | `None`, `Rle`, `Lz4` (needs `img-lz4` at run time) | `None` |
/// | `dither` | `true` / `false` | `false` |
/// | `premultiply` | `true` / `false` (`Argb8888`, `Rgb565A8`) | `false` |
/// | `background` | `0xRRGGBB`: composite over this color first | alpha kept |
///
/// The generated code names the image crate through the `twine` facade when the crate
/// depends on it (`twine::image`), else through `twine-image`. The input file is tracked:
/// changing it recompiles the crate.
///
/// # Errors
/// A compile error for an unknown option or value, an unreadable or undecodable file, or
/// anything else `twine_build::image::Image::build` refuses.
///
/// [`twine::image::Image`]: https://docs.rs/twine-image/latest/twine_image/struct.Image.html
#[cfg(feature = "assets")]
#[proc_macro]
pub fn include_image(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as assets::AssetInput);
    assets::include_image(&input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Turns a function describing a piece of UI into a **component**: a view with a props
/// builder, used exactly like the built-in widgets.
///
/// ```
/// use core::ops::RangeInclusive;
/// use twine_view::prelude::*;
///
/// /// A captioned meter.
/// #[component]
/// fn meter(
///     /// The value shown (a constant, signal, memo or closure).
///     value: Prop<i32>,
///     /// The scale.
///     #[prop(default = 0..=100)]
///     range: Prop<RangeInclusive<i32>>,
///     /// The caption above the bar.
///     #[prop(default)]
///     caption: TextProp,
/// ) -> impl ViewExt<Widget = Obj> {
///     column((label(caption), bar(value).range(range).fill_width())).gap(4)
/// }
///
/// let cx = twine_reactive::Runtime::take().unwrap().create_root();
/// let rpm = cx.signal(3200);
/// let _v = column((
///     meter(rpm).range(0..=8000).caption("rpm").padding(4).test_id("rpm"), // modifiers too
///     meter(42),                                                           // the defaults
/// ));
/// cx.dispose();
/// ```
///
/// # What it generates
///
/// For `fn meter(value: Prop<i32>, #[prop(default = 0..=100)] range: Prop<..>, ..) -> impl ViewExt<..>`:
///
/// - **`fn meter(value: impl IntoProp<i32, M>) -> MeterProps`** — the constructor, with the
///   function's visibility and documentation (plus a generated `# Props` list). **Required
///   props are its arguments**, in order; nothing is built yet.
/// - **`struct MeterProps`** — the props builder (name: the function's name in `PascalCase` +
///   `Props`; `#[component(props = Name)]` chooses another). It holds every prop by value and
///   has **one method per optional prop** (`.range(0..=8000)`), documented with the parameter's doc
///   comment and its default.
/// - **`impl View for MeterProps`** — building it runs the function's body once, with the props
///   (and the scope, see below), and builds the view it returns.
/// - **`impl ViewExt for MeterProps`** when the return type is
///   `impl ViewExt<Widget = W>`: every modifier of `ViewExt` and `StyleExt` (`.padding`,
///   `.bg`, `.on_click`, `.test_id`, `.node_ref` with a `NodeRef<W>`, …) applies to the root of
///   the returned view, exactly as if written on it at the end of the body. With
///   `-> impl View` (or any other return type) the component is a `View` without modifiers,
///   like the built-in `when` and `dynamic`.
///
/// # Props
///
/// Each parameter is a prop; its type decides what callers may pass:
///
/// | Parameter type | Accepts (argument or method) | Notes |
/// |----------------|------------------------------|-------|
/// | `Prop<T>` | `impl IntoProp<T, M>`: a constant converting into `T`, or a closure, signal or memo of one | reactive; pass it on to any modifier or widget argument (`Prop` is accepted as is), or read it with `Prop::get` |
/// | `TextProp` | `impl IntoText<M>`: `&'static str`, `String`, `text!`, closures, signals | pass it on to `label(..)` etc. |
/// | `Model<T>` | `impl IntoModel<T>`: a value (owned by the widget) or a `Signal<T>` (two-way) | pass it on to `slider(..)`, `switch(..)`, … |
/// | `Box<dyn Trait>` | `impl Trait + 'static` (boxed by the builder) | callbacks: `on_pick: Box<dyn FnMut(usize)>` |
/// | `Option<Box<dyn Trait>>` | `impl Trait + 'static` (stored as `Some`) | optional callbacks with `#[prop(default)]` |
/// | `impl Trait` (required only) | any type implementing it (a type parameter of the builder: no boxing) | children: `content: impl ViewSeq` |
/// | any other `T` | exactly `T`; `impl Into<T>` with `#[prop(into)]` | |
///
/// Parameter attributes:
///
/// - `#[prop(default)]` — optional, defaulting to `Default::default()` (`Prop`: the constant
///   `T::default()`; `TextProp`: `""`; `Model`: an owned `T::default()`; `Option`: `None`).
/// - `#[prop(default = expr)]` — optional, defaulting to `expr`, converted like a value passed
///   to its method (`#[prop(default = 100)]` for a `Prop<i32>`, `#[prop(default = Color::RED)]`
///   for a `Prop<ColorValue>`, `#[prop(default = "OK")]` for a `TextProp`). Evaluated when the
///   component is created.
/// - `#[prop(into)]` — a plain-typed prop accepts `impl Into<T>`.
/// - Doc comments (`///`) document the prop: they become the method's documentation (optional
///   props) and the constructor's `# Props` list.
///
/// A first parameter of type `Scope` is not a prop: it receives the scope the component is
/// built in (the scope owning its bindings, disposed with it — e.g. with the `when` branch it
/// is in), for signals, memos and effects of the component's own:
///
/// ```
/// use twine_view::prelude::*;
///
/// /// A counter with its own state.
/// #[component]
/// fn stepper(cx: Scope, #[prop(default = 1)] step: i32) -> impl ViewExt<Widget = Button> {
///     let n = cx.signal(0);
///     button(label(text!("{}", n.get()))).on_click(move || n.update(|n| *n += step))
/// }
///
/// let _v = row((stepper(), stepper().step(10)));
/// ```
///
/// # Required props are arguments
///
/// A missing required prop is a compile-time error, reported by the compiler with the
/// parameter's name ("this function takes 1 argument but 0 arguments were supplied …
/// argument #1 is missing", with the suggestion `meter(/* value */)`), exactly like the built-ins
/// (`label(text)`, `bar(value)`, `slider(model)`, `button(child)` take what they need as
/// arguments). There is no runtime check, no `Option` per required prop and no type-state
/// parameter on the builder.
///
/// # Cost
///
/// The builder is what you would write by hand: a struct of the props by value (constants are
/// `Prop::Static`, no allocation; closures, signals and memos are one box each, as for every
/// property), plain methods replacing a field, and a `View::build` that calls the body. Nothing
/// is boxed unless the prop's type is a box (`Box<dyn Trait>`, or the boxed closure inside a
/// dynamic `Prop`); children (`impl ViewSeq`) are type parameters. A component with
/// modifiers keeps them in one `Vec` (empty, so not allocated, without modifiers) and hands
/// them to the returned view when it is built. Methods and the build are generic and small,
/// so they inline into the caller like hand-written code.
///
/// # Rules
///
/// - Parameters are plain identifiers (`value`, `mut value`); no `self`, no lifetime
///   parameters (views are `'static`), not `async`, `const`, `unsafe` or `extern`.
/// - An optional prop's type cannot be `impl Trait` (the builder's type would change when it
///   is set): use a `Box<dyn Trait>` (callbacks) or a type parameter of the function.
/// - A method of an optional prop has the prop's name: it takes precedence over a modifier of
///   the same name (a prop called `width` hides `StyleExt::width` on the component), so prefer
///   names that are not modifiers.
/// - The body is the function's body unchanged; it runs when the view is built (once).
///
/// # Without the macro
///
/// A component is just a function returning a view; the macro only adds the builder. Without
/// it (projects avoiding procedural macros), take required props as arguments and optional ones
/// as `Option`s or a props struct, or build a newtype view like the built-in widgets (see the
/// custom-widget guide, `twine::guide::custom_widgets`):
///
/// ```
/// use twine_view::prelude::*;
///
/// /// The plain-function form: every prop is an argument.
/// fn meter<M1, M2>(
///     value: impl IntoProp<i32, M1>,
///     caption: impl IntoText<M2>,
/// ) -> impl ViewExt<Widget = Obj> {
///     column((label(caption), bar(value).fill_width())).gap(4)
/// }
///
/// let _v = meter(42, "rpm").padding(4);
/// ```
///
/// # Crate paths
///
/// The generated code names `twine-view` as the crate using the macro depends on it: directly
/// (`twine_view`, also renamed) or through the `twine` facade (`twine::view`). A crate that
/// reaches it differently passes the path: `#[component(crate = my_ui::twine_view)]`.
///
/// # Errors
///
/// Unsupported signatures are rejected with a message at the offending part (see Rules), as
/// are unknown `#[component(..)]` or `#[prop(..)]` arguments. A wrong value for a prop is
/// reported by the conversion traits (`` `&str` cannot be used as a property of type `i32` ``).
#[proc_macro_attribute]
pub fn component(attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut args = Args::default();
    let parser = syn::meta::parser(|meta| {
        if meta.path.is_ident("crate") {
            args.krate = Some(meta.value()?.parse()?);
            Ok(())
        } else if meta.path.is_ident("props") {
            args.props = Some(meta.value()?.parse()?);
            Ok(())
        } else {
            Err(meta.error("unknown `#[component]` argument: expected `crate = path` or `props = Name`"))
        }
    });
    if let Err(e) = syn::parse::Parser::parse(parser, attr) {
        return e.to_compile_error().into();
    }
    let func = match syn::parse::<ItemFn>(item) {
        Ok(f) => f,
        Err(e) => return e.to_compile_error().into(),
    };
    match expand(args, &func) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// The arguments of `#[component(..)]`.
#[derive(Default)]
struct Args {
    krate: Option<syn::Path>,
    props: Option<Ident>,
}

/// How a prop's value is taken and stored.
enum Kind {
    /// `Prop<T>`: `impl IntoProp<T, M>`.
    Prop(Type),
    /// `TextProp`: `impl IntoText<M>`.
    Text,
    /// `Model<T>`: `impl IntoModel<T>`.
    Model(Type),
    /// `Box<dyn Bounds>`: `impl Bounds + 'static`, boxed.
    Boxed(TokenStream2),
    /// `Option<Box<dyn Bounds>>`: `impl Bounds + 'static`, boxed, `Some`.
    OptBoxed(TokenStream2),
    /// `impl Bounds` (required): a type parameter of the builder.
    Impl(Ident),
    /// Any other type: exactly it, or `impl Into<T>` with `into`.
    Plain { into: bool },
}

impl Kind {
    /// Whether the conversion trait has a marker parameter.
    fn has_marker(&self) -> bool {
        matches!(self, Kind::Prop(_) | Kind::Text)
    }
}

/// A prop's default.
enum PropDefault {
    /// `#[prop(default)]`.
    Trait,
    /// `#[prop(default = expr)]`.
    Expr(Expr),
}

/// One prop (a parameter that is not the scope).
struct PropParam {
    docs: Vec<Attribute>,
    ident: Ident,
    /// The type of the struct field (the synthesized type parameter for `impl Trait`).
    field_ty: Type,
    kind: Kind,
    default: Option<PropDefault>,
}

/// The struct field of prop `id`: prefixed, so that calling a required prop as a method reports
/// "no method named `value`" rather than pointing at a private field.
fn field(id: &Ident) -> Ident {
    format_ident!("__twine_prop_{}", id)
}

/// Generates everything (see [`component`]).
fn expand(args: Args, func: &ItemFn) -> syn::Result<TokenStream2> {
    let k = match args.krate {
        Some(p) => p.into_token_stream(),
        None => krate()?,
    };
    let sig = &func.sig;
    reject_unsupported(sig)?;

    let name = &sig.ident;
    let vis = &func.vis;
    let props_ty = args.props.unwrap_or_else(|| props_name(name));

    let widget = view_ext_widget(name, &sig.output)?;

    // Attributes: docs go to the constructor, `cfg`s to every item, the rest to the body.
    let mut docs = Vec::new();
    let mut cfgs = Vec::new();
    let mut body_attrs = Vec::new();
    for a in &func.attrs {
        if a.path().is_ident("doc") {
            docs.push(a.clone());
        } else if a.path().is_ident("cfg") || a.path().is_ident("cfg_attr") {
            cfgs.push(a.clone());
        } else {
            body_attrs.push(a.clone());
        }
    }

    // Parameters.
    let mut body_inputs: Punctuated<FnArg, Token![,]> = Punctuated::new();
    let mut props = Vec::new();
    let mut scope_param = false;
    let mut synth = 0usize;
    let mut struct_generics: Generics = sig.generics.clone();
    for (i, arg) in sig.inputs.iter().enumerate() {
        let FnArg::Typed(pt) = arg else {
            return Err(syn::Error::new(arg.span(), "a component cannot take `self`"));
        };
        let Pat::Ident(pi) = &*pt.pat else {
            return Err(syn::Error::new(
                pt.pat.span(),
                "a component's parameters must be plain names (`value: Prop<i32>`): each one is a prop",
            ));
        };
        if pi.by_ref.is_some() || pi.subpat.is_some() {
            return Err(syn::Error::new(pi.span(), "a prop must be a plain name"));
        }
        let mut docs = Vec::new();
        let mut default = None;
        let mut into = false;
        for a in &pt.attrs {
            if a.path().is_ident("doc") {
                docs.push(a.clone());
            } else if a.path().is_ident("prop") {
                a.parse_nested_meta(|meta| {
                    if meta.path.is_ident("default") {
                        default = Some(if meta.input.peek(Token![=]) {
                            PropDefault::Expr(meta.value()?.parse()?)
                        } else {
                            PropDefault::Trait
                        });
                        Ok(())
                    } else if meta.path.is_ident("into") {
                        into = true;
                        Ok(())
                    } else {
                        Err(meta.error(
                            "unknown `#[prop]` argument: expected `default`, `default = expr` or `into`",
                        ))
                    }
                })?;
            } else {
                return Err(syn::Error::new(
                    a.span(),
                    "a prop takes only doc comments and `#[prop(..)]`",
                ));
            }
        }
        // The body keeps the parameter as written, without the attributes.
        let mut body_pt = pt.clone();
        body_pt.attrs.clear();
        body_inputs.push(FnArg::Typed(body_pt));

        if i == 0 && is_scope(&pt.ty) {
            if !docs.is_empty() || default.is_some() || into {
                return Err(syn::Error::new(
                    pt.span(),
                    "the `Scope` parameter is not a prop: it receives the scope the component is built in",
                ));
            }
            scope_param = true;
            continue;
        }

        reject_nested_impl(&pt.ty)?;
        let (kind, field_ty) = if let Type::ImplTrait(it) = &*pt.ty {
            if default.is_some() {
                return Err(syn::Error::new(
                    pt.ty.span(),
                    "an optional prop cannot be `impl Trait` (setting it would change the builder's type): \
                     use a `Box<dyn Trait>` or a type parameter of the function",
                ));
            }
            let id = format_ident!("__TwineImpl{}", synth);
            synth += 1;
            let bounds = &it.bounds;
            struct_generics
                .params
                .push(GenericParam::Type(syn::parse_quote!(#id: #bounds)));
            (Kind::Impl(id.clone()), syn::parse_quote!(#id))
        } else {
            (classify(&pt.ty, into), (*pt.ty).clone())
        };
        if into && !matches!(kind, Kind::Plain { .. }) {
            return Err(syn::Error::new(
                pt.ty.span(),
                "`#[prop(into)]` is for plain types: `Prop`, `TextProp`, `Model`, boxes and `impl Trait` already convert",
            ));
        }
        props.push(PropParam {
            docs,
            ident: pi.ident.clone(),
            field_ty,
            kind,
            default,
        });
    }

    // The builder's type parameters (for `PhantomData` and the `'static` bounds).
    let mut view_where = struct_generics.clone().make_where_clause().clone();
    let type_params: Vec<&Ident> = struct_generics.type_params().map(|t| &t.ident).collect();
    let (s_impl, s_ty, s_where) = struct_generics.split_for_impl();
    for t in &type_params {
        view_where.predicates.push(syn::parse_quote!(#t: 'static));
    }

    // The struct.
    let fields = props.iter().map(|p| {
        let f = field(&p.ident);
        let ty = &p.field_ty;
        quote!(#f: #ty,)
    });
    let mods_field = widget
        .as_ref()
        .map(|_| quote!(__twine_mods: #k::component::Modifiers,));
    let struct_doc = format!(
        "The props builder of [`{name}`](fn@{name}) (generated by `#[component]`): set its optional \
         props with its methods and use it as a view."
    );
    let item_struct = quote! {
        #(#cfgs)*
        #[doc = #struct_doc]
        #[must_use = "a component does nothing until it is used as a view"]
        #vis struct #props_ty #s_impl #s_where {
            #(#fields)*
            #mods_field
            __twine_marker: ::core::marker::PhantomData<fn() -> (#(#type_params,)*)>,
        }
    };

    // The constructor.
    let mut ctor_generics = struct_generics.clone();
    let mut ctor_args = Vec::new();
    let mut inits = Vec::new();
    let mut markers = 0usize;
    for p in &props {
        let id = &p.ident;
        match &p.default {
            None => {
                let marker = p.kind.has_marker().then(|| {
                    let m = format_ident!("__TwineM{}", markers);
                    markers += 1;
                    ctor_generics
                        .params
                        .push(GenericParam::Type(syn::parse_quote!(#m)));
                    m
                });
                let ty = arg_type(&k, p, marker.as_ref());
                ctor_args.push(quote!(#id: #ty));
                let v = convert(&k, p, marker.as_ref(), &quote!(#id));
                let f = field(id);
                inits.push(quote!(#f: #v,));
            }
            Some(d) => {
                let v = default_value(&k, p, d);
                let f = field(id);
                inits.push(quote!(#f: #v,));
            }
        }
    }
    let (c_impl, _, c_where) = ctor_generics.split_for_impl();
    let mods_init = widget
        .as_ref()
        .map(|_| quote!(__twine_mods: #k::component::Modifiers::new(),));
    let ctor_doc = props_doc(name, &props_ty, &props, widget.as_ref());
    let item_ctor = quote! {
        #(#cfgs)*
        #(#docs)*
        #(#ctor_doc)*
        #[inline]
        #vis fn #name #c_impl (#(#ctor_args),*) -> #props_ty #s_ty #c_where {
            #props_ty {
                #(#inits)*
                #mods_init
                __twine_marker: ::core::marker::PhantomData,
            }
        }
    };

    // The methods of the optional props.
    let setters = props.iter().filter(|p| p.default.is_some()).map(|p| {
        let id = &p.ident;
        let f = field(id);
        let marker = p.kind.has_marker().then(|| format_ident!("__TwineM"));
        let generics = marker.as_ref().map(|m| quote!(<#m>));
        let ty = arg_type(&k, p, marker.as_ref());
        let v = convert(&k, p, marker.as_ref(), &quote!(#id));
        let docs = &p.docs;
        let default_doc = match &p.default {
            Some(PropDefault::Expr(e)) => format!("Optional; default: `{}`.", source_text(e)),
            _ => "Optional; default: `Default::default()`.".to_string(),
        };
        let sep = (!docs.is_empty()).then(|| quote!(#[doc = ""]));
        quote! {
            #(#docs)*
            #sep
            #[doc = #default_doc]
            #[inline]
            #vis fn #id #generics (mut self, #id: #ty) -> Self {
                self.#f = #v;
                self
            }
        }
    });
    let item_setters = quote! {
        #(#cfgs)*
        impl #s_impl #props_ty #s_ty #s_where {
            #(#setters)*
        }
    };

    // The view: the body as a nested function, called with the props.
    let body_name = format_ident!("__twine_body");
    let body_generics = &sig.generics;
    let body_where = &sig.generics.where_clause;
    let output = &sig.output;
    let block = &func.block;
    let field_names: Vec<&Ident> = props.iter().map(|p| &p.ident).collect();
    let fields: Vec<Ident> = props.iter().map(|p| field(&p.ident)).collect();
    let scope_arg = scope_param.then(|| quote!(#k::BuildCx::scope(__twine_cx),));
    let mods_pat = widget.as_ref().map(|_| quote!(__twine_mods,));
    let view_expr = if widget.is_some() {
        quote!(__twine_mods.apply(__twine_view))
    } else {
        quote!(__twine_view)
    };
    let item_view = quote! {
        #(#cfgs)*
        impl #s_impl #k::View for #props_ty #s_ty #view_where {
            fn build(self, __twine_cx: &mut #k::BuildCx<'_>) -> #k::__private::NodeId {
                #(#body_attrs)*
                #[allow(clippy::too_many_arguments)]
                #[inline]
                fn #body_name #body_generics (#body_inputs) #output #body_where #block

                let #props_ty { #(#fields: #field_names,)* #mods_pat .. } = self;
                let __twine_view = #body_name(#scope_arg #(#field_names),*);
                #k::View::build(#view_expr, __twine_cx)
            }
        }
    };
    let item_view_ext = widget.map(|w| {
        quote! {
            #(#cfgs)*
            impl #s_impl #k::ViewExt for #props_ty #s_ty #view_where {
                type Widget = #w;

                #[inline]
                fn push_op(mut self, op: #k::BuildOp) -> Self {
                    self.__twine_mods.push(op);
                    self
                }
            }
        }
    });

    Ok(quote! {
        #item_ctor
        #item_struct
        #item_setters
        #item_view
        #item_view_ext
    })
}

/// The path of `twine-view` in the crate being compiled.
fn krate() -> syn::Result<TokenStream2> {
    use proc_macro_crate::{FoundCrate, crate_name};
    // `Itself`: a doctest or the library of `twine-view` (whose doctests see it as an extern
    // crate; the library itself does not use the macro).
    if let Ok(found) = crate_name("twine-view") {
        return Ok(match found {
            FoundCrate::Itself => quote!(::twine_view),
            FoundCrate::Name(n) => {
                let id = Ident::new(&n, Span::call_site());
                quote!(::#id)
            }
        });
    }
    match crate_name("twine") {
        Ok(FoundCrate::Itself) => Ok(quote!(::twine::view)),
        Ok(FoundCrate::Name(n)) => {
            let id = Ident::new(&n, Span::call_site());
            Ok(quote!(::#id::view))
        }
        Err(_) => Err(syn::Error::new(
            Span::call_site(),
            "`#[component]` needs `twine` or `twine-view` as a dependency; otherwise name twine-view's \
             path: `#[component(crate = path::to::twine_view)]`",
        )),
    }
}

/// `titled_card` → `TitledCardProps`.
fn props_name(name: &Ident) -> Ident {
    let s = name.to_string();
    let s = s.strip_prefix("r#").unwrap_or(&s);
    let mut out = String::new();
    for part in s.split('_').filter(|p| !p.is_empty()) {
        let mut c = part.chars();
        if let Some(f) = c.next() {
            out.extend(f.to_uppercase());
            out.push_str(c.as_str());
        }
    }
    out.push_str("Props");
    Ident::new(&out, name.span())
}

/// Rejects signatures a component cannot have.
fn reject_unsupported(sig: &syn::Signature) -> syn::Result<()> {
    if let Some(t) = &sig.asyncness {
        return Err(syn::Error::new(t.span(), "a component cannot be `async`"));
    }
    if let Some(t) = &sig.constness {
        return Err(syn::Error::new(t.span(), "a component cannot be `const`"));
    }
    if let Some(t) = &sig.unsafety {
        return Err(syn::Error::new(t.span(), "a component cannot be `unsafe`"));
    }
    if let Some(t) = &sig.abi {
        return Err(syn::Error::new(t.span(), "a component cannot be `extern`"));
    }
    if let Some(t) = &sig.variadic {
        return Err(syn::Error::new(t.span(), "a component cannot be variadic"));
    }
    if let Some(l) = sig.generics.lifetimes().next() {
        return Err(syn::Error::new(
            l.span(),
            "a component cannot have lifetime parameters: views are `'static`",
        ));
    }
    Ok(())
}

/// The widget of `-> impl ViewExt<Widget = W>`; `None` for any other view type.
fn view_ext_widget(name: &Ident, output: &ReturnType) -> syn::Result<Option<Type>> {
    let ReturnType::Type(_, ty) = output else {
        return Err(syn::Error::new(
            name.span(),
            "a component returns its view: add `-> impl View`, or `-> impl ViewExt<Widget = W>` for modifiers",
        ));
    };
    let Type::ImplTrait(TypeImplTrait { bounds, .. }) = &**ty else {
        return Ok(None);
    };
    for b in bounds {
        let TypeParamBound::Trait(t) = b else { continue };
        let Some(seg) = t.path.segments.last() else {
            continue;
        };
        if seg.ident != "ViewExt" {
            continue;
        }
        if let PathArguments::AngleBracketed(a) = &seg.arguments {
            for arg in &a.args {
                if let GenericArgument::AssocType(at) = arg {
                    if at.ident == "Widget" {
                        return Ok(Some(at.ty.clone()));
                    }
                }
            }
        }
        return Err(syn::Error::new(
            seg.span(),
            "name the widget of the returned view's root: `impl ViewExt<Widget = W>` (its modifiers and \
             `node_ref` apply to it)",
        ));
    }
    Ok(None)
}

/// Whether `ty` is `Scope` (any path ending in `Scope`, without arguments).
fn is_scope(ty: &Type) -> bool {
    last_segment(ty).is_some_and(|(id, args)| id == "Scope" && args.is_none())
}

/// The last segment of a path type and its angle-bracketed arguments.
fn last_segment(ty: &Type) -> Option<(&Ident, Option<&AngleBracketedGenericArguments>)> {
    let Type::Path(TypePath { qself: None, path }) = ty else {
        return None;
    };
    let seg = path.segments.last()?;
    match &seg.arguments {
        PathArguments::None => Some((&seg.ident, None)),
        PathArguments::AngleBracketed(a) => Some((&seg.ident, Some(a))),
        PathArguments::Parenthesized(_) => None,
    }
}

/// The single type argument of `X<T>`.
fn single_type_arg(args: Option<&AngleBracketedGenericArguments>) -> Option<&Type> {
    let args = args?;
    if args.args.len() != 1 {
        return None;
    }
    match args.args.first()? {
        GenericArgument::Type(t) => Some(t),
        _ => None,
    }
}

/// The bounds of `Box<dyn Bounds>`, without lifetimes (`'static` is added).
fn boxed_bounds(ty: &Type) -> Option<TokenStream2> {
    let (id, args) = last_segment(ty)?;
    if id != "Box" {
        return None;
    }
    let Type::TraitObject(obj) = single_type_arg(args)? else {
        return None;
    };
    let bounds = obj
        .bounds
        .iter()
        .filter(|b| !matches!(b, TypeParamBound::Lifetime(_)));
    Some(quote!(#(#bounds)+*))
}

/// The kind of a prop of type `ty` (not `impl Trait`).
fn classify(ty: &Type, into: bool) -> Kind {
    if let Some((id, args)) = last_segment(ty) {
        if id == "Prop" {
            if let Some(t) = single_type_arg(args) {
                return Kind::Prop(t.clone());
            }
        } else if id == "TextProp" && args.is_none() {
            return Kind::Text;
        } else if id == "Model" {
            if let Some(t) = single_type_arg(args) {
                return Kind::Model(t.clone());
            }
        } else if let Some(b) = boxed_bounds(ty) {
            return Kind::Boxed(b);
        } else if id == "Option" {
            if let Some(b) = single_type_arg(args).and_then(boxed_bounds) {
                return Kind::OptBoxed(b);
            }
        }
    }
    Kind::Plain { into }
}

/// Rejects `impl Trait` inside a parameter type (`Option<impl Fn()>`): only a whole parameter
/// type can be one.
fn reject_nested_impl(ty: &Type) -> syn::Result<()> {
    struct Finder(Option<Span>);
    impl<'a> Visit<'a> for Finder {
        fn visit_type_impl_trait(&mut self, i: &'a TypeImplTrait) {
            self.0.get_or_insert(i.span());
        }
    }
    if matches!(ty, Type::ImplTrait(_)) {
        // The whole type: check its bounds only.
        let Type::ImplTrait(it) = ty else { return Ok(()) };
        let mut f = Finder(None);
        for b in &it.bounds {
            f.visit_type_param_bound(b);
        }
        return f.0.map_or(Ok(()), |s| Err(nested_impl_error(s)));
    }
    let mut f = Finder(None);
    f.visit_type(ty);
    f.0.map_or(Ok(()), |s| Err(nested_impl_error(s)))
}

fn nested_impl_error(span: Span) -> syn::Error {
    syn::Error::new(
        span,
        "only a whole prop type can be `impl Trait`: use a type parameter of the function",
    )
}

/// The type of the argument (or method parameter) of prop `p`.
fn arg_type(k: &TokenStream2, p: &PropParam, marker: Option<&Ident>) -> TokenStream2 {
    let ty = &p.field_ty;
    match &p.kind {
        Kind::Prop(t) => quote!(impl #k::IntoProp<#t, #marker>),
        Kind::Text => quote!(impl #k::IntoText<#marker>),
        Kind::Model(t) => quote!(impl #k::IntoModel<#t>),
        Kind::Boxed(b) | Kind::OptBoxed(b) => quote!(impl #b + 'static),
        Kind::Impl(id) => quote!(#id),
        Kind::Plain { into: true } => quote!(impl ::core::convert::Into<#ty>),
        Kind::Plain { into: false } => quote!(#ty),
    }
}

/// Converts the argument `v` of prop `p` into its stored value.
fn convert(k: &TokenStream2, p: &PropParam, marker: Option<&Ident>, v: &TokenStream2) -> TokenStream2 {
    let ty = &p.field_ty;
    match &p.kind {
        Kind::Prop(t) => quote!(#k::IntoProp::<#t, #marker>::into_prop(#v)),
        Kind::Text => quote!(#k::IntoText::<#marker>::into_text(#v)),
        Kind::Model(t) => quote!(#k::IntoModel::<#t>::into_model(#v)),
        Kind::Boxed(_) => quote!(#k::__private::Box::new(#v)),
        Kind::OptBoxed(_) => quote!(::core::option::Option::Some(#k::__private::Box::new(#v))),
        Kind::Impl(_) | Kind::Plain { into: false } => quote!(#v),
        Kind::Plain { into: true } => quote!(::core::convert::Into::<#ty>::into(#v)),
    }
}

/// The stored default of prop `p`.
fn default_value(k: &TokenStream2, p: &PropParam, d: &PropDefault) -> TokenStream2 {
    match d {
        PropDefault::Trait => quote!(::core::default::Default::default()),
        PropDefault::Expr(e) => {
            // As a value passed to the method; the marker is inferred.
            let marker = p.kind.has_marker().then(|| quote!(_));
            let ty = &p.field_ty;
            match &p.kind {
                Kind::Prop(t) => quote!(#k::IntoProp::<#t, #marker>::into_prop(#e)),
                Kind::Text => quote!(#k::IntoText::<#marker>::into_text(#e)),
                Kind::Model(t) => quote!(#k::IntoModel::<#t>::into_model(#e)),
                Kind::Boxed(_) => quote!(#k::__private::Box::new(#e)),
                Kind::OptBoxed(_) => quote!(::core::option::Option::Some(#k::__private::Box::new(#e))),
                Kind::Impl(_) | Kind::Plain { into: false } => quote!(#e),
                Kind::Plain { into: true } => quote!(::core::convert::Into::<#ty>::into(#e)),
            }
        }
    }
}

/// The source text of `e` (its tokens if unavailable).
fn source_text(e: &Expr) -> String {
    e.span()
        .source_text()
        .unwrap_or_else(|| e.to_token_stream().to_string())
}

/// The first line of a prop's doc comment.
fn first_doc_line(docs: &[Attribute]) -> String {
    for a in docs {
        if let syn::Meta::NameValue(nv) = &a.meta {
            if let Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(s),
                ..
            }) = &nv.value
            {
                let line = s.value();
                let line = line.trim();
                if !line.is_empty() {
                    return line.to_string();
                }
            }
        }
    }
    String::new()
}

/// The generated `# Props` section of the constructor's documentation.
fn props_doc(name: &Ident, props_ty: &Ident, props: &[PropParam], widget: Option<&Type>) -> Vec<Attribute> {
    let mut lines = vec![
        String::new(),
        "# Props".to_string(),
        String::new(),
        format!(
            "A component (`#[component]`): the required props are the arguments; the optional ones \
             are methods of [`{props_ty}`]. `{name}(..)` is a view."
        ),
    ];
    if let Some(w) = widget {
        let w = w.to_token_stream().to_string();
        lines.push(format!(
            "Every modifier of `ViewExt` and `StyleExt` applies to the root of the view (a `{w}`)."
        ));
    }
    lines.push(String::new());
    for p in props {
        let doc = first_doc_line(&p.docs);
        let sep = if doc.is_empty() { "" } else { ": " };
        let id = &p.ident;
        lines.push(match &p.default {
            None => format!("- `{id}` (required){sep}{doc}"),
            Some(PropDefault::Expr(e)) => format!(
                "- [`{id}`]({props_ty}::{id}) (default `{}`){sep}{doc}",
                source_text(e)
            ),
            Some(PropDefault::Trait) => {
                format!("- [`{id}`]({props_ty}::{id}) (default: `Default::default()`){sep}{doc}")
            }
        });
    }
    lines
        .into_iter()
        .map(|l| syn::parse_quote!(#[doc = #l]))
        .collect()
}
