# Twine — agent instructions

Twine is a Rust GUI library for embedded devices (LVGL feature parity, declarative signal-based
API, `no_std` + `alloc`). Low compute/power is the top priority.

## Where things are
- `docs/plan/api-evolution.md` — the current plan: decisions (§1), platform-neutrality rules (§2),
  phases R0–R6 with steps `Rn.Smm`, definition of done (§4), progress (§5), and every deviation
  from the plan with its rationale (§6). Pick the first unchecked step of the current phase.
- `docs/safety/profile.md` — the `safety` profile: excluded parts, feature propagation, the
  safety lints with their per-crate status table, release profile, panic-handler contract and
  no-reachable-panics rules (§ 7), bounded memory (§ 8), bounded time and the measured stack per
  tree level (§ 10), display integrity (§ 11), partitioned runtimes (§ 13, feature
  `partitions`, `docs/design/adr-001-partitioned-runtimes.md`). `docs/safety/panics.md` — the generated panic inventory.
- `docs/safety/manual.md` — the safety manual (assumptions of use, configuration constraints,
  known limitations, evidence index); `docs/safety/requirements.md` — the SAF requirements,
  traced to tests in the generated `docs/safety/traceability.md`.
- `docs/design/principles.md` — the design principles P1–P11 cited as `(Pn)` in the code
  (reconstructed in R5.S08, accepted by the user).
- `docs/safety/coding-guidelines-deviations.md` — where the code deviates from the Safety-Critical
  Rust Consortium coding guidelines, with impact and the plan step that addresses each gap.
- `README.md`, `CONTRIBUTING.md` — overview, Definition of Done, hard rules, code guidelines.
- `twine::guide::custom_widgets` (rustdoc) — the custom-widget guide; `twine::guide::app_state` —
  the store pattern (application state as context).

## Workflow for every step
1. Read the step in `docs/plan/api-evolution.md`, the §6 deviations of earlier steps, and the code
   it builds on.
2. Implement the scope of the step with the architecturally sound design (no quick patches or
   shims). No stubs, `todo!()`, `unimplemented!()`, or untracked TODOs — only
   `// NOTE(Rn.Smm): …` pointing to a later step. Every added or changed API gets complete rustdoc
   with a doctest.
3. Write the tests the step lists (and any others needed). Keep behaviour observable through logs.
4. Per step, run quick checks only: `cargo fmt --all`, clippy / tests / `cargo doc` (with
   `-D warnings`) for the crates touched, `cargo xtask ci --only todo-check --only layers`. The full
   `cargo xtask ci` (no_std, firmware, sim-smoke, Miri), benchmarks and firmware-size comparisons
   run once at the end of each phase.
5. Tick the step in §5 and record deviations in §6 of `docs/plan/api-evolution.md`, then report.
   **Never run `git commit`, `git push`, tag or publish; the user handles all git operations.**

## Commands
- `cargo xtask ci` — everything CI runs (fmt, clippy, tests, no_std builds, docs, todo-check, layers).
  Clippy runs three ways: default features (`clippy`), every feature except `defmt`
  (`clippy-all-features`), and each library crate with `defmt` as its logging backend plus every
  `no_std` feature (`clippy-defmt`, one `cargo clippy -p` per crate). A fourth pass lints the facade
  in the safety profile (`clippy-safety`: panic/index/overflow lints denied, then the lint-status
  ratchet and the panic-inventory check); `safety-exclusions` checks that each excluded part
  fails to compile with `safety` and that the removed panicking APIs are absent
  (`tools/safety-api/`).
- `cargo xtask safety-lints [--update]` — count the remaining safety-lint sites per crate and
  compare (or rewrite) the table in `docs/safety/profile.md`; a lint with no site left must move
  from the crate's allow-list to `deny`.
- `cargo xtask panics [--check]` — write (or verify) the panic inventory `docs/safety/panics.md`;
  every explicit panic site needs a `// PANIC: …` justification above it, and every
  `// SAFE-INDEX: …` / `// SAFE-ARITH: …` proof (listed as proven sites) a non-empty text.
- `cargo xtask stack-budget [--update]` — measure (release build) the stack each recursive tree
  walk (draw, cover search, hit test, layout) costs per tree level and fail when the worst case
  at the reference depth exceeds the budget; `--update` rewrites the table in
  `docs/safety/profile.md` § 10.4 (also a slow CI stage).
- `cargo xtask progress` — plan progress per phase and the next step (from §5 of
  `docs/plan/api-evolution.md`); fails if §5 and the step headings disagree (also a CI stage).
- `cargo xtask sim <example>` — run a simulator example (Cargo examples of `twine`:
  `crates/twine/examples/<example>.rs` or `<example>/main.rs`; = `cargo run -p twine --example <example>`).
- `cargo xtask sim-smoke` — run every simulator example headless.
- `cargo xtask snapshots [--update | --propose | --accept | --check]` — golden images: run the
  snapshot tests against the hash manifests `crates/*/tests/snapshots.toml` and report; `--update`
  approves every rendered image (user-approved only, after reviewing the diffs), `--propose` adds
  new images as pending entries, `--accept` approves pending entries, `--check` (CI) fails on
  pending entries.
- `cargo xtask traceability [--check]` — write (or verify) `docs/safety/traceability.md` from
  `docs/safety/requirements.md` and the tests' `req:` / `fault:` doc lines.
- `cargo xtask shifts [--check]` — write (or verify) the shift-amount inventory
  `docs/safety/shifts.md`; every shift by a run-time amount in library code needs a
  `// SHIFT: …` proof next to it (named constants are exempt: rustc checks them) (R5.S09, guideline G5; CI stage `shifts`).
- `cargo xtask divisions [--check]` — write (or verify) the divisor inventory
  `docs/safety/divisions.md`; every integer division or remainder by a run-time divisor in
  library code (named constants are exempt: rustc checks them) must be non-zero by a `NonZero*` type, a `.max(1)`/`.clamp(1, ..)` guard or a
  `// DIV: …` proof next to it (R5.S10, guideline G3; CI stage `divisions`).
- `cargo xtask guidelines [--check]` — rewrite (or verify) the measured block (§ 1.1) of
  `docs/safety/coding-guidelines-deviations.md`: recursion cycles, divisions, shifts, macros
  (R5.S10; CI stage `guidelines`).
- `cargo xtask coverage [--mcdc]` — line coverage with thresholds; `--mcdc`: MC/DC of core,
  reactive, engine, render, layout, style (nightly + llvm-tools; falls back to line coverage).
- `cargo xtask fuzz <target>` — cargo-fuzz targets in `fuzz/` (decoders; `tree_ops`,
  `input_events`, `reactive_ops`).
- `cargo xtask nostd` — build no_std crates for all embedded targets (and the facade in the
  safety profile for each target), then `link-check`.
- `cargo xtask link-check` — link `tools/link-fixtures` and check their symbols against
  `fixtures.toml` (pay for what you use; `until = "R7.Sxx"` marks the known R7 baseline failures).
- `cargo xtask firmware [board]` — build firmware crates.
- `cargo xtask theme-size` — theme flash per part (styles, class chain, tables, icons, curves,
  fonts) in the rp2040/F411 demo images, attributed by symbol.
- `cargo xtask theme-recipes -- <cargo test args>` — run tests that call
  `twine_testing::assert_theme_covers` and print the minimal recipe list of their UIs (for a
  `DesignSystem`'s `RECIPES` or a built-in theme's `.recipes(&[..])`) and the widget classes
  without a recipe.
- `RUST_LOG=twine=debug` — enable logs in simulator/tests (`twine::refresh=trace` for invalidations).

## Hard rules
- No floating point in render/engine/layout/style/text hot paths (`clippy::float_arithmetic` denied there).
- No `Arc`, `alloc::sync`, or CAS atomics in no_std crates (RP2040 = thumbv6m). Use `Rc`,
  `portable-atomic`, `critical-section`.
- Widget setters are idempotent: unchanged value → no invalidation, no layout.
- No heap allocation during rendering/flush in steady state.
- Never panic on user input in release; log `warn!` and no-op.
- Safety-profile crates must not regress: a lint denied under `safety` stays denied; never
  `#[allow]` a safety lint on a single site (`docs/safety/profile.md` § 4).
- Respect crate layering (`cargo xtask layers`). Only dependencies from the allowed list.
- Golden-image manifests change only intentionally: `cargo xtask snapshots --update` after
  reviewing the diffs (with the user's approval); new images go in as pending
  (`--propose`) for the user to approve.
