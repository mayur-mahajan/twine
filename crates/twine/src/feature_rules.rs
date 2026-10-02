//! [`feature_rules!`](crate::feature_rules): compile-time checks of an application's own cargo
//! feature combinations.

/// Checks combinations of the **calling crate's** cargo features at compile time: each rule
/// expands to `#[cfg(..)] compile_error!(..)` items, evaluated in the crate that invokes the
/// macro, so a wrong selection is a compile error naming the features — not a link error or a
/// misbehaving board. Nothing is generated when the selection is valid (no code, no flash).
///
/// Firmware templates often offer alternatives as features (one display of several, at most
/// one touch controller, one demo); this replaces the hand-written `compile_error!` matrices
/// (one `cfg(all(..))` per pair) with one line per group:
///
/// | Rule | Error when |
/// |------|------------|
/// | `exactly_one "group": ["a", "b", ..];` | none or more than one of the features is enabled |
/// | `at_most_one "group": ["a", "b", ..];` | two of the features are enabled |
/// | `requires "f": ["a", "b", ..];` | `f` is enabled without any of the features |
/// | `excludes "f": ["a", "b", ..];` | `f` is enabled together with one of the features |
///
/// Feature names are string literals; a name the crate does not define is reported by the
/// compiler's `unexpected_cfgs` lint.
///
/// In a firmware crate with these features (`Cargo.toml` defines them; the default selects one
/// display):
///
/// ```text
/// twine::feature_rules! {
///     exactly_one "display": ["panel-ili9341", "panel-st7789", "oled-ssd1306"];
///     at_most_one "touch": ["touch-xpt2046", "touch-ft6x36"];
///     requires "demo-calibrate": ["touch-xpt2046", "touch-ft6x36"];
///     excludes "oled-ssd1306": ["touch-ft6x36"];
/// }
/// ```
///
/// Compiled here, in a crate that has none of these features, `exactly_one` reports the
/// missing display:
///
/// ```compile_fail
/// // No display feature enabled: "enable one display feature: `panel-a` `panel-b`".
/// twine::feature_rules! {
///     exactly_one "display": ["panel-a", "panel-b"];
/// }
/// # fn main() {}
/// ```
///
/// ```compile_fail
/// // `twine`'s tests always build it with both colour formats: they exclude each other here.
/// twine::feature_rules! {
///     at_most_one "colour": ["color-rgb565", "color-rgb565-swapped"];
/// }
/// # fn main() {}
/// ```
///
/// ```
/// // None or one of them: fine.
/// twine::feature_rules! {
///     at_most_one "touch": ["touch-a", "touch-b", "touch-c"];
///     excludes "oled": ["touch-a"];
/// }
/// # fn main() {}
/// ```
#[macro_export]
macro_rules! feature_rules {
    (@rule exactly_one $group:literal [$($f:literal),+]) => {
        #[cfg(not(any($(feature = $f),+)))]
        ::core::compile_error!(::core::concat!("enable one ", $group, " feature:", $(" `", $f, "`"),+));
        $crate::feature_rules!(@pairs $group [$($f),+]);
    };
    (@rule at_most_one $group:literal [$($f:literal),+]) => {
        $crate::feature_rules!(@pairs $group [$($f),+]);
    };
    (@rule requires $feature:literal [$($f:literal),+]) => {
        #[cfg(all(feature = $feature, not(any($(feature = $f),+))))]
        ::core::compile_error!(::core::concat!("feature `", $feature, "` needs one of:", $(" `", $f, "`"),+));
    };
    (@rule excludes $feature:literal [$($f:literal),+]) => {
        $(
            #[cfg(all(feature = $feature, feature = $f))]
            ::core::compile_error!(::core::concat!("features `", $feature, "` and `", $f, "` exclude each other"));
        )+
    };
    (@pairs $group:literal [$head:literal $(, $rest:literal)*]) => {
        $(
            #[cfg(all(feature = $head, feature = $rest))]
            ::core::compile_error!(::core::concat!(
                "features `", $head, "` and `", $rest, "` exclude each other: enable at most one ", $group, " feature"
            ));
        )*
        $crate::feature_rules!(@pairs $group [$($rest),*]);
    };
    (@pairs $group:literal []) => {};
    ($($rule:ident $name:literal : [$($f:literal),+ $(,)?];)+) => {
        $($crate::feature_rules!(@rule $rule $name [$($f),+]);)+
    };
}
