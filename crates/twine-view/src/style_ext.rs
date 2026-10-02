//! [`StyleExt`]: the style modifiers (whole styles and style properties) of every view and of every [`StyleScope`]
//! (per-part / per-state styling with [`ViewExt::part`] and [`ViewExt::on_state`]).
//!
//! The modifiers are generated once, from the property and shorthand tables of `twine-style`
//! (the names are the `style!` keys and the `StyleBuf` builder methods), and implemented by
//! both: on a view they set local properties of the `Main` part in the default state; in a
//! scope, local properties for the scope's [`Selector`]. So a scope has exactly the modifiers
//! a view has, and both accept constants, signals, memos and closures ([`IntoProp`]).

use alloc::boxed::Box;

use twine_engine::{Engine, NodeId, State};
use twine_style::Part;
use twine_style::{GridTracks, Length, PropId, Selector, StyleProp, StyleRef, TransitionValue};

use crate::bind::bind_node;
use crate::build::{BuildCx, BuildOp};
use crate::containers::Layout;
use crate::modifiers::ViewExt;
use crate::prop::{IntoProp, Prop};

// The build steps of the generic modifiers, as free functions: a closure written inside a
// trait method has a distinct type (and machine code) per implementing type; these depend
// only on their own parameters, so all views and scopes share one copy per value type.

/// The build step of [`StyleExt::style_prop`] on a view (the `Main` part in the default state:
/// a constant, so a view's binding stores no selector).
fn main_prop_op<T: 'static>(
    p: Prop<T>,
    make: fn(T) -> StyleProp,
) -> impl FnOnce(&mut BuildCx<'_>, NodeId) + 'static {
    move |cx, node| {
        bind_node(cx, node, p, move |e, n, v| {
            e.set_local_prop(n, Selector::MAIN, make(v));
        });
    }
}

/// The build step of [`StyleExt::style_props`] on a view.
fn main_props_op<T: 'static, const N: usize>(
    p: Prop<T>,
    make: fn(T) -> [StyleProp; N],
) -> impl FnOnce(&mut BuildCx<'_>, NodeId) + 'static {
    move |cx, node| {
        bind_node(cx, node, p, move |e, n, v| {
            set_props(e, n, Selector::MAIN, make(v));
        });
    }
}

/// The build step of [`StyleExt::style_prop`] in a [`StyleScope`].
fn style_prop_op<T: 'static>(
    p: Prop<T>,
    make: fn(T) -> StyleProp,
    sel: Selector,
) -> impl FnOnce(&mut BuildCx<'_>, NodeId) + 'static {
    move |cx, node| bind_node(cx, node, p, move |e, n, v| e.set_local_prop(n, sel, make(v)))
}

/// The build step of [`StyleExt::style_props`] in a [`StyleScope`].
fn style_props_op<T: 'static, const N: usize>(
    p: Prop<T>,
    make: fn(T) -> [StyleProp; N],
    sel: Selector,
) -> impl FnOnce(&mut BuildCx<'_>, NodeId) + 'static {
    move |cx, node| bind_node(cx, node, p, move |e, n, v| set_props(e, n, sel, make(v)))
}

/// The build step of the grid template modifiers (`grid_column_tracks`, `grid_row_tracks`).
fn tracks_op(
    p: Prop<GridTracks>,
    prop: PropId,
    sel: Selector,
) -> impl FnOnce(&mut BuildCx<'_>, NodeId) + 'static {
    move |cx, node| {
        bind_node(cx, node, p, move |e: &mut Engine, n, t| {
            e.set_local_grid_tracks(n, sel, prop, t);
        });
    }
}

/// The build step of the `transition` modifier.
fn transition_op(p: Prop<TransitionValue>, sel: Selector) -> impl FnOnce(&mut BuildCx<'_>, NodeId) + 'static {
    move |cx, node| {
        bind_node(cx, node, p, move |e: &mut Engine, n, t| {
            e.set_local_transition(n, sel, t);
        });
    }
}

/// The build step of the `layout` modifier.
fn layout_op(p: Prop<Layout>, sel: Selector) -> impl FnOnce(&mut BuildCx<'_>, NodeId) + 'static {
    move |cx, node| bind_node(cx, node, p, move |e, n, l: Layout| l.apply(e, n, sel))
}

/// The build step of [`StyleExt::style`] (`theme`: [`StyleExt::class_style`]). One copy for
/// every view and scope type.
fn style_op(s: StyleRef, sel: Selector, theme: bool) -> impl FnOnce(&mut BuildCx<'_>, NodeId) + 'static {
    move |cx, node| {
        if theme {
            cx.engine().add_theme_style(node, s, sel);
        } else {
            cx.engine().add_style(node, s, sel);
        }
    }
}

/// Sets `props` as local properties of `node` for `sel` (idempotent in the engine).
fn set_props<const N: usize>(e: &mut Engine, node: NodeId, sel: Selector, props: [StyleProp; N]) {
    for p in props {
        e.set_local_prop(node, sel, p);
    }
}

/// One style-property modifier of [`StyleExt`] (see `prop_modifiers!`), by the row's kind:
/// `tracks` takes any grid template ([`GridTracks`]), `trans` any transition
/// ([`TransitionValue`]), `layout` a whole [`Layout`], every other
/// one any property of the payload type (`Length` properties take pixels, `Length::pct`,
/// `Length::dp`, …, `Radius` ones pixels or `Radius::Circle`, `DurationMs` ones a `Duration`:
/// every `From` conversion).
macro_rules! prop_modifier {
    (trans $name:ident $key:ident [$($ty:tt)+] [$(#[$m:meta])*]) => {
        $(#[$m])*
        ///
        /// Takes a [`Transition`](twine_style::Transition) by value (built where it is used,
        /// moved into the node's local style once at build time), a `&'static Transition` or an
        /// `Rc<Transition>` (any [`TransitionValue`]), or a signal or closure of one. On a view
        /// it applies to every state change of the `Main` part; in a state scope, when the node
        /// enters that state. [`Transition::all`](twine_style::Transition::all) animates what the
        /// state change alters (no property list to keep in sync);
        /// [`Transition::of`](twine_style::Transition::of) a [`Props`](twine_style::Props) set.
        /// Honours the [`Motion`](twine_anim::Motion) preference. Starting a transition
        /// allocates nothing.
        ///
        /// ```
        /// use twine_view::prelude::*;
        /// let _v = button(label("OK"))
        ///     .transition(Transition::all(Duration::ms(150)).ease_out())
        ///     .on_state(State::PRESSED, |s| s.bg(Color::hex(0x1565C0)).transform_scale(Scale::pct(97)));
        /// ```
        #[must_use]
        fn $key<M>(self, t: impl IntoProp<TransitionValue, M>) -> Self {
            let sel = self.__style_selector();
            self.__push_style_op(Box::new(transition_op(t.into_prop(), sel)))
        }
    };
    (tracks $name:ident $key:ident [$($ty:tt)+] [$(#[$m:meta])*]) => {
        $(#[$m])*
        ///
        /// Takes any grid template ([`GridTracks`]): a `'static` slice or array (stored as is),
        /// a `Vec` such as [`grid_tracks!`](twine_style::grid_tracks) (held by the node's local
        /// style, released with it), a signal or a closure. Equal tracks do nothing.
        #[must_use]
        fn $key<M>(self, tracks: impl IntoProp<GridTracks, M>) -> Self {
            let sel = self.__style_selector();
            self.__push_style_op(Box::new(tracks_op(tracks.into_prop(), PropId::$name, sel)))
        }
    };
    (layout $name:ident $key:ident [$($ty:tt)+] [$(#[$m:meta])*]) => {
        /// How the node arranges its children: [`Layout::row()`](crate::Layout::row),
        /// [`Layout::column()`](crate::Layout::column),
        /// [`Layout::flex(flow)`](crate::Layout::flex),
        /// [`Layout::grid(columns, rows)`](crate::Layout::grid) or
        /// [`Layout::none()`](crate::Layout::none). Sets the `layout` style property (and the
        /// flow or the grid tracks) exactly like the [`row`](crate::row) /
        /// [`column`](crate::column) / [`flex`](crate::flex) / [`grid`](crate::grid) containers, but keeps the view's look
        /// (e.g. a [`container`](crate::container) keeps the theme's card look). May be dynamic,
        /// and may depend on the state (in a [`StyleScope`]).
        ///
        /// ```
        /// use twine_view::prelude::*;
        /// let _v = container((label("a"), label("b"))).layout(Layout::row()).gap(8);
        /// ```
        $(#[$m])*
        #[must_use]
        fn $key<M>(self, layout: impl IntoProp<Layout, M>) -> Self {
            let sel = self.__style_selector();
            self.__push_style_op(Box::new(layout_op(layout.into_prop(), sel)))
        }
    };
    ($kind:ident $name:ident $key:ident [$($ty:tt)+] [$(#[$m:meta])*]) => {
        $(#[$m])*
        #[must_use]
        fn $key<M>(self, v: impl IntoProp<::twine_style::__prop_ty!($($ty)+), M>) -> Self {
            self.style_prop(v, ::twine_style::StyleProp::$name)
        }
    };
}

/// Generates one modifier per style property from `twine_style::__prop_table!`, named like
/// the `style!` key and the `StyleBuf` builder method.
macro_rules! prop_modifiers {
    (
        []
        $(
            $(#[doc = $gdoc:literal])*
            $group:ident {
                $(
                    $(#[doc = $doc:literal])*
                    $name:ident ( $key:ident ) : $kind:ident [$($ty:tt)+] [ $($flag:ident)* ] $default:ident
                        [ $($alias:literal)* ];
                )*
            }
        )*
    ) => {
        $($(
            prop_modifier! {
                $kind $name $key [$($ty)+]
                [
                    $(#[doc = $doc])*
                    ///
                    #[doc = ::core::concat!(
                        "Sets the local style property [`StyleProp::", ::core::stringify!($name),
                        "`](::twine_style::StyleProp::", ::core::stringify!($name), ") (of the ",
                        "`Main` part on a view, for the scope's selector in a [`StyleScope`])."
                    )]
                    $(#[doc(alias = $alias)])*
                ]
            }
        )*)*
    };
}

/// Generates the shorthand modifiers (`padding`, `size`, `border`, …) from
/// `twine_style::__shorthand_table!`: each parameter is a property of its payload type (with
/// its own [`IntoProp`] marker, the `<G>` of the row) and one binding that sets its properties.
macro_rules! shorthand_modifiers {
    (
        []
        $(
            $(#[doc = $doc:literal])*
            $name:ident (
                $(
                    $p:ident : $pk:ident <$g:ident> [$($pty:tt)+]
                        => $( $var:ident $( ( $($sel:tt)+ ) )? $( = $c:ident )? ),+
                );+
            ) [ $($alias:literal)* ] { $(#[doc = $ex:literal])* };
        )*
    ) => {
        $(
            $(#[doc = $doc])*
            $(#[doc(alias = $alias)])*
            #[must_use]
            fn $name<$($g),+>(
                self,
                $( $p: impl IntoProp<::twine_style::__prop_ty!($($pty)+), $g> ),+
            ) -> Self {
                self $(
                    .style_props($p, |$p: ::twine_style::__prop_ty!($($pty)+)| {
                        [$( ::twine_style::__shorthand_prop!([$var] [$($($sel)+)?] [$($c)?] val $p) ),+]
                    })
                )+
            }
        )*
    };
}

mod sealed {
    /// Only views and style scopes have style modifiers.
    pub trait Sealed {}
}

impl<V: ViewExt> sealed::Sealed for V {}
impl<V: ViewExt> sealed::Sealed for StyleScope<V> {}

/// The style modifiers: whole styles ([`style`](Self::style), [`class_style`](Self::class_style))
/// and the style-property modifiers, one per style property and one per shorthand (generated
/// from the property table of `twine-style`: the names are the `style!` keys and the
/// `StyleBuf` builder methods, see `twine_style::PROPERTIES.md`), plus [`fill`](Self::fill)
/// and the generic [`style_prop`](Self::style_prop) / [`style_props`](Self::style_props).
///
/// Implemented by every view ([`ViewExt`]), where the modifiers set **local style
/// properties** of the `Main` part in the default state, and by [`StyleScope`], where they set
/// local properties for the scope's part and state (see [`ViewExt::part`] and
/// [`ViewExt::on_state`]). Every value may be a constant, a signal, a memo or a closure
/// ([`IntoProp`]): a constant is set once at build time (no binding), a dynamic value is one
/// binding whose re-runs with an unchanged value do nothing (no invalidation, no layout: the
/// engine setters are idempotent).
///
/// ```
/// use twine_view::prelude::*;
///
/// let cx = twine_reactive::Runtime::take().unwrap().create_root();
/// let hot = cx.signal(false);
/// let _v = label("21 °C")
///     .text_color(move || if hot.get() { Color::RED } else { Color::BLACK })
///     .padding(4)
///     .bg(Color::hex(0xEEEEEE))
///     .radius(6)
///     .on_state(State::PRESSED, |s| s.bg(Color::hex(0xCCCCCC)))
///     .test_id("temp");
/// cx.dispose();
/// ```
pub trait StyleExt: Sized + sealed::Sealed {
    /// The selector the modifiers set local properties for.
    #[doc(hidden)]
    fn __style_selector(&self) -> Selector;

    /// Appends a build step of the underlying view.
    #[doc(hidden)]
    #[must_use]
    fn __push_style_op(self, op: BuildOp) -> Self;

    /// The build step of [`style_prop`](Self::style_prop) (a view's needs no selector).
    #[doc(hidden)]
    #[must_use]
    fn __style_prop<T: 'static>(self, p: Prop<T>, make: fn(T) -> StyleProp) -> Self;

    /// The build step of [`style_props`](Self::style_props).
    #[doc(hidden)]
    #[must_use]
    fn __style_props<T: 'static, const N: usize>(self, p: Prop<T>, make: fn(T) -> [StyleProp; N]) -> Self;

    /// Adds a whole style to the node: a `static` [`Style`](twine_style::Style) written with
    /// `style!` (`&CARD`, in flash), or a heap one — a
    /// [`StyleBuf`](twine_style::StyleBuf) (moved into an `Rc`) or a shared
    /// `Rc<StyleBuf>` / [`StyleRef`] (one style for many nodes, e.g. a theme's
    /// `styles()`). On a view it applies to the `Main` part in the default state; in a
    /// [`StyleScope`] to the scope's part and states (`.on_state(State::PRESSED, |s|
    /// s.style(&CARD_PRESSED))`).
    ///
    /// Priority: above the theme's styles and [`class_style`](Self::class_style), below local
    /// properties (the other modifiers); a later `style` wins over an earlier one for the same
    /// selector (LVGL `lv_obj_add_style`). Compose styles at definition time
    /// (`style! { ..BASE, radius: 4 }`, `Style::merge`), not by stacking many: each added style
    /// is one entry the cascade visits. Added once at build time (no binding, nothing per
    /// frame); a heap style is shared, not copied. Never panics.
    ///
    /// ```
    /// use std::rc::Rc;
    /// use twine_view::prelude::*;
    ///
    /// static CARD: Style = style! { bg: Color::WHITE, radius: 8, padding: 12 };
    /// static CARD_PRESSED: Style = style! { ..CARD, bg: Color::hex(0xEEEEEE) };
    /// let accent = Rc::new(StyleBuf::from(&CARD).border(2, Color::BLUE)); // built at run time
    ///
    /// let _v = column((
    ///     container(label("static")).style(&CARD).on_state(State::PRESSED, |s| s.style(&CARD_PRESSED)),
    ///     container(label("heap")).style(StyleBuf::new().bg(Color::RED)),
    ///     container(label("shared")).style(accent.clone()),
    ///     container(label("shared too")).style(accent),
    /// ));
    /// ```
    #[doc(alias = "lv_obj_add_style")]
    #[must_use]
    fn style(self, s: impl Into<StyleRef>) -> Self {
        let sel = self.__style_selector();
        self.__push_style_op(Box::new(style_op(s.into(), sel, false)))
    }

    /// Adds a whole style with theme priority (below every [`style`](Self::style) and local
    /// property, like the theme's own class styles), for the `Main` part on a view or the
    /// scope's selector in a [`StyleScope`]. Takes the same styles as `style`. Never panics.
    ///
    /// ```
    /// use twine_view::prelude::*;
    /// static FALLBACK: Style = style! { bg: Color::hex(0xF5F5F5), radius: 4 };
    /// let _v = container(()).class_style(&FALLBACK).radius(8); // the local radius wins
    /// ```
    #[must_use]
    fn class_style(self, s: impl Into<StyleRef>) -> Self {
        let sel = self.__style_selector();
        self.__push_style_op(Box::new(style_op(s.into(), sel, true)))
    }

    /// Binds a local style property built by `make` from the value (e.g. a `StyleProp`
    /// variant: `self.style_prop(w, StyleProp::LineWidth)`), for the `Main` part on a view or
    /// the scope's selector in a [`StyleScope`].
    ///
    /// `make` is a `fn` pointer, not a closure: the build step and its binding are then
    /// compiled once per value type, shared by every modifier, view and scope type (the
    /// generated modifiers are trait methods, so a closure type would differ per implementing
    /// type and per modifier). A dynamic value pays one indirect call per binding run, next to
    /// the style update itself.
    /// ```
    /// use twine_view::prelude::*;
    ///
    /// fn app(cx: Scope) -> impl View {
    ///     let grow = cx.signal(1i32);
    ///     row((
    ///         container(()).style_prop(2, StyleProp::FlexGrow), // a constant
    ///         container(()).style_prop(grow, StyleProp::FlexGrow), // bound to a signal
    ///     ))
    /// }
    /// # let _ = app;
    /// ```
    #[must_use]
    fn style_prop<T: 'static, M>(self, v: impl IntoProp<T, M>, make: fn(T) -> StyleProp) -> Self {
        self.__style_prop(v.into_prop(), make)
    }

    /// Binds several local style properties built by `make` from one value (a `fn` pointer,
    /// see [`style_prop`](Self::style_prop)).
    /// ```
    /// use twine_view::prelude::*;
    ///
    /// /// One value, two properties: the same grid cell column and row.
    /// fn diagonal(i: i32) -> [StyleProp; 2] {
    ///     [StyleProp::GridCellColumn(i), StyleProp::GridCellRow(i)]
    /// }
    /// let _v = container(()).style_props(1, diagonal);
    /// ```
    #[must_use]
    fn style_props<T: 'static, M, const N: usize>(
        self,
        v: impl IntoProp<T, M>,
        make: fn(T) -> [StyleProp; N],
    ) -> Self {
        self.__style_props(v.into_prop(), make)
    }

    ::twine_style::__prop_table!(prop_modifiers);

    ::twine_style::__shorthand_table!(shorthand_modifiers);

    /// Fills the parent: width and height 100 % of the parent's content area (padding
    /// excluded), i.e. `.size(Length::pct(100), Length::pct(100))`.
    ///
    /// This is a size, not a share of free space: in a flex [`row`](crate::row) or
    /// [`column`](crate::column) with siblings it takes the whole line and the siblings
    /// overflow (or wrap). To take the space the siblings leave, use
    /// [`flex_grow(1)`](Self::flex_grow) on the main axis (or a [`spacer`](crate::spacer)).
    ///
    /// ```
    /// use twine_view::prelude::*;
    /// let _v = column(label("a")).fill();
    /// ```
    #[doc(alias = "size_full")]
    #[must_use]
    fn fill(self) -> Self {
        let sel = self.__style_selector();
        self.__push_style_op(Box::new(move |cx: &mut BuildCx<'_>, node| {
            set_props(
                cx.engine(),
                node,
                sel,
                [
                    StyleProp::Width(Length::pct(100).into()),
                    StyleProp::Height(Length::pct(100).into()),
                ],
            );
        }))
    }

    /// Width 100 % of the parent's content area, i.e. `.width(Length::pct(100))` (a size, not
    /// a share of free space: see [`fill`](Self::fill)).
    /// ```
    /// use twine_view::prelude::*;
    /// let _v = button(label("OK")).fill_width();
    /// ```
    #[must_use]
    fn fill_width(self) -> Self {
        let sel = self.__style_selector();
        self.__push_style_op(Box::new(move |cx: &mut BuildCx<'_>, node| {
            cx.engine()
                .set_local_prop(node, sel, StyleProp::Width(Length::pct(100).into()));
        }))
    }

    /// Height 100 % of the parent's content area, i.e. `.height(Length::pct(100))` (a size, not
    /// a share of free space: see [`fill`](Self::fill)).
    /// ```
    /// use twine_view::prelude::*;
    /// let _v = column(label("a")).fill_height();
    /// ```
    #[must_use]
    fn fill_height(self) -> Self {
        let sel = self.__style_selector();
        self.__push_style_op(Box::new(move |cx: &mut BuildCx<'_>, node| {
            cx.engine()
                .set_local_prop(node, sel, StyleProp::Height(Length::pct(100).into()));
        }))
    }
}

impl<V: ViewExt> StyleExt for V {
    #[inline]
    fn __style_selector(&self) -> Selector {
        Selector::MAIN
    }

    #[inline]
    fn __push_style_op(self, op: BuildOp) -> Self {
        self.push_op(op)
    }

    #[inline]
    fn __style_prop<T: 'static>(self, p: Prop<T>, make: fn(T) -> StyleProp) -> Self {
        self.push_op(Box::new(main_prop_op(p, make)))
    }

    #[inline]
    fn __style_props<T: 'static, const N: usize>(self, p: Prop<T>, make: fn(T) -> [StyleProp; N]) -> Self {
        self.push_op(Box::new(main_props_op(p, make)))
    }
}

/// The style modifiers of a view for one part and/or state: what the closure of
/// [`ViewExt::part`], [`ViewExt::on_state`] and [`ViewExt::styled`] receives. It has exactly
/// the modifiers of a view ([`StyleExt`]); they set local properties of the view's node for the
/// scope's [`Selector`] instead of the `Main` part in the default state.
///
/// # Combining selectors
///
/// Scopes nest in either order: [`on_state`](Self::on_state) adds states to the scope's
/// states (`PRESSED` inside `CHECKED` is `PRESSED | CHECKED`), [`part`](Self::part) sets the
/// part (the innermost part applies), so `part(Knob, on_state(PRESSED))` and
/// `on_state(PRESSED, part(Knob))` style the same selector. Each selector is one local style
/// of the node: setting a property twice for the same selector keeps the last value (a
/// binding and a constant for the same property and selector overwrite each other in build
/// order, then on each binding run). Different selectors coexist and are chosen at
/// resolution like any styles: the matching selector with the highest
/// [state precedence](twine_style::State#precedence) wins (table at [`ViewExt::on_state`]), so
/// a `PRESSED` look overrides the default look while pressed and a more specific
/// `PRESSED | CHECKED` look overrides both.
///
/// Constants become local style properties for the selector (set once, no binding); signals,
/// memos and closures become one binding each, which sets the local property for the selector
/// when its value changes (a re-run with an unchanged value does nothing). A node that uses no
/// scope pays nothing for them.
///
/// ```
/// use twine_view::prelude::*;
///
/// let cx = twine_reactive::Runtime::take().unwrap().create_root();
/// let level = cx.signal(40);
/// let alarm = cx.signal(false);
/// let _v = slider(level)
///     // The indicator's colour follows a signal.
///     .part(Part::Indicator, |s| {
///         s.bg(move || if alarm.get() { Color::RED } else { Color::hex(0x2E7D32) })
///             // A part in a state: the indicator while the slider is pressed.
///             .on_state(State::PRESSED, |s| s.bg(Color::hex(0x1B5E20)))
///     })
///     .on_state(State::PRESSED, |s| s.transform_scale(Scale::pct(97)));
/// cx.dispose();
/// ```
#[derive(Debug)]
#[must_use]
pub struct StyleScope<V> {
    view: V,
    selector: Selector,
}

impl<V: ViewExt> StyleScope<V> {
    /// Runs `f` on a scope of `view` for `selector` and returns the view.
    fn run(view: V, selector: Selector, f: impl FnOnce(Self) -> Self) -> V {
        f(Self { view, selector }).view
    }

    /// The part and states the modifiers apply to.
    ///
    /// ```
    /// use twine_view::prelude::*;
    /// let _v = slider(twine_reactive::Runtime::take().unwrap().create_root().signal(0)).part(Part::Knob, |s| {
    ///     assert_eq!(s.selector(), Selector::part(Part::Knob));
    ///     s.on_state(State::PRESSED, |s| {
    ///         assert_eq!(s.selector(), Selector::part(Part::Knob).with_state(State::PRESSED));
    ///         s
    ///     })
    /// });
    /// ```
    #[must_use]
    pub fn selector(&self) -> Selector {
        self.selector
    }

    /// Styles part `part` in the scope's states: `.on_state(State::PRESSED, |s| s.part(Part::Knob,
    /// ..))` is the same selector as `.part(Part::Knob, |s| s.on_state(State::PRESSED, ..))`.
    /// The innermost part applies (a part replaces the scope's part; states add up). The scope
    /// returned keeps its own selector. Never panics.
    ///
    /// ```
    /// use twine_view::prelude::*;
    /// let _v = slider(twine_reactive::Runtime::take().unwrap().create_root().signal(0)).on_state(State::PRESSED, |s| {
    ///     s.transform_scale(Scale::pct(97)) // the pressed slider
    ///         .part(Part::Knob, |s| {
    ///             assert_eq!(s.selector(), Selector::part(Part::Knob).with_state(State::PRESSED));
    ///             s.bg(Color::RED) // its knob while pressed
    ///         })
    /// });
    /// ```
    pub fn part(self, part: Part, f: impl FnOnce(Self) -> Self) -> Self {
        let outer = self.selector;
        Self {
            view: StyleScope::run(
                self.view,
                Selector {
                    part,
                    state: outer.state,
                },
                f,
            ),
            selector: outer,
        }
    }

    /// Styles the scope's part while `state` is also active (the states add up:
    /// `.part(Part::Knob, |s| s.on_state(State::PRESSED, ..))` styles the pressed knob). The
    /// scope returned keeps its own selector, so modifiers after the call apply to it again.
    /// Never panics.
    ///
    /// ```
    /// use twine_view::prelude::*;
    /// let _v = switch(twine_reactive::Runtime::take().unwrap().create_root().signal(false)).part(Part::Indicator, |s| {
    ///     s.bg(Color::hex(0x9E9E9E))
    ///         .on_state(State::CHECKED, |s| s.bg(Color::hex(0x2E7D32)))
    ///         .radius(Radius::Circle) // back to the indicator in every state
    /// });
    /// ```
    pub fn on_state(self, state: State, f: impl FnOnce(Self) -> Self) -> Self {
        let outer = self.selector;
        let inner = outer.with_state(outer.state | state);
        Self {
            view: StyleScope::run(self.view, inner, f),
            selector: outer,
        }
    }
}

impl<V: ViewExt> StyleExt for StyleScope<V> {
    #[inline]
    fn __style_selector(&self) -> Selector {
        self.selector
    }

    #[inline]
    fn __push_style_op(self, op: BuildOp) -> Self {
        Self {
            view: self.view.push_op(op),
            selector: self.selector,
        }
    }

    fn __style_prop<T: 'static>(self, p: Prop<T>, make: fn(T) -> StyleProp) -> Self {
        let sel = self.selector;
        self.__push_style_op(Box::new(style_prop_op(p, make, sel)))
    }

    fn __style_props<T: 'static, const N: usize>(self, p: Prop<T>, make: fn(T) -> [StyleProp; N]) -> Self {
        let sel = self.selector;
        self.__push_style_op(Box::new(style_props_op(p, make, sel)))
    }
}

/// [`ViewExt::part`] / [`ViewExt::on_state`] / [`ViewExt::styled`].
pub(crate) fn scoped<V: ViewExt>(
    view: V,
    selector: Selector,
    f: impl FnOnce(StyleScope<V>) -> StyleScope<V>,
) -> V {
    StyleScope::run(view, selector, f)
}
