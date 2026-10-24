//! `cargo xtask theme-recipes -- <cargo test arguments>`: the minimal recipe list of a UI
//! (design `docs/plan/r7-theming-design.md` §2.9.4, R7.S03).
//!
//! Runs the given tests with `TWINE_THEME_RECIPES=1` (single-threaded, output not captured):
//! every `twine_testing::assert_theme_covers` call then prints one line per theme class its UI
//! uses — `twine-theme-recipes: <class> <recipe or ->`. The command merges the lines of all
//! tests and prints the recipes in use, ready to paste into a `DesignSystem`'s `RECIPES` or a
//! built-in theme's `.recipes(&[..])` (`Lvgl`, `Mono`, `Simple`; prefix each with its module
//! path, e.g. `lvgl::BUTTON`), and the classes without a recipe.
//!
//! Example: `cargo xtask theme-recipes -- -p my-app --test screens`.

use std::fmt::Write as _;
use std::process::Command;

use crate::util::{R, display_command};

/// The report line prefix (`twine_testing::theme_coverage::RECIPES_LINE`).
const LINE: &str = "twine-theme-recipes:";

/// The environment variable that turns the report on
/// (`twine_testing::theme_coverage::RECIPES_ENV`).
const ENV: &str = "TWINE_THEME_RECIPES";

/// The merged report: the recipes in use (with the classes each styles) and the classes
/// without a recipe, each once, in the order first met.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// `(recipe, classes it styles)`.
    pub recipes: Vec<(String, Vec<String>)>,
    /// Classes the theme has no recipe for.
    pub missing: Vec<String>,
}

/// Collects the report lines of `output` (other lines are ignored).
#[must_use]
pub fn parse(output: &str) -> Report {
    let mut r = Report::default();
    for line in output.lines() {
        // With `--nocapture` the first line a test prints follows libtest's `test <name> ... `
        // on the same line: the report starts anywhere in a line.
        let Some(rest) = line
            .find(LINE)
            .and_then(|i| line.get(i.saturating_add(LINE.len())..))
        else {
            continue;
        };
        let mut words = rest.split_whitespace();
        let (Some(class), Some(recipe)) = (words.next(), words.next()) else {
            continue;
        };
        if recipe == "-" {
            if !r.missing.iter().any(|c| c == class) {
                r.missing.push(class.to_owned());
            }
            continue;
        }
        match r.recipes.iter_mut().find(|(n, _)| n == recipe) {
            Some((_, classes)) => {
                if !classes.iter().any(|c| c == class) {
                    classes.push(class.to_owned());
                }
            }
            None => r.recipes.push((recipe.to_owned(), vec![class.to_owned()])),
        }
    }
    r
}

/// The report as text: a recipe list to paste (a `DesignSystem`'s `RECIPES`, a built-in
/// theme's `.recipes(..)`) and the missing classes.
#[must_use]
pub fn render(r: &Report) -> String {
    let mut s = format!("Recipes in use ({}):\n", r.recipes.len());
    s.push_str("// `RECIPES` of a design system, or `.recipes(..)` of Lvgl / Mono / Simple:\n&[\n");
    // Writing to a `String` cannot fail.
    for (name, classes) in &r.recipes {
        let _ = writeln!(s, "    &{name}, // {}", classes.join(", "));
    }
    s.push_str("];\n");
    if !r.missing.is_empty() {
        let _ = writeln!(s, "Classes without a recipe: {}", r.missing.join(", "));
    }
    s
}

/// Runs the tests named by `args` and prints the merged report.
///
/// # Errors
/// When the tests cannot be run, no test printed a report (no `assert_theme_covers` ran), or
/// the tests failed (e.g. a class without a recipe: the report is printed first).
pub fn run(args: &[String]) -> R {
    let mut cmd = Command::new(env!("CARGO"));
    cmd.arg("test")
        .args(args)
        .args(["--", "--nocapture", "--test-threads=1"])
        .env(ENV, "1")
        .current_dir(crate::util::workspace_root());
    let line = display_command(&cmd);
    eprintln!("\x1b[1m$ {line}\x1b[0m");
    let out = cmd
        .output()
        .map_err(|e| format!("failed to spawn `{line}`: {e}"))?;
    let report = parse(&String::from_utf8_lossy(&out.stdout));
    if report.recipes.is_empty() && report.missing.is_empty() {
        eprint!("{}", String::from_utf8_lossy(&out.stderr));
        return Err(format!(
            "no theme coverage report: the tests run by `{line}` call no `twine_testing::assert_theme_covers`"
        )
        .into());
    }
    print!("{}", render(&report));
    if out.status.success() {
        Ok(())
    } else {
        eprint!("{}", String::from_utf8_lossy(&out.stderr));
        Err(format!("`{line}` failed with {}", out.status).into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_lines_of_several_tests() {
        let out = "running 2 tests\n\
                   twine-theme-recipes: button BUTTON\n\
                   twine-theme-recipes: button.tinted BUTTON\n\
                   twine-theme-recipes: label LABEL\n\
                   test a ... ok\n\
                   twine-theme-recipes: button BUTTON\n\
                   twine-theme-recipes: slider -\n\
                   twine-theme-recipes: slider -\n";
        let r = parse(out);
        assert_eq!(
            r.recipes,
            [
                (
                    "BUTTON".to_owned(),
                    vec!["button".to_owned(), "button.tinted".to_owned()]
                ),
                ("LABEL".to_owned(), vec!["label".to_owned()]),
            ]
        );
        assert_eq!(r.missing, ["slider"]);
        let text = render(&r);
        assert!(text.contains("    &BUTTON, // button, button.tinted\n"));
        assert!(text.contains("Classes without a recipe: slider\n"));
    }

    #[test]
    fn regression_a_report_after_the_libtest_test_name_is_read() {
        let out = "running 1 test\n\
                   test counter ... twine-theme-recipes: obj OBJ\n\
                   twine-theme-recipes: label LABEL\n\
                   ok\n";
        let report = parse(out);
        let names: Vec<&str> = report.recipes.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["OBJ", "LABEL"]);
    }

    #[test]
    fn ignores_other_and_malformed_lines() {
        assert_eq!(parse("twine-theme-recipes: lonely\nnoise\n"), Report::default());
    }
}
