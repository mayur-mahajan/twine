//! `cargo xtask layers`: enforces Twine's crate layering (table [`LAYERS`] below).
//!
//! Every normal and build dependency between two twine crates must point to a strictly lower
//! layer (or be an explicitly allowed same-layer pair). Dev-dependencies are exempt
//! (tests may use higher-level crates such as `twine-testing`).

use serde_json::Value;

use crate::util::{R, cargo, output};

/// Crate → layer. A crate may only depend on crates in lower layers.
pub const LAYERS: &[(&str, u8)] = &[
    ("twine-core", 0),
    ("twine-hal", 1),
    ("twine-reactive", 1),
    ("twine-anim", 1),
    ("twine-render", 2),
    ("twine-text", 3),
    ("twine-image", 3),
    ("twine-vector", 3),
    ("twine-fs", 3),
    ("twine-style", 4),
    ("twine-layout", 5),
    ("twine-engine", 6),
    ("twine-theme", 7),
    ("twine-widgets", 7),
    ("twine-widgets-ext", 8),
    ("twine-view", 9),
    ("twine-extra", 10),
    ("twine-lottie", 10),
    ("twine-assets", 4),
    ("twine", 11),
    ("twine-drivers", 11),
    ("twine-embassy", 11),
    ("twine-accel-stm32", 11),
    ("twine-embedded-graphics", 11),
    ("twine-demos", 12),
    ("twine-sim", 12),
    ("twine-testing", 12),
];

/// Same-layer dependencies that are allowed: `(dependent, dependency)`.
pub const SAME_LAYER_ALLOWED: &[(&str, &str)] = &[("twine-widgets", "twine-theme")];

/// Workspace members that are not part of the layered library (tools, examples).
pub const EXEMPT: &[&str] = &["xtask", "twine-cli", "twine-bench", "twine-examples"];

fn layer(name: &str) -> Option<u8> {
    LAYERS.iter().find(|(n, _)| *n == name).map(|(_, l)| *l)
}

/// Checks `cargo metadata --no-deps` JSON; returns every violation as a message.
pub fn check_metadata(json: &str) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let meta: Value = serde_json::from_str(json)?;
    let packages = meta["packages"]
        .as_array()
        .ok_or("metadata: missing `packages`")?;
    let mut errors = Vec::new();
    for pkg in packages {
        let name = pkg["name"].as_str().ok_or("metadata: package without name")?;
        if EXEMPT.contains(&name) {
            continue;
        }
        let Some(own) = layer(name) else {
            errors.push(format!(
                "crate `{name}` is not in the layer table (add it to LAYERS in tools/xtask/src/cmd/layers.rs)"
            ));
            continue;
        };
        let deps = pkg["dependencies"].as_array().map_or(&[][..], Vec::as_slice);
        for dep in deps {
            if dep["kind"].as_str() == Some("dev") {
                continue;
            }
            let Some(dep_name) = dep["name"].as_str() else {
                continue;
            };
            if name == "twine-engine" && dep_name == "twine-reactive" {
                errors.push(
                    "`twine-engine` must not depend on `twine-reactive` (the engine \
                     is usable imperatively; reactivity is layered on top in twine-view)"
                        .to_string(),
                );
                continue;
            }
            let Some(dep_layer) = layer(dep_name) else {
                if dep_name.starts_with("twine") || EXEMPT.contains(&dep_name) {
                    errors.push(format!("`{name}` depends on non-library crate `{dep_name}`"));
                }
                continue;
            };
            let allowed = dep_layer < own || SAME_LAYER_ALLOWED.contains(&(name, dep_name));
            if !allowed {
                errors.push(format!(
                    "`{name}` (layer {own}) must not depend on `{dep_name}` (layer {dep_layer}); \
                     lower layers must never depend on higher ones"
                ));
            }
        }
    }
    Ok(errors)
}

/// Runs the check on the current workspace.
pub fn run() -> R {
    let json = output(cargo().args(["metadata", "--format-version", "1", "--no-deps"]))?;
    let errors = check_metadata(&json)?;
    if errors.is_empty() {
        println!("layers: ok ({} layered crates)", LAYERS.len());
        Ok(())
    } else {
        for e in &errors {
            println!("layers: {e}");
        }
        Err(format!("layers: {} violation(s)", errors.len()).into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Deps<'a> = &'a [(&'a str, Option<&'a str>)];

    fn meta(pkgs: &[(&str, Deps<'_>)]) -> String {
        let packages: Vec<Value> = pkgs
            .iter()
            .map(|(name, deps)| {
                let deps: Vec<Value> = deps
                    .iter()
                    .map(|(d, kind)| serde_json::json!({ "name": d, "kind": kind }))
                    .collect();
                serde_json::json!({ "name": name, "dependencies": deps })
            })
            .collect();
        serde_json::json!({ "packages": packages }).to_string()
    }

    #[test]
    fn accepts_downward_dep() {
        let j = meta(&[
            ("twine-core", &[("log", None)]),
            ("twine-render", &[("twine-core", None)]),
            (
                "twine-widgets",
                &[("twine-theme", None), ("twine-core", Some("build"))],
            ),
            ("xtask", &[("twine-view", None)]),
        ]);
        assert_eq!(check_metadata(&j).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn rejects_upward_dep() {
        let j = meta(&[("twine-core", &[("twine-view", None)])]);
        let e = check_metadata(&j).unwrap();
        assert_eq!(e.len(), 1);
        assert!(
            e[0].contains("twine-core") && e[0].contains("twine-view"),
            "{e:?}"
        );
        let j = meta(&[("twine-theme", &[("twine-widgets", None)])]);
        assert_eq!(check_metadata(&j).unwrap().len(), 1);
    }

    #[test]
    fn rejects_engine_on_reactive() {
        let j = meta(&[("twine-engine", &[("twine-reactive", None)])]);
        let e = check_metadata(&j).unwrap();
        assert_eq!(e.len(), 1);
        assert!(e[0].contains("must not depend on `twine-reactive`"));
    }

    #[test]
    fn ignores_dev_dependencies() {
        let j = meta(&[("twine-core", &[("twine-testing", Some("dev"))])]);
        assert!(check_metadata(&j).unwrap().is_empty());
    }

    #[test]
    fn rejects_unknown_crate() {
        let j = meta(&[("twine-mystery", &[])]);
        let e = check_metadata(&j).unwrap();
        assert_eq!(e.len(), 1);
        assert!(e[0].contains("twine-mystery"));
    }
}
