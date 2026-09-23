//! `twine font`: a deterministic TTF → Rust bitmap-font generator.
//!
//! The generator rasterizes the requested code points with `fontdue`, quantizes them to the
//! requested bits per pixel, optionally merges Font Awesome symbols, reads kerning from the
//! font's `GPOS`/`kern` tables, optionally compresses bitmaps with LVGL's RLE scheme and writes
//! a Rust file with a `pub static <NAME>: twine_text::Font`. The output only depends on the
//! inputs, so regenerated files diff cleanly.
//!
//! Metrics:
//! - `line_height = ceil(ascent) + ceil(-descent)`, `base_line = ceil(-descent)` (hhea metrics
//!   from fontdue);
//! - underline position/thickness from the `post` table (fallback `-(descent / 2)` and
//!   `max(1, size / 14)`);
//! - advances in 1/16 px (`round(advance × 16)`), boxes trimmed to their inked area;
//! - coverage quantized with `q = (v × max + 127) / 255`.

#![allow(clippy::cast_precision_loss)] // font sizes and advances are small

mod emit;
mod gpos;
mod ranges;
mod raster;
mod sha256;

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use twine_text::encode::{CmapData, FontData, KernData, encode_glyph};
use twine_text::{BitmapFormat, CmapKind, GlyphDsc, Subpx};

pub use emit::render_rust;
pub use ranges::{merge_ranges, parse_range};
pub use raster::{Cov, quantize, raster, subpixel};
pub use sha256::sha256_hex;

/// Result type of the generator (errors are human-readable messages).
pub type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// Default Font Awesome files for `--symbols fa` (searched in order), relative to the working
/// directory: the solid set plus the brands set (USB and Bluetooth are brand icons).
pub const DEFAULT_SYMBOLS_TTF: &[&str] = &["assets/fonts/fa-solid-900.ttf", "assets/fonts/fa-brands-400.ttf"];

/// Symbols that Font Awesome Free 5 does not contain, drawn from another glyph turned 90°
/// clockwise: `NEW_LINE` (U+F8A2, a custom glyph in LVGL's merged font) uses "level-down-alt"
/// (U+F3BE), which turned clockwise is a "↵" return arrow.
pub const SYMBOL_ALIASES: &[(char, char)] = &[('\u{F8A2}', '\u{F3BE}')];

/// Which symbol set to merge.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum Symbols {
    /// No symbols.
    #[default]
    None,
    /// The Font Awesome subset of `twine_text::symbols::ALL`.
    Fa,
}

/// Subpixel mode argument.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum SubpxArg {
    /// Grayscale.
    #[default]
    None,
    /// Horizontal RGB subpixels.
    Hor,
    /// Vertical RGB subpixels.
    Ver,
}

impl From<SubpxArg> for Subpx {
    fn from(s: SubpxArg) -> Self {
        match s {
            SubpxArg::None => Subpx::None,
            SubpxArg::Hor => Subpx::Hor,
            SubpxArg::Ver => Subpx::Ver,
        }
    }
}

fn parse_bpp(s: &str) -> std::result::Result<u8, String> {
    match s {
        "1" | "2" | "4" | "8" => Ok(s.parse().unwrap_or(4)),
        _ => Err(format!("bpp must be 1, 2, 4 or 8 (got `{s}`)")),
    }
}

/// Command-line arguments of `twine font`.
#[derive(Clone, Debug, clap::Args)]
#[command(about = "Generate a Rust bitmap font from a TTF file")]
pub struct FontArgs {
    /// Source TrueType font.
    #[arg(long, required_unless_present = "dump")]
    pub ttf: Option<PathBuf>,
    /// Pixel size (em height).
    #[arg(long)]
    pub size: u16,
    /// Bits per pixel: 1, 2, 4 or 8.
    #[arg(long, default_value = "4", value_parser = parse_bpp)]
    pub bpp: u8,
    /// Code points: `0x20-0x7F`, `0xB0`, `32-126` or `chars:°•` (repeatable).
    #[arg(long = "range")]
    pub ranges: Vec<String>,
    /// Merge the built-in symbol set.
    #[arg(long, value_enum, default_value_t = Symbols::None)]
    pub symbols: Symbols,
    /// Symbol font files, searched in order (repeatable; default
    /// `assets/fonts/fa-solid-900.ttf` and `assets/fonts/fa-brands-400.ttf`).
    #[arg(long)]
    pub symbols_ttf: Vec<PathBuf>,
    /// RLE-compress the glyph bitmaps.
    #[arg(long)]
    pub compress: bool,
    /// With `--compress`: skip the XOR row prefilter.
    #[arg(long)]
    pub no_prefilter: bool,
    /// Do not emit kerning.
    #[arg(long)]
    pub no_kerning: bool,
    /// Subpixel rendering.
    #[arg(long, value_enum, default_value_t = SubpxArg::None)]
    pub subpx: SubpxArg,
    /// Name of the generated static (e.g. `MONTSERRAT_14`).
    #[arg(long, required_unless_present = "dump")]
    pub name: Option<String>,
    /// Output `.rs` file.
    #[arg(long, required_unless_present = "dump")]
    pub out: Option<PathBuf>,
    /// Print glyph metrics of this TTF instead of generating (debug aid).
    #[arg(long, conflicts_with_all = ["ttf", "out", "name"])]
    pub dump: Option<PathBuf>,
    /// With `--dump`: the characters to print.
    #[arg(long, default_value = "Ag")]
    pub chars: String,
}

/// Everything that determines a generated font.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontOptions {
    /// Source TTF (as written in the header; read relative to the generator's root directory).
    pub ttf: PathBuf,
    /// Pixel size.
    pub size: u16,
    /// Bits per pixel.
    pub bpp: u8,
    /// Range specs (as given).
    pub ranges: Vec<String>,
    /// Symbol set.
    pub symbols: Symbols,
    /// Symbol fonts searched in order (empty = [`DEFAULT_SYMBOLS_TTF`]).
    pub symbols_ttf: Vec<PathBuf>,
    /// RLE compression.
    pub compress: bool,
    /// XOR prefilter (with compression).
    pub prefilter: bool,
    /// Kerning.
    pub kerning: bool,
    /// Subpixel mode.
    pub subpx: SubpxArg,
    /// Static name.
    pub name: String,
    /// Output path (as written in the header).
    pub out: PathBuf,
}

impl FontOptions {
    /// Options from parsed CLI arguments (not for `--dump`).
    pub fn from_args(a: &FontArgs) -> Result<Self> {
        Ok(Self {
            ttf: a.ttf.clone().ok_or("--ttf is required")?,
            size: a.size,
            bpp: a.bpp,
            ranges: a.ranges.clone(),
            symbols: a.symbols,
            symbols_ttf: a.symbols_ttf.clone(),
            compress: a.compress,
            prefilter: !a.no_prefilter,
            kerning: !a.no_kerning,
            subpx: a.subpx,
            name: a.name.clone().ok_or("--name is required")?,
            out: a.out.clone().ok_or("--out is required")?,
        })
    }

    /// The canonical `twine font …` command line that reproduces these options.
    #[must_use]
    pub fn command_line(&self) -> String {
        let mut s = format!(
            "twine font --ttf {} --size {} --bpp {}",
            self.ttf.display(),
            self.size,
            self.bpp
        );
        for r in &self.ranges {
            if r.contains(' ') || r.starts_with("chars:") {
                let _ = write!(s, " --range \"{r}\"");
            } else {
                let _ = write!(s, " --range {r}");
            }
        }
        if self.symbols == Symbols::Fa {
            s.push_str(" --symbols fa");
        }
        for p in &self.symbols_ttf {
            let _ = write!(s, " --symbols-ttf {}", p.display());
        }
        if self.compress {
            s.push_str(" --compress");
            if !self.prefilter {
                s.push_str(" --no-prefilter");
            }
        }
        if !self.kerning {
            s.push_str(" --no-kerning");
        }
        match self.subpx {
            SubpxArg::None => {}
            SubpxArg::Hor => s.push_str(" --subpx hor"),
            SubpxArg::Ver => s.push_str(" --subpx ver"),
        }
        let _ = write!(s, " --name {} --out {}", self.name, self.out.display());
        s
    }

    fn format(&self) -> BitmapFormat {
        match (self.compress, self.prefilter) {
            (false, _) => BitmapFormat::Plain,
            (true, true) => BitmapFormat::Compressed,
            (true, false) => BitmapFormat::CompressedNoPrefilter,
        }
    }
}

/// A generated font: the in-memory description and its Rust source.
#[derive(Clone, Debug)]
pub struct Generated {
    /// The font data (load it with [`FontData::leak`]).
    pub data: FontData,
    /// Code point of every glyph id (index 0 = reserved, `'\0'`).
    pub code_points: Vec<char>,
    /// The Rust source file.
    pub source: String,
    /// Bytes of static data (bitmaps, descriptors, cmaps, kerning).
    pub flash_bytes: usize,
    /// Non-fatal problems (missing glyphs, clamped kerning).
    pub warnings: Vec<String>,
}

/// One rasterized glyph before encoding.
struct Glyph {
    cp: char,
    adv16: u16,
    cov: Cov,
}

fn load_font(path: &Path, size: u16) -> Result<(Vec<u8>, fontdue::Font)> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let font = fontdue::Font::from_bytes(
        bytes.as_slice(),
        fontdue::FontSettings {
            scale: f32::from(size),
            ..fontdue::FontSettings::default()
        },
    )
    .map_err(|e| format!("cannot parse {}: {e}", path.display()))?;
    Ok((bytes, font))
}

/// Rasterizes `cp` (turned 90° clockwise when `turn`) and returns its coverage and advance.
fn rasterize(font: &fontdue::Font, cp: char, size: u16, subpx: Subpx, turn: bool) -> (Cov, u16) {
    let px = f32::from(size);
    let raw = |px: f32| {
        let (c, a) = raster::raster(font, cp, px);
        if turn { (c.turn_cw(), px) } else { (c, a) }
    };
    let (cov, adv) = match subpx {
        Subpx::None => {
            let (c, a) = raw(px);
            (c.trim(1, 1), a)
        }
        Subpx::Hor | Subpx::Ver => {
            let (c3, a3) = raw(px * 3.0);
            (subpixel(&c3, subpx), a3 / 3.0)
        }
    };
    let adv16 = (adv * 16.0).round().clamp(0.0, f32::from(u16::MAX)) as u16;
    (cov, adv16)
}

/// Generates the font described by `opts`, reading input files relative to `root`.
pub fn generate(opts: &FontOptions, root: &Path) -> Result<Generated> {
    if !matches!(opts.bpp, 1 | 2 | 4 | 8) {
        return Err(format!("bpp must be 1, 2, 4 or 8 (got {})", opts.bpp).into());
    }
    if opts.size == 0 || opts.size > 255 {
        return Err(format!("size must be 1..=255 px (got {})", opts.size).into());
    }
    if !is_ident(&opts.name) {
        return Err(format!("--name `{}` is not a Rust identifier", opts.name).into());
    }
    let subpx: Subpx = opts.subpx.into();
    let mut warnings = Vec::new();
    let (main_bytes, main) = load_font(&root.join(&opts.ttf), opts.size)?;
    let cps = merge_ranges(&opts.ranges)?;
    if cps.is_empty() && opts.symbols == Symbols::None {
        return Err("no code points: pass at least one --range".into());
    }

    // Line metrics.
    let lm = main
        .horizontal_line_metrics(f32::from(opts.size))
        .ok_or("the font has no horizontal line metrics")?;
    let ascent = lm.ascent.ceil() as i32;
    let descent = (-lm.descent).ceil() as i32;
    let line_height = ascent + descent;
    let kerning_src = gpos::Kerning::new(&main_bytes).ok_or("ttf-parser cannot parse the font")?;
    let scale = f32::from(opts.size) / f32::from(kerning_src.units_per_em());
    let (underline_position, underline_thickness) = match kerning_src.underline() {
        Some((pos, thick)) => (
            (f32::from(pos) * scale).round() as i32,
            ((f32::from(thick) * scale).round() as i32).max(1),
        ),
        None => (-(descent / 2), i32::from(opts.size / 14).max(1)),
    };

    let mut glyphs: BTreeMap<char, Glyph> = BTreeMap::new();
    for &cp in &cps {
        if !main.has_glyph(cp) {
            if cp.is_control() {
                continue; // e.g. U+007F in "0x20-0x7F": never drawn, silently skipped
            }
            warnings.push(format!(
                "U+{:04X} is not in {}; skipped",
                u32::from(cp),
                opts.ttf.display()
            ));
            continue;
        }
        let (cov, adv16) = rasterize(&main, cp, opts.size, subpx, false);
        glyphs.insert(cp, Glyph { cp, adv16, cov });
    }
    let main_cps: Vec<char> = glyphs.keys().copied().collect();

    let mut symbols_hash = None;
    if opts.symbols == Symbols::Fa {
        let paths: Vec<PathBuf> = if opts.symbols_ttf.is_empty() {
            DEFAULT_SYMBOLS_TTF.iter().map(PathBuf::from).collect()
        } else {
            opts.symbols_ttf.clone()
        };
        let mut sym_fonts = Vec::new();
        let mut hashes = Vec::new();
        for path in &paths {
            let (bytes, f) = load_font(&root.join(path), opts.size)?;
            hashes.push((file_name(path), sha256_hex(&bytes)));
            sym_fonts.push(f);
        }
        symbols_hash = Some(hashes);
        for &cp in twine_text::symbols::ALL {
            let alias = SYMBOL_ALIASES.iter().find(|(c, _)| *c == cp);
            let src = alias.map_or(cp, |&(_, a)| a);
            let Some(sym) = sym_fonts.iter().find(|f| f.has_glyph(src)) else {
                warnings.push(format!(
                    "symbol U+{:04X} is not in the symbol fonts; skipped",
                    u32::from(cp)
                ));
                continue;
            };
            let (cov, adv16) = rasterize(sym, src, opts.size, subpx, alias.is_some());
            // Center the symbol vertically on the main font's line box
            // (`[-descent, ascent]` around the baseline).
            let h_px = if subpx == Subpx::Ver { cov.h / 3 } else { cov.h };
            let target = (ascent - descent - h_px).div_euclid(2);
            let cov = if cov.h == 0 {
                cov
            } else {
                let unit = if subpx == Subpx::Ver { 3 } else { 1 };
                let dy = target * unit - cov.y0;
                cov.shifted_y(dy)
            };
            glyphs.insert(cp, Glyph { cp, adv16, cov });
        }
    }

    // Glyph ids: sorted by code point, id 0 reserved.
    let ordered: Vec<&Glyph> = glyphs.values().collect();
    let id_of: BTreeMap<char, u16> = ordered
        .iter()
        .enumerate()
        .map(|(i, g)| (g.cp, (i + 1) as u16))
        .collect();
    if ordered.len() >= usize::from(u16::MAX) {
        return Err("too many glyphs (max 65 534)".into());
    }

    // Bitmaps and descriptors.
    let format = opts.format();
    let mut bitmap = Vec::new();
    let mut dscs = vec![GlyphDsc::default()];
    for g in &ordered {
        let c = &g.cov;
        let sub_unit_y = if subpx == Subpx::Ver { 3 } else { 1 };
        let sub_unit_x = if subpx == Subpx::Hor { 3 } else { 1 };
        let (ofs_x, ofs_y) = (c.x0.div_euclid(sub_unit_x), c.y0.div_euclid(sub_unit_y));
        if c.w > 255 || c.h > 255 || !(-128..=127).contains(&ofs_x) || !(-128..=127).contains(&ofs_y) {
            return Err(format!(
                "glyph U+{:04X} is too large for the bitmap format ({}×{} at offset {ofs_x},{ofs_y})",
                u32::from(g.cp),
                c.w,
                c.h
            )
            .into());
        }
        let values: Vec<u8> = c.data.iter().map(|&v| quantize(v, opts.bpp)).collect();
        let bitmap_index = bitmap.len() as u32;
        bitmap.extend(encode_glyph(&values, c.w as usize, opts.bpp, format));
        dscs.push(GlyphDsc {
            bitmap_index: if c.w == 0 { 0 } else { bitmap_index },
            adv_w: g.adv16,
            box_w: c.w as u8,
            box_h: c.h as u8,
            ofs_x: ofs_x as i8,
            ofs_y: ofs_y as i8,
        });
    }

    let cmaps = build_cmaps(&ordered.iter().map(|g| g.cp).collect::<Vec<_>>());

    // Kerning between main-font glyphs.
    let mut kern = KernData::None;
    if opts.kerning && kerning_src.has_kerning() {
        let mut ids = Vec::new();
        let mut values = Vec::new();
        for &l in &main_cps {
            for &r in &main_cps {
                let units = kerning_src.kern(l, r);
                if units == 0 {
                    continue;
                }
                let v16 = (units as f32 * scale * 16.0).round() as i32;
                if v16 == 0 {
                    continue;
                }
                let clamped = v16.clamp(-127, 127);
                if clamped != v16 {
                    warnings.push(format!(
                        "kerning {l:?}→{r:?} of {v16}/16 px clamped to {clamped}/16 px"
                    ));
                }
                ids.push([id_of[&l], id_of[&r]]);
                values.push(clamped as i8);
            }
        }
        if !ids.is_empty() {
            kern = KernData::Pairs { ids, values };
        }
    }

    let data = FontData {
        line_height: line_height as i16,
        base_line: descent as i16,
        underline_position: underline_position.clamp(-128, 127) as i8,
        underline_thickness: underline_thickness.clamp(0, 255) as u8,
        subpx,
        bpp: opts.bpp,
        format,
        bitmap,
        glyphs: dscs,
        cmaps,
        kern,
        kern_scale: 16,
    };
    let mut code_points = vec!['\0'];
    code_points.extend(ordered.iter().map(|g| g.cp));
    let source_hash = (file_name(&opts.ttf), sha256_hex(&main_bytes));
    let (source, flash_bytes) = render_rust(opts, &data, &code_points, &source_hash, symbols_hash.as_deref());
    Ok(Generated {
        data,
        code_points,
        source,
        flash_bytes,
        warnings,
    })
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned())
}

fn is_ident(s: &str) -> bool {
    let mut c = s.chars();
    c.next().is_some_and(|f| f == '_' || f.is_ascii_alphabetic())
        && c.all(|x| x == '_' || x.is_ascii_alphanumeric())
}

/// Cmaps for glyphs sorted by code point with ids `1..`: runs of ≥ 3 consecutive code points
/// become `Format0Tiny`; the code points between runs are grouped into `SparseTiny` cmaps (a
/// group never spans a run, so its glyph ids stay consecutive, and never exceeds 65 535 code
/// points).
#[must_use]
pub fn build_cmaps(cps: &[char]) -> Vec<CmapData> {
    let mut cmaps = Vec::new();
    let mut sparse: Vec<(u32, u16)> = Vec::new(); // (code point, glyph id)
    let flush = |sparse: &mut Vec<(u32, u16)>, cmaps: &mut Vec<CmapData>| {
        if let Some(&(start, id)) = sparse.first() {
            let last = sparse.last().map_or(start, |p| p.0);
            cmaps.push(CmapData {
                range_start: start,
                range_length: (last - start + 1) as u16,
                glyph_id_start: id,
                unicode_list: sparse.iter().map(|&(c, _)| (c - start) as u16).collect(),
                ofs_u8: Vec::new(),
                ofs_u16: Vec::new(),
                kind: CmapKind::SparseTiny,
            });
        }
        sparse.clear();
    };
    let mut i = 0;
    while i < cps.len() {
        let mut j = i + 1;
        while j < cps.len() && u32::from(cps[j]) == u32::from(cps[j - 1]) + 1 && j - i < usize::from(u16::MAX)
        {
            j += 1;
        }
        let id = (i + 1) as u16;
        if j - i >= 3 {
            flush(&mut sparse, &mut cmaps);
            cmaps.push(CmapData {
                range_start: u32::from(cps[i]),
                range_length: (j - i) as u16,
                glyph_id_start: id,
                unicode_list: Vec::new(),
                ofs_u8: Vec::new(),
                ofs_u16: Vec::new(),
                kind: CmapKind::Format0Tiny,
            });
        } else {
            for (k, &ch) in cps.iter().enumerate().take(j).skip(i) {
                let cp = u32::from(ch);
                if sparse
                    .first()
                    .is_some_and(|&(s, _)| cp - s > u32::from(u16::MAX) - 1)
                {
                    flush(&mut sparse, &mut cmaps);
                }
                sparse.push((cp, (k + 1) as u16));
            }
        }
        i = j;
    }
    flush(&mut sparse, &mut cmaps);
    cmaps
}

/// Prints glyph metrics of `chars` in `ttf` at `size` px (the `--dump` debug aid).
pub fn dump(ttf: &Path, size: u16, chars: &str) -> Result<String> {
    const RAMP: &[u8] = b" .:-=+*#%@";
    let (bytes, font) = load_font(ttf, size)?;
    let mut out = String::new();
    if let Some(lm) = font.horizontal_line_metrics(f32::from(size)) {
        let _ = write!(
            out,
            "{} at {size} px: ascent {:.2}, descent {:.2}, line gap {:.2}",
            ttf.display(),
            lm.ascent,
            lm.descent,
            lm.line_gap
        );
        out.push('\n');
    }
    let k = gpos::Kerning::new(&bytes);
    let mut prev: Option<char> = None;
    for c in chars.chars() {
        let m = font.metrics(c, f32::from(size));
        let _ = write!(
            out,
            "U+{:04X} {c:?}: advance {:.3} px, box {}×{} at ({}, {}), glyph index {}",
            u32::from(c),
            m.advance_width,
            m.width,
            m.height,
            m.xmin,
            m.ymin,
            font.lookup_glyph_index(c)
        );
        if let (Some(p), Some(k)) = (prev, &k) {
            let units = k.kern(p, c);
            if units != 0 {
                let _ = write!(
                    out,
                    ", kerning after {p:?}: {units} units = {:.3} px",
                    units as f32 * f32::from(size) / f32::from(k.units_per_em())
                );
            }
        }
        out.push('\n');
        // Coverage as ASCII art (` .:-=+*#%@` by coverage).
        let (cov, _) = raster::raster(&font, c, f32::from(size));
        for row in cov.data.chunks(cov.w.max(1) as usize) {
            out.push_str("    |");
            out.extend(
                row.iter()
                    .map(|&v| char::from(RAMP[usize::from(v) * (RAMP.len() - 1) / 255])),
            );
            out.push_str("|\n");
        }
        prev = Some(c);
    }
    Ok(out)
}

/// Runs `twine font` with parsed arguments (writes the output file or prints the dump).
pub fn run(args: &FontArgs) -> Result<()> {
    if let Some(ttf) = &args.dump {
        print!("{}", dump(ttf, args.size, &args.chars)?);
        return Ok(());
    }
    let opts = FontOptions::from_args(args)?;
    let g = generate(&opts, Path::new("."))?;
    for w in &g.warnings {
        eprintln!("warning: {w}");
    }
    if let Some(dir) = opts.out.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&opts.out, &g.source).map_err(|e| format!("cannot write {}: {e}", opts.out.display()))?;
    eprintln!(
        "{}: {} glyphs, {} bytes of font data → {}",
        opts.name,
        g.code_points.len() - 1,
        g.flash_bytes,
        opts.out.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cmaps_runs_and_sparse_groups() {
        let cps: Vec<char> = [
            'A', 'B', 'C', 'D', '°', '•', '\u{F000}', '\u{F001}', '\u{F002}', '\u{F010}',
        ]
        .into_iter()
        .collect();
        let c = build_cmaps(&cps);
        assert_eq!(c.len(), 4);
        assert_eq!(
            (
                c[0].kind,
                c[0].range_start,
                c[0].range_length,
                c[0].glyph_id_start
            ),
            (CmapKind::Format0Tiny, 0x41, 4, 1)
        );
        assert_eq!((c[1].kind, c[1].glyph_id_start), (CmapKind::SparseTiny, 5));
        assert_eq!(c[1].unicode_list, vec![0, 0x2022 - 0xB0]);
        assert_eq!((c[2].kind, c[2].glyph_id_start), (CmapKind::Format0Tiny, 7));
        assert_eq!(
            (c[3].kind, c[3].glyph_id_start, c[3].range_length),
            (CmapKind::SparseTiny, 10, 1)
        );
    }

    #[test]
    fn identifiers() {
        assert!(is_ident("MONTSERRAT_14"));
        assert!(!is_ident("14"));
        assert!(!is_ident("A-B"));
    }
}
