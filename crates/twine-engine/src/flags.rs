//! Object flags ([`ObjFlags`], LVGL `lv_obj_flag_t`) and pending layout work ([`LayoutDirty`]).

use core::fmt;

bitflags::bitflags! {
    /// Behaviour flags of a node, with the names of LVGL 9's `LV_OBJ_FLAG_*`.
    ///
    /// ```
    /// use twine_engine::ObjFlags;
    /// let f = ObjFlags::CLICKABLE | ObjFlags::SCROLL_CHAIN;
    /// assert!(f.contains(ObjFlags::SCROLL_CHAIN_HOR));
    /// assert_eq!(twine_engine::flag_names(f).to_string(), "CLICKABLE|SCROLL_CHAIN_HOR|SCROLL_CHAIN_VER");
    /// ```
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub struct ObjFlags: u32 {
        /// Not drawn, not hit-tested, ignored by layouts.
        const HIDDEN = 1 << 0;
        /// Receives pointer presses.
        const CLICKABLE = 1 << 1;
        /// Gets the `FOCUSED` state when clicked.
        const CLICK_FOCUSABLE = 1 << 2;
        /// Toggles the `CHECKED` state when clicked.
        const CHECKABLE = 1 << 3;
        /// Can be scrolled.
        const SCROLLABLE = 1 << 4;
        /// Elastic overscroll.
        const SCROLL_ELASTIC = 1 << 5;
        /// Scrolling continues with momentum after release.
        const SCROLL_MOMENTUM = 1 << 6;
        /// Scrolls at most one snappable child per gesture.
        const SCROLL_ONE = 1 << 7;
        /// Propagates horizontal scrolling to the parent at the edge.
        const SCROLL_CHAIN_HOR = 1 << 8;
        /// Propagates vertical scrolling to the parent at the edge.
        const SCROLL_CHAIN_VER = 1 << 9;
        /// Scrolls to make itself visible when focused.
        const SCROLL_ON_FOCUS = 1 << 10;
        /// Scrolls with the arrow keys (keypad / encoder).
        const SCROLL_WITH_ARROW = 1 << 11;
        /// Can be a snap target of the parent's scroll snapping.
        const SNAPPABLE = 1 << 12;
        /// Keeps being pressed while the pointer slides off.
        const PRESS_LOCK = 1 << 13;
        /// Events also go to the parent.
        const EVENT_BUBBLE = 1 << 14;
        /// Gestures also go to the parent.
        const GESTURE_BUBBLE = 1 << 15;
        /// Precise hit test (rounded corners, custom `Widget::hit_test`).
        const ADV_HITTEST = 1 << 16;
        /// Positioned by hand, ignored by the parent's layout.
        const IGNORE_LAYOUT = 1 << 17;
        /// Ignored by layouts and by the parent's scroll bounds.
        const FLOATING = 1 << 18;
        /// Sends draw task events.
        const SEND_DRAW_TASK_EVENTS = 1 << 19;
        /// Children are not clipped to this node's area.
        const OVERFLOW_VISIBLE = 1 << 20;
        /// Starts a new flex track.
        const FLEX_IN_NEW_TRACK = 1 << 21;
        /// A transparent wrapper (not in LVGL; used by the declarative control-flow views):
        /// the layout lays out its children as if they were children of its parent (flex and
        /// grid positions, `%` sizes and content sizes are unchanged), its coordinates become
        /// the bounding box of its children, it draws nothing, does not clip its children and
        /// is transparent to hit-testing.
        const LAYOUT_PASSTHROUGH = 1 << 22;
        /// A safety-critical node (not in LVGL; display integrity, see
        /// `docs/safety/profile.md` § 11): a value whose wrong display would be a hazard. The
        /// engine draws it on its own terms —
        ///
        /// - **whole on every change:** a partial invalidation of the node
        ///   ([`Engine::invalidate_node_area`](crate::Engine::invalidate_node_area), widget
        ///   setters redrawing a part) redraws all of it;
        /// - **no caches:** its drawing runs in fresh mode (circle, gradient, shadow and glyph
        ///   caches recomputed from their sources), so a corrupted cache cannot change it;
        /// - **no transitions:** state changes apply its new style at once (no style
        ///   transition, no theme motion);
        /// - **no opacity or layer from ancestors:** it is drawn opaque to its ancestors'
        ///   opacity and outside their layers (an ancestor's opacity group, blend mode,
        ///   transform or bitmap mask does not apply to it; it is drawn right after that
        ///   ancestor's layer, at its own coordinates); its own styles still apply;
        /// - **reported:** [`Engine::critical_nodes`](crate::Engine::critical_nodes) lists the
        ///   critical nodes and [`Engine::critical_report`](crate::Engine::critical_report)
        ///   gives each one's drawn rectangle, its value digest and the frame it was last
        ///   drawn in, for cross-checks ([`Engine::check_region`](crate::Engine::check_region)).
        ///
        /// Costs nothing for other nodes beyond one flag test where the draw walk reads the
        /// flags anyway; the engine keeps the critical nodes in a list (registration allocates
        /// once per node, at start-up).
        #[doc(alias = "certified")]
        #[cfg(feature = "display-integrity")]
        const SAFETY_CRITICAL = 1 << 23;
        /// Reserved for widgets.
        const WIDGET_1 = 1 << 25;
        /// Reserved for widgets.
        const WIDGET_2 = 1 << 26;
        /// Free for the application.
        const USER_1 = 1 << 27;
        /// Free for the application.
        const USER_2 = 1 << 28;
        /// Free for the application.
        const USER_3 = 1 << 29;
        /// Free for the application.
        const USER_4 = 1 << 30;
        /// Docked to the bottom of the parent (not in LVGL; an on-screen keyboard reserving its
        /// height, see `Keyboard::set_docked` in `twine-widgets`): the node gets the full
        /// width of the parent's content area and its own height, at the bottom of that area,
        /// and the parent's other children are laid out in the area above it (`Pct` sizes,
        /// `Align`, flex and grid; see `twine_layout::LayoutFlags::DOCK_BOTTOM`). Like
        /// `FLOATING`, it is not a flex/grid item, does not scroll with the parent and does not
        /// count for the parent's content size or scroll extents. A `HIDDEN` node reserves
        /// nothing.
        const DOCK_BOTTOM = 1 << 31;
        /// Both scroll chain directions.
        const SCROLL_CHAIN = Self::SCROLL_CHAIN_HOR.bits() | Self::SCROLL_CHAIN_VER.bits();
    }
}

impl ObjFlags {
    /// Children that neither scroll with their parent nor count for its scroll extents or
    /// snapping (`FLOATING`, `DOCK_BOTTOM`).
    pub(crate) const OUT_OF_FLOW: ObjFlags = ObjFlags::FLOATING.union(ObjFlags::DOCK_BOTTOM);
}

#[cfg(feature = "defmt")]
impl defmt::Format for ObjFlags {
    fn format(&self, f: defmt::Formatter<'_>) {
        defmt::write!(f, "ObjFlags({=u32:#x})", self.bits());
    }
}

bitflags::bitflags! {
    /// Layout work pending on a node (consumed by the layout pass).
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub struct LayoutDirty: u8 {
        /// The node's own size/position must be recomputed.
        const SELF = 1 << 0;
        /// The node's children must be laid out again.
        const CHILDREN = 1 << 1;
        /// A descendant has pending layout work (guides the layout pass to it).
        const DESCENDANTS = 1 << 2;
    }
}

#[cfg(feature = "defmt")]
impl defmt::Format for LayoutDirty {
    fn format(&self, f: defmt::Formatter<'_>) {
        defmt::write!(f, "LayoutDirty({=u8:#x})", self.bits());
    }
}

/// Formats the set flags of a bitflags value as `A|B|C` (`-` when empty), e.g. for
/// [`ObjFlags`] in tree dumps ([`State`](crate::State) has its own `Display`, which names
/// application states).
#[must_use]
pub fn flag_names<F: bitflags::Flags>(flags: F) -> impl fmt::Display {
    struct N<F>(F);
    impl<F: bitflags::Flags> fmt::Display for N<F> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            let mut first = true;
            for (name, _) in self.0.iter_names() {
                if !first {
                    f.write_str("|")?;
                }
                first = false;
                f.write_str(name)?;
            }
            if first {
                f.write_str("-")?;
            }
            Ok(())
        }
    }
    N(flags)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;

    #[test]
    fn scroll_chain_is_both_directions() {
        assert_eq!(
            ObjFlags::SCROLL_CHAIN,
            ObjFlags::SCROLL_CHAIN_HOR | ObjFlags::SCROLL_CHAIN_VER
        );
        assert_eq!(format!("{}", flag_names(ObjFlags::empty())), "-");
        assert_eq!(
            format!("{}", flag_names(ObjFlags::HIDDEN | ObjFlags::USER_4)),
            "HIDDEN|USER_4"
        );
    }

    #[test]
    fn all_flags_distinct() {
        let mut seen = 0u32;
        for (name, f) in ObjFlags::all().iter_names() {
            if name == "SCROLL_CHAIN" {
                continue;
            }
            assert_eq!(seen & f.bits(), 0, "{name} overlaps");
            seen |= f.bits();
        }
        // Bit 24 is free (LVGL's `LAYOUT_2`; `LAYOUT_1`'s bit is `SAFETY_CRITICAL`, which
        // exists with the `display-integrity` feature).
        assert_eq!(
            seen.count_ones(),
            if cfg!(feature = "display-integrity") {
                31
            } else {
                30
            }
        );
        assert_eq!(seen & (1 << 24), 0);
    }
}
