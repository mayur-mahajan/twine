//! Static draw buffers: [`draw_buffers!`](crate::draw_buffers) and [`DrawBuffers`].

use twine_hal::DrawBuffer;
use twine_reactive::TakeOnce;

/// `N` draw buffers of `BYTES` bytes each in a `static`, handed out once: the type
/// [`draw_buffers!`](crate::draw_buffers) declares.
///
/// [`take`](TakeOnce::take) returns `Some(&'static mut [DrawBuffer<BYTES>; N])` the first
/// time and `None` afterwards (one critical section; no compare-and-swap, so it works on
/// `thumbv6m`). Pass the result to
/// [`BufferMode::partial_double_from`](twine_engine::BufferMode::partial_double_from) (`N = 2`)
/// or [`BufferMode::partial_single_from`](twine_engine::BufferMode::partial_single_from)
/// (`N = 1`); a count that does not match is a compile error.
///
/// Every [`DrawBuffer`] is 4-byte aligned by its type, and the array is constant-initialised
/// with zeros, so it is placed in `.bss`: it costs RAM, not flash.
pub type DrawBuffers<const N: usize, const BYTES: usize> = TakeOnce<[DrawBuffer<BYTES>; N]>;

/// A `static` layer buffer of `BYTES` bytes, handed out once: give it to
/// [`UiBuilder::layer_buf`](crate::UiBuilder::layer_buf) to keep the engine's layer buffer
/// (opacity groups, transforms, blend modes) out of the heap.
///
/// Zero-initialised ([`TakeOnce::zeroed`]), so it is placed in `.bss`: it costs RAM, not
/// flash. At least 4 KiB; the engine's default heap layer buffer is 24 KiB
/// ([`EngineConfig::layer_buf_bytes`](twine_engine::EngineConfig::layer_buf_bytes)).
///
/// ```
/// use twine_view::LayerBuffer;
///
/// static LAYER: LayerBuffer<{ 16 * 1024 }> = LayerBuffer::zeroed();
/// let buf: &'static mut [u8] = LAYER.take().expect("taken once");
/// assert_eq!(buf.len(), 16 * 1024);
/// assert!(LAYER.take().is_none());
/// ```
pub type LayerBuffer<const BYTES: usize> = TakeOnce<[u8; BYTES]>;

/// Declares `static` draw buffers sized at compile time, aligned by their type and handed out
/// once — no `static mut`, no `unsafe`, no hand-computed sizes.
///
/// ```text
/// draw_buffers!(static NAME: COUNT x ROWS rows x WIDTH px @ FORMAT);
/// ```
///
/// - `COUNT`: number of buffers (`2` for DMA ping-pong, `1` otherwise);
/// - `ROWS`: full-width rows per buffer (more rows: fewer, larger flushes);
/// - `WIDTH`: the display's **logical** width in pixels (after rotation);
/// - `FORMAT`: the display's [`ColorFormat`](twine_core::ColorFormat) variant (`Rgb565`,
///   `Rgb565Swapped`, `I1`, …).
///
/// `COUNT`, `ROWS` and `WIDTH` are each a literal, a constant's name or a parenthesised
/// constant expression (`(W / 2)`); `ROWS` and `WIDTH` are `u16`. The `x` separators need
/// spaces around them (`2 x 40`, not `2x40`). Attributes (`///` docs, `#[cfg(..)]`,
/// `#[link_section = ".."]`) and a visibility go before `static`; several declarations may be
/// separated by `;`.
///
/// The expansion is a `static NAME: DrawBuffers<COUNT, { buffer_bytes(WIDTH, ROWS, FORMAT) }>`
/// ([`DrawBuffers`], [`buffer_bytes`](twine_hal::buffer_bytes)). Zero buffers or zero-byte
/// buffers are compile errors. Take the buffers once with `NAME.take()` and pass them to
/// [`BufferMode::partial_double_from`](twine_engine::BufferMode::partial_double_from) /
/// [`partial_single_from`](twine_engine::BufferMode::partial_single_from).
///
/// ```
/// use twine_view::prelude::*;
///
/// draw_buffers! {
///     /// Two DMA ping-pong buffers of 40 rows of a 480 px wide RGB565 panel (2 × 38 400 B).
///     static BUFS: 2 x 40 rows x 480 px @ Rgb565Swapped;
/// }
///
/// let bufs = BUFS.take().expect("draw buffers taken once");
/// assert_eq!(bufs[0].len(), 480 * 2 * 40);
/// let mode = BufferMode::partial_double_from(bufs);
/// assert!(matches!(mode, BufferMode::Partial { b: Some(_), .. }));
/// assert!(BUFS.take().is_none()); // a second take fails
/// ```
///
/// Feature-dependent panels declare one `static` per configuration:
///
/// ```
/// # use twine_view::prelude::*;
/// const W: u16 = 128;
/// draw_buffers! {
///     #[cfg(not(feature = "oled"))]
///     static BUFS: 2 x 20 rows x 320 px @ Rgb565Swapped;
///     /// The whole 128 × 64 OLED at 1 bpp.
///     #[cfg(feature = "oled")]
///     static BUFS: 2 x 64 rows x W px @ I1;
/// }
/// # let _ = BUFS.take().unwrap();
/// ```
#[macro_export]
macro_rules! draw_buffers {
    ($(
        $(#[$attr:meta])*
        $vis:vis static $name:ident : $count:tt x $rows:tt rows x $width:tt px @ $format:ident
    );+ $(;)?) => {$(
        $(#[$attr])*
        // `(W / 2)` arguments are passed on as written.
        #[allow(unused_parens)]
        $vis static $name: $crate::DrawBuffers<
            { $count },
            { $crate::__private::buffer_bytes($width, $rows, $crate::__private::ColorFormat::$format) },
        > = {
            ::core::assert!($count > 0, "draw_buffers!: declare at least one buffer");
            ::core::assert!(
                $crate::__private::buffer_bytes($width, $rows, $crate::__private::ColorFormat::$format) > 0,
                "draw_buffers!: a buffer needs at least one row of at least one pixel",
            );
            $crate::__private::TakeOnce::new([$crate::__private::DrawBuffer::ZEROED; $count])
        };
    )+};
}

#[cfg(test)]
mod tests {
    use twine_core::ColorFormat;
    use twine_hal::{DrawBufferMem, buffer_bytes};

    crate::draw_buffers! {
        static ONE: 1 x 8 rows x 64 px @ Rgb565;
        static TWO: 2 x 40 rows x 480 px @ Rgb565Swapped;
        static MONO: 2 x 64 rows x 128 px @ I1;
        static ODD: 2 x 3 rows x 5 px @ Rgb888;
    }

    #[test]
    fn statics_are_sized_and_aligned() {
        // Compile time: the types carry the computed sizes, and every element is aligned.
        const _: () = assert!(core::mem::align_of::<crate::DrawBuffers<2, 45>>() >= 4);
        const _: () = assert!(core::mem::size_of::<[twine_hal::DrawBuffer<45>; 2]>() == 96);
        let _: &'static crate::DrawBuffers<1, 1_024> = &ONE;
        let _: &'static crate::DrawBuffers<2, 38_400> = &TWO;
        let _: &'static crate::DrawBuffers<2, 1_024> = &MONO;
        let _: &'static crate::DrawBuffers<2, 45> = &ODD;
        // Run time: lengths and addresses of what `take` hands out.
        let one = ONE.take().unwrap();
        assert_eq!(one[0].len(), buffer_bytes(64, 8, ColorFormat::Rgb565));
        let two = TWO.take().unwrap();
        assert_eq!(two.each_ref().map(twine_hal::DrawBuffer::len), [38_400, 38_400]);
        let mono = MONO.take().unwrap();
        assert_eq!(mono[1].len(), 128 * 64 / 8);
        let odd = ODD.take().unwrap();
        assert_eq!(odd[0].len(), 45); // 5 px × 3 B × 3 rows: not a multiple of 4
        for m in odd.each_mut().map(DrawBufferMem::from) {
            assert!(m.is_aligned(4), "{m:?}");
        }
        assert!(ONE.take().is_none() && TWO.take().is_none());
    }
}
