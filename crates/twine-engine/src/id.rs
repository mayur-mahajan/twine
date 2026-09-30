//! Identifiers: [`NodeId`] and [`DisplayId`].

use core::fmt;

use crate::tree::Node;

/// Handle of a node in the widget tree: a generational [`Id`](twine_core::Id) (`u16` slot index
/// + `u16` generation), so a handle to a deleted node never resolves again, even after its slot
///   is reused. At most 65 535 nodes can be alive at once.
///
/// Tree dumps print it as `n<index>g<generation>` (see [`fmt_node_id`]).
pub type NodeId = twine_core::Id<Node>;

/// A [`NodeId`] that is never valid in any tree: the node id of a widget that could not be
/// created (the view layer returns it from `BuildCx::create` instead of aliasing another node).
///
/// It is [`Id::INVALID`](twine_core::Id::INVALID): the tree's arena never hands it out (its
/// slot index is out of range and its generation is 0), so every engine call given it behaves
/// as for a deleted node — lookups return `None`, mutations are ignored (logged) — and no
/// other node is ever touched by mistake. Checking for it is one integer comparison.
///
/// ```
/// use twine_engine::{DEAD_NODE, Engine, EngineConfig, Obj};
/// let mut e = Engine::new(EngineConfig::default()).unwrap();
/// let n = e.create_root(Box::new(Obj)).unwrap();
/// assert_ne!(n, DEAD_NODE);
/// assert!(!e.tree().contains(DEAD_NODE));
/// assert!(e.create(DEAD_NODE, Box::new(Obj)).is_err());
/// ```
pub const DEAD_NODE: NodeId = NodeId::INVALID;

/// Formats a [`NodeId`] as `n<index>g<generation>` (e.g. `n12g3`), the notation of tree dumps
/// and engine logs.
///
/// ```
/// use twine_engine::{NodeId, fmt_node_id};
/// assert_eq!(fmt_node_id(NodeId::from_raw((3 << 16) | 12)).to_string(), "n12g3");
/// ```
#[must_use]
pub fn fmt_node_id(id: NodeId) -> NodeIdFmt {
    NodeIdFmt(id)
}

/// Formats a [`NodeId`] as `n<index>g<generation>` (see [`fmt_node_id`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NodeIdFmt(pub NodeId);

impl fmt::Display for NodeIdFmt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "n{}g{}", self.0.index(), self.0.generation())
    }
}

#[cfg(feature = "defmt")]
impl defmt::Format for NodeIdFmt {
    fn format(&self, f: defmt::Formatter<'_>) {
        defmt::write!(f, "n{=u16}g{=u16}", self.0.index(), self.0.generation());
    }
}

/// Handle of a display registered with [`Engine::add_display`](crate::Engine::add_display) or
/// [`Engine::add_framebuffer_display`](crate::Engine::add_framebuffer_display). Printed as
/// `d0`, `d1`, ….
///
/// ```
/// use twine_engine::{Engine, EngineConfig};
/// let e = Engine::new(EngineConfig::default()).unwrap();
/// assert!(e.default_display().is_none()); // displays are added with `add_display`
/// ```
#[derive(Copy, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct DisplayId(pub(crate) u8);

impl DisplayId {
    /// The display's index in registration order (0 = first display).
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Display for DisplayId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "d{}", self.0)
    }
}

impl fmt::Debug for DisplayId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "d{}", self.0)
    }
}

#[cfg(feature = "defmt")]
impl defmt::Format for DisplayId {
    fn format(&self, f: defmt::Formatter<'_>) {
        defmt::write!(f, "d{=u8}", self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;

    #[test]
    fn display_id_formats() {
        assert_eq!(format!("{}", DisplayId(2)), "d2");
        assert_eq!(format!("{:?}", DisplayId(0)), "d0");
        assert_eq!(DisplayId(3).index(), 3);
    }

    #[test]
    fn node_id_notation() {
        let id = NodeId::from_raw((7 << 16) | 0x2A);
        assert_eq!(format!("{}", fmt_node_id(id)), "n42g7");
    }
}
