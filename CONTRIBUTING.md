# Contributing to Twine

Thanks for helping build Twine! Twine runs on microcontrollers with a few hundred kilobytes of RAM
and on battery power, so the rules below focus on keeping it small, fast, deterministic and
well tested. Please read them before opening a pull request.

## Getting set up

- **Rust**: the stable toolchain and the embedded targets are pinned in `rust-toolchain.toml`;
  `rustup` installs them automatically the first time you build.
- **Optional tools** (the checks that need them are skipped with a warning if missing):
  - nightly toolchain with Miri: `rustup toolchain install nightly --component miri`
  - coverage: `cargo install cargo-llvm-cov`; MC/DC (`cargo xtask coverage --mcdc`) also needs
    `rustup toolchain install nightly --component llvm-tools-preview`
  - Xtensa (ESP32, ESP32-S3) firmware builds: [`espup`](https://github.com/esp-rs/espup)
    (`espup install`); `cargo xtask firmware` builds the example firmware in `firmware/` and
    skips examples whose toolchain or target is missing
- **Simulator**: `cargo xtask sim <example>` opens a desktop window (pure Rust, no SDL needed).
  The examples are Cargo examples of the facade crate (`crates/twine/examples/`, also run with
  `cargo run -p twine --example <example>`); `cargo test` compiles them without running them.
  Set `TWINE_SIM_HEADLESS=1` to run without a window and write PNG screenshots instead.
- **Logs**: `RUST_LOG=twine=debug` (or e.g. `twine::refresh=trace`) in the simulator and tests.

## Before you open a pull request

Every PR must meet this **Definition of Done**:

1. **`cargo xtask ci` passes.** It runs formatting, clippy with `-D warnings` (default features, all
   features, every library crate with its `defmt` logging backend, and the facade in the
   `safety` profile with its lint-status ratchet and the panic-inventory check), all tests, `no_std` builds for every
   embedded target (also in the `safety` profile), the check that the profile refuses its
   excluded parts and lacks the panicking convenience APIs, rustdoc, the layering check, the work-marker check,
   the generated-fonts, sample-images and style-property-table checks, a benchmark build, snapshot tests, Miri and
   the example firmware builds. Use
   `cargo xtask ci --quick` while iterating.
2. **Tests cover the change.** New behaviour has tests; bug fixes add a regression test named
   `regression_<short_description>` that fails without the fix.
3. **Public API is documented.** Every public item has rustdoc (the build denies missing docs);
   a public function returning `Result` has an `# Errors` section and one that can panic a
   `# Panics` section (clippy `missing_errors_doc` / `missing_panics_doc`, enforced by CI).
   Explain *what* and *why*, document panics and errors, and add a short example (doctest) for
   types and functions a user will call directly.
4. **Visual changes are reviewed.** Golden images are the hashes in each crate's committed
   `tests/snapshots.toml`. If rendering output changes on purpose, run `cargo xtask snapshots`,
   inspect the actual and diff images it lists under `target/twine-snapshots/`, then approve
   with `cargo xtask snapshots --update`, and attach before/after screenshots to the PR. New
   snapshots are proposed with `cargo xtask snapshots --propose` (a `[pending]` block, refused by
   CI until approved with `--accept`). Never edit a manifest by hand.
5. **Performance is considered.** For changes in rendering, layout, styles, input or the
   reactive runtime, run `cargo xtask bench` (renderer, text) and `cargo bench -p twine-bench`
   (engine refresh, layout and animation frames; `--bench theme` for themed resolution, state
   changes and mode switches) and mention the impact in the PR. Regressions
   need a justification.
6. **Generated files are regenerated, not edited.** Built-in fonts come from
   `assets/fonts/fonts.toml` via `cargo xtask fonts`; `cargo xtask fonts --check` fails when a
   generated file is out of date. Sample images in `assets/images/` are drawn by
   `cargo xtask gen-assets` (`--check` too); examples, tests and demos convert images and
   subset fonts while they build (`twine::include_image!`, `twine-build` in `build.rs`), so
   no converted copies are committed. The style property table
   `crates/twine-style/PROPERTIES.md` is written by `cargo xtask style-props` (`--check` too). New asset files record their source URL, license and SHA-256
   in the `SOURCES.md` of their folder.
7. **Nothing is left half-done.** No `todo!()`, `unimplemented!()`, placeholder return values,
   ignored tests or `TODO`/`FIXME` comments. Track follow-up work in an issue instead.
8. **The simulator example still runs** if you touched anything visual or interactive
   (`cargo xtask sim-smoke` runs every example headless).

## Hard rules

These keep Twine usable on small, battery-powered devices. CI or review will reject violations.

- **No work when idle.** When nothing is animating, pressed or scheduled, `update()` must report
  idle and do no rendering. Don't add periodic polling; use deadlines and interrupts.
- **Work proportional to change.** A property change invalidates only the affected area and
  re-runs only dependent bindings.
- **Idempotent setters.** Setting a property to its current value must not invalidate, relayout
  or allocate.
- **No heap allocation while rendering.** The render/flush path is allocation-free in steady
  state; caches have fixed budgets and evict instead of growing.
- **No floating point in hot paths.** Rendering, layout, styles, text rendering, vector
  rasterization and the engine use integer/fixed-point math (`clippy::float_arithmetic` is denied
  there). This keeps output identical on every platform and fast on MCUs without an FPU.
- **Divide `i128` through `twine_core::math::div_rem_i128`** (or `div_rem_euclid_i128`), not
  with `/`, `%` or `checked_div*` by a non-constant divisor: 32-bit targets have no 128-bit division in their runtime
  library, so LLVM expands each one inline (several hundred bytes of flash per site); the shared
  helper is one out-of-line copy with a 64-bit fast path.
- **`no_std` + `alloc` compatible.** Library crates must build for `thumbv6m-none-eabi`
  (RP2040), which has no atomic compare-and-swap: don't use `Arc`, `alloc::sync` or CAS atomics;
  use `Rc`, `portable-atomic` and `critical-section`.
- **Never panic on user input.** Invalid ids or parameters are logged with `warn!` and ignored.
  Use `debug_assert!` for internal invariants.
- **Keep the safety profile clean.** Under the `safety` feature the library crates deny
  `clippy::{indexing_slicing, unwrap_used, expect_used, panic, unreachable,
  arithmetic_side_effects}` (since R5.S14 every profile crate is clean of indexing and overflow
  sites; the only listed lint is twine-reactive's two deliberate `panic!` sites), and CI
  ratchets the counts (`cargo xtask safety-lints`). Write new code so that it adds no site: iterators or
  `get` instead of indexing, explicit `saturating_*`/`wrapping_*`/`checked_*` arithmetic, no
  `unwrap`; a `wrapping_*` operation that relies on a range argument, or a checked access whose
  fallback is never taken, carries a `// SAFE-ARITH: …` / `// SAFE-INDEX: …` proof. An API that panics by contract gets a `Result` form and is removed from the profile
  (`#[cfg(not(feature = "safety"))]`); a panic that must stay carries a `// PANIC: …`
  justification. After changing a panic site, run `cargo xtask panics` (the inventory
  `docs/safety/panics.md`, checked by CI). See `docs/safety/profile.md` § 4 and § 7.
- **Keep recursion over the tree bounded and lean.** A new walk over the widget tree is
  iterative unless it must recurse; a recursive one is bounded by `Limits::max_tree_depth`,
  keeps per-level work in `#[inline(never)]` helpers, and gets a scene in
  `twine-engine/tests/stack_depth.rs` (`cargo xtask stack-budget`, `docs/safety/profile.md` § 10).
- **No process-wide mutable state in library crates** outside the partition slots (feature
  `partitions`, `docs/design/adr-001-partitioned-runtimes.md`): state that belongs to a UI lives
  in its reactive runtime (one slot per `Partition`), its engine, or a per-partition table
  selected with `Partition::select` (like the seal records of `twine_core::alloc_guard`); a
  growth point notes its owner's partition. The only process-wide objects are the ones listed
  in ADR-001 § 1.1 (the waker pool, the interrupt probe, warn-once log flags) — a new one needs
  the ADR updated and `docs/safety/profile.md` § 13.2 (N6). Partitioning must stay zero-cost
  without the feature (const size asserts in both configurations).
- **Respect the crate layering.** A crate may only depend on crates in lower layers
  (`cargo xtask layers`). In particular, `twine-engine` never depends on `twine-reactive`.
- **Keep dependencies minimal.** Open an issue before adding a new external dependency. No_std
  crates must use `default-features = false`, and new dependencies must build for all embedded
  targets.

## Code guidelines

- **Idiomatic Rust**: edition 2024, `cargo fmt`, clippy pedantic (with the workspace allow-list).
  Derive `Debug`, `Clone`, `Copy`, `PartialEq`, `Eq`, `Hash` and `Default` where they make sense;
  use `#[must_use]` on functions whose result should not be ignored.
- **`unsafe`** only where there is no reasonable safe alternative. Every `unsafe` block needs a
  `// SAFETY:` comment explaining why it is sound, and tests that run under Miri.
- **Errors**: each crate has its own `Error` enum; fallible functions return `Result`.
- **Units**: pixels are `i32`, opacity is `Opa`, angles are `Angle` (0.1°), time is
  `Instant`/`Duration` (µs), scale factors are `Scale` (256 = 1.0). Rectangles are half-open.
- **Logging**: use the `twine_core` log macros with a `twine::<area>` target (e.g.
  `warn!(target: "twine::engine", "node {} not found", id)`) and only `{}` / `{:?}` placeholders
  so messages work with both `log` and `defmt`. Levels: `error!` for recovered invariant
  violations, `warn!` for invalid input, `info!` for lifecycle, `debug!` for per-frame summaries,
  `trace!` for per-event detail.
- **Modules** stay focused and reasonably small; unit tests live next to the code, integration
  tests in `tests/` of the crate whose behaviour they check.
- **Standard Cargo layout**: runnable examples are Cargo examples (`examples/<name>.rs`, or
  `examples/<name>/main.rs` with helper modules; helpers shared by several examples in
  `examples/common/`) and contain no tests; `src/bin/` is only for real binaries; crates used
  only by tests or examples (`twine-sim`, `twine-testing`, …) are dev-dependencies.

## Testing

- **Snapshot tests** (golden images) hash the rendered RGB pixels (XXH64) and compare with the
  crate's committed manifest `tests/snapshots.toml`; no image is needed to pass, so CI checks
  every snapshot. On a mismatch the actual image (and, with the local reference cache
  `tests/snapshots/*.png`, which is git-ignored and filled by passing runs, a diff image) is
  written to `target/twine-snapshots/<crate>/`; an image without an entry fails and is written to
  `target/twine-snapshots/<crate>/new/`. Only `cargo xtask snapshots --update | --propose |
  --accept` changes a manifest (`twine_testing::snapshot`).
- **Requirements**: a test that verifies a safety requirement says so in its doc comment
  (`/// req: SAF-012`, or `//! req: …` for a whole file); a fault-injection test also names the
  kind (``/// fault: `FlushError` ``). `cargo xtask traceability` writes the matrix
  `docs/safety/traceability.md` (CI checks it).
- **Fuzzing**: decoders and parsers have cargo-fuzz targets in `fuzz/`; the widget tree, input
  pipeline and reactive runtime have operation-sequence interpreters (`twine_testing::fuzz`)
  run by property tests on stable and by cargo-fuzz targets on nightly.
- **Harnesses** in `twine-testing`: `RenderHarness` (drawing primitives), `EngineHarness`
  (widget tree without reactivity) and `TestUi` (full declarative UI with scripted input and a
  mock clock). Use them only from integration tests (`tests/`), not from `#[cfg(test)]` modules.
- **Property tests** (`proptest`) for math, geometry, layout invariants and parsers; decoders and
  parsers must never panic on arbitrary input.
- **Allocation and idle checks**: use `CountingAllocator` to assert that rendering doesn't
  allocate, and assert that the UI is idle after an interaction finishes.
- **Widgets** need tests for defaults, every setter's idempotency, events, keypad/encoder
  navigation, and snapshots in each state (default, pressed, checked, disabled, focused) for the
  light and dark themes.

## Pull requests

- Keep PRs focused: one feature or fix per PR, with a description of what changed and why.
- Mention any change to public API, performance or memory use.
- For hardware-specific changes, say which board and display you tested on.
- By contributing you agree that your work is dual-licensed under MIT OR Apache-2.0, like the rest
  of the project.
