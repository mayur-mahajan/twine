//! The safety profile's checks (`docs/safety/profile.md`).
//!
//! - **Profile crates** ([`check_metadata`], run by `cargo xtask layers`): every crate the facade
//!   links, except the [`EXCLUDED_CRATES`] and the host-only proc macros, has a `safety` feature,
//!   and its `lib.rs` carries the profile's lint attributes: [`LINTS`] each either denied or in
//!   a tracked allow-list (a step-tagged `NOTE` comment above it naming the R5 step), plus `clippy::float_arithmetic`
//!   denied in every profile.
//! - **`cargo xtask safety-lints [--update]`** ([`run_lint_status`]): counts the remaining sites
//!   of every lint per profile crate and compares them with the status table in
//!   [`PROFILE_DOC`] (a ratchet: a count that changes fails until the table is updated, and a
//!   lint whose last site is gone must move from the allow-list to `deny`).
//! - **CI stage `clippy-safety`** ([`clippy_safety`]): clippy over the facade in the profile
//!   with `-D warnings` (every denied lint enforced), then the lint-status check.
//! - **CI stage `safety-exclusions`** ([`check_exclusions`]): each excluded facade feature and
//!   crate fails to compile together with the profile, with the profile's message; and each
//!   panicking convenience form the profile removes ([`REMOVED_APIS`]) fails to compile in a
//!   fixture crate, while their `Result` forms compile ([`check_removed_apis`]).

use std::collections::BTreeMap;
use std::process::Command;

use serde_json::Value;

use crate::cmd::ci::{FeatureTables, feature_implies};
use crate::cmd::nostd::HOST_CRATES;
use crate::util::{R, cargo, cargo_ci, display_command, output, run as run_cmd, workspace_root};

/// The profile's feature, on the facade and on every profile crate.
pub const FEATURE: &str = "safety";

/// The facade crate.
const FACADE: &str = "twine";

/// Clippy lints the profile denies in library code (crates not yet clean list the remaining
/// ones in their allow-list). `float_arithmetic` is denied in every profile (checked
/// separately), so it is not part of this set.
pub const LINTS: [&str; LINT_COUNT] = [
    "indexing_slicing",
    "unwrap_used",
    "expect_used",
    "panic",
    "unreachable",
    "arithmetic_side_effects",
];

/// The number of [`LINTS`].
pub const LINT_COUNT: usize = 6;

/// Further clippy lints that find explicit panics, counted by the panic inventory
/// (`cargo xtask panics`) but not part of the crates' lint attributes (`todo-check` already
/// refuses the macros in the workspace).
pub const INVENTORY_LINTS: &[&str] = &["todo", "unimplemented"];

/// Facade features the profile refuses at compile time (`full` because it enables the others).
pub const EXCLUDED_FEATURES: &[&str] = &["perf-monitor", "fs", "ttf", "img-jpeg", "full"];

/// Crates the profile refuses as a whole (a constant assertion on `twine_core::SAFETY_PROFILE`).
pub const EXCLUDED_CRATES: &[&str] = &["twine-fs", "twine-lottie"];

/// The comment that must precede an allow-list: a step-tagged note naming an R5 step (spelled
/// in two parts so that `todo-check` does not read this line as an untagged marker).
const NOTE_TAG: &str = concat!("// NOTE", "(R5.S");

/// The alternative to [`NOTE_TAG`] for an allow-list that stays: its remaining sites are
/// deliberate, justified one by one (`// PANIC:`) and stated as a limitation of the profile in
/// the safety manual (`docs/safety/manual.md`, R5.S08). The lint count still ratchets.
const MANUAL_TAG: &str = "// SAFETY-MANUAL:";

/// Text every refusal message contains.
const REFUSAL: &str = "the `safety` profile";

/// The profile's documentation, holding the lint status table.
pub const PROFILE_DOC: &str = "docs/safety/profile.md";

const TABLE_BEGIN: &str = "<!-- safety-lints:begin (cargo xtask safety-lints --update) -->";
const TABLE_END: &str = "<!-- safety-lints:end -->";

/// The profile's lint attributes (whitespace removed) as they must appear in a crate root.
const DENY_PREFIX: &str = "#![cfg_attr(all(feature=\"safety\",not(test)),deny(";
const ALLOW_PREFIX: &str = "#![cfg_attr(all(feature=\"safety\",not(test)),allow(";

/// Lint attributes of one crate root.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct LintAttrs {
    /// Lints the profile denies.
    pub deny: Vec<String>,
    /// Lints of the tracked allow-list.
    pub allow: Vec<String>,
    /// Whether the allow-list is preceded by a step-tagged `NOTE` comment ([`NOTE_TAG`]) or a
    /// safety-manual reference ([`MANUAL_TAG`]).
    pub allow_noted: bool,
    /// Whether `clippy::float_arithmetic` is denied (outside tests) in every profile.
    pub float_denied: bool,
}

/// The lint list of the first attribute starting with `prefix` (whitespace-free source).
fn lint_list(compact: &str, prefix: &str) -> Option<Vec<String>> {
    let start = compact.find(prefix)? + prefix.len();
    let end = start + compact[start..].find(')')?;
    Some(
        compact[start..end]
            .split(',')
            .filter(|l| !l.is_empty())
            .map(|l| l.trim_start_matches("clippy::").to_owned())
            .collect(),
    )
}

/// Parses the profile's lint attributes of a crate root (`lib.rs` source).
#[must_use]
pub fn parse_lint_attrs(src: &str) -> LintAttrs {
    let compact: String = src.chars().filter(|c| !c.is_whitespace()).collect();
    let allow = lint_list(&compact, ALLOW_PREFIX);
    // The comment lines directly above the allow-list attribute must name the step clearing it,
    // or the safety-manual entry that states the remaining sites as a limitation.
    let allow_noted = src.match_indices("#![cfg_attr(").any(|(i, _)| {
        let attr: String = src[i..]
            .chars()
            .take(200)
            .filter(|c| !c.is_whitespace())
            .collect();
        attr.starts_with(ALLOW_PREFIX)
            && src[..i]
                .lines()
                .rev()
                .take_while(|l| l.trim_start().starts_with("//"))
                .any(|l| l.contains(NOTE_TAG) || l.trim_start().starts_with(MANUAL_TAG))
    });
    LintAttrs {
        deny: lint_list(&compact, DENY_PREFIX).unwrap_or_default(),
        allow: allow.unwrap_or_default(),
        allow_noted,
        float_denied: compact.contains("deny(clippy::float_arithmetic)"),
    }
}

/// Problems with a crate's lint attributes (empty when they are well-formed).
#[must_use]
pub fn check_lint_attrs(name: &str, attrs: &LintAttrs) -> Vec<String> {
    let mut errors = Vec::new();
    for lint in LINTS {
        let denied = attrs.deny.iter().any(|l| l == lint);
        let allowed = attrs.allow.iter().any(|l| l == lint);
        match (denied, allowed) {
            (false, false) => errors.push(format!(
                "`{name}`: `clippy::{lint}` must be denied under `safety` (or listed in the tracked allow-list)"
            )),
            (true, true) => errors.push(format!(
                "`{name}`: `clippy::{lint}` is both denied and allowed under `safety`"
            )),
            _ => {}
        }
    }
    for lint in attrs.deny.iter().chain(&attrs.allow) {
        if !LINTS.contains(&lint.as_str()) {
            errors.push(format!(
                "`{name}`: `{lint}` is not a safety-profile lint (expected one of {LINTS:?})"
            ));
        }
    }
    if !attrs.allow.is_empty() && !attrs.allow_noted {
        errors.push(format!(
            "`{name}`: the safety allow-list needs a comment starting with `{NOTE_TAG}mm)` naming the step that clears it, or `{MANUAL_TAG}` naming the safety-manual limitation that keeps it"
        ));
    }
    if !attrs.float_denied {
        errors.push(format!(
            "`{name}`: add `#![cfg_attr(not(test), deny(clippy::float_arithmetic))]` (no floats in the profile)"
        ));
    }
    errors
}

/// The profile crates of the workspace: every package with a [`FEATURE`] feature and the path
/// of its library root.
fn profile_crates(meta: &Value) -> Vec<(String, String)> {
    let mut crates: Vec<(String, String)> = meta["packages"]
        .as_array()
        .map_or(&[][..], Vec::as_slice)
        .iter()
        .filter(|p| p["features"].get(FEATURE).is_some())
        .filter_map(|p| {
            let lib = p["targets"]
                .as_array()?
                .iter()
                .find(|t| t["kind"].as_array().is_some_and(|k| k.iter().any(|k| k == "lib")))?;
            Some((
                p["name"].as_str()?.to_owned(),
                lib["src_path"].as_str()?.to_owned(),
            ))
        })
        .collect();
    crates.sort();
    crates
}

/// Checks `cargo metadata --no-deps` JSON (run by `cargo xtask layers`): every crate the facade
/// depends on has the profile feature (except the excluded and host-only crates), and every
/// profile crate's root carries well-formed lint attributes. Returns the violations.
pub fn check_metadata(json: &str) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let meta: Value = serde_json::from_str(json)?;
    let packages = meta["packages"]
        .as_array()
        .ok_or("metadata: missing `packages`")?;
    let mut errors = Vec::new();
    let has_feature = |name: &str| {
        packages
            .iter()
            .any(|p| p["name"].as_str() == Some(name) && p["features"].get(FEATURE).is_some())
    };
    if let Some(facade) = packages.iter().find(|p| p["name"].as_str() == Some(FACADE)) {
        let deps = facade["dependencies"].as_array().map_or(&[][..], Vec::as_slice);
        for dep in deps {
            // Normal dependencies only: tests and examples pick their own crates.
            if !dep["kind"].is_null() {
                continue;
            }
            let Some(dep) = dep["name"].as_str() else { continue };
            let in_workspace = packages.iter().any(|p| p["name"].as_str() == Some(dep));
            let exempt = EXCLUDED_CRATES.contains(&dep) || HOST_CRATES.contains(&dep);
            if in_workspace && !exempt && !has_feature(dep) {
                errors.push(format!(
                    "`{dep}` is linked by the facade but has no `{FEATURE}` feature (add it, forwarded \
                     to its dependencies, and the profile's lint attributes; see {PROFILE_DOC})"
                ));
            }
        }
        if !has_feature(FACADE) {
            errors.push(format!("the facade `{FACADE}` has no `{FEATURE}` feature"));
        }
    }
    for (name, src_path) in profile_crates(&meta) {
        match std::fs::read_to_string(&src_path) {
            Ok(src) => errors.extend(check_lint_attrs(&name, &parse_lint_attrs(&src))),
            Err(e) => errors.push(format!("`{name}`: cannot read {src_path}: {e}")),
        }
    }
    Ok(errors)
}

/// The facade features the profile is linted and counted with: every feature except
/// `default`, the profile itself, the `defmt` backend (linted by `clippy-defmt`; `log` is used
/// here), `std` and features implying it (host-only), and the [`EXCLUDED_FEATURES`] and
/// features implying them.
#[must_use]
pub fn profile_features(features: &FeatureTables<'_>) -> Vec<String> {
    let Some(facade) = features.get(FACADE) else {
        return Vec::new();
    };
    let refused = |pkg: &str, feat: &str| {
        feat == "std"
            || feat.ends_with("-std")
            || feat == "defmt"
            || (pkg == FACADE && EXCLUDED_FEATURES.contains(&feat))
    };
    let mut list: Vec<String> = facade
        .keys()
        .filter(|f| *f != "default" && *f != FEATURE)
        .filter(|f| !feature_implies(features, FACADE, f, &refused, 0))
        .cloned()
        .collect();
    list.sort();
    list
}

/// Feature tables of every package in `meta`.
fn feature_tables(meta: &Value) -> FeatureTables<'_> {
    meta["packages"]
        .as_array()
        .map_or(&[][..], Vec::as_slice)
        .iter()
        .filter_map(|p| Some((p["name"].as_str()?, p["features"].as_object()?)))
        .collect()
}

fn metadata() -> Result<String, Box<dyn std::error::Error>> {
    output(cargo().args(["metadata", "--format-version", "1", "--no-deps"]))
}

/// A clippy command over the facade in the profile (library code only).
fn profile_clippy(features: &[String]) -> Command {
    let mut cmd = cargo_ci();
    let mut list = vec![FEATURE.to_owned()];
    list.extend_from_slice(features);
    cmd.args([
        "clippy",
        "-p",
        FACADE,
        "--lib",
        "--no-default-features",
        "--features",
        &list.join(","),
    ]);
    cmd
}

/// Per crate, the number of sites of each of [`LINTS`] (indexed like [`LINTS`]).
pub type LintCounts = BTreeMap<String, [usize; LINT_COUNT]>;

/// One site clippy reported: the lint (without `clippy::`), the crate directory name, the file
/// (workspace-relative), the position of the primary span and the first line of its source.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Site {
    /// The lint, e.g. `indexing_slicing`.
    pub lint: String,
    /// The crate's directory under `crates/` (its package name).
    pub krate: String,
    /// The file, relative to the workspace root.
    pub file: String,
    /// Line of the primary span.
    pub line: u64,
    /// Column of the primary span.
    pub col: u64,
    /// The first source line of the primary span, trimmed.
    pub code: String,
}

/// The sites of clippy's JSON messages in `crates/<crate>/`: unique primary spans (a site
/// reported in several compilation units or expansions counts once), sorted.
#[must_use]
pub fn parse_sites(json_lines: &str) -> Vec<Site> {
    let mut sites = std::collections::BTreeSet::new();
    for line in json_lines.lines() {
        let Ok(m) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if m["reason"] != "compiler-message" {
            continue;
        }
        let msg = &m["message"];
        let Some(lint) = msg["code"]["code"]
            .as_str()
            .and_then(|c| c.strip_prefix("clippy::"))
        else {
            continue;
        };
        let Some(span) = msg["spans"]
            .as_array()
            .and_then(|s| s.iter().find(|s| s["is_primary"] == true))
        else {
            continue;
        };
        let file = span["file_name"].as_str().unwrap_or_default();
        let Some(krate) = file.strip_prefix("crates/").and_then(|f| f.split('/').next()) else {
            continue;
        };
        let code = span["text"]
            .as_array()
            .and_then(|t| t.first())
            .and_then(|t| t["text"].as_str())
            .unwrap_or_default()
            .trim()
            .to_owned();
        sites.insert(Site {
            lint: lint.to_owned(),
            krate: krate.to_owned(),
            file: file.to_owned(),
            line: span["line_start"].as_u64().unwrap_or_default(),
            col: span["column_start"].as_u64().unwrap_or_default(),
            code,
        });
    }
    sites.into_iter().collect()
}

/// Counts the remaining sites of every lint of [`LINTS`] per profile crate.
#[must_use]
pub fn count_sites(sites: &[Site], crates: &[String]) -> LintCounts {
    let mut counts: LintCounts = crates.iter().map(|c| (c.clone(), [0; LINT_COUNT])).collect();
    for site in sites {
        let Some(index) = LINTS.iter().position(|l| *l == site.lint) else {
            continue;
        };
        if let Some(c) = counts.get_mut(&site.krate) {
            c[index] += 1;
        }
    }
    counts
}

/// The lint status table (Markdown, between the markers).
#[must_use]
pub fn render_table(counts: &LintCounts) -> String {
    use std::fmt::Write as _;
    let mut s = String::from("| Crate |");
    for lint in LINTS {
        let _ = write!(s, " `{lint}` |");
    }
    s.push_str(" Total |\n|---|");
    s.push_str(&"---:|".repeat(LINTS.len() + 1));
    s.push('\n');
    let mut totals = [0usize; LINT_COUNT];
    for (krate, c) in counts {
        let _ = write!(s, "| {krate} |");
        for (t, n) in totals.iter_mut().zip(c) {
            *t += n;
            let _ = write!(s, " {n} |");
        }
        match c.iter().sum::<usize>() {
            0 => s.push_str(" **clean** |\n"),
            total => {
                let _ = writeln!(s, " {total} |");
            }
        }
    }
    s.push_str("| **all** |");
    for t in totals {
        let _ = write!(s, " {t} |");
    }
    let _ = writeln!(s, " {} |", totals.iter().sum::<usize>());
    s
}

/// The table currently in `doc` (between the markers), if any.
fn table_in(doc: &str) -> Option<(usize, usize)> {
    let start = doc.find(TABLE_BEGIN)? + TABLE_BEGIN.len();
    let end = start + doc[start..].find(TABLE_END)?;
    Some((start, end))
}

/// Ratchet checks: a lint with no site left must be denied, a denied lint must have none.
#[must_use]
pub fn ratchet(counts: &LintCounts, attrs: &BTreeMap<String, LintAttrs>) -> Vec<String> {
    let mut errors = Vec::new();
    for (krate, c) in counts {
        let Some(a) = attrs.get(krate) else { continue };
        for (lint, n) in LINTS.iter().zip(c) {
            let allowed = a.allow.iter().any(|l| l == lint);
            if allowed && *n == 0 {
                errors.push(format!(
                    "`{krate}`: no `clippy::{lint}` site is left: move it from the allow-list to `deny`"
                ));
            }
            if !allowed && *n > 0 {
                errors.push(format!(
                    "`{krate}`: {n} `clippy::{lint}` site(s), but the lint is denied"
                ));
            }
        }
    }
    errors
}

/// Every site of [`LINTS`] and [`INVENTORY_LINTS`] in the profile crates: clippy over the
/// facade in the profile with every one of these lints forced to warn (`--force-warn`
/// overrides the crates' `deny`/`allow` attributes, so every site is reported).
pub fn clippy_sites() -> Result<Vec<Site>, Box<dyn std::error::Error>> {
    let meta: Value = serde_json::from_str(&metadata()?)?;
    let features = profile_features(&feature_tables(&meta));
    let mut cmd = profile_clippy(&features);
    cmd.arg("--message-format=json").arg("--");
    for lint in LINTS.iter().chain(INVENTORY_LINTS) {
        cmd.args(["--force-warn", &format!("clippy::{lint}")]);
    }
    eprintln!("\x1b[1m$ {}\x1b[0m", display_command(&cmd));
    let out = cmd.output()?;
    if !out.status.success() {
        return Err(format!("safety: clippy failed:\n{}", String::from_utf8_lossy(&out.stderr)).into());
    }
    Ok(parse_sites(&String::from_utf8(out.stdout)?))
}

/// The profile crates' names and library roots.
pub fn profile_crate_roots() -> Result<Vec<(String, String)>, Box<dyn std::error::Error>> {
    let meta: Value = serde_json::from_str(&metadata()?)?;
    Ok(profile_crates(&meta))
}

/// `cargo xtask safety-lints [--update]`: counts the lint sites ([`clippy_sites`]) and checks
/// them against the status table of [`PROFILE_DOC`] (`update`: rewrite the table).
pub fn run_lint_status(update: bool) -> R {
    lint_status(&clippy_sites()?, update)
}

/// Checks the counts of `sites` against the status table of [`PROFILE_DOC`] and the ratchet
/// (`update`: rewrite the table).
pub fn lint_status(sites: &[Site], update: bool) -> R {
    let crates = profile_crate_roots()?;
    let names: Vec<String> = crates.iter().map(|(n, _)| n.clone()).collect();
    let counts = count_sites(sites, &names);
    let attrs: BTreeMap<String, LintAttrs> = crates
        .iter()
        .map(|(n, p)| Ok((n.clone(), parse_lint_attrs(&std::fs::read_to_string(p)?))))
        .collect::<Result<_, std::io::Error>>()?;
    let mut errors = ratchet(&counts, &attrs);
    let path = workspace_root().join(PROFILE_DOC);
    let doc = std::fs::read_to_string(&path)?;
    let (start, end) = table_in(&doc).ok_or_else(|| format!("{PROFILE_DOC}: lint table markers missing"))?;
    let table = format!("\n{}", render_table(&counts));
    print!("{table}");
    if doc[start..end] != table {
        if update {
            std::fs::write(&path, format!("{}{table}{}", &doc[..start], &doc[end..]))?;
            println!("safety-lints: {PROFILE_DOC} updated");
        } else {
            errors.push(format!(
                "{PROFILE_DOC}: the lint status table is out of date (run `cargo xtask safety-lints --update` and review the change)"
            ));
        }
    }
    if errors.is_empty() {
        println!("safety-lints: ok ({} profile crates)", crates.len());
        Ok(())
    } else {
        for e in &errors {
            println!("safety-lints: {e}");
        }
        Err(format!("safety-lints: {} problem(s)", errors.len()).into())
    }
}

/// CI stage `clippy-safety`: clippy over the facade in the profile with `-D warnings`, then the
/// lint-status ratchet ([`lint_status`]) and the panic inventory check
/// ([`panics::write_or_check`](crate::cmd::panics::write_or_check)), both from one counting
/// run ([`clippy_sites`]).
pub fn clippy_safety() -> R {
    let meta: Value = serde_json::from_str(&metadata()?)?;
    let features = profile_features(&feature_tables(&meta));
    run_cmd(profile_clippy(&features).args(["--", "-D", "warnings"]))?;
    let sites = clippy_sites()?;
    let status = lint_status(&sites, false);
    let inventory = crate::cmd::panics::write_or_check(&sites, true);
    status.and(inventory)
}

/// Runs `cmd`, expecting it to fail with the profile's refusal message.
fn expect_refusal(cmd: &mut Command) -> R {
    let line = display_command(cmd);
    eprintln!("\x1b[1m$ {line}\x1b[0m (must fail)");
    let out = cmd.output()?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    if out.status.success() {
        return Err(format!("`{line}` built, but the safety profile must refuse it").into());
    }
    if !stderr.contains(REFUSAL) {
        return Err(format!("`{line}` failed without the profile's message:\n{stderr}").into());
    }
    Ok(())
}

/// CI stage `safety-exclusions`: every [`EXCLUDED_FEATURES`] feature of the facade and every
/// [`EXCLUDED_CRATES`] crate fails to compile together with the profile, with its message.
pub fn check_exclusions() -> R {
    for feature in EXCLUDED_FEATURES {
        expect_refusal(cargo_ci().args([
            "check",
            "-p",
            FACADE,
            "--lib",
            "--no-default-features",
            "--features",
            &format!("{FEATURE},{feature}"),
        ]))?;
    }
    for krate in EXCLUDED_CRATES {
        expect_refusal(cargo_ci().args([
            "check",
            "-p",
            krate,
            "--lib",
            "--features",
            &format!("twine-core/{FEATURE}"),
        ]))?;
    }
    println!(
        "safety-exclusions: ok ({} features, {} crates refused)",
        EXCLUDED_FEATURES.len(),
        EXCLUDED_CRATES.len()
    );
    check_removed_apis()
}

/// The fixture crate of the API checks (its own workspace, see its `Cargo.toml`).
const API_FIXTURE: &str = "tools/safety-api";

/// The panicking convenience forms the profile removes: `(fixture binary, text the compile
/// error must contain)`.
pub const REMOVED_APIS: &[(&str, &str)] = &[
    ("no_build", "no method named `build`"),
    ("no_expect_context", "no method named `expect_context`"),
    ("no_missing_context_panic", "no method named `panic`"),
    ("no_expect_navigator", "cannot find function `expect_navigator`"),
];

/// A `cargo check` of one binary of the [`API_FIXTURE`] crate (its own target directory under
/// the workspace's `target/`).
fn fixture_check(bin: &str) -> Command {
    let root = workspace_root();
    let mut cmd = cargo_ci();
    cmd.env("CARGO_TARGET_DIR", root.join("target").join("safety-api"))
        .args(["check", "--manifest-path"])
        .arg(root.join(API_FIXTURE).join("Cargo.toml"))
        .args(["--bin", bin]);
    cmd
}

/// Part of CI stage `safety-exclusions`: in the profile, each of [`REMOVED_APIS`] fails to
/// compile naming the missing item, and the `Result` forms compile (`fallible_forms`). The
/// fixture resolves with a copy of the workspace's `Cargo.lock` (same versions, no network).
pub fn check_removed_apis() -> R {
    let root = workspace_root();
    std::fs::copy(root.join("Cargo.lock"), root.join(API_FIXTURE).join("Cargo.lock"))?;
    run_cmd(&mut fixture_check("fallible_forms"))?;
    for (bin, needle) in REMOVED_APIS {
        let mut cmd = fixture_check(bin);
        let line = display_command(&cmd);
        eprintln!("\x1b[1m$ {line}\x1b[0m (must fail)");
        let out = cmd.output()?;
        let stderr = String::from_utf8_lossy(&out.stderr);
        if out.status.success() {
            return Err(format!("`{bin}` compiled, but the safety profile must not offer it").into());
        }
        if !stderr.contains(needle) {
            return Err(format!("`{bin}` failed without `{needle}`:\n{stderr}").into());
        }
    }
    println!(
        "safety-exclusions: ok ({} panicking APIs absent from the profile)",
        REMOVED_APIS.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLEAN: &str = r#"#![no_std]
#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::float_arithmetic))]
// The safety profile's lints.
#![cfg_attr(
    all(feature = "safety", not(test)),
    deny(
        clippy::indexing_slicing,
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::arithmetic_side_effects,
    )
)]
"#;

    const PENDING: &str = r#"#![cfg_attr(not(test), deny(clippy::float_arithmetic))]
#![cfg_attr(
    all(feature = "safety", not(test)),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
    )
)]
// NOTE(R5.S02): sites of these lints remain.
#![cfg_attr(
    all(feature = "safety", not(test)),
    allow(
        clippy::indexing_slicing,
        clippy::arithmetic_side_effects,
    )
)]
"#;

    #[test]
    fn parses_deny_and_allow_lists() {
        let a = parse_lint_attrs(CLEAN);
        assert_eq!(a.deny.len(), LINT_COUNT);
        assert!(a.allow.is_empty() && a.float_denied);
        assert_eq!(check_lint_attrs("c", &a), Vec::<String>::new());
        let a = parse_lint_attrs(PENDING);
        assert_eq!(a.deny, ["unwrap_used", "expect_used", "panic", "unreachable"]);
        assert_eq!(a.allow, ["indexing_slicing", "arithmetic_side_effects"]);
        assert!(a.allow_noted);
        assert_eq!(check_lint_attrs("c", &a), Vec::<String>::new());
    }

    #[test]
    fn rejects_missing_lints_untracked_allow_lists_and_floats() {
        let a = parse_lint_attrs("#![no_std]\n");
        let e = check_lint_attrs("c", &a);
        assert_eq!(e.len(), LINTS.len() + 1, "{e:?}");
        let untracked = PENDING.replace("// NOTE(R5.S02): sites of these lints remain.\n", "");
        let e = check_lint_attrs("c", &parse_lint_attrs(&untracked));
        assert!(e.iter().any(|m| m.contains(NOTE_TAG)), "{e:?}");
        let manual = PENDING.replace(
            "// NOTE(R5.S02): sites of these lints remain.",
            "// SAFETY-MANUAL: limitation L1 keeps these sites.",
        );
        assert_eq!(
            check_lint_attrs("c", &parse_lint_attrs(&manual)),
            Vec::<String>::new()
        );
        // The predicate must be exactly the profile's: a typo leaves the lints inactive.
        let typo = CLEAN.replace("feature = \"safety\"", "feature = \"safe\"");
        assert_eq!(check_lint_attrs("c", &parse_lint_attrs(&typo)).len(), LINTS.len());
    }

    #[test]
    fn counts_unique_primary_spans_per_crate() {
        let msg = |lint: &str, file: &str, line: u32| {
            serde_json::json!({ "reason": "compiler-message", "message": {
                "code": { "code": format!("clippy::{lint}") },
                "spans": [{ "is_primary": true, "file_name": file, "line_start": line, "column_start": 1 }]
            }})
            .to_string()
        };
        let lines = [
            msg("indexing_slicing", "crates/twine-core/src/a.rs", 1),
            msg("indexing_slicing", "crates/twine-core/src/a.rs", 1), // same site, another unit
            msg("arithmetic_side_effects", "crates/twine-core/src/a.rs", 2),
            msg("panic", "crates/twine-view/src/b.rs", 3),
            msg("panic", "crates/twine-macros/src/c.rs", 3), // not a profile crate
            msg("needless_return", "crates/twine-core/src/a.rs", 4),
        ]
        .join("\n");
        let counts = count_sites(&parse_sites(&lines), &["twine-core".into(), "twine-view".into()]);
        assert_eq!(counts["twine-core"], [1, 0, 0, 0, 0, 1]);
        assert_eq!(counts["twine-view"], [0, 0, 0, 1, 0, 0]);
        let table = render_table(&counts);
        assert!(
            table.contains("| twine-core | 1 | 0 | 0 | 0 | 0 | 1 | 2 |"),
            "{table}"
        );
        assert!(
            table.contains("| **all** | 1 | 0 | 0 | 1 | 0 | 1 | 3 |"),
            "{table}"
        );
    }

    #[test]
    fn ratchet_requires_denying_clean_lints() {
        let counts: LintCounts = [("c".to_owned(), [0, 0, 0, 0, 0, 3])].into();
        let attrs: BTreeMap<_, _> = [("c".to_owned(), parse_lint_attrs(PENDING))].into();
        let e = ratchet(&counts, &attrs);
        assert_eq!(e.len(), 1, "{e:?}");
        assert!(e[0].contains("indexing_slicing") && e[0].contains("move it"));
        let counts: LintCounts = [("c".to_owned(), [1, 1, 0, 0, 0, 3])].into();
        let e = ratchet(&counts, &attrs);
        assert_eq!(e.len(), 1, "{e:?}");
        assert!(e[0].contains("unwrap_used") && e[0].contains("denied"));
    }

    #[test]
    fn profile_features_leave_out_excluded_std_and_defmt() {
        let tables = serde_json::json!({
            "twine": {
                "default": ["perf-monitor"], "safety": [], "log": [], "defmt": [], "std": [],
                "perf-monitor": [], "fs": [], "full": ["fs", "vector"], "vector": [],
                "drivers-fbdev": ["std"], "img-png": ["twine-image/img-png"],
                "ttf": ["twine-text/ttf"]
            },
            "twine-image": { "img-png": [] },
            "twine-text": { "ttf": [] }
        });
        let map = tables.as_object().unwrap();
        let features: FeatureTables<'_> = map
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_object().unwrap()))
            .collect();
        assert_eq!(profile_features(&features), ["img-png", "log", "vector"]);
    }

    #[test]
    fn workspace_profile_crates_are_well_formed() {
        let json = metadata().unwrap();
        let errors = check_metadata(&json).unwrap();
        assert!(errors.is_empty(), "{errors:#?}");
        let meta: Value = serde_json::from_str(&json).unwrap();
        let names: Vec<_> = profile_crates(&meta).into_iter().map(|(n, _)| n).collect();
        assert!(names.contains(&"twine-core".to_owned()) && names.contains(&FACADE.to_owned()));
        assert!(!names.iter().any(|n| EXCLUDED_CRATES.contains(&n.as_str())));
    }

    #[test]
    fn status_table_has_markers() {
        let doc = std::fs::read_to_string(workspace_root().join(PROFILE_DOC)).unwrap();
        assert!(table_in(&doc).is_some());
    }
}
