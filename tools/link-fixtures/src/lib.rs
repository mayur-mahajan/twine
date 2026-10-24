//! Linking fixtures (R7.S01, design `docs/plan/r7-theming-design.md` §2.9.7): minimal `no_std`
//! images, each building one small UI the way an application does. `cargo xtask nostd` links
//! them for an embedded target and checks their symbol tables against `fixtures.toml`: an image
//! must not link a theme, mode table, recipe, font or icon that its source does not reference.
//!
//! This library is what every fixture shares: the entry point's runner ([`run`]), a heap, a
//! panic handler, a display that drops its pixels and a clock that never advances — and
//! [`design`], a small design language shipped the way a theme crate ships one, for the
//! `design_system` fixture. The images are linked, never flashed.
#![no_std]

extern crate alloc;

pub mod design;

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicUsize, Ordering};

use twine::core::{ColorFormat, Instant, Rect};
use twine::hal::{Clock, DisplayDriver, DisplayInfo, DrawBufferMem};
use twine::prelude::*;

/// Heap size: enough for the fixtures' small UIs (never used: the images do not run).
const HEAP_BYTES: usize = 64 * 1024;

/// A bump allocator over a static array (frees nothing): the smallest correct global allocator,
/// so that the allocator adds next to nothing to the images' symbol tables.
struct Bump {
    next: AtomicUsize,
    heap: UnsafeCell<[u8; HEAP_BYTES]>,
}

// SAFETY: `next` is only advanced atomically, and each successful `alloc` hands out a disjoint
// range of `heap`, so no two callers ever get the same bytes.
unsafe impl Sync for Bump {}

// SAFETY: `alloc` returns a pointer into `heap` aligned to `layout.align()` with
// `layout.size()` bytes available that no other allocation overlaps, or null when the heap is
// exhausted; `dealloc` does nothing, which is always sound.
unsafe impl core::alloc::GlobalAlloc for Bump {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        let base = self.heap.get().cast::<u8>();
        let mut cur = self.next.load(Ordering::Relaxed);
        loop {
            let addr = base as usize + cur;
            let start = addr.next_multiple_of(layout.align()) - base as usize;
            let Some(end) = start.checked_add(layout.size()).filter(|e| *e <= HEAP_BYTES) else {
                return core::ptr::null_mut();
            };
            match self
                .next
                .compare_exchange(cur, end, Ordering::Relaxed, Ordering::Relaxed)
            {
                Ok(_) => return base.wrapping_add(start),
                Err(now) => cur = now,
            }
        }
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: core::alloc::Layout) {}
}

#[global_allocator]
static HEAP: Bump = Bump {
    next: AtomicUsize::new(0),
    heap: UnsafeCell::new([0; HEAP_BYTES]),
};

/// Critical sections of an image without interrupt handlers and with one core: nothing can
/// preempt the code inside, so there is nothing to mask.
struct NoInterrupts;

critical_section::set_impl!(NoInterrupts);

// SAFETY: the images have one core and no interrupt handlers (`link.x` has no vector table), so
// no other code can run between `acquire` and `release`.
unsafe impl critical_section::Impl for NoInterrupts {
    unsafe fn acquire() -> critical_section::RawRestoreState {}

    unsafe fn release(_restore: critical_section::RawRestoreState) {}
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {
        core::hint::spin_loop();
    }
}

/// A 320 × 240 RGB565 panel that drops the pixels (a blocking driver: every flush is done at
/// once).
struct NullPanel {
    done: Option<DrawBufferMem>,
}

impl DisplayDriver for NullPanel {
    type Error = ();

    fn info(&self) -> DisplayInfo {
        DisplayInfo::new(320, 240, ColorFormat::Rgb565)
    }

    fn begin_flush(&mut self, _area: Rect, buf: DrawBufferMem) -> Result<(), ()> {
        self.done = Some(buf);
        Ok(())
    }

    fn poll_flush(&mut self) -> Option<DrawBufferMem> {
        self.done.take()
    }
}

/// A clock that stays at zero.
struct Frozen;

impl Clock for Frozen {
    fn now(&self) -> Instant {
        Instant::from_micros(0)
    }
}

/// Builds the UI `app` with `config` on a [`NullPanel`] and updates it forever: what an
/// application's `main` does, so the image links everything the UI needs to run.
pub fn run<V: View>(config: AppConfig, app: impl FnOnce(Scope) -> V) -> ! {
    let built = Runtime::take().map(|rt| {
        Ui::builder(NullPanel { done: None })
            .runtime(rt)
            .clock(Frozen)
            .buffers(BufferMode::alloc(BufferSpec::default()))
            .app_config(config)
            .try_build(app)
    });
    if let Some(Ok(mut ui)) = built {
        loop {
            let _ = ui.update();
        }
    }
    loop {
        core::hint::spin_loop();
    }
}
