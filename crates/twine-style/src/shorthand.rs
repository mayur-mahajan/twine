//! Shorthands: one name that sets several properties (`padding`, `size`, `border`, …),
//! generated from `__shorthand_table!` (see `table.rs`) for `StyleBuf` and `style!` (and, in
//! `twine-view`, for the view modifiers).

use crate::prop::PropId;

/// Static metadata of one shorthand (see [`SHORTHANDS`]).
#[derive(Clone, Copy, Debug)]
pub struct ShorthandMeta {
    /// Name: the `style!` key, the `StyleBuf` method and the view modifier (`"padding_x"`).
    pub name: &'static str,
    /// Parameters as `(name, type)`, in order; `style!` takes several as a tuple.
    pub params: &'static [(&'static str, &'static str)],
    /// The properties it sets, in order (a property may appear once per parameter).
    pub props: &'static [PropId],
    /// Former names (`#[doc(alias)]`es; not accepted by `style!`).
    pub aliases: &'static [&'static str],
}

/// Generates the shorthands from the rows of `__shorthand_table!`.
macro_rules! define_shorthands {
    (
        [$d:tt]
        $(
            $(#[doc = $doc:literal])*
            $name:ident (
                $(
                    $p:ident : $pk:ident $(<$g:ident>)? [$($pty:tt)+]
                        => $( $var:ident $( ( $($sel:tt)+ ) )? $( = $c:ident )? ),+
                );+
            ) [ $($alias:literal)* ] { $(#[doc = $ex:literal])* };
        )*
    ) => {
        /// Every shorthand, in table order (for documentation and tools).
        pub static SHORTHANDS: &[$crate::ShorthandMeta] = &[
            $(
                $crate::ShorthandMeta {
                    name: ::core::stringify!($name),
                    params: &[$( (::core::stringify!($p), $crate::__prop_type_name!($($pty)+)) ),+],
                    props: &[$( $( $crate::PropId::$var ),+ ),+],
                    aliases: &[$($alias),*],
                },
            )*
        ];

        /// Shorthand builder methods (the same names as the `style!` shorthands and the view
        /// modifiers).
        impl $crate::StyleBuf {
            $(
                $(#[doc = $doc])*
                ///
                $(#[doc = $ex])*
                $(#[doc(alias = $alias)])*
                #[must_use]
                pub fn $name(mut self, $( $p: impl ::core::convert::Into<$crate::__prop_ty!($($pty)+)> ),+) -> Self {
                    $(
                        let $p: $crate::__prop_ty!($($pty)+) = ::core::convert::Into::into($p);
                        $( self.set($crate::__shorthand_prop!([$var] [$($($sel)+)?] [$($c)?] val $p)); )+
                    )+
                    self
                }
            )*
        }

        /// Expands one `style!` entry: a shorthand (the value is always parenthesized by
        /// `__style_props!`; several parameters are a tuple) or, for any other key, the
        /// property of `__style_prop!`. Continues the `__style_props!` muncher.
        #[doc(hidden)]
        #[macro_export]
        macro_rules! __style_shorthand {
            $(
                ($name, ( $( $d $p:expr ),+ $d(,)? ), [$d($d acc:expr),*] $d($d rest:tt)*) => {
                    $crate::__style_props!(
                        [$d($d acc,)* $( $( $crate::__shorthand_prop!([$var] [$($($sel)+)?] [$($c)?] $pk $d $p) ),+ ),+]
                        $d($d rest)*
                    )
                };
            )*
            ($d key:ident, ($d v:expr), [$d($d acc:expr),*] $d($d rest:tt)*) => {
                $crate::__style_props!([$d($d acc,)* $crate::__style_prop!($d key, $d v)] $d($d rest)*)
            };
            ($d key:ident, $d v:expr, [$d($d acc:expr),*] $d($d rest:tt)*) => {
                $crate::__style_props!([$d($d acc,)* $crate::__style_prop!($d key, $d v)] $d($d rest)*)
            };
        }
    };
}

crate::__shorthand_table!(define_shorthands $);
