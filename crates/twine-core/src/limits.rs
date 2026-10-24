//! [`Limits`]: the memory bounds of a Twine application, in one value.
//!
//! Every structure of Twine that grows with the application — the widget tree, the reactive
//! graph, event handlers, timers, animations, queues, the navigation stack, keyed lists, the
//! nodes' theme classes, the waker pool —
//! is bounded by a field of [`Limits`]. Reaching a bound is a
//! [`FaultKind::Capacity`](crate::fault::FaultKind::Capacity) fault whose code
//! ([`CapacityFault`](crate::fault::CapacityFault)) names the field, and the operation is
//! refused, cut or dropped: **Twine never grows past a limit**. The checks sit at the insertion
//! points, which already branch on "is there room", so they cost nothing on hot paths.
//!
//! With [`preallocate`](Limits::preallocate) the storage of the bounded structures is reserved
//! up front, when the `Ui` (or the engine and the runtime) is configured, so that staying
//! within the limits never allocates; together with sealing
//! ([`alloc_guard`](crate::alloc_guard), `Ui::seal`) this is the evidence that the application
//! runs in a fixed amount of memory after start-up.
//!
//! The view layer's `AppConfig::limits` holds the application's value; the builders apply it to
//! the engine (`Engine::set_limits`), the reactive runtime (`Runtime::set_limits`) and the
//! waker pool. [`Limits::summary`] (and the complete `AppConfig::audit_report`) dump it for
//! audit evidence.
//!
//! ```
//! use twine_core::Limits;
//!
//! // A product sized at design time: every bound explicit, storage reserved at start-up.
//! const LIMITS: Limits = Limits::DEFAULT
//!     .max_nodes(400)
//!     .max_reactive_nodes(600)
//!     .max_scopes(200)
//!     .max_pending_effects(64)
//!     .max_text_len(64)
//!     .heap_budget(Some(48 * 1024))
//!     .preallocated();
//! assert!(LIMITS.validate().is_ok());
//! assert!(LIMITS.summary().to_string().contains("limits.max_nodes=400"));
//! ```

use crate::Error;
use crate::audit::{Audit, AuditSink, AuditValue, KeyValues};

/// The memory bounds of an application (see the [module docs](self)).
///
/// The fields are public so an audit can read them; the `const` setters build a value in a
/// `const` (`Limits::DEFAULT.max_nodes(400)…`). Later releases add fields (new bounded
/// structures); write values as `Limits::DEFAULT.<setters>` or `Limits { .., ..Limits::DEFAULT }`
/// so they keep compiling.
///
/// [`DEFAULT`](Self::DEFAULT) keeps the structural maxima of each structure and grows storage
/// on demand (no RAM is spent on headroom): what an application gets without configuring
/// anything, and what the `safety` profile refuses ([`validate`](Self::validate)).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Limits {
    /// Live engine nodes: widgets, screens and every display's four layers (`1..=65 535`,
    /// default 65 535). Enforced by `Engine::create` and friends
    /// ([`CapacityFault::Nodes`](crate::fault::CapacityFault::Nodes)). Preallocated: the
    /// node arena and the delete scratch.
    pub max_nodes: u16,
    /// Levels below a root (a screen or layer is level 0, its child level 1, …;
    /// `1..=255`, default 255). Bounds the recursion of drawing and layout. Enforced when a
    /// node is created or moved ([`CapacityFault::TreeDepth`](crate::fault::CapacityFault::TreeDepth)).
    pub max_tree_depth: u8,
    /// Bytes of one widget text (label, checkbox, textarea; `1..=65 535`, default 65 535). A
    /// longer text is cut at a character boundary
    /// ([`CapacityFault::TextLen`](crate::fault::CapacityFault::TextLen)). Preallocated: a
    /// label's buffers are reserved to this size when it first gets an owned text.
    pub max_text_len: u16,
    /// Event handlers registered on engine nodes, all nodes together (default 65 535; `0`
    /// allows none). Enforced by `Engine::add_event_handler`
    /// ([`CapacityFault::Handlers`](crate::fault::CapacityFault::Handlers)).
    pub max_handlers: u16,
    /// Engine timers alive at once (`1..=65 534`, default 65 534). Enforced by
    /// `Engine::timer_add` ([`CapacityFault::Timers`](crate::fault::CapacityFault::Timers)).
    /// Preallocated: the timer table.
    pub max_timers: u16,
    /// Engine animations running at once (`1..=65 534`, default 65 534). Enforced by
    /// `Engine::anim_start` ([`CapacityFault::Animations`](crate::fault::CapacityFault::Animations)).
    /// Preallocated: the animation table.
    pub max_anims: u16,
    /// Live reactive nodes: signals, memos, stored values and effects (`1..=65 535`, default
    /// 65 535). Enforced by the reactive runtime
    /// ([`CapacityFault::ReactiveNodes`](crate::fault::CapacityFault::ReactiveNodes)).
    /// Preallocated: the node arena and the propagation stack.
    pub max_reactive_nodes: u16,
    /// Live reactive scopes (`1..=65 535`, default 65 535)
    /// ([`CapacityFault::ReactiveScopes`](crate::fault::CapacityFault::ReactiveScopes)).
    /// Preallocated: the scope arena.
    pub max_scopes: u16,
    /// Effects waiting to run at once (`1..=65 535`, default 65 535): queued after a write,
    /// or deferred until a UI update provides their context
    /// ([`CapacityFault::PendingEffects`](crate::fault::CapacityFault::PendingEffects)).
    /// Preallocated: both effect queues.
    pub max_pending_effects: u16,
    /// Messages each channel delivers per `Ui` update (`1..=65 535`, default 16): bounds the
    /// work of one update; the rest waits for the next.
    pub messages_per_channel: u16,
    /// Engine commands a `Ui` can queue while it does not hold the engine (default 16; `0`
    /// drops every such command). The ring is allocated once, when the `Ui` is built
    /// ([`CapacityFault::EngineQueue`](crate::fault::CapacityFault::EngineQueue)).
    pub engine_queue: u16,
    /// Routes on a navigator's stack (`1..=255`, default 255)
    /// ([`CapacityFault::NavDepth`](crate::fault::CapacityFault::NavDepth)). Preallocated:
    /// the route and page stacks.
    pub max_nav_depth: u8,
    /// Rows of one keyed collection (`1..=65 535`, default 65 535): the rows of a
    /// `for_each`, the visible rows of a `virtual_list`. A collection with more items shows
    /// the first ones ([`CapacityFault::ListItems`](crate::fault::CapacityFault::ListItems));
    /// a `for_each` can lower its own bound (`ForEach::max_items`). Preallocated: each
    /// collection's row list and reconcile scratch, reserved when it is built (so a keyed
    /// change — insert, remove, move — allocates nothing beyond the rows it creates). Size it
    /// for the longest list: the reservation is per collection.
    pub max_list_items: u16,
    /// Nodes with a theme class of their own — a variant (`ButtonVariant::Tinted`) or a role
    /// (`.role(&METRIC_TILE)`) set with `Engine::set_theme_class` — all nodes together
    /// (default 65 535; `0` allows none). Enforced by `Engine::set_theme_class`
    /// ([`CapacityFault::ThemeClasses`](crate::fault::CapacityFault::ThemeClasses)): the node
    /// keeps its widget class's look. Preallocated: the engine's theme-class side table.
    pub max_theme_classes: u16,
    /// Wakers the process-wide waker pool may allocate on the heap once its 4 static slots are
    /// leased (default 60, i.e. 64 `Ui`s alive at once; `0` forbids heap wakers). A heap waker
    /// is allocated once and never freed (it is reused), so this bounds that memory
    /// ([`CapacityFault::WakerPool`](crate::fault::CapacityFault::WakerPool)). Process-wide: the
    /// last `Ui` built sets it.
    pub max_heap_wakers: u16,
    /// Heap bytes the `Ui`'s memory report may show (`MemoryReport::heap_bytes`: engine,
    /// runtime, queues, waker pool; not the application's own allocations). `None` (default):
    /// not checked. Checked when the `Ui` is sealed
    /// ([`CapacityFault::HeapBudget`](crate::fault::CapacityFault::HeapBudget)); `Some(0)`
    /// is invalid.
    pub heap_budget: Option<usize>,
    /// Reserve the storage of the bounded structures up front (default `false`: grow on
    /// demand, up to the limits). Required by the `safety` profile. Size the limits for the
    /// product: the reservation is made in full (e.g. `max_nodes` × the node slot size).
    pub preallocate: bool,
}

impl Default for Limits {
    /// [`Limits::DEFAULT`].
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl Limits {
    /// The structural maxima, growing on demand (see each field). Not valid in the `safety`
    /// profile, which needs [`preallocated`](Self::preallocated) limits.
    ///
    /// ```
    /// use twine_core::Limits;
    /// assert_eq!(Limits::DEFAULT.max_nodes, 65_535);
    /// assert_eq!(Limits::DEFAULT.messages_per_channel, 16);
    /// assert!(!Limits::DEFAULT.preallocate);
    /// assert_eq!(Limits::default(), Limits::DEFAULT);
    /// ```
    pub const DEFAULT: Limits = Limits {
        max_nodes: u16::MAX,
        max_tree_depth: u8::MAX,
        max_text_len: u16::MAX,
        max_handlers: u16::MAX,
        max_timers: u16::MAX - 1,
        max_anims: u16::MAX - 1,
        max_reactive_nodes: u16::MAX,
        max_scopes: u16::MAX,
        max_pending_effects: u16::MAX,
        messages_per_channel: 16,
        engine_queue: 16,
        max_nav_depth: u8::MAX,
        max_list_items: u16::MAX,
        max_theme_classes: u16::MAX,
        max_heap_wakers: 60,
        heap_budget: None,
        preallocate: false,
    };

    /// Sets [`max_nodes`](Self::max_nodes).
    ///
    /// ```
    /// use twine_core::Limits;
    /// assert_eq!(Limits::DEFAULT.max_nodes(300).max_nodes, 300);
    /// ```
    #[must_use]
    pub const fn max_nodes(mut self, n: u16) -> Self {
        self.max_nodes = n;
        self
    }

    /// Sets [`max_tree_depth`](Self::max_tree_depth).
    ///
    /// ```
    /// use twine_core::Limits;
    /// assert_eq!(Limits::DEFAULT.max_tree_depth(12).max_tree_depth, 12);
    /// ```
    #[must_use]
    pub const fn max_tree_depth(mut self, n: u8) -> Self {
        self.max_tree_depth = n;
        self
    }

    /// Sets [`max_text_len`](Self::max_text_len).
    ///
    /// ```
    /// use twine_core::Limits;
    /// assert_eq!(Limits::DEFAULT.max_text_len(64).max_text_len, 64);
    /// ```
    #[must_use]
    pub const fn max_text_len(mut self, n: u16) -> Self {
        self.max_text_len = n;
        self
    }

    /// Sets [`max_handlers`](Self::max_handlers).
    ///
    /// ```
    /// use twine_core::Limits;
    /// assert_eq!(Limits::DEFAULT.max_handlers(100).max_handlers, 100);
    /// ```
    #[must_use]
    pub const fn max_handlers(mut self, n: u16) -> Self {
        self.max_handlers = n;
        self
    }

    /// Sets [`max_timers`](Self::max_timers).
    ///
    /// ```
    /// use twine_core::Limits;
    /// assert_eq!(Limits::DEFAULT.max_timers(8).max_timers, 8);
    /// ```
    #[must_use]
    pub const fn max_timers(mut self, n: u16) -> Self {
        self.max_timers = n;
        self
    }

    /// Sets [`max_anims`](Self::max_anims).
    ///
    /// ```
    /// use twine_core::Limits;
    /// assert_eq!(Limits::DEFAULT.max_anims(16).max_anims, 16);
    /// ```
    #[must_use]
    pub const fn max_anims(mut self, n: u16) -> Self {
        self.max_anims = n;
        self
    }

    /// Sets [`max_reactive_nodes`](Self::max_reactive_nodes).
    ///
    /// ```
    /// use twine_core::Limits;
    /// assert_eq!(Limits::DEFAULT.max_reactive_nodes(500).max_reactive_nodes, 500);
    /// ```
    #[must_use]
    pub const fn max_reactive_nodes(mut self, n: u16) -> Self {
        self.max_reactive_nodes = n;
        self
    }

    /// Sets [`max_scopes`](Self::max_scopes).
    ///
    /// ```
    /// use twine_core::Limits;
    /// assert_eq!(Limits::DEFAULT.max_scopes(100).max_scopes, 100);
    /// ```
    #[must_use]
    pub const fn max_scopes(mut self, n: u16) -> Self {
        self.max_scopes = n;
        self
    }

    /// Sets [`max_pending_effects`](Self::max_pending_effects).
    ///
    /// ```
    /// use twine_core::Limits;
    /// assert_eq!(Limits::DEFAULT.max_pending_effects(32).max_pending_effects, 32);
    /// ```
    #[must_use]
    pub const fn max_pending_effects(mut self, n: u16) -> Self {
        self.max_pending_effects = n;
        self
    }

    /// Sets [`messages_per_channel`](Self::messages_per_channel).
    ///
    /// ```
    /// use twine_core::Limits;
    /// assert_eq!(Limits::DEFAULT.messages_per_channel(4).messages_per_channel, 4);
    /// ```
    #[must_use]
    pub const fn messages_per_channel(mut self, n: u16) -> Self {
        self.messages_per_channel = n;
        self
    }

    /// Sets [`engine_queue`](Self::engine_queue).
    ///
    /// ```
    /// use twine_core::Limits;
    /// assert_eq!(Limits::DEFAULT.engine_queue(64).engine_queue, 64);
    /// ```
    #[must_use]
    pub const fn engine_queue(mut self, n: u16) -> Self {
        self.engine_queue = n;
        self
    }

    /// Sets [`max_nav_depth`](Self::max_nav_depth).
    ///
    /// ```
    /// use twine_core::Limits;
    /// assert_eq!(Limits::DEFAULT.max_nav_depth(8).max_nav_depth, 8);
    /// ```
    #[must_use]
    pub const fn max_nav_depth(mut self, n: u8) -> Self {
        self.max_nav_depth = n;
        self
    }

    /// Sets [`max_list_items`](Self::max_list_items).
    ///
    /// ```
    /// use twine_core::Limits;
    /// assert_eq!(Limits::DEFAULT.max_list_items(32).max_list_items, 32);
    /// ```
    #[must_use]
    pub const fn max_list_items(mut self, n: u16) -> Self {
        self.max_list_items = n;
        self
    }

    /// Sets [`max_theme_classes`](Self::max_theme_classes).
    ///
    /// ```
    /// use twine_core::Limits;
    /// assert_eq!(Limits::DEFAULT.max_theme_classes(8).max_theme_classes, 8);
    /// ```
    #[must_use]
    pub const fn max_theme_classes(mut self, n: u16) -> Self {
        self.max_theme_classes = n;
        self
    }

    /// Sets [`max_heap_wakers`](Self::max_heap_wakers).
    ///
    /// ```
    /// use twine_core::Limits;
    /// assert_eq!(Limits::DEFAULT.max_heap_wakers(0).max_heap_wakers, 0);
    /// ```
    #[must_use]
    pub const fn max_heap_wakers(mut self, n: u16) -> Self {
        self.max_heap_wakers = n;
        self
    }

    /// Sets [`heap_budget`](Self::heap_budget).
    ///
    /// ```
    /// use twine_core::Limits;
    /// assert_eq!(Limits::DEFAULT.heap_budget(Some(4096)).heap_budget, Some(4096));
    /// ```
    #[must_use]
    pub const fn heap_budget(mut self, bytes: Option<usize>) -> Self {
        self.heap_budget = bytes;
        self
    }

    /// Sets [`preallocate`](Self::preallocate) to `true`.
    ///
    /// ```
    /// use twine_core::Limits;
    /// assert!(Limits::DEFAULT.max_nodes(100).preallocated().preallocate);
    /// ```
    #[must_use]
    pub const fn preallocated(mut self) -> Self {
        self.preallocate = true;
        self
    }

    /// Checks the ranges documented on each field, and — in the `safety` profile — that the
    /// limits are [`preallocated`](Self::preallocated). The engine, the runtime and the `Ui`
    /// builders call it before applying a value.
    ///
    /// # Errors
    /// [`Error::InvalidArgument`] naming the first value out of range. Never panics.
    ///
    /// ```
    /// use twine_core::{Error, Limits};
    ///
    /// assert!(Limits::DEFAULT.max_nodes(10).preallocated().validate().is_ok());
    /// assert_eq!(
    ///     Limits::DEFAULT.max_nodes(0).validate(),
    ///     Err(Error::InvalidArgument("Limits::max_nodes must be >= 1"))
    /// );
    /// ```
    pub const fn validate(&self) -> Result<(), Error> {
        let msg = if self.max_nodes == 0 {
            "Limits::max_nodes must be >= 1"
        } else if self.max_tree_depth == 0 {
            "Limits::max_tree_depth must be >= 1"
        } else if self.max_text_len == 0 {
            "Limits::max_text_len must be >= 1"
        } else if self.max_timers == 0 || self.max_timers == u16::MAX {
            "Limits::max_timers must be in 1..=65534"
        } else if self.max_anims == 0 || self.max_anims == u16::MAX {
            "Limits::max_anims must be in 1..=65534"
        } else if self.max_reactive_nodes == 0 {
            "Limits::max_reactive_nodes must be >= 1"
        } else if self.max_scopes == 0 {
            "Limits::max_scopes must be >= 1"
        } else if self.max_pending_effects == 0 {
            "Limits::max_pending_effects must be >= 1"
        } else if self.messages_per_channel == 0 {
            "Limits::messages_per_channel must be >= 1"
        } else if self.max_nav_depth == 0 {
            "Limits::max_nav_depth must be >= 1"
        } else if self.max_list_items == 0 {
            "Limits::max_list_items must be >= 1"
        } else if matches!(self.heap_budget, Some(0)) {
            "Limits::heap_budget must be > 0 when set"
        } else if crate::SAFETY_PROFILE && !self.preallocate {
            "the safety profile needs preallocated Limits (Limits::preallocated)"
        } else {
            return Ok(());
        };
        Err(Error::InvalidArgument(msg))
    }

    /// The limits as `limits.<field>=<value>` lines (see [`audit`](crate::audit) for the
    /// format): `Display`, and `defmt::Format` with the `defmt` feature. For the complete
    /// configuration of an application use the view layer's `AppConfig::audit_report`.
    /// Formatting code is linked only when this is used.
    ///
    /// ```
    /// use twine_core::Limits;
    ///
    /// let s = Limits::DEFAULT.max_nodes(300).summary().to_string();
    /// assert!(s.starts_with("limits.max_nodes=300\n"));
    /// assert!(s.contains("limits.heap_budget_bytes=none\n"));
    /// assert!(s.contains("limits.preallocate=false\n"));
    /// ```
    #[must_use]
    pub fn summary(&self) -> KeyValues<'_, Limits> {
        KeyValues(self)
    }
}

impl Audit for Limits {
    /// Every field as `limits.<field>` (`heap_budget` as `heap_budget_bytes`), in declaration
    /// order.
    fn audit(&self, out: &mut dyn AuditSink) {
        let n = |v: u16| AuditValue::Uint(u64::from(v));
        out.entry("limits", "max_nodes", n(self.max_nodes));
        out.entry(
            "limits",
            "max_tree_depth",
            AuditValue::Uint(u64::from(self.max_tree_depth)),
        );
        out.entry("limits", "max_text_len", n(self.max_text_len));
        out.entry("limits", "max_handlers", n(self.max_handlers));
        out.entry("limits", "max_timers", n(self.max_timers));
        out.entry("limits", "max_anims", n(self.max_anims));
        out.entry("limits", "max_reactive_nodes", n(self.max_reactive_nodes));
        out.entry("limits", "max_scopes", n(self.max_scopes));
        out.entry("limits", "max_pending_effects", n(self.max_pending_effects));
        out.entry("limits", "messages_per_channel", n(self.messages_per_channel));
        out.entry("limits", "engine_queue", n(self.engine_queue));
        out.entry(
            "limits",
            "max_nav_depth",
            AuditValue::Uint(u64::from(self.max_nav_depth)),
        );
        out.entry("limits", "max_list_items", n(self.max_list_items));
        out.entry("limits", "max_theme_classes", n(self.max_theme_classes));
        out.entry("limits", "max_heap_wakers", n(self.max_heap_wakers));
        out.entry(
            "limits",
            "heap_budget_bytes",
            AuditValue::opt_usize(self.heap_budget),
        );
        out.entry("limits", "preallocate", AuditValue::Bool(self.preallocate));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    #[test]
    fn default_is_valid_outside_the_safety_profile() {
        assert_eq!(Limits::DEFAULT.validate().is_ok(), !crate::SAFETY_PROFILE);
        assert!(Limits::DEFAULT.preallocated().validate().is_ok());
    }

    #[test]
    fn each_range_is_checked() {
        let bad = [
            Limits::DEFAULT.max_nodes(0),
            Limits::DEFAULT.max_tree_depth(0),
            Limits::DEFAULT.max_text_len(0),
            Limits::DEFAULT.max_timers(0),
            Limits::DEFAULT.max_timers(u16::MAX),
            Limits::DEFAULT.max_anims(0),
            Limits::DEFAULT.max_reactive_nodes(0),
            Limits::DEFAULT.max_scopes(0),
            Limits::DEFAULT.max_pending_effects(0),
            Limits::DEFAULT.messages_per_channel(0),
            Limits::DEFAULT.max_nav_depth(0),
            Limits::DEFAULT.max_list_items(0),
            Limits::DEFAULT.heap_budget(Some(0)),
        ];
        for l in bad {
            assert!(l.preallocated().validate().is_err(), "{l:?}");
        }
        assert!(
            Limits::DEFAULT
                .max_handlers(0)
                .max_theme_classes(0)
                .engine_queue(0)
                .max_heap_wakers(0)
                .preallocated()
                .validate()
                .is_ok()
        );
    }

    #[test]
    fn summary_lists_every_field_once() {
        let s = Limits::DEFAULT.summary().to_string();
        assert_eq!(s.lines().count(), 17);
        assert!(s.lines().all(|l| l.starts_with("limits.") && l.contains('=')));
        assert!(s.contains("limits.max_heap_wakers=60\n"));
    }
}
