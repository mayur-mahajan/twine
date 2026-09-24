//! [`StyleCache`]: lazily cached hot properties of a node's `Main` part ([`MainStyle`]).

use core::cell::Cell;

use twine_core::{Color, Insets, Opa};
use twine_text::Font;

/// The most used properties of a node's `Main` part in its current state, resolved once and
/// cached (see [`Engine::cached_main`](crate::Engine::cached_main)).
#[derive(Clone, Copy, Debug)]
pub struct MainStyle {
    /// `BgColor`.
    pub bg_color: Color,
    /// `BgOpa`.
    pub bg_opa: Opa,
    /// `Radius`.
    pub radius: i32,
    /// `BorderWidth`.
    pub border_width: i32,
    /// `BorderColor`.
    pub border_color: Color,
    /// `Opa`.
    pub opa: Opa,
    /// `PadLeft`, `PadTop`, `PadRight`, `PadBottom`.
    pub pad: Insets,
    /// `TextColor` (inherited).
    pub text_color: Color,
    /// `TextFont` (inherited).
    pub font: &'static Font,
    /// `Recolor`.
    pub recolor: Color,
    /// `RecolorOpa`.
    pub recolor_opa: Opa,
}

impl PartialEq for MainStyle {
    fn eq(&self, o: &Self) -> bool {
        self.bg_color == o.bg_color
            && self.bg_opa == o.bg_opa
            && self.radius == o.radius
            && self.border_width == o.border_width
            && self.border_color == o.border_color
            && self.opa == o.opa
            && self.pad == o.pad
            && self.text_color == o.text_color
            && core::ptr::eq(self.font, o.font)
            && self.recolor == o.recolor
            && self.recolor_opa == o.recolor_opa
    }
}

/// Per-node cache of [`MainStyle`]. Interior mutability through `Cell` only (no borrow can
/// fail): filled on first read after a style or state change, and considered stale when the
/// tree's style epoch moved (an inherited property changed somewhere).
#[derive(Debug, Default)]
pub(crate) struct StyleCache {
    values: Cell<Option<MainStyle>>,
    epoch: Cell<u32>,
}

impl StyleCache {
    /// The cached values if valid for `epoch`.
    #[inline]
    pub(crate) fn get(&self, epoch: u32) -> Option<MainStyle> {
        if self.epoch.get() == epoch {
            self.values.get()
        } else {
            None
        }
    }

    /// Stores freshly resolved values.
    #[inline]
    pub(crate) fn set(&self, v: MainStyle, epoch: u32) {
        self.values.set(Some(v));
        self.epoch.set(epoch);
    }

    /// Drops the cached values.
    #[inline]
    pub(crate) fn invalidate(&self) {
        self.values.set(None);
    }

    /// Whether values are cached for `epoch`.
    pub(crate) fn is_valid(&self, epoch: u32) -> bool {
        self.get(epoch).is_some()
    }
}
