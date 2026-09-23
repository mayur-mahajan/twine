# Contributing to Twine

Twine is built step by step from [`docs/plan/`](docs/plan/README.md). Coding agents and humans
follow the same rules; agents additionally read [`CLAUDE.md`](CLAUDE.md).

## Workflow

1. Pick the first unchecked step in [`docs/plan/PROGRESS.md`](docs/plan/PROGRESS.md).
2. Read the step, every design section it references (`docs/design/`, normative for names and
   signatures) and the code it builds on.
3. Implement exactly the scope of the step. If the design and the step conflict, or a step is
   impossible as written, make the smallest change that keeps the design intent, record it in
   [`docs/plan/DEVIATIONS.md`](docs/plan/DEVIATIONS.md) and update the design doc in the same step.

## Definition of Done (every step)

- `cargo xtask ci` passes (fmt, clippy `-D warnings`, tests, no_std builds, docs, todo-check, layers).
- New public items are documented; new behaviour is covered by the tests listed in the step
  (more are welcome).
- The step's **Demo** runs (simulator example or test command) and shows what the step describes.
- `docs/plan/PROGRESS.md`: tick the step, add the date and one line of notes (perf numbers if
  measured), e.g. `- [x] P00.S02 — … — 2026-09-23 — note`.
- No stubs: no `todo!()`, `unimplemented!()`, placeholder returns or ignored tests. Work a later
  step will extend gets a `// NOTE(Pxx.Syy): <what comes later>` comment — the only allowed marker.
- **Do not commit.** Agents never run git commands that change history (commit, push, tag,
  rebase…) and never publish. The user reviews each step and commits it.

## Hard rules

- No floating point in render/engine/layout/style/text hot paths.
- No `Arc`, `alloc::sync` or CAS atomics in `no_std` crates; use `Rc`, `portable-atomic`,
  `critical-section`.
- Widget setters are idempotent; no heap allocation while rendering in steady state.
- Never panic on user input in release: log `warn!` and no-op.
- Respect crate layering (`cargo xtask layers`) and the allowed dependency list
  (`docs/design/01-architecture.md` §4).
- Snapshot PNGs are regenerated only intentionally (`cargo xtask snapshots --update`) after
  reviewing the diff images.
