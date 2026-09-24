# Contributing to Twine

Thanks for helping build Twine! Twine runs on microcontrollers with a few hundred kilobytes of RAM
and on battery power, so the rules below focus on keeping it small, fast, deterministic and
well tested. Please read them before opening a pull request.

## Getting set up

- **Rust**: the stable toolchain and the embedded targets are pinned in `rust-toolchain.toml`;
  `rustup` installs them automatically the first time you build.
- **Optional tools** (the checks that need them are skipped with a warning if missing):
  - nightly toolchain with Miri: `rustup toolchain install nightly --component miri`
  - coverage: `cargo install cargo-llvm-cov`
  - ESP32-S3 firmware builds: [`espup`](https://github.com/esp-rs/espup)
- **Simulator**: `cargo xtask sim <example>` opens a desktop window (pure Rust, no SDL needed).
  Set `TWINE_SIM_HEADLESS=1` to run without a window and write PNG screenshots instead.
- **Logs**: `RUST_LOG=twine=debug` (or e.g. `twine::refresh=trace`) in the simulator and tests.

## Before you open a pull request

Every PR must meet this **Definition of Done**:

1. **`cargo xtask ci` passes.** It runs formatting, clippy with `-D warnings`, all tests,
   `no_std` builds for every embedded target, rustdoc, the layering check, the work-marker check,
   the generated-fonts, generated-images and style-property-table checks, a benchmark build, snapshot tests and Miri. Use
   `cargo xtask ci --quick` while iterating.
2. **Tests cover the change.** New behaviour has tests; bug fixes add a regression test named
   `regression_<short_description>` that fails without the fix.
3. **Public API is documented.** Every public item has rustdoc (the build denies missing docs).
   Explain *what* and *why*, document panics and errors, and add a short example (doctest) for
   types and functions a user will call directly.
4. **Visual changes are reviewed.** If rendering output changes on purpose, regenerate snapshots
   with `cargo xtask snapshots --update`, inspect the diff images, and attach before/after
   screenshots to the PR. Never edit snapshot PNGs by hand.
5. **Performance is considered.** For changes in rendering, layout, styles, input or the
   reactive runtime, run `cargo xtask bench` (renderer, text) and `cargo bench -p twine-bench`
   (engine refresh, layout and animation frames) and mention the impact in the PR. Regressions
   need a justification.
6. **Generated files are regenerated, not edited.** Built-in fonts come from
   `assets/fonts/fonts.toml` via `cargo xtask fonts`; `cargo xtask fonts --check` fails when a
   generated file is out of date. Sample images in `assets/images/` are drawn by
   `cargo xtask gen-assets` and converted from `assets/images/images.toml` by
   `cargo xtask images` (both take `--check`). The style property table
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
- **`no_std` + `alloc` compatible.** Library crates must build for `thumbv6m-none-eabi`
  (RP2040), which has no atomic compare-and-swap: don't use `Arc`, `alloc::sync` or CAS atomics;
  use `Rc`, `portable-atomic` and `critical-section`.
- **Never panic on user input.** Invalid ids or parameters are logged with `warn!` and ignored.
  Use `debug_assert!` for internal invariants.
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
  tests in `tests/`.

## Testing

- **Snapshot tests** compare rendered output pixel-by-pixel against reference PNGs in
  `tests/snapshots/`. The references are local and git-ignored: the first run creates them,
  later runs compare against them. On mismatch the actual and diff images are written to
  `target/twine-snapshots/`. In CI (`CI` set) there are no references, so snapshot comparisons are
  skipped. Run the snapshot tests locally before and after your change to catch visual
  regressions.
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
