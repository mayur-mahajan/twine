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

*API preview — the declarative layer is still being implemented.*

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
    .align_items(FlexAlign::Center)
    .size(Length::Pct(100), Length::Pct(100))
}

fn main() {
    twine_sim::run(SimConfig::new(320, 240).title("Counter").scale(2), counter);
}
```

## Crate map

Crates are strictly layered: a crate only depends on crates below it (`cargo xtask layers`
enforces this).

| Crate | Role |
|-------|------|
| `twine-core` | geometry, fixed-point math, colors & pixel formats, time, arenas, `RectSet`, logging |
| `twine-hal` | display / input / clock traits |
| `twine-reactive` | signals, memos, effects, scopes |
| `twine-anim` | animations, easing, timelines, timers |
| `twine-render` | integer software renderer |
| `twine-text`, `twine-image`, `twine-vector`, `twine-fs` | fonts & text, images & decoders, vector/SVG, file systems |
| `twine-style`, `twine-layout` | style system, flex & grid layout |
| `twine-engine` | widget tree, events, input, scrolling, invalidation, refresh pipeline |
| `twine-theme`, `twine-widgets`, `twine-widgets-ext` | themes, basic and complex widgets |
| `twine-view` | declarative view layer and `Ui` runtime |
| `twine` | facade: re-exports and `prelude` |
| `twine-drivers`, `twine-embassy`, `twine-accel-stm32`, `twine-embedded-graphics` | drivers, async run loop, DMA2D, embedded-graphics adapter |
| `twine-sim`, `twine-testing`, `twine-demos` | desktop simulator, test harnesses, demo apps |

## Supported targets

All boards use a 2.8" ILI9341 320×240 SPI TFT with XPT2046 touch, except the STM32F429 Discovery,
which uses its on-board display.

| Firmware crate | Board | Target | Toolchain |
|----------------|-------|--------|-----------|
| `rp2040-ili9341` | Raspberry Pi Pico | `thumbv6m-none-eabi` | stable |
| `rp2350-ili9341` | Raspberry Pi Pico 2 | `thumbv8m.main-none-eabihf` | stable |
| `stm32f411-ili9341` | WeAct BlackPill F411CE | `thumbv7em-none-eabihf` | stable |
| `stm32f429i-disco` | STM32F429I-DISC1 (LTDC + DMA2D) | `thumbv7em-none-eabihf` | stable |
| `esp32c3-ili9341` | ESP32-C3-DevKitM-1 | `riscv32imc-unknown-none-elf` | stable |
| `esp32s3-ili9341` | ESP32-S3-DevKitC-1 (N8R8) | `xtensa-esp32s3-none-elf` | `esp` (espup) |

## Building

Prerequisites:

- [`rustup`](https://rustup.rs) — `rust-toolchain.toml` installs stable plus the embedded targets.
- Optional: [`cargo-llvm-cov`](https://github.com/taiki-e/cargo-llvm-cov)
  (`cargo install cargo-llvm-cov`) for `cargo xtask coverage`; a nightly toolchain with Miri
  (`rustup toolchain install nightly --component miri`) for `cargo xtask miri`;
  [`probe-rs`](https://probe.rs) to flash ARM/RP boards; [`espup`](https://github.com/esp-rs/espup)
  for the ESP32-S3 toolchain.

Commands:

| Command | What it does |
|---------|--------------|
| `cargo xtask ci` | everything CI runs (fmt, clippy, tests, no_std builds, docs, Miri, todo-check, layers); `--quick` skips the slow stages, `--only <stage>` runs single stages |
| `cargo xtask sim <example>` | run a simulator example (`examples/src/bin/<example>.rs`) |
| `cargo xtask snapshots [--update]` | run / refresh snapshot tests (review diffs before updating) |
| `cargo xtask nostd` | build the `no_std` crates for all embedded targets |
| `cargo xtask firmware [board]` | build firmware crates |
| `cargo xtask coverage` | per-crate line coverage with thresholds |
| `cargo xtask miri [crate…]` | run tests under Miri (nightly) to detect undefined behaviour |

Logging: `RUST_LOG=twine=debug` enables logs in the simulator and tests
(`twine::refresh=trace` shows invalidations).

## Simulator

`cargo xtask sim <example>` opens a desktop window (winit + softbuffer, no SDL needed) that
emulates the panel's pixel format, bus speed and input devices. Debug hotkeys (refresh areas,
layout bounds, perf monitor, slow motion, screenshots) are listed with `F1`.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.
