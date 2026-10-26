//! The layer cache: the finished pixels of expensive nodes that stay unchanged, copied instead
//! of drawing the nodes again ([`EngineConfig::layer_cache_bytes`](crate::EngineConfig::layer_cache_bytes)).
//!
//! # What is kept
//!
//! An entry holds, for node `n`'s visible area, the screen's pixels *right after `n` was
//! drawn*: the background, `n`'s ancestors and earlier siblings, `n` and its children — every
//! node drawn before `n` or inside it, in the display's own format. A dirty area inside it is
//! then drawn by copying those pixels and drawing only what comes after `n` (its later
//! siblings, its ancestors' post drawing, the layers above). These are the pixels the normal
//! drawing produces at that point, so the result is the same bit for bit; nothing is
//! composited.
//!
//! # When the pixels go stale
//!
//! A change (an invalidation) inside the entry's area from a node drawn at or before `n`
//! ([`Engine::draws_at_or_before`]) makes it stale; a change from a node drawn after `n` (the
//! content in front of it) does not. A change with no known origin (an explicit
//! `invalidate_area`) makes every entry it touches stale; redrawing after a failed flush does
//! not (the content did not change).
//!
//! # What is cached, automatically
//!
//! Nodes whose widget reports [`DrawCost::Expensive`](crate::DrawCost::Expensive) are watched.
//! One that was drawn in [`STABLE_FRAMES`] frames without changing is cached at its next draw,
//! if it is eligible ([`Engine::cache_eligible`]) and its pixels fit the budget next to the
//! other entries. An entry that is dropped after few uses needs twice as many stable frames
//! the next time (up to [`MAX_STABLE_FRAMES`]): content that changes often is not cached over
//! and over. A node that keeps changing frees its pixels.

use alloc::vec::Vec;

use twine_core::{ColorFormat, Rect};

use crate::invalidate::InvalidateReason;
use crate::{DrawCost, Engine, NodeId, ObjFlags};

/// Nodes watched or cached per engine.
pub(crate) const MAX_ENTRIES: usize = 8;

/// Stable frames before a node is first cached.
pub(crate) const STABLE_FRAMES: u16 = 2;

/// Most stable frames a node can be asked for (the backoff's ceiling).
pub(crate) const MAX_STABLE_FRAMES: u16 = 64;

/// Uses of the pixels below which an entry dropped for a change counts as wasted.
const WORTH_USES: u32 = 4;

/// Frames in a row with a change after which a node's pixels are freed.
const DROP_AFTER: u8 = 4;

/// One watched node.
#[derive(Debug)]
struct Entry {
    node: NodeId,
    display: usize,
    /// The area of the pixels (when `valid`).
    rect: Rect,
    format: ColorFormat,
    /// `rect`'s pixels, rows packed.
    pixels: Vec<u8>,
    valid: bool,
    /// Frames drawn without a change since the last change.
    stable: u16,
    /// Stable frames needed before the node is cached.
    required: u16,
    /// Copies of the pixels since they were drawn.
    uses: u32,
    /// Frames in a row with a change.
    changed_streak: u8,
    /// Drawn in the current frame.
    seen: bool,
    /// A change reached it since the last frame.
    changed: bool,
}

/// The engine's layer cache (see the module documentation).
#[derive(Debug, Default)]
pub(crate) struct LayerCache {
    entries: Vec<Entry>,
    /// The budget, resolved on first use.
    budget: Option<usize>,
    hits: u64,
    builds: u64,
}

/// What the layer cache holds ([`Engine::layer_cache_stats`]).
///
/// ```
/// use twine_engine::{Engine, EngineConfig};
///
/// let e = Engine::new(EngineConfig::default()).unwrap();
/// let s = e.layer_cache_stats();
/// assert_eq!((s.cached, s.bytes, s.hits), (0, 0, 0));
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct LayerCacheStats {
    /// Expensive nodes watched (cached or not).
    pub watched: usize,
    /// Nodes whose pixels are cached and valid.
    pub cached: usize,
    /// Bytes held for pixels.
    pub bytes: usize,
    /// The budget in bytes (0 until the cache was first needed or when it is off).
    pub budget: usize,
    /// Dirty areas drawn from cached pixels, since the engine was created.
    pub hits: u64,
    /// Times pixels were drawn into the cache, since the engine was created.
    pub builds: u64,
}

impl LayerCache {
    fn bytes(&self) -> usize {
        self.entries
            .iter()
            .fold(0usize, |n, e| n.saturating_add(e.pixels.capacity()))
    }
}

/// Bytes per pixel of formats the cache can hold (whole bytes per pixel), else `None`.
fn pixel_bytes(format: ColorFormat) -> Option<usize> {
    let bits = usize::from(format.bpp());
    (bits >= 8 && bits % 8 == 0).then_some(bits / 8)
}

impl Engine {
    /// What the layer cache holds (see [`EngineConfig::layer_cache_bytes`](crate::EngineConfig::layer_cache_bytes)).
    #[must_use]
    pub fn layer_cache_stats(&self) -> LayerCacheStats {
        let c = &self.layer_cache;
        LayerCacheStats {
            watched: c.entries.len(),
            cached: c.entries.iter().filter(|e| e.valid).count(),
            bytes: c.bytes(),
            budget: c.budget.unwrap_or(0),
            hits: c.hits,
            builds: c.builds,
        }
    }

    /// The layer cache budget (resolved on first use, see
    /// [`EngineConfig::layer_cache_bytes`](crate::EngineConfig::layer_cache_bytes)).
    fn layer_cache_budget(&mut self) -> usize {
        if let Some(b) = self.layer_cache.budget {
            return b;
        }
        let b = match self.config.layer_cache_bytes {
            Some(b) => b,
            None if twine_core::SAFETY_PROFILE => 0,
            None => {
                let pixels = self.displays.iter().fold(0usize, |n, d| {
                    let px = usize::from(d.info.width).saturating_mul(usize::from(d.info.height));
                    let bytes = pixel_bytes(d.info.format).unwrap_or(0);
                    n.saturating_add(px.saturating_mul(bytes))
                });
                self.config
                    .mem_info
                    .map_or(0, |f| (f().free as usize / 4).min(pixels.saturating_mul(2)))
            }
        };
        twine_core::info!(target: "twine::perf", "layer cache: budget {} B", b);
        self.layer_cache.budget = Some(b);
        b
    }

    /// Notes a change of `area` of display `d` (from `origin`, when known) for the entries
    /// whose pixels it may make stale.
    pub(crate) fn layer_cache_change(
        &mut self,
        d: usize,
        area: Rect,
        origin: Option<NodeId>,
        reason: InvalidateReason,
    ) {
        if self.layer_cache.entries.is_empty() || reason == InvalidateReason::FlushRetry {
            return;
        }
        for i in 0..self.layer_cache.entries.len() {
            let Some(e) = self.layer_cache.entries.get(i) else {
                break;
            };
            if e.display != d {
                continue;
            }
            let node = e.node;
            let current = self.visible_area(node).map(|(_, r)| r);
            let touches = e.valid && e.rect.intersects(&area) || current.is_some_and(|r| r.intersects(&area));
            if !touches {
                continue;
            }
            let before = origin.is_none_or(|o| self.draws_at_or_before(o, node));
            if !before {
                continue;
            }
            if let Some(e) = self.layer_cache.entries.get_mut(i) {
                e.changed = true;
                // Not due again until it settles (also within the current frame).
                e.stable = 0;
                if e.valid {
                    e.valid = false;
                    // Pixels dropped after few uses: ask for twice the stable frames next time.
                    e.required = if e.uses < WORTH_USES {
                        e.required.saturating_mul(2).min(MAX_STABLE_FRAMES)
                    } else {
                        STABLE_FRAMES
                    };
                }
            }
        }
    }

    /// Notes that `id` (an expensive node) was drawn in the current frame.
    pub(crate) fn layer_cache_seen(&self, id: NodeId) {
        if let Ok(mut seen) = self.cache_seen.try_borrow_mut()
            && !seen.contains(&id)
        {
            let _ = seen.push(id);
        }
    }

    /// The bookkeeping after a frame: new expensive nodes are watched, the stable frames are
    /// counted, entries of deleted or volatile nodes are dropped.
    pub(crate) fn layer_cache_frame(&mut self) {
        let seen: heapless::Vec<NodeId, MAX_ENTRIES> = self
            .cache_seen
            .try_borrow_mut()
            .map_or_else(|_| heapless::Vec::new(), |mut s| core::mem::take(&mut *s));
        for id in seen {
            let display = self.visible_area(id).map(|(d, _)| d);
            let c = &mut self.layer_cache;
            let full = c.entries.len() >= MAX_ENTRIES;
            match c.entries.iter_mut().find(|e| e.node == id) {
                Some(e) => e.seen = true,
                None if !full => {
                    if let Some(display) = display
                        && c.entries.try_reserve(1).is_ok()
                    {
                        c.entries.push(Entry {
                            node: id,
                            display,
                            rect: Rect::ZERO,
                            format: ColorFormat::Rgb565,
                            pixels: Vec::new(),
                            valid: false,
                            stable: 0,
                            required: STABLE_FRAMES,
                            uses: 0,
                            changed_streak: 0,
                            seen: true,
                            changed: false,
                        });
                    }
                }
                None => {}
            }
        }
        let alive: heapless::Vec<bool, MAX_ENTRIES> = self
            .layer_cache
            .entries
            .iter()
            .map(|e| self.tree.contains(e.node))
            .collect();
        let mut k = 0usize;
        self.layer_cache.entries.retain(|_| {
            let keep = alive.get(k).copied().unwrap_or(false);
            // SAFE-ARITH: `k < entries.len() ≤ MAX_ENTRIES`.
            k = k.wrapping_add(1);
            keep
        });
        self.layer_cache.entries.retain_mut(|e| {
            if e.changed {
                e.stable = 0;
                e.changed_streak = e.changed_streak.saturating_add(1);
                if e.changed_streak >= DROP_AFTER {
                    // Volatile: free the pixels and stop watching (it is watched again when
                    // drawn, with its backoff forgotten only after it settles).
                    return false;
                }
            } else if e.seen {
                e.stable = e.stable.saturating_add(1);
                e.changed_streak = 0;
            }
            e.changed = false;
            e.seen = false;
            true
        });
    }

    /// Whether `x` is drawn at or before `n`: `x` is `n`, inside `n`, an ancestor of `n`
    /// (counting its post drawing too, conservatively), or in a branch drawn before `n`'s. Nodes
    /// of roots that are not drawn below `n`'s root count as after.
    pub(crate) fn draws_at_or_before(&self, x: NodeId, n: NodeId) -> bool {
        if x == n || self.tree.ancestors(n).any(|a| a == x) || self.tree.ancestors(x).any(|a| a == n) {
            return true;
        }
        // The lowest common ancestor: the first ancestor of `x` that is an ancestor of `n`;
        // `xb` and `nb` are the children of it on the way to `x` and `n`.
        let mut xb = x;
        for p in self.tree.ancestors(x) {
            let mut nb = n;
            for q in self.tree.ancestors(n) {
                if q == p {
                    // Whichever branch comes first among `p`'s children.
                    for c in self.tree.children(p) {
                        if c == xb {
                            return true;
                        }
                        if c == nb {
                            return false;
                        }
                    }
                    return false;
                }
                nb = q;
            }
            xb = p;
        }
        // Different roots: their order on `n`'s display.
        let (Some(rx), Some(rn)) = (self.root_of(x), self.root_of(n)) else {
            return false;
        };
        self.displays.iter().find_map(|d| {
            let order = d.draw_order();
            let ix = order.iter().position(|&r| r == rx)?;
            let iy = order.iter().position(|&r| r == rn)?;
            Some(ix < iy)
        }) == Some(true)
    }

    /// The root of `id`'s tree.
    fn root_of(&self, id: NodeId) -> Option<NodeId> {
        self.tree.node(id)?;
        Some(self.tree.ancestors(id).last().unwrap_or(id))
    }

    /// Whether `id`'s pixels can stand in for drawing it and what is below: no ancestor (or
    /// itself) draws through a layer (the pixels would not be the screen's), it is shown, and no
    /// safety-critical node exists (those are always drawn fresh).
    pub(crate) fn cache_eligible(&self, id: NodeId) -> bool {
        let Some(n) = self.tree.node(id) else {
            return false;
        };
        if n.is_hidden() || n.flags().contains(ObjFlags::LAYOUT_PASSTHROUGH) {
            return false;
        }
        #[cfg(feature = "display-integrity")]
        if self.critical_nodes().len() > 0 {
            return false;
        }
        !core::iter::once(id)
            .chain(self.tree.ancestors(id))
            .any(|a| self.needs_layer(a) || self.tree.node(a).is_some_and(crate::Node::is_hidden))
    }

    /// The entry whose pixels draw `clip` of display `d` (in `format`): valid, or due to be
    /// built; `clip` inside its area, not crossing a rounded corner its ancestors clip to, and
    /// not covered by something drawn after it. The one drawn last wins.
    pub(crate) fn layer_cache_pick(&mut self, d: usize, clip: Rect, format: ColorFormat) -> Option<usize> {
        if self.layer_cache.entries.is_empty() || pixel_bytes(format).is_none() {
            return None;
        }
        let mut best: Option<usize> = None;
        for i in 0..self.layer_cache.entries.len() {
            let Some(e) = self.layer_cache.entries.get(i) else {
                break;
            };
            let due = !e.valid && e.stable >= e.required;
            if e.display != d || !(e.valid || due) {
                continue;
            }
            let node = e.node;
            let area = if e.valid {
                e.rect
            } else {
                match self.visible_area(node) {
                    Some((_, r)) => r,
                    None => continue,
                }
            };
            if !area.contains_rect(&clip) || !self.cache_eligible(node) || !self.corners_allow(node, clip) {
                continue;
            }
            if self.covered_after(d, node, clip) {
                continue;
            }
            best = match best {
                Some(b) => {
                    let bn = self.layer_cache.entries.get(b).map(|e| e.node);
                    if bn.is_some_and(|bn| self.draws_at_or_before(bn, node)) {
                        Some(i)
                    } else {
                        Some(b)
                    }
                }
                None => Some(i),
            };
        }
        best
    }

    /// Whether an ancestor of `id` clips its children to rounded corners that reach into
    /// `clip` (drawing after `id` does not apply those masks; the cover search does the same).
    fn corners_allow(&self, id: NodeId, clip: Rect) -> bool {
        self.tree.ancestors(id).all(|a| {
            let Some(n) = self.tree.node(a) else {
                return true;
            };
            if !crate::refresh::clips_corners(self, a) {
                return true;
            }
            let r = crate::widget::effective_radius(n.coords(), self.cached_main(a).radius);
            let c = n.coords();
            c.inset(twine_core::Insets::new(r, 0, r, 0)).contains_rect(&clip)
                || c.inset(twine_core::Insets::new(0, r, 0, r)).contains_rect(&clip)
        })
    }

    /// Whether the topmost opaque cover of `clip` is drawn after `id` (then `id` does not show).
    fn covered_after(&self, d: usize, id: NodeId, clip: Rect) -> bool {
        let Some(disp) = self.displays.get(d) else {
            return true;
        };
        let order = disp.draw_order();
        order
            .iter()
            .skip(1)
            .rev()
            .find_map(|&r| crate::refresh::find_top_cover(self, r, clip))
            .is_some_and(|top| !self.draws_at_or_before(top, id))
    }

    /// Makes entry `i`'s pixels valid (drawing them when due). Returns `false` when they cannot
    /// be held (budget, memory, format).
    pub(crate) fn layer_cache_ready(
        &mut self,
        i: usize,
        res: &mut crate::engine::RenderRes,
        frame: u32,
    ) -> bool {
        let Some(e) = self.layer_cache.entries.get(i) else {
            return false;
        };
        if e.valid {
            return true;
        }
        let (node, d) = (e.node, e.display);
        let Some((_, rect)) = self.visible_area(node) else {
            return false;
        };
        let Some(format) = self.displays.get(d).map(|x| x.info.format) else {
            return false;
        };
        let Some(bpp) = pixel_bytes(format) else {
            return false;
        };
        let stride = usize::try_from(rect.width()).unwrap_or(0).saturating_mul(bpp);
        let needed = stride.saturating_mul(usize::try_from(rect.height()).unwrap_or(0));
        let budget = self.layer_cache_budget();
        let held = self.layer_cache.bytes();
        let mut pixels = self
            .layer_cache
            .entries
            .get_mut(i)
            .map(|e| core::mem::take(&mut e.pixels))
            .unwrap_or_default();
        let others = held.saturating_sub(pixels.capacity());
        if needed == 0 || others.saturating_add(needed) > budget {
            if let Some(e) = self.layer_cache.entries.get_mut(i) {
                e.pixels = pixels;
                // Not now: count the stable frames again.
                e.stable = 0;
            }
            return false;
        }
        pixels.clear();
        if pixels.try_reserve_exact(needed).is_err() {
            if let Some(e) = self.layer_cache.entries.get_mut(i) {
                e.stable = 0;
            }
            return false;
        }
        pixels.resize(needed, 0);
        // Drawn without the accelerator (its queued work would race the copy-out).
        let accel = res.accel.0.take();
        let ok = crate::refresh::render_until(self, res, d, &mut pixels, format, stride, rect, node, frame);
        res.accel.0 = accel;
        self.layer_cache.builds = self.layer_cache.builds.saturating_add(1);
        if let Some(e) = self.layer_cache.entries.get_mut(i) {
            e.pixels = pixels;
            e.rect = rect;
            e.format = format;
            e.valid = ok;
            e.uses = 0;
        }
        ok
    }

    /// Copies `clip` of entry `i`'s pixels into `buf` (pixels of `buf_area`, rows `stride`
    /// bytes apart) and counts the use. `clip` lies inside the entry's area and `buf_area`.
    pub(crate) fn layer_cache_copy(
        &mut self,
        i: usize,
        buf: &mut [u8],
        stride: usize,
        buf_area: Rect,
        clip: Rect,
    ) -> Option<NodeId> {
        let c = &mut self.layer_cache;
        let e = c.entries.get_mut(i)?;
        let bpp = pixel_bytes(e.format)?;
        let src_stride = usize::try_from(e.rect.width()).ok()?.saturating_mul(bpp);
        let w = usize::try_from(clip.width()).ok()?.saturating_mul(bpp);
        let col = |area: Rect| {
            usize::try_from(clip.x0.saturating_sub(area.x0))
                .ok()
                .map(|x| x.saturating_mul(bpp))
        };
        let (sx, dx) = (col(e.rect)?, col(buf_area)?);
        for y in clip.y0..clip.y1 {
            let sy = usize::try_from(y.saturating_sub(e.rect.y0)).ok()?;
            let dy = usize::try_from(y.saturating_sub(buf_area.y0)).ok()?;
            let s = sy.saturating_mul(src_stride).saturating_add(sx);
            let t = dy.saturating_mul(stride).saturating_add(dx);
            let src = e.pixels.get(s..s.saturating_add(w))?;
            buf.get_mut(t..t.saturating_add(w))?.copy_from_slice(src);
        }
        e.uses = e.uses.saturating_add(1);
        c.hits = c.hits.saturating_add(1);
        Some(e.node)
    }

    /// Forgets the entries of display `d` (its size, format or rotation changed).
    pub(crate) fn layer_cache_forget_display(&mut self, d: usize) {
        self.layer_cache.entries.retain(|e| e.display != d);
    }

    /// One-line summary for the `twine::perf` log.
    pub(crate) fn layer_cache_log(&self) {
        let s = self.layer_cache_stats();
        if s.watched > 0 {
            twine_core::info!(
                target: "twine::perf",
                "layer cache: {} of {} nodes cached, {}/{} B, {} hits, {} builds",
                s.cached,
                s.watched,
                s.bytes,
                s.budget,
                s.hits,
                s.builds
            );
        }
    }
}

/// Whether a widget's drawing cost makes it worth watching.
pub(crate) fn watched(cost: DrawCost) -> bool {
    cost == DrawCost::Expensive
}
