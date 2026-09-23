//! [`RenderHarness`]: an in-memory draw buffer with [`RenderCaches`] for renderer tests and
//! snapshots (feature `render`).

use twine_core::{Color, ColorFormat, Rect};
use twine_render::{DrawBuf, Painter, RenderCaches, RenderConfig};

use crate::snapshot::{SnapshotConfig, Tolerance, assert_rgb_snapshot};

/// A `w × h` buffer in one pixel format plus the renderer caches.
///
/// ```
/// use twine_core::{Color, ColorFormat, Opa, Rect};
/// use twine_testing::RenderHarness;
///
/// let mut h = RenderHarness::new(4, 4, ColorFormat::Rgb565);
/// h.paint(|p| p.fill(Rect::from_xywh(0, 0, 2, 2), Color::RED, Opa::COVER));
/// assert_eq!(&h.rgb888()[..3], &[255, 0, 0]);
/// assert_eq!(&h.rgb888()[3 * 3..3 * 4], &[255, 255, 255]);
/// ```
#[derive(Debug)]
pub struct RenderHarness {
    w: u16,
    h: u16,
    format: ColorFormat,
    data: Vec<u8>,
    caches: RenderCaches,
}

impl RenderHarness {
    /// A buffer cleared to white (opaque for `Argb8888`).
    ///
    /// # Panics
    /// If `format` cannot be drawn into or an `I1` width is not a multiple of 8.
    #[must_use]
    pub fn new(w: u16, h: u16, format: ColorFormat) -> Self {
        let len = format.stride(u32::from(w)) as usize * usize::from(h);
        let mut s = Self {
            w,
            h,
            format,
            data: vec![0; len],
            caches: RenderCaches::new(&RenderConfig::default()),
        };
        s.clear(Color::WHITE);
        s
    }

    /// Replaces the caches with ones built from `cfg`.
    #[must_use]
    pub fn with_config(mut self, cfg: RenderConfig) -> Self {
        self.caches = RenderCaches::new(&cfg);
        self
    }

    /// The screen area of the buffer.
    #[must_use]
    pub fn area(&self) -> Rect {
        Rect::from_xywh(0, 0, i32::from(self.w), i32::from(self.h))
    }

    /// The pixel format.
    #[must_use]
    pub fn format(&self) -> ColorFormat {
        self.format
    }

    /// The raw buffer bytes.
    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// The caches (e.g. to read their statistics).
    #[must_use]
    pub fn caches(&self) -> &RenderCaches {
        &self.caches
    }

    fn buf(&mut self) -> DrawBuf<'_> {
        let area = self.area();
        DrawBuf::new_packed(&mut self.data, self.format, area).expect("valid harness buffer")
    }

    /// Fills the whole buffer with `c` (opaque).
    pub fn clear(&mut self, c: Color) {
        self.buf().clear(c);
    }

    /// Draws with a painter over the whole buffer.
    pub fn paint<R>(&mut self, f: impl FnOnce(&mut Painter<'_>) -> R) -> R {
        let area = self.area();
        let buf = DrawBuf::new_packed(&mut self.data, self.format, area).expect("valid harness buffer");
        let mut p = Painter::new(buf, &mut self.caches);
        f(&mut p)
    }

    /// Draws in horizontal strips of `rows` rows, each into a separate strip buffer that starts
    /// with the current content and is copied back — like the engine's partial buffers. Used to
    /// prove that chunked rendering is pixel-identical to a full render.
    pub fn paint_chunked(&mut self, rows: u16, mut f: impl FnMut(&mut Painter<'_>)) {
        let stride = self.format.stride(u32::from(self.w)) as usize;
        let rows = rows.max(1);
        let mut y = 0u16;
        while y < self.h {
            let n = rows.min(self.h - y);
            let range = usize::from(y) * stride..usize::from(y + n) * stride;
            let mut strip = self.data[range.clone()].to_vec();
            {
                let area = Rect::from_xywh(0, i32::from(y), i32::from(self.w), i32::from(n));
                let buf = DrawBuf::new_packed(&mut strip, self.format, area).expect("valid strip");
                let mut p = Painter::new(buf, &mut self.caches);
                f(&mut p);
            }
            self.data[range].copy_from_slice(&strip);
            y += n;
        }
    }

    /// The buffer as 8-bit RGB (`Argb8888` composited over mid gray, `I1` as black/white).
    #[must_use]
    pub fn rgb888(&self) -> Vec<u8> {
        crate::convert::to_rgb888(
            &self.data,
            self.format,
            u32::from(self.w),
            u32::from(self.h),
            self.format.stride(u32::from(self.w)),
            (Color::BLACK, Color::WHITE),
        )
    }

    /// Compares the buffer with the reference snapshot `name` of `cfg`, pixel-exact. Usually
    /// called through [`assert_render_snapshot!`](crate::assert_render_snapshot).
    pub fn assert_snapshot_with(&self, cfg: &SnapshotConfig, name: &str) {
        assert_rgb_snapshot(
            cfg,
            name,
            u32::from(self.w),
            u32::from(self.h),
            &self.rgb888(),
            Tolerance::EXACT,
        );
    }
}

/// Asserts a [`RenderHarness`] snapshot stored in the calling crate's `tests/snapshots/`.
///
/// ```no_run
/// use twine_core::ColorFormat;
/// let h = twine_testing::RenderHarness::new(8, 8, ColorFormat::Rgb565);
/// twine_testing::assert_render_snapshot!(h, "empty");
/// ```
#[macro_export]
macro_rules! assert_render_snapshot {
    ($h:expr, $name:expr) => {
        $h.assert_snapshot_with(&$crate::snapshot_config!(), $name)
    };
}
