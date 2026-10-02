//! Panel feature → colour feature coupling (run by `cargo xtask layers`).
//!
//! Every display driver of `twine-drivers` draws in one pixel format, which the renderer must be
//! compiled for (`twine/color-*`). `twine-drivers` depends only on `twine-hal`, so its own
//! features cannot enable it; the `twine` facade's `drivers-<panel>` features do. The table
//! `[package.metadata.twine.panel-color]` of `twine-drivers` names the colour feature of each
//! panel (checked against the drivers' formats by `twine-drivers/tests/panel_formats.rs`); this
//! check makes sure that, for every entry, the facade has `drivers-<panel>` enabling `drivers`,
//! `twine-drivers/<panel>` and that colour feature, and that every other `drivers-<name>` of the
//! facade names a `twine-drivers` feature (buses, touch and input drivers; that each of those
//! has its facade feature is checked by [`feature_forwarding`](super::feature_forwarding)).

use serde_json::Value;

/// The crate with the display drivers.
const DRIVERS: &str = "twine-drivers";
/// The facade crate.
const FACADE: &str = "twine";

/// Checks `cargo metadata --no-deps` JSON; returns every violation as a message.
pub fn check_metadata(json: &str) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let meta: Value = serde_json::from_str(json)?;
    let packages = meta["packages"]
        .as_array()
        .ok_or("metadata: missing `packages`")?;
    let find = |name: &str| packages.iter().find(|p| p["name"].as_str() == Some(name));
    let (Some(drivers), Some(facade)) = (find(DRIVERS), find(FACADE)) else {
        return Ok(vec![format!(
            "`{DRIVERS}` or `{FACADE}` is missing from the workspace"
        )]);
    };
    let mut errors = Vec::new();
    let Some(table) = drivers["metadata"]["twine"]["panel-color"].as_object() else {
        errors.push(format!(
            "`{DRIVERS}` has no [package.metadata.twine.panel-color] table"
        ));
        return Ok(errors);
    };
    let driver_features = drivers["features"].as_object().ok_or("metadata: features")?;
    let facade_features = facade["features"].as_object().ok_or("metadata: features")?;
    let enables = |feature: &str, item: &str| {
        facade_features
            .get(feature)
            .and_then(Value::as_array)
            .is_some_and(|list| list.iter().any(|v| v.as_str() == Some(item)))
    };
    for (panel, color) in table {
        let Some(color) = color.as_str() else {
            errors.push(format!("panel-color `{panel}`: the value must be a feature name"));
            continue;
        };
        if !driver_features.contains_key(panel) {
            errors.push(format!("panel-color `{panel}` is not a feature of `{DRIVERS}`"));
        }
        if !facade_features.contains_key(color) {
            errors.push(format!(
                "panel-color `{panel}` = `{color}`: `{FACADE}` has no feature `{color}`"
            ));
        }
        let feature = format!("drivers-{panel}");
        if !facade_features.contains_key(&feature) {
            errors.push(format!(
                "`{FACADE}` needs `{feature} = [\"drivers\", \"{DRIVERS}/{panel}\", \"{color}\"]`"
            ));
            continue;
        }
        for item in [
            "drivers".to_string(),
            format!("{DRIVERS}/{panel}"),
            color.to_string(),
        ] {
            if !enables(&feature, &item) {
                errors.push(format!("`{FACADE}/{feature}` must enable `{item}`"));
            }
        }
    }
    for feature in facade_features.keys() {
        if let Some(name) = feature.strip_prefix("drivers-")
            && !driver_features.contains_key(name)
        {
            errors.push(format!(
                "`{FACADE}/{feature}`: `{name}` is not a feature of `{DRIVERS}`"
            ));
        }
    }
    Ok(errors)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(table: &Value, facade: &Value) -> String {
        serde_json::json!({ "packages": [
            {
                "name": DRIVERS,
                "features": { "ili9341": [], "ssd1306": [] },
                "metadata": { "twine": { "panel-color": table } },
            },
            { "name": FACADE, "features": facade },
        ]})
        .to_string()
    }

    fn table() -> Value {
        serde_json::json!({ "ili9341": "color-rgb565-swapped", "ssd1306": "color-i1" })
    }

    #[test]
    fn accepts_coupled_features() {
        let facade = serde_json::json!({
            "color-rgb565-swapped": [], "color-i1": [], "drivers": ["dep:twine-drivers"],
            "drivers-ili9341": ["drivers", "twine-drivers/ili9341", "color-rgb565-swapped"],
            "drivers-ssd1306": ["drivers", "twine-drivers/ssd1306", "color-i1"],
        });
        assert_eq!(
            check_metadata(&meta(&table(), &facade)).unwrap(),
            Vec::<String>::new()
        );
    }

    #[test]
    fn rejects_a_panel_feature_without_its_colour() {
        let facade = serde_json::json!({
            "color-rgb565-swapped": [], "color-i1": [], "drivers": [],
            "drivers-ili9341": ["drivers", "twine-drivers/ili9341"],
            "drivers-ssd1306": ["drivers", "twine-drivers/ssd1306", "color-rgb565-swapped"],
        });
        let e = check_metadata(&meta(&table(), &facade)).unwrap();
        assert_eq!(e.len(), 2, "{e:?}");
        assert!(
            e.iter()
                .any(|m| m.contains("drivers-ili9341") && m.contains("color-rgb565-swapped"))
        );
        assert!(
            e.iter()
                .any(|m| m.contains("drivers-ssd1306") && m.contains("color-i1"))
        );
    }

    #[test]
    fn accepts_non_panel_driver_features() {
        let mut m: Value = serde_json::from_str(&meta(
            &table(),
            &serde_json::json!({
                "color-rgb565-swapped": [], "color-i1": [], "drivers": [],
                "drivers-ili9341": ["drivers", "twine-drivers/ili9341", "color-rgb565-swapped"],
                "drivers-ssd1306": ["drivers", "twine-drivers/ssd1306", "color-i1"],
                "drivers-gt911": ["drivers", "twine-drivers/gt911"],
            }),
        ))
        .unwrap();
        m["packages"][0]["features"]["gt911"] = serde_json::json!([]);
        assert_eq!(check_metadata(&m.to_string()).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn rejects_missing_and_unknown_panel_features() {
        let facade = serde_json::json!({
            "color-rgb565-swapped": [], "color-i1": [], "drivers": [],
            "drivers-ili9341": ["drivers", "twine-drivers/ili9341", "color-rgb565-swapped"],
            "drivers-st7789": ["drivers", "twine-drivers/st7789", "color-rgb565-swapped"],
        });
        let e = check_metadata(&meta(&table(), &facade)).unwrap();
        assert_eq!(e.len(), 2, "{e:?}");
        assert!(e.iter().any(|m| m.contains("drivers-ssd1306")));
        assert!(e.iter().any(|m| m.contains("drivers-st7789")));
    }

    #[test]
    fn rejects_unknown_driver_feature_and_colour() {
        let table = serde_json::json!({ "ili9999": "color-rgb999" });
        let facade = serde_json::json!({
            "drivers": [], "drivers-ili9999": ["drivers", "twine-drivers/ili9999", "color-rgb999"],
        });
        let e = check_metadata(&meta(&table, &facade)).unwrap();
        // Unknown panel (table entry and facade feature) and unknown colour.
        assert_eq!(e.len(), 3, "{e:?}");
    }
}
