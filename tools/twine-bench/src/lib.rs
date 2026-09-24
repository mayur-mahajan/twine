//! Benchmarks of the Twine engine (refresh, layout, animations), kept in their own package so
//! they are built exactly like an application builds the engine (without the expensive
//! invariant checks the engine's tests enable).
//!
//! Run with `cargo bench -p twine-bench` (or `--bench refresh|layout|anim`). The library is
//! empty; the benchmarks are in `benches/`.
