# Twine — agent instructions

Twine is a Rust GUI library for embedded devices (LVGL feature parity, declarative signal-based
API, `no_std` + `alloc`). Low compute/power is the top priority.

## Where things are
- `docs/plan/api-evolution.md` — the current plan: decisions (§1), platform-neutrality rules (§2),
  phases R0–R6 with steps `Rn.Smm`, definition of done (§4), progress (§5), and every deviation
  from the plan with its rationale (§6). Pick the first unchecked step of the current phase.
- `docs/safety/coding-guidelines-deviations.md` — where the code deviates from the Safety-Critical
  Rust Consortium coding guidelines, with impact and the plan step that addresses each gap.
- `README.md`, `CONTRIBUTING.md` — overview, Definition of Done, hard rules, code guidelines.
- `twine::guide::custom_widgets` (rustdoc) — the custom-widget guide.

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
  `no_std` feature (`clippy-defmt`, one `cargo clippy -p` per crate).
- `cargo xtask progress` — plan progress per phase and the next step (from §5 of
  `docs/plan/api-evolution.md`); fails if §5 and the step headings disagree (also a CI stage).
- `cargo xtask sim <example>` — run a simulator example (Cargo examples of `twine`:
  `crates/twine/examples/<example>.rs` or `<example>/main.rs`; = `cargo run -p twine --example <example>`).
- `cargo xtask sim-smoke` — run every simulator example headless.
- `cargo xtask snapshots [--update]` — run/refresh snapshot tests (review diffs before updating).
- `cargo xtask nostd` — build no_std crates for all embedded targets.
- `cargo xtask firmware [board]` — build firmware crates.
- `RUST_LOG=twine=debug` — enable logs in simulator/tests (`twine::refresh=trace` for invalidations).

## Hard rules
- No floating point in render/engine/layout/style/text hot paths (`clippy::float_arithmetic` denied there).
- No `Arc`, `alloc::sync`, or CAS atomics in no_std crates (RP2040 = thumbv6m). Use `Rc`,
  `portable-atomic`, `critical-section`.
- Widget setters are idempotent: unchanged value → no invalidation, no layout.
- No heap allocation during rendering/flush in steady state.
- Never panic on user input in release; log `warn!` and no-op.
- Respect crate layering (`cargo xtask layers`). Only dependencies from the allowed list.
- Snapshot PNGs are regenerated only intentionally (`cargo xtask snapshots --update`) after reviewing diffs.
