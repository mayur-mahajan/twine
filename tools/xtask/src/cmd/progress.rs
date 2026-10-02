//! `cargo xtask progress`: reports the progress of the API evolution plan
//! (`docs/plan/api-evolution.md`) from its §5 checklist, and checks that the checklist and the
//! plan's steps agree.
//!
//! The plan has phase headings (`## R3 — Integration kit …`), step headings
//! (`### R3.S01 — `Platform` trait`) and, in §5, one checklist line per phase
//! (`- [x] R3.S01 · [ ] R3.S02 · …`). The report lists done/total per phase and the next unchecked
//! step. The check fails when a step heading has no checkbox, a checkbox names no step, or a step
//! appears twice — so ticks cannot drift from the steps they track.
//!
//! The plan lives under `docs/`, which is not part of every checkout: without it the command (and
//! the `progress` CI stage) reports that and succeeds.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;

use crate::util::{R, workspace_root};

/// The plan file, relative to the workspace root.
pub const PLAN: &str = "docs/plan/api-evolution.md";

/// A step heading: `### Rn.Smm — title`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// `Rn.Smm`.
    pub id: String,
    /// The heading text after the id.
    pub title: String,
}

/// A phase: its id (`Rn`), title and steps in file order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Phase {
    /// `Rn`.
    pub id: String,
    /// The heading text after the id.
    pub title: String,
    /// The phase's step headings, in file order.
    pub steps: Vec<Step>,
}

/// The parsed plan: phases with their steps, and the §5 checklist.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Plan {
    /// Phases in file order.
    pub phases: Vec<Phase>,
    /// §5 checkboxes, in file order: `(id, ticked)`.
    pub checklist: Vec<(String, bool)>,
}

/// Whether `s` is a step id `Rn.Smm` (one phase digit, two step digits).
fn is_step_id(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 6
        && b[0] == b'R'
        && b[1].is_ascii_digit()
        && b[2] == b'.'
        && b[3] == b'S'
        && b[4].is_ascii_digit()
        && b[5].is_ascii_digit()
}

/// Whether `s` is a phase id `Rn`.
fn is_phase_id(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 2 && b[0] == b'R' && b[1].is_ascii_digit()
}

/// Splits `"<id> — <title>"` into id and title (the title may be empty).
fn split_heading(rest: &str) -> (&str, &str) {
    let rest = rest.trim();
    match rest.split_once(' ') {
        Some((id, tail)) => (id, tail.trim_start_matches(['—', '-', ' ']).trim()),
        None => (rest, ""),
    }
}

/// Parses the plan text.
#[must_use]
pub fn parse(text: &str) -> Plan {
    let mut plan = Plan::default();
    let mut in_progress = false;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("## ") {
            // `## 5. Progress` starts the checklist; any other `## ` heading ends it.
            in_progress = rest.trim_start().starts_with("5.");
            let (id, title) = split_heading(rest);
            if is_phase_id(id) {
                plan.phases.push(Phase {
                    id: id.to_string(),
                    title: title.to_string(),
                    steps: Vec::new(),
                });
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("### ") {
            let (id, title) = split_heading(rest);
            if is_step_id(id) {
                if let Some(phase) = plan.phases.iter_mut().rev().find(|p| id.starts_with(&p.id)) {
                    phase.steps.push(Step {
                        id: id.to_string(),
                        title: title.to_string(),
                    });
                }
            }
            continue;
        }
        if in_progress {
            // `- [x] R0.S01 · [ ] R0.S02 · …`
            let mut rest = line;
            while let Some(pos) = rest.find('[') {
                let after = &rest[pos..];
                let ticked = match after.get(..3) {
                    Some("[x]" | "[X]") => true,
                    Some("[ ]") => false,
                    _ => {
                        rest = &rest[pos + 1..];
                        continue;
                    }
                };
                let tail = after[3..].trim_start();
                let id: String = tail.chars().take(6).collect();
                if is_step_id(&id) {
                    plan.checklist.push((id, ticked));
                }
                rest = &after[3..];
            }
        }
    }
    plan
}

/// Consistency problems between the steps and the §5 checklist.
#[must_use]
pub fn check(plan: &Plan) -> Vec<String> {
    let mut problems = Vec::new();
    let mut headings: BTreeMap<&str, usize> = BTreeMap::new();
    for step in plan.phases.iter().flat_map(|p| &p.steps) {
        *headings.entry(step.id.as_str()).or_default() += 1;
    }
    let mut boxes: BTreeMap<&str, usize> = BTreeMap::new();
    for (id, _) in &plan.checklist {
        *boxes.entry(id.as_str()).or_default() += 1;
    }
    for (id, n) in &headings {
        if *n > 1 {
            problems.push(format!("step {id} has {n} headings"));
        }
        if !boxes.contains_key(id) {
            problems.push(format!("step {id} has no checkbox in §5"));
        }
    }
    for (id, n) in &boxes {
        if *n > 1 {
            problems.push(format!("§5 has {n} checkboxes for {id}"));
        }
        if !headings.contains_key(id) {
            problems.push(format!("§5 checkbox {id} has no step heading"));
        }
    }
    problems
}

/// The progress report: one line per phase (done/total, next step) and a total.
#[must_use]
pub fn report(plan: &Plan) -> String {
    let ticked: BTreeMap<&str, bool> = plan.checklist.iter().map(|(id, t)| (id.as_str(), *t)).collect();
    let is_done = |id: &str| ticked.get(id).copied().unwrap_or(false);
    let width = plan
        .phases
        .iter()
        .map(|p| p.id.len() + 1 + p.title.chars().count())
        .max()
        .unwrap_or(0)
        .min(60);
    let mut out = String::new();
    let (mut done_all, mut total_all) = (0usize, 0usize);
    let mut next_overall: Option<&Step> = None;
    for phase in &plan.phases {
        let total = phase.steps.len();
        let done = phase.steps.iter().filter(|s| is_done(&s.id)).count();
        done_all += done;
        total_all += total;
        let next = phase.steps.iter().find(|s| !is_done(&s.id));
        if next_overall.is_none() {
            next_overall = next;
        }
        let name: String = format!("{} {}", phase.id, phase.title).chars().take(60).collect();
        let status = match next {
            None if total > 0 => "done".to_string(),
            None => "no steps".to_string(),
            Some(s) => format!("next: {} — {}", s.id, s.title),
        };
        let _ = writeln!(out, "{name:width$}  {done:>2}/{total:<2}  {status}");
    }
    let _ = writeln!(out, "\n{done_all}/{total_all} steps done");
    if let Some(s) = next_overall {
        let _ = writeln!(out, "next step: {} — {}", s.id, s.title);
    }
    out
}

fn plan_path() -> PathBuf {
    workspace_root().join(PLAN)
}

/// Whether the plan file exists (the `progress` CI stage is skipped without it).
#[must_use]
pub fn plan_exists() -> bool {
    plan_path().is_file()
}

/// Prints the progress report; fails when the checklist and the steps disagree. Without the plan
/// file (it is not in every checkout) it says so and succeeds.
pub fn run() -> R {
    let Ok(text) = std::fs::read_to_string(plan_path()) else {
        println!("progress: {PLAN} not found (not part of this checkout); nothing to report");
        return Ok(());
    };
    let plan = parse(&text);
    print!("{}", report(&plan));
    let problems = check(&plan);
    if problems.is_empty() {
        Ok(())
    } else {
        for p in &problems {
            println!("progress: {p}");
        }
        Err(format!(
            "progress: {} inconsistency(ies) between the steps and §5",
            problems.len()
        )
        .into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
# Plan

## R0 — Correctness

### R0.S01 — Fault plumbing
text
### R0.S02 — Flush retry

## R1 — Polish

### R1.S01 — Closures

## 5. Progress

- [x] R0.S01 · [ ] R0.S02
- [ ] R1.S01

## 6. Deviations

- [x] R9.S99 is not a checkbox here (outside §5)
";

    #[test]
    fn parses_phases_steps_and_checklist() {
        let p = parse(SAMPLE);
        assert_eq!(p.phases.len(), 2);
        assert_eq!(p.phases[0].id, "R0");
        assert_eq!(p.phases[0].title, "Correctness");
        assert_eq!(p.phases[0].steps[1].title, "Flush retry");
        assert_eq!(p.phases[1].steps.len(), 1);
        assert_eq!(
            p.checklist,
            [
                ("R0.S01".into(), true),
                ("R0.S02".into(), false),
                ("R1.S01".into(), false)
            ]
        );
        assert!(check(&p).is_empty());
    }

    #[test]
    fn report_lists_progress_and_next_step() {
        let r = report(&parse(SAMPLE));
        assert!(r.contains("1/2"), "{r}");
        assert!(r.contains("next: R0.S02 — Flush retry"), "{r}");
        assert!(r.contains("1/3 steps done"), "{r}");
        assert!(r.contains("next step: R0.S02"), "{r}");
    }

    #[test]
    fn check_finds_missing_extra_and_duplicate_entries() {
        let text = SAMPLE
            .replace("- [ ] R1.S01", "- [ ] R1.S02 · [x] R0.S01")
            .replace(
                "### R1.S01 — Closures",
                "### R1.S01 — Closures\n### R1.S01 — Again",
            );
        let problems = check(&parse(&text));
        assert!(
            problems.iter().any(|p| p.contains("R1.S01 has 2 headings")),
            "{problems:?}"
        );
        assert!(
            problems.iter().any(|p| p.contains("R1.S01 has no checkbox")),
            "{problems:?}"
        );
        assert!(
            problems.iter().any(|p| p.contains("R1.S02 has no step heading")),
            "{problems:?}"
        );
        assert!(
            problems.iter().any(|p| p.contains("2 checkboxes for R0.S01")),
            "{problems:?}"
        );
    }

    #[test]
    fn the_real_plan_is_consistent_when_present() {
        if let Ok(text) = std::fs::read_to_string(plan_path()) {
            let plan = parse(&text);
            assert!(!plan.phases.is_empty());
            assert_eq!(check(&plan), Vec::<String>::new());
        }
    }
}
