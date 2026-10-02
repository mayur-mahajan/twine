# Twine

Twine is a Rust GUI library for resource-constrained embedded devices — MCUs such as STM32,
RP2040/RP2350 and ESP32 — that also runs on microprocessors and desktop hosts. It targets
**feature parity with LVGL v9** while exposing a **declarative, SwiftUI/Xilem-like API** built on
**fine-grained reactive signals**. Low compute and low power come first: work done is proportional
to what changed, and an idle UI executes no code at all.

> **Status: pre-alpha** — the library is under active development.

## Highlights

- **Signals, not diffing.** Your app function runs once; every dynamic value is a signal-bound
  property, so a change updates exactly the affected widget properties.
- **Partial rendering + DMA pipelining.** Only dirty areas are redrawn, in the display's native
  pixel format; with two buffers the CPU renders the next chunk while DMA sends the previous one.
- **`no_std` + `alloc` everywhere** (except the simulator and tools), including `thumbv6m`
  (RP2040, no atomic CAS). Integer-only, deterministic renderer: identical pixels on every platform.
- **LVGL parity**: widgets, styles, themes, flex/grid layout, animations, scrolling, input devices,
  fonts, images, vector graphics.

## Example

The `counter` example of the `twine` crate (run it with `cargo xtask sim counter`; the app
function is `twine_demos::counter::app`, compiled and tested with the workspace). The
declarative layer runs on the simulator and in the board examples under `firmware/`; the API
may still change before the first release.

```rust
use twine::prelude::*;

pub fn counter(cx: Scope) -> impl View {
    let count = cx.signal(0u32);

    column((
        label(text!("Clicked {} times", count.get()))
            .font(&fonts::MONTSERRAT_20)
            .test_id("count"),
        button(label("Click me"))
            .on_click(move || count.update(|c| *c += 1)),
        button(label("Reset"))
            .disabled(move || count.get() == 0)
            .on_click(move || count.set(0)),
    ))
    .gap(12)
    .padding(16)
    .align_items(CrossAlign::Center)
    .fill()
}

fn main() {
    twine_sim::run(SimConfig::new(320, 240).title("Counter").scale(2).theme(DefaultTheme::light()), counter);
}
```

## Guides

- **Writing a custom widget** (`twine::guide::custom_widgets` in the `twine` crate docs,
  [source](crates/twine/src/guide/custom_widgets.md)): a gauge with its own class and parts,
  drawing, keypad/encoder input, theming with design elements in every theme mode, a typed
  view, animations within the motion preference, and tests. The finished widget is the
  `gauge` example: `cargo xtask sim gauge`.

## Crate map

Crates are strictly layered: a crate only depends on crates below it (`cargo xtask layers`
enforces this).

| Crate | Role |
|-------|------|
| `twine-core` | geometry, fixed-point math, colors & pixel formats, time, arenas, `RectSet`, logging |
| `twine-hal` | display / input / clock / platform traits (opt-in `platform-*` implementations) |
| `twine-reactive` | signals, memos, effects, scopes |
| `twine-anim` | animations, easing, timelines, timers |
| `twine-render` | integer software renderer |
| `twine-text`, `twine-image`, `twine-vector`, `twine-fs` | fonts & text, images & decoders, vector/SVG, file systems |
| `twine-style`, `twine-layout` | style system, flex & grid layout |
| `twine-engine` | widget tree, events, input, scrolling, invalidation, refresh pipeline |
| `twine-theme`, `twine-widgets`, `twine-widgets-ext` | themes, basic and complex widgets |
| `twine-view` | declarative view layer, `Ui` runtime and the blocking run loop (`twine::run`) |
| `twine` | facade: re-exports and `prelude` |
| `twine-drivers`, `twine-embassy`, `twine-accel-stm32`, `twine-embedded-graphics` | drivers, embassy run loop (`Ui` and `AsyncUi`), DMA2D, embedded-graphics adapter |
| `twine-sim`, `twine-testing`, `twine-demos` | desktop simulator, test harnesses, demo apps |

## Supported targets

Twine ships drivers, not board support: every display, touch and input driver and every bus
interface in `twine-drivers` is a cargo feature that you enable and wire to your own board's
pins. Through the facade each one is `twine/drivers-<name>` (`drivers-spi`, `drivers-xpt2046`,
`drivers-encoder`, …); a panel's feature (`drivers-ili9341`, …) also enables the panel's pixel
format. The
`firmware/` directory holds one small example per chip family — a template with a single
"Wiring: edit for your board" block — that proves the stack builds and fits on that chip:

| Example | Chip | Target | Toolchain |
|---------|------|--------|-----------|
| `firmware/rp2040` | RP2040 | `thumbv6m-none-eabi` | stable |
| `firmware/rp2350` | RP2350 | `thumbv8m.main-none-eabihf` | stable |
| `firmware/stm32f411` | STM32F411 | `thumbv7em-none-eabihf` | stable |
| `firmware/esp32c3` | ESP32-C3 | `riscv32imc-unknown-none-elf` | stable |
| `firmware/esp32c6` | ESP32-C6 | `riscv32imac-unknown-none-elf` | stable |
| `firmware/esp32s3` | ESP32-S3 (SPI panels and QSPI AMOLEDs) | `xtensa-esp32s3-none-elf` | `esp` (espup) |
| `firmware/esp32` | ESP32 | `xtensa-esp32-none-elf` | `esp` (espup) |

`firmware/README.md` lists the pins and features for known boards (the common 2.8" ILI9341 +
XPT2046 module, the ESP32 "Cheap Yellow Display", Waveshare and LilyGO ESP32-S3 AMOLED boards,
Waveshare's ESP32-C6 1.47" LCD)
and the hardware checklist.

## Building

Prerequisites:

- [`rustup`](https://rustup.rs) — `rust-toolchain.toml` installs stable plus the embedded targets.
- Optional: [`cargo-llvm-cov`](https://github.com/taiki-e/cargo-llvm-cov)
  (`cargo install cargo-llvm-cov`) for `cargo xtask coverage`; a nightly toolchain with Miri
  (`rustup toolchain install nightly --component miri`) for `cargo xtask miri`;
  [`probe-rs`](https://probe.rs) to flash ARM/RP boards; [`espup`](https://github.com/esp-rs/espup)
  for the Xtensa (ESP32, ESP32-S3) toolchain.

Commands:

| Command | What it does |
|---------|--------------|
| `cargo xtask ci` | everything CI runs (fmt, clippy, tests, no_std builds, docs, Miri, todo-check, layers); `--quick` skips the slow stages, `--only <stage>` runs single stages |
| `cargo xtask sim <example>` | run a simulator example (a Cargo example of `twine` in `crates/twine/examples/`; same as `cargo run -p twine --example <example>`) |
| `cargo xtask sim-smoke` | run every simulator example headless |
| `cargo xtask snapshots [--update]` | run / refresh snapshot tests (review diffs before updating) |
| `cargo xtask nostd` | build the `no_std` crates for all embedded targets |
| `cargo xtask firmware [example] [--strict]` | build the example firmware and print flash/RAM sizes |
| `cargo xtask coverage` | per-crate line coverage with thresholds |
| `cargo xtask miri [crate…]` | run tests under Miri (nightly) to detect undefined behaviour |

Logging: `RUST_LOG=twine=debug` enables logs in the simulator and tests
(`twine::refresh=trace` shows invalidations).

## Simulator

`cargo xtask sim <example>` opens a desktop window (winit + softbuffer, no SDL needed) that
emulates the panel's pixel format, bus speed and input devices. Debug hotkeys (refresh areas,
layout bounds, perf monitor, slow motion, screenshots) are listed with `F1`.

`twine-sim` is the primary simulator: it also runs headless with scripts (CI smoke runs and
snapshot tests), controls time (pause, slow motion, single step) and emulates bus bandwidth.
For projects already built on [embedded-graphics], `cargo xtask sim eg_simulator` runs the
counter demo inside `embedded-graphics-simulator` through the `twine-embedded-graphics`
adapter (an example of `twine-embedded-graphics`). It needs SDL2 (`brew install sdl2` on macOS,
`apt install libsdl2-dev` on Linux) and the `twine-embedded-graphics` feature `eg-sim`, which the
xtask enables. `cargo xtask ci` skips it with a warning when SDL2 is missing.

[embedded-graphics]: https://docs.rs/embedded-graphics

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.
