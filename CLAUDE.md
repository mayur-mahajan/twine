# Twine — agent instructions

Twine is a Rust GUI library for embedded devices (LVGL feature parity, declarative signal-based
API, `no_std` + `alloc`). Low compute/power is the top priority.

## Where things are
- `docs/design/` — **normative** design. Start with `00-overview.md` (principles P1–P11) and
  `01-architecture.md` (crates, layering, naming registry, allowed dependencies).
- `docs/plan/README.md` — execution rules + phase index. `docs/plan/phase-XX-*.md` — steps.
- `docs/plan/PROGRESS.md` — what is done. Pick the first unchecked step.
- `docs/plan/DEVIATIONS.md` — every departure from the docs, with rationale.

## Workflow for every step
1. Read the step, the design sections it references, and any code it builds on.
2. Implement exactly the scope of the step. No stubs, `todo!()`, `unimplemented!()`, or
   un-tracked TODOs (only `// NOTE(Pxx.Syy): …` pointing to a future step).
3. Write the tests the step lists (and any others needed). Keep behaviour observable through logs.
4. Run `cargo xtask ci` (after P00.S02 exists; before that `cargo fmt --check && cargo clippy
   --workspace -- -D warnings && cargo test --workspace`). Run the step's demo.
5. Update `docs/plan/PROGRESS.md`, then stop and report to the user. **Never run `git commit`,
   `git push`, tag or publish; the user handles all git operations.**

## Commands
- `cargo xtask ci` — everything CI runs (fmt, clippy, tests, no_std builds, docs, todo-check, layers).
- `cargo xtask sim <example>` — run a simulator example (`examples/src/bin/<example>.rs`).
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
