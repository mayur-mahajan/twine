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
    entries: heapless::Vec<Entry, SVG_CACHE_ENTRIES>,
    clock: u32,
    parses: u32,
}

impl core::fmt::Debug for SvgCache {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SvgCache")
            .field("entries", &self.entries.len())
            .field("parses", &self.parses)
            .finish_non_exhaustive()
    }
}

impl SvgCache {
    /// The document of `bytes`, parsed on first use.
    pub(crate) fn get(&mut self, bytes: &'static [u8]) -> Result<&SvgDocument, SvgError> {
        let key = (bytes.as_ptr() as usize, bytes.len());
        self.clock = self.clock.wrapping_add(1);
        let i = if let Some(i) = self.entries.iter().position(|e| e.key == key) {
            i
        } else {
            self.parses += 1;
            let doc = parse_svg(bytes).map(Box::new);
            match &doc {
                Ok(d) => twine_core::debug!(target: "twine::image", "svg parsed: {} items", d.scene.len()),
                Err(e) => twine_core::warn!(target: "twine::image", "svg: {}", e),
            }
            let entry = Entry {
                key,
                last_use: self.clock,
                doc,
            };
            if self.entries.is_full() {
                let clock = self.clock;
                let lru = (0..self.entries.len())
                    .max_by_key(|&i| clock.wrapping_sub(self.entries[i].last_use))
                    .unwrap_or(0);
                self.entries[lru] = entry;
                lru
            } else {
                let _ = self.entries.push(entry);
                self.entries.len() - 1
            }
        };
        let e = &mut self.entries[i];
        e.last_use = self.clock;
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
