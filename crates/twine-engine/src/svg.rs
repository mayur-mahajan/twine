//! [`SvgCache`]: SVG documents of `ImageSource::Svg`, parsed once (feature `svg`).

use alloc::boxed::Box;

use twine_vector::svg::{SvgDocument, SvgError, parse_svg};

/// Documents kept parsed.
pub(crate) const SVG_CACHE_ENTRIES: usize = 4;

struct Entry {
    /// Address and length of the source bytes.
    key: (usize, usize),
    last_use: u32,
    doc: Result<Box<SvgDocument>, SvgError>,
}

/// The last [`SVG_CACHE_ENTRIES`] parsed documents, least recently used out; keyed by the
/// address of the `'static` bytes (a parse error is cached too, so a broken file is not
/// parsed every frame).
#[derive(Default)]
pub(crate) struct SvgCache {
    slots: [Option<Entry>; SVG_CACHE_ENTRIES],
    clock: u32,
    parses: u32,
}

impl core::fmt::Debug for SvgCache {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SvgCache")
            .field("entries", &self.slots.iter().flatten().count())
            .field("parses", &self.parses)
            .finish_non_exhaustive()
    }
}

/// How strongly a cache slot is chosen for `key` at `clock`: the slot holding `key` first,
/// then an empty slot, then the least recently used one.
fn slot_rank(slot: Option<&Entry>, key: (usize, usize), clock: u32) -> u64 {
    match slot {
        Some(e) if e.key == key => u64::MAX,
        None => u64::MAX - 1,
        Some(e) => u64::from(clock.wrapping_sub(e.last_use)),
    }
}

impl SvgCache {
    /// The document of `bytes`, parsed on first use.
    pub(crate) fn get(&mut self, bytes: &'static [u8]) -> Result<&SvgDocument, SvgError> {
        let key = (bytes.as_ptr() as usize, bytes.len());
        self.clock = self.clock.wrapping_add(1);
        let clock = self.clock;
        // The slot for `key` (no index: the array has a first slot, the rest compete with it).
        let [first, rest @ ..] = &mut self.slots;
        let slot = rest.iter_mut().fold(first, |best, s| {
            if slot_rank(s.as_ref(), key, clock) > slot_rank(best.as_ref(), key, clock) {
                s
            } else {
                best
            }
        });
        // Taken out and put back (a few bytes): the hit, or a new parse replacing the slot.
        let entry = match slot.take() {
            Some(e) if e.key == key => e,
            _ => {
                self.parses = self.parses.saturating_add(1);
                let doc = parse_svg(bytes).map(Box::new);
                match &doc {
                    Ok(d) => {
                        twine_core::debug!(target: "twine::image", "svg parsed: {} items", d.scene.len());
                    }
                    Err(e) => twine_core::warn!(target: "twine::image", "svg: {}", e),
                }
                Entry {
                    key,
                    last_use: clock,
                    doc,
                }
            }
        };
        let e = slot.insert(entry);
        e.last_use = clock;
        match &e.doc {
            Ok(d) => Ok(d),
            Err(err) => Err(*err),
        }
    }

    /// Documents parsed so far (for tests: a document is parsed once while cached).
    pub(crate) fn parses(&self) -> u32 {
        self.parses
    }
}
