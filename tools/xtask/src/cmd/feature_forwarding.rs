//! Feature forwarding (run by `cargo xtask layers`).
//!
//! A crate that offers a cross-cutting feature (`std`, `log`, `defmt`, `async`) must forward it
//! to **every** workspace crate it depends on that has the same feature — directly, not by
//! relying on another dependency to forward it (that indirection is how a crate ends up built
//! without `defmt` while its siblings have it). The facade `twine` is held to a stricter rule:
//! *every* one of its features is forwarded to every dependency with a feature of that name,
//! and the features users pick on the facade (fonts, platforms, drivers) exist there for every
//! feature of the crate that implements them.
//!
//! A dependency whose declaration always enables the feature (`features = ["async"]`) needs no
//! forwarding. Dev-dependencies are exempt (tests pick their own features).

use serde_json::{Map, Value};

use super::layers::EXEMPT;

/// Features every crate forwards to its dependencies.
pub const PROPAGATED: &[&str] = &["std", "log", "defmt", "async"];

/// The facade crate.
const FACADE: &str = "twine";

/// Features the facade offers for every feature of a crate, under the same name:
/// `(crate, feature-name prefix, features not offered)`.
const MIRRORED: &[(&str, &str, &[&str])] = &[
    // Fonts and font bundles.
    ("twine-assets", "", &["default"]),
    // Platform implementations.
    ("twine-hal", "platform-", &[]),
];

/// Driver features the facade does not offer as `drivers-<name>` (logging and `async` are
/// forwarded instead; `all` would also need every colour; `testkit` is for tests).
const DRIVERS_NOT_OFFERED: &[&str] = &["default", "all", "async", "defmt", "log", "std", "testkit"];

/// Whether `list` (a feature's enable list) forwards `feature` to `dep` (`dep/feature` or
/// `dep?/feature`).
fn forwards(list: &[Value], dep: &str, feature: &str) -> bool {
    let plain = format!("{dep}/{feature}");
    let weak = format!("{dep}?/{feature}");
    list.iter()
        .filter_map(Value::as_str)
        .any(|item| item == plain || item == weak)
}

fn features(pkg: &Value) -> Option<&Map<String, Value>> {
    pkg["features"].as_object()
}

/// Checks `cargo metadata --no-deps` JSON; returns every violation as a message.
pub fn check_metadata(json: &str) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let meta: Value = serde_json::from_str(json)?;
    let packages = meta["packages"]
        .as_array()
        .ok_or("metadata: missing `packages`")?;
    let find = |name: &str| packages.iter().find(|p| p["name"].as_str() == Some(name));
    let mut errors = Vec::new();
    for pkg in packages {
        let name = pkg["name"].as_str().ok_or("metadata: package without name")?;
        if EXEMPT.contains(&name) {
            continue;
        }
        let Some(own) = features(pkg) else { continue };
        let deps = pkg["dependencies"].as_array().map_or(&[][..], Vec::as_slice);
        for dep in deps {
            if dep["kind"].as_str() == Some("dev") {
                continue;
            }
            let Some(dep_name) = dep["name"].as_str() else {
                continue;
            };
            let Some(dep_features) = find(dep_name).and_then(features) else {
                continue; // not a workspace crate
            };
            let always_on = dep["features"].as_array().map_or(&[][..], Vec::as_slice);
            // The facade forwards every one of its features; other crates the cross-cutting ones.
            let checked: Vec<&str> = if name == FACADE {
                own.keys()
                    .map(String::as_str)
                    .filter(|f| *f != "default")
                    .collect()
            } else {
                PROPAGATED.to_vec()
            };
            for feature in checked {
                let Some(list) = own.get(feature).and_then(Value::as_array) else {
                    continue;
                };
                if !dep_features.contains_key(feature)
                    || always_on.iter().any(|f| f.as_str() == Some(feature))
                    || forwards(list, dep_name, feature)
                {
                    continue;
                }
                let weak = if dep["optional"].as_bool() == Some(true) {
                    "?"
                } else {
                    ""
                };
                errors.push(format!(
                    "`{name}/{feature}` must forward to `{dep_name}`: add \"{dep_name}{weak}/{feature}\""
                ));
            }
        }
    }
    if let Some(facade) = find(FACADE).and_then(features) {
        for (krate, prefix, skipped) in MIRRORED {
            let Some(theirs) = find(krate).and_then(features) else {
                continue;
            };
            for feature in theirs.keys() {
                if !feature.starts_with(prefix) || skipped.contains(&feature.as_str()) {
                    continue;
                }
                let ok = facade
                    .get(feature)
                    .and_then(Value::as_array)
                    .is_some_and(|list| forwards(list, krate, feature));
                if !ok {
                    errors.push(format!(
                        "`{FACADE}` needs feature `{feature}` forwarding to \"{krate}/{feature}\""
                    ));
                }
            }
        }
        if let Some(drivers) = find("twine-drivers").and_then(features) {
            for feature in drivers.keys() {
                if DRIVERS_NOT_OFFERED.contains(&feature.as_str()) {
                    continue;
                }
                let facade_feature = format!("drivers-{feature}");
                let ok = facade
                    .get(&facade_feature)
                    .and_then(Value::as_array)
                    .is_some_and(|list| {
                        forwards(list, "twine-drivers", feature)
                            && list.iter().any(|v| v.as_str() == Some("drivers"))
                    });
                if !ok {
                    errors.push(format!(
                        "`{FACADE}` needs `{facade_feature} = [\"drivers\", \"twine-drivers/{feature}\", ..]`"
                    ));
                }
            }
        }
    }
    Ok(errors)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkg(name: &str, features: &Value, deps: &[(&str, Option<&str>, bool, &[&str])]) -> Value {
        let deps: Vec<Value> = deps
            .iter()
            .map(|(d, kind, optional, always)| {
                serde_json::json!({ "name": d, "kind": kind, "optional": optional, "features": always })
            })
            .collect();
        serde_json::json!({ "name": name, "features": features, "dependencies": deps })
    }

    fn check(pkgs: &[Value]) -> Vec<String> {
        check_metadata(&serde_json::json!({ "packages": pkgs }).to_string()).unwrap()
    }

    #[test]
    fn accepts_direct_forwarding() {
        let e = check(&[
            pkg("twine-core", &serde_json::json!({ "log": [], "std": [] }), &[]),
            pkg(
                "twine-text",
                &serde_json::json!({ "log": ["twine-core/log"], "std": ["twine-core?/std"] }),
                &[("twine-core", None, true, &[])],
            ),
        ]);
        assert_eq!(e, Vec::<String>::new());
    }

    #[test]
    fn regression_indirect_forwarding_is_rejected() {
        // `twine-view/defmt` reaches `twine-core` only through `twine-text`: the R2 bug pattern.
        let e = check(&[
            pkg("twine-core", &serde_json::json!({ "defmt": [] }), &[]),
            pkg(
                "twine-text",
                &serde_json::json!({ "defmt": ["twine-core/defmt"] }),
                &[("twine-core", None, false, &[])],
            ),
            pkg(
                "twine-view",
                &serde_json::json!({ "defmt": ["twine-text/defmt"] }),
                &[("twine-core", None, false, &[]), ("twine-text", None, false, &[])],
            ),
        ]);
        assert_eq!(e.len(), 1, "{e:?}");
        assert!(e[0].contains("twine-view/defmt") && e[0].contains("\"twine-core/defmt\""));
    }

    #[test]
    fn always_enabled_dev_and_featureless_dependencies_are_exempt() {
        let e = check(&[
            pkg("twine-hal", &serde_json::json!({ "async": [] }), &[]),
            pkg("twine-assets", &serde_json::json!({}), &[]),
            pkg(
                "twine-embassy",
                &serde_json::json!({ "async": [], "log": [] }),
                &[
                    ("twine-hal", None, false, &["async"]),
                    ("twine-assets", None, false, &[]),
                    ("twine-testing", Some("dev"), false, &[]),
                ],
            ),
            pkg("twine-testing", &serde_json::json!({ "log": [] }), &[]),
        ]);
        assert_eq!(e, Vec::<String>::new());
    }

    #[test]
    fn optional_dependency_hint_uses_weak_syntax() {
        let e = check(&[
            pkg("twine-fs", &serde_json::json!({ "log": [] }), &[]),
            pkg(
                "twine-engine",
                &serde_json::json!({ "log": [] }),
                &[("twine-fs", None, true, &[])],
            ),
        ]);
        assert_eq!(e.len(), 1);
        assert!(e[0].contains("\"twine-fs?/log\""), "{e:?}");
    }

    #[test]
    fn facade_forwards_every_feature_and_mirrors_fonts_platforms_and_drivers() {
        let e = check(&[
            pkg("twine-render", &serde_json::json!({ "color-l8": [] }), &[]),
            pkg(
                "twine-assets",
                &serde_json::json!({ "default": [], "unscii-8": [], "fonts-mono": [] }),
                &[],
            ),
            pkg(
                "twine-hal",
                &serde_json::json!({ "platform-std": [], "async": [] }),
                &[],
            ),
            pkg(
                "twine-drivers",
                &serde_json::json!({ "all": [], "spi": [], "gt911": [] }),
                &[],
            ),
            pkg(
                FACADE,
                &serde_json::json!({
                    "default": [],
                    "color-l8": [],
                    "unscii-8": ["twine-assets/unscii-8"],
                    "drivers": ["dep:twine-drivers"],
                    "drivers-spi": ["drivers", "twine-drivers/spi"],
                }),
                &[
                    ("twine-render", None, false, &[]),
                    ("twine-assets", None, false, &[]),
                    ("twine-hal", None, false, &[]),
                    ("twine-drivers", None, true, &[]),
                ],
            ),
        ]);
        assert_eq!(e.len(), 4, "{e:?}");
        assert!(
            e.iter()
                .any(|m| m.contains("twine/color-l8") && m.contains("twine-render/color-l8"))
        );
        assert!(e.iter().any(|m| m.contains("`fonts-mono`")));
        assert!(e.iter().any(|m| m.contains("`platform-std`")));
        assert!(e.iter().any(|m| m.contains("drivers-gt911")));
    }
}
