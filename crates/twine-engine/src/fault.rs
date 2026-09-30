//! The engine's fault stream: [`FaultRecord`], [`FaultHook`] and the fault methods of
//! [`Engine`].
//!
//! Every fault the engine recovers from (see [`FaultKind`]) — and every fault the view layer
//! forwards from the reactive runtime — goes through [`Engine::raise_fault`], which
//!
//! 1. stamps the record with the engine's current time,
//! 2. adds it to the pending set returned by [`Engine::take_faults`],
//! 3. counts it ([`Engine::fault_counts`], saturating, never reset),
//! 4. keeps it as the last record of its kind ([`Engine::last_fault`]),
//! 5. calls the [`FaultHook`], if one is set.
//!
//! Nothing allocates: the state is a fixed-size table inside the engine. Twine assigns no
//! severity and takes no action beyond its documented recovery; what a fault means for the
//! product (ignore, retry, fail-safe screen, reset, report) is the application's policy, applied
//! in the hook or when polling [`Engine::take_faults`].

use twine_core::Instant;
use twine_core::fault::{FaultCounts, FaultKind, Faults};

use crate::{DisplayId, Engine, InputId, NodeId};

/// One fault: what happened, where, when and how often.
///
/// Created with [`FaultRecord::new`] and the builder methods; [`Engine::raise_fault`] sets
/// [`at`](Self::at).
///
/// ```
/// use twine_engine::FaultRecord;
/// use twine_core::fault::FaultKind;
///
/// let r = FaultRecord::new(FaultKind::ChannelOverflow).occurrences(3).code(7);
/// assert_eq!(r.kind, FaultKind::ChannelOverflow);
/// assert_eq!((r.occurrences, r.code), (3, 7));
/// assert!(r.display.is_none() && r.node.is_none() && r.input.is_none());
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct FaultRecord {
    /// What happened.
    pub kind: FaultKind,
    /// The display concerned, if any.
    pub display: Option<DisplayId>,
    /// The node concerned, if any (e.g. the parent a node could not be created under).
    pub node: Option<NodeId>,
    /// The input device concerned, if any ([`FaultKind::InputDevice`]).
    pub input: Option<InputId>,
    /// When the engine raised it (the engine's current time: the running update's time, else
    /// the last known one).
    pub at: Instant,
    /// How many occurrences this record stands for (≥ 1; e.g. the number of dropped channel
    /// messages, or faults merged by the view layer since the last update).
    pub occurrences: u32,
    /// A detail code whose meaning depends on the source (e.g. a driver error code), `0` when
    /// there is none.
    pub code: u32,
}

impl FaultRecord {
    /// A record of one occurrence of `kind`, with no display, node or code.
    #[must_use]
    pub const fn new(kind: FaultKind) -> FaultRecord {
        FaultRecord {
            kind,
            display: None,
            node: None,
            input: None,
            at: Instant::from_micros(0),
            occurrences: 1,
            code: 0,
        }
    }

    /// Sets the display concerned.
    #[must_use]
    pub const fn display(mut self, d: DisplayId) -> FaultRecord {
        self.display = Some(d);
        self
    }

    /// Sets the node concerned.
    #[must_use]
    pub const fn node(mut self, n: NodeId) -> FaultRecord {
        self.node = Some(n);
        self
    }

    /// Sets the input device concerned.
    #[must_use]
    pub const fn input(mut self, i: InputId) -> FaultRecord {
        self.input = Some(i);
        self
    }

    /// Sets the number of occurrences (at least 1).
    #[must_use]
    pub const fn occurrences(mut self, n: u32) -> FaultRecord {
        self.occurrences = if n == 0 { 1 } else { n };
        self
    }

    /// Sets the detail code.
    #[must_use]
    pub const fn code(mut self, code: u32) -> FaultRecord {
        self.code = code;
        self
    }
}

#[cfg(feature = "defmt")]
impl defmt::Format for FaultRecord {
    fn format(&self, f: defmt::Formatter<'_>) {
        defmt::write!(
            f,
            "{} display={} node={} input={} at={} x{=u32} code={=u32}",
            self.kind,
            self.display,
            self.node.map(crate::fmt_node_id),
            self.input,
            self.at,
            self.occurrences,
            self.code
        );
    }
}

/// Called for every raised fault. A plain function (it cannot capture state, and setting it
/// allocates nothing): record the fault, bump a counter, or signal a supervisor through a
/// `static`. It runs where the fault is detected — possibly in the middle of rendering or
/// flushing — so it must be short and must not panic.
pub type FaultHook = fn(&FaultRecord);

/// The engine's fault state (fixed size, allocated with the engine).
#[derive(Debug, Default)]
pub(crate) struct FaultState {
    /// Kinds raised since the last `take_faults`.
    pending: Faults,
    /// Occurrences since start-up.
    counts: FaultCounts,
    /// The last record of each kind.
    last: [Option<FaultRecord>; FaultKind::COUNT],
    hook: Option<FaultHook>,
}

impl Engine {
    /// Raises a fault: stamps it with the current time, records it (pending set, counters, last
    /// record of its kind) and calls the [`FaultHook`].
    ///
    /// The engine raises its own faults; the view layer raises the ones it detects or forwards
    /// (build failures, reactive runtime faults). Widget and driver integrations may raise
    /// faults of the matching kinds too.
    ///
    /// ```
    /// use twine_engine::{Engine, EngineConfig, FaultRecord};
    /// use twine_core::fault::FaultKind;
    ///
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// e.raise_fault(FaultRecord::new(FaultKind::InputDevice).code(2));
    /// assert!(e.pending_faults().contains(FaultKind::InputDevice));
    /// assert_eq!(e.last_fault(FaultKind::InputDevice).map(|r| r.code), Some(2));
    /// ```
    pub fn raise_fault(&mut self, mut record: FaultRecord) {
        record.at = self.anim_now();
        let st = &mut self.faults;
        st.pending.insert(record.kind);
        st.counts.add(record.kind, record.occurrences);
        st.last[record.kind.index()] = Some(record);
        let hook = st.hook;
        twine_core::debug!(
            target: "twine::fault",
            "fault {} display={:?} input={:?} x{} code={}",
            record.kind.name(),
            record.display,
            record.input,
            record.occurrences,
            record.code
        );
        if let Some(hook) = hook {
            hook(&record);
        }
    }

    /// The kinds raised since the last call, clearing them (for applications that poll, e.g.
    /// once per update).
    ///
    /// Also raises [`FaultKind::FormatDisabled`] (no display, `code` = the format's LVGL
    /// discriminant) for each format the software renderer was asked to draw into although it
    /// is not compiled in (see `twine_render::take_format_disabled`), first.
    ///
    /// ```
    /// use twine_engine::{Engine, EngineConfig, FaultRecord};
    /// use twine_core::fault::{FaultKind, Faults};
    ///
    /// let mut e = Engine::new(EngineConfig::default()).unwrap();
    /// e.raise_fault(FaultRecord::new(FaultKind::Capacity));
    /// assert_eq!(e.take_faults(), Faults::from(FaultKind::Capacity));
    /// assert!(e.take_faults().is_empty());
    /// assert_eq!(e.fault_counts().get(FaultKind::Capacity), 1); // counters are kept
    /// ```
    pub fn take_faults(&mut self) -> Faults {
        // Defensive fallback: displays in a disabled format are refused when added, but code
        // drawing into its own buffers (canvases, custom renderers) can still hit a disabled
        // format. The renderer records each such format once per process; the first engine
        // asking raises it here (not per frame: one relaxed load when nothing was recorded).
        while let Some(format) = twine_render::take_format_disabled() {
            self.raise_fault(FaultRecord::new(FaultKind::FormatDisabled).code(u32::from(format as u8)));
        }
        core::mem::take(&mut self.faults.pending)
    }

    /// The kinds raised since the last [`take_faults`](Self::take_faults), without clearing
    /// them.
    #[must_use]
    pub fn pending_faults(&self) -> Faults {
        self.faults.pending
    }

    /// Occurrences of every kind since the engine was created (saturating).
    #[must_use]
    pub fn fault_counts(&self) -> FaultCounts {
        self.faults.counts
    }

    /// The last record raised of `kind` (kept until another one of that kind replaces it).
    #[must_use]
    pub fn last_fault(&self, kind: FaultKind) -> Option<&FaultRecord> {
        self.faults.last[kind.index()].as_ref()
    }

    /// Sets the function called for every raised fault (`None` removes it).
    pub fn set_fault_hook(&mut self, hook: Option<FaultHook>) {
        self.faults.hook = hook;
    }
}
