//! `[package.metadata.twine.panel-color]` of this crate's `Cargo.toml` names the `twine` colour
//! feature every display feature needs; the facade's `drivers-<panel>` features enable it with
//! the panel (`cargo xtask layers` checks that side). This test checks the other side: the
//! table matches the format each driver actually reports, and lists every panel.

use twine_core::ColorFormat;
use twine_drivers::interface::I2cInterface;
use twine_drivers::sh1106::Sh1106;
use twine_drivers::ssd1306::{Ssd1306, Ssd1306Size};
use twine_drivers::testkit::Recorder;
use twine_drivers::{
    co5300, gc9a01, ili9341, ili9342, ili9488, jd9853, rm67162, sh8601, st7735, st7789, st7796,
};
use twine_hal::{DisplayDriver, Rotation};

/// The `twine` feature that compiles the renderer for `format`.
fn color_feature(format: ColorFormat) -> &'static str {
    match format {
        ColorFormat::Rgb565 => "color-rgb565",
        ColorFormat::Rgb565Swapped => "color-rgb565-swapped",
        ColorFormat::Rgb888 => "color-rgb888",
        ColorFormat::Xrgb8888 => "color-xrgb8888",
        ColorFormat::Argb8888 => "color-argb8888",
        ColorFormat::L8 => "color-l8",
        ColorFormat::I1 => "color-i1",
        other => panic!("no colour feature for {other:?}"),
    }
}

/// `(feature, format the driver reports)` for every display feature of the crate.
fn panels() -> Vec<(&'static str, ColorFormat)> {
    let rec = Recorder::new();
    let ssd1306 = Ssd1306::new(
        I2cInterface::new(rec.i2c(), 0x3C),
        Ssd1306Size::Size128x64,
        Rotation::Deg0,
    )
    .expect("mock bus")
    .info()
    .format;
    let sh1106 = Sh1106::new(I2cInterface::new(rec.i2c(), 0x3C), Rotation::Deg0)
        .expect("mock bus")
        .info()
        .format;
    vec![
        ("ili9341", ili9341::ILI9341.format()),
        ("ili9342", ili9342::ILI9342C.format()),
        ("ili9488", ili9488::ILI9488.format()),
        ("st7789", st7789::ST7789.format()),
        ("st7735", st7735::ST7735R_REDTAB.format()),
        ("st7796", st7796::ST7796.format()),
        ("gc9a01", gc9a01::GC9A01.format()),
        ("jd9853", jd9853::JD9853_172X320.format()),
        ("co5300", co5300::CO5300_410X502.format()),
        ("sh8601", sh8601::SH8601_368X448.format()),
        ("rm67162", rm67162::RM67162_240X536.format()),
        ("ssd1306", ssd1306),
        ("sh1106", sh1106),
    ]
}

/// The `panel = "color-…"` lines of the metadata table.
fn metadata_table() -> Vec<(String, String)> {
    let manifest = include_str!("../Cargo.toml");
    let start = manifest
        .find("\n[package.metadata.twine.panel-color]\n")
        .expect("Cargo.toml has a [package.metadata.twine.panel-color] table")
        + 1;
    manifest[start..]
        .lines()
        .skip(1)
        .take_while(|l| !l.trim_start().starts_with('['))
        .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        .map(|l| {
            let (k, v) = l.split_once('=').expect("`panel = \"color-…\"`");
            (k.trim().to_string(), v.trim().trim_matches('"').to_string())
        })
        .collect()
}

#[test]
fn panel_color_metadata_matches_the_drivers() {
    let table = metadata_table();
    let panels = panels();
    for (feature, format) in &panels {
        let entry = table.iter().find(|(k, _)| k == feature);
        assert_eq!(
            entry.map(|(_, v)| v.as_str()),
            Some(color_feature(*format)),
            "`{feature}` reports {format:?}: its panel-color entry must be `{}`",
            color_feature(*format)
        );
    }
    assert_eq!(
        table.len(),
        panels.len(),
        "every panel-color entry is a display checked here: {table:?}"
    );
}

#[test]
fn every_display_feature_has_a_panel_color_entry() {
    // The features of the "Displays" group (`#! ### Displays` up to the next group).
    let manifest = include_str!("../Cargo.toml");
    let start = manifest
        .find("#! ### Displays")
        .expect("a Displays feature group");
    let group = &manifest[start..];
    let end = group[1..].find("#! ###").map_or(group.len(), |i| i + 1);
    let features: Vec<&str> = group[..end]
        .lines()
        .filter(|l| !l.starts_with('#') && l.contains(" = ["))
        .filter_map(|l| l.split_once(" = ").map(|(k, _)| k.trim()))
        .collect();
    let table = metadata_table();
    assert!(!features.is_empty());
    for f in &features {
        assert!(
            table.iter().any(|(k, _)| k == f),
            "display feature `{f}` needs a [package.metadata.twine.panel-color] entry"
        );
    }
    assert_eq!(features.len(), table.len(), "{features:?} vs {table:?}");
}
