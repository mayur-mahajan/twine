//! # twine-layout
//!
//! Sizing, positioning, flex and grid layout of the Twine GUI library, with the semantics of
//! LVGL 9 (`lv_obj_pos.c`, `lv_flex.c`, `lv_grid.c`). The crate is independent of the widget
//! tree: it works on any [`LayoutTree`], reads the layout style properties of
//! `twine-style` (`Width`, `MinWidth`, `X`, `Align`, `Pad*`, `Margin*`, `Flex*`, `Grid*`,
//! `BaseDir`, `Translate*`…) and writes absolute, half-open rectangles back.
//!
//! Integer math only; divisions that share space (flex grow, grid `fr` tracks) round each
//! share to the closest integer and carry the rest to the next one, so sizes always add up
//! exactly and results are deterministic.
//!
//! ## Model
//!
//! - **Size** ([`resolve_length`], [`clamp_size`], [`content_size_of`]): `Px`, `Pct` of the
//!   parent's content area (minus the node's margins), or `Content`; clamped by min/max
//!   (`min` wins over `max`).
//! - **Position** ([`align_offset`], [`resolve_align`]): children of a container without
//!   layout are placed by `Align` (21 values) plus `X`/`Y`, mirrored in right-to-left
//!   containers, then shifted by `TranslateX/Y`. Margins are ignored here.
//! - **Flex** and **grid** containers place their items (children without `HIDDEN`,
//!   `IGNORE_LAYOUT`, `FLOATING`); the other children are positioned as above.
//! - [`layout_subtree`] runs everything top-down for a subtree; [`LayoutScratch`] keeps its
//!   buffers so repeated layouts do not allocate.
//!
//! ## Example
//!
//! Implementing [`LayoutTree`] for a tiny tree and laying out a flex row:
//!
//! ```
//! use twine_core::{Rect, Size};
//! use twine_layout::{LayoutFlags, LayoutTree, layout_subtree};
//! use twine_style::{FlexFlow, LayoutKind, PropId, StyleBuf, StyleValue};
//!
//! struct Tree {
//!     styles: Vec<StyleBuf>,
//!     coords: Vec<Rect>,
//! }
//!
//! impl LayoutTree for Tree {
//!     type Id = usize;
//!     fn children(&self, id: usize, out: &mut dyn FnMut(usize)) {
//!         if id == 0 {
//!             (1..self.styles.len()).for_each(out);
//!         }
//!     }
//!     fn style_prop(&self, id: usize, prop: PropId) -> StyleValue {
//!         self.styles[id].get(prop).unwrap_or(prop.meta().default)
//!     }
//!     fn content_size(&self, _: usize) -> Size {
//!         Size::new(0, 0)
//!     }
//!     fn flags(&self, _: usize) -> LayoutFlags {
//!         LayoutFlags::empty()
//!     }
//!     fn set_coords(&mut self, id: usize, r: Rect) {
//!         self.coords[id] = r;
//!     }
//!     fn coords(&self, id: usize) -> Rect {
//!         self.coords[id]
//!     }
//! }
//!
//! let row = StyleBuf::new()
//!     .width(300)
//!     .height(100)
//!     .layout(LayoutKind::Flex)
//!     .flex_flow(FlexFlow::Row)
//!     .pad_column(10);
//! let mut t = Tree {
//!     styles: vec![row, StyleBuf::new().width(50).height(20), StyleBuf::new().flex_grow(1).height(20)],
//!     coords: vec![Rect::ZERO; 3],
//! };
//! layout_subtree(&mut t, 0, Rect::from_xywh(0, 0, 300, 100));
//! assert_eq!(t.coords[1], Rect::from_xywh(0, 0, 50, 20));
//! assert_eq!(t.coords[2], Rect::from_xywh(60, 0, 240, 20)); // grows into the free space
//! ```
//!
//! ## Features
//!
//! `toy` enables `ToyTree`, an in-memory tree for tests of dependent crates; `log`/`defmt`
//! select the logging backend (target `"twine::layout"`; warnings for invalid grid cells);
//! `std` enables std-only conveniences of the dependencies.
#![no_std]
#![deny(clippy::float_arithmetic)]

extern crate alloc;
#[cfg(test)]
extern crate std;

mod flex;
mod grid;
mod layout;
mod position;
mod size;
#[cfg(any(test, feature = "toy"))]
mod toy;
mod tree;

#[cfg(test)]
mod tests;

pub use layout::{LayoutScratch, layout_children, layout_children_with, layout_subtree, layout_subtree_with};
pub use position::{align_offset, is_outside, resolve_align};
pub use size::{clamp_size, content_size_of, resolve_length};
#[cfg(any(test, feature = "toy"))]
pub use toy::ToyTree;
pub use tree::{AlignTo, Axis, LayoutFlags, LayoutTree};
