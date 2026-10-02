//! Application-themed widget classes: the `.class(&MY_CLASS, |cx| ..)` registrations of the
//! theme builders, and the [`ClassCx`] their closures style a node through.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::ops::{Deref, DerefMut};

use twine_engine::{MAX_CLASS_DEPTH, ThemeCx, WidgetClass};

/// What a class registration (`.class(&MY_CLASS, |cx| ..)` of a theme builder) styles a node
/// through: the engine's [`ThemeCx`] (it derefs to it: [`add_style`](ThemeCx::add_style),
/// [`parent_class`](ThemeCx::parent_class), [`dpx`](ThemeCx::dpx), …) plus the theme's style
/// set `S` for the node's display ([`styles`](Self::styles)), so a custom widget can reuse the
/// theme's own styles (its card, button, pressed, focus outline, …) and look native.
///
/// ```
/// use std::rc::Rc;
/// use twine_engine::WidgetClass;
/// use twine_style::{Part, Selector, State};
/// use twine_theme::DefaultTheme;
///
/// /// A gauge: a card with a primary indicator that darkens when pressed.
/// static GAUGE_CLASS: WidgetClass = WidgetClass::new("gauge").parts(&[Part::Main, Part::Indicator]);
///
/// let theme = DefaultTheme::builder()
///     .class(&GAUGE_CLASS, |cx| {
///         let s = cx.styles();
///         let (card, primary, pressed) = (s.card.clone(), s.bg_color_primary.clone(), s.pressed.clone());
///         cx.add_style(Selector::MAIN, card);
///         cx.add_style(Selector::part(Part::Indicator), primary);
///         cx.add_style(Selector::state(State::PRESSED), pressed);
///     })
///     .build();
/// # let _ = Rc::new(theme);
/// ```
pub struct ClassCx<'a, 'b, S> {
    cx: &'a mut ThemeCx<'b>,
    styles: &'a S,
    class: &'static WidgetClass,
}

impl<S> core::fmt::Debug for ClassCx<'_, '_, S> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ClassCx")
            .field("node", &self.cx.node())
            .field("class", &self.class.name)
            .finish_non_exhaustive()
    }
}

impl<'a, S> ClassCx<'a, '_, S> {
    /// The theme's styles for the node's display (e.g. the default theme's
    /// [`Styles`](crate::default::styles::Styles): `card`, `btn`, `pressed`, …).
    #[must_use]
    pub fn styles(&self) -> &'a S {
        self.styles
    }

    /// The class of the node being styled: the registered class itself or a class derived
    /// from it through [`WidgetClass::base`].
    #[must_use]
    pub fn class(&self) -> &'static WidgetClass {
        self.class
    }
}

impl<'b, S> Deref for ClassCx<'_, 'b, S> {
    type Target = ThemeCx<'b>;
    fn deref(&self) -> &ThemeCx<'b> {
        self.cx
    }
}

impl<S> DerefMut for ClassCx<'_, '_, S> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.cx
    }
}

/// A class registration's styling function.
type Styler<S> = Box<dyn Fn(&mut ClassCx<'_, '_, S>)>;

/// The application's class registrations of a theme, in registration order.
pub(crate) struct ClassStyles<S> {
    entries: Vec<(&'static WidgetClass, Styler<S>)>,
}

impl<S> ClassStyles<S> {
    pub(crate) fn new() -> Self {
        Self { entries: Vec::new() }
    }

    /// Registers `style` for `class` (and every class derived from it).
    pub(crate) fn add(
        &mut self,
        class: &'static WidgetClass,
        style: impl Fn(&mut ClassCx<'_, '_, S>) + 'static,
    ) {
        self.entries.push((class, Box::new(style)));
    }

    /// Runs the registrations that match `class`: those of its bases first (the most distant
    /// first), then its own, so the most specific class's styles win. Nothing to do (and no
    /// lineage walk) without registrations; allocates nothing.
    pub(crate) fn apply(&self, cx: &mut ThemeCx<'_>, class: &'static WidgetClass, styles: &S) {
        if self.entries.is_empty() {
            return;
        }
        let mut chain: [Option<&'static WidgetClass>; MAX_CLASS_DEPTH] = [None; MAX_CLASS_DEPTH];
        for (slot, c) in chain.iter_mut().zip(class.lineage()) {
            *slot = Some(c);
        }
        let mut ccx = ClassCx { cx, styles, class };
        for c in chain.iter().rev().flatten() {
            for (k, style) in &self.entries {
                if k.is(c) {
                    style(&mut ccx);
                }
            }
        }
    }
}

/// Generates the builder methods every built-in theme builder shares: `mode`, `parent`,
/// `element`, `element_in` and `class`. The builder is a tuple struct around the theme under
/// construction, whose fields are `mode`, `parent`, `design` and `classes`; `$styles` is the
/// theme's style set type and `$theme` the theme type (named in the doctests).
macro_rules! common_builder_methods {
    ($styles:ty, $theme:ident) => {
        /// The mode the theme starts in when installed (default [`ThemeMode::Light`]);
        /// switch it at run time with
        /// [`Engine::set_theme_mode`](twine_engine::Engine::set_theme_mode). A mode the theme
        /// does not support ([`ThemeHook::modes`](twine_engine::ThemeHook::modes)) starts as
        /// `Light` instead, with a warning. Never panics.
        ///
        /// ```
        #[doc = concat!("use twine_theme::{", stringify!($theme), ", ThemeHook, ThemeMode};")]
        ///
        #[doc = concat!("let t = ", stringify!($theme), "::builder().mode(ThemeMode::HighContrast).build();")]
        /// assert_eq!(ThemeHook::mode(&t), ThemeMode::HighContrast);
        /// ```
        pub fn mode(mut self, mode: ThemeMode) -> Self {
            self.0.mode = mode;
            self
        }

        /// Applies `parent` before this theme on every node (LVGL `lv_theme_set_parent`): the
        /// parent's styles come first, so this theme's styles win. The parent's design
        /// elements are kept where this theme does not define them (e.g. the parent's
        /// application elements). Never panics; the parent is shared, not copied.
        ///
        /// ```
        /// use std::rc::Rc;
        /// use twine_core::Color;
        /// use twine_style::design::ColorElement;
        #[doc = concat!("use twine_theme::", stringify!($theme), ";")]
        ///
        /// const BRAND: ColorElement = ColorElement::custom(0);
        /// let base = Rc::new(twine_theme::DefaultTheme::builder().element(BRAND, Color::hex(0x0000_6600)).build());
        #[doc = concat!("let t = ", stringify!($theme), "::builder().parent(base).build(); // keeps BRAND")]
        /// # let _ = t;
        /// ```
        pub fn parent(mut self, parent: Rc<dyn Theme>) -> Self {
            self.0.parent = Some(parent);
            self
        }

        /// Gives the [design element](twine_style::design) `element` the value `value` in
        /// every mode: an application element ([`Element::custom`]) or an override of a
        /// standard one.
        pub fn element<T: ElementType>(mut self, element: Element<T>, value: T) -> Self {
            self.0.design.set(None, element, value);
            self
        }

        /// Gives `element` the value `value` in `mode` only (applied after
        /// [`element`](Self::element), so it wins in that mode). Never panics.
        ///
        /// ```
        /// use twine_core::Color;
        /// use twine_style::design::ColorElement;
        #[doc = concat!("use twine_theme::{", stringify!($theme), ", ThemeMode};")]
        ///
        /// const BRAND: ColorElement = ColorElement::custom(0);
        #[doc = concat!("let t = ", stringify!($theme), "::builder()")]
        ///     .element(BRAND, Color::hex(0x0000_6600))
        ///     .element_in(ThemeMode::Dark, BRAND, Color::hex(0x0066_FF66)) // lighter on dark
        ///     .build();
        /// # let _ = t;
        /// ```
        pub fn element_in<T: ElementType>(mut self, mode: ThemeMode, element: Element<T>, value: T) -> Self {
            self.0.design.set(Some(mode), element, value);
            self
        }

        /// Styles nodes of `class` — and of every class derived from it through
        /// [`WidgetClass::base`] — with `style`, after the theme's own styles (so `style`
        /// wins over them, e.g. over the button look of a custom class whose base is the
        /// button). `style` gets a [`ClassCx`]: the node (add styles with
        /// [`add_style`](twine_engine::ThemeCx::add_style)) and the theme's styles
        /// ([`ClassCx::styles`]) to look native. Registrations of a class's bases run first;
        /// several registrations of one class run in order. The theme looks registrations up
        /// when a node is created or re-themed (a pointer comparison per registration), never
        /// while drawing.
        pub fn class(
            mut self,
            class: &'static WidgetClass,
            style: impl Fn(&mut ClassCx<'_, '_, $styles>) + 'static,
        ) -> Self {
            self.0.classes.add(class, style);
            self
        }
    };
}

pub(crate) use common_builder_methods;
