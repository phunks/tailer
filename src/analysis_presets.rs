//! Persistent editor settings, not compiled programs or analysis results.
use serde_json::{Value, json};

pub const MAX_PRESETS: usize = 100;

pub fn settings(value: &Value) -> Result<Value, String> {
    let object = value
        .as_object()
        .ok_or("Preset settings must be an object")?;
    let mut clean = json!({});
    for (key, default, limit) in [
        ("format", "regex", 16),
        ("delimiter", ",", 128),
        ("filter", "", 16 * 1024),
        ("filterMode", "contains", 16),
        ("regex", "", 16 * 1024),
        ("script", "", 64 * 1024),
        ("query", "count()", 4096),
    ] {
        let text = match object.get(key) {
            None => default,
            Some(Value::String(s)) => s,
            _ => return Err(format!("{key} must be a string")),
        };
        if text.len() > limit {
            return Err(format!("Preset {key} exceeds {limit} bytes"));
        }
        clean[key] = json!(text);
    }
    if !["regex", "delimiter", "whitespace", "json"].contains(&clean["format"].as_str().unwrap()) {
        return Err("Unknown extraction format".into());
    }
    if !["contains", "prefix", "regex"].contains(&clean["filterMode"].as_str().unwrap()) {
        return Err("Unknown line filter mode".into());
    }
    for (key, default) in [
        ("ignoreCase", false),
        ("exclude", false),
        ("useRoto", true),
        ("mergeFirstColumn", false),
    ] {
        clean[key] = json!(match object.get(key) {
            None => default,
            Some(Value::Bool(b)) => *b,
            _ => return Err(format!("{key} must be a boolean")),
        });
    }
    let fields = object.get("fields").cloned().unwrap_or_else(|| json!([]));
    let input = crate::analysis::InputConfig::from_value(&json!({"fields":fields}))?;
    let mut mappings = Vec::new();
    for field in input.fields {
        if field.name.is_empty() || field.name.len() > 64 || field.source.len() > 1024 {
            return Err("Invalid preset field name or source length".into());
        }
        mappings.push(json!({"name":field.name,"source":field.source,
            "type":if field.numeric { "number" } else { "text" },"required":field.required}));
    }
    clean["fields"] = json!(mappings);
    // Save bounded drafts without JIT compilation; Apply validates executable settings.
    Ok(clean)
}

pub fn sanitize(value: &Value) -> Value {
    let mut result: Vec<Value> = Vec::new();
    if let Some(rows) = value.as_array() {
        for row in rows.iter().take(MAX_PRESETS) {
            let id = row["id"].as_str().unwrap_or_default();
            let name = row["name"].as_str().unwrap_or_default().trim();
            if !id.starts_with("user-")
                || id.len() > 128
                || name.is_empty()
                || name.len() > 256
                || result.iter().any(|p| p["id"] == id || p["name"] == name)
            {
                continue;
            }
            if let Ok(settings) = settings(&row["settings"]) {
                result.push(json!({"id":id,"name":name,"settings":settings}));
            }
        }
    }
    json!(result)
}

pub fn defaults() -> Vec<Value> {
    let mut presets: Vec<Value> = [("builtin-apache", "Apache CLF + %D", crate::analysis::APACHE_REGEX, crate::analysis::APACHE_SCRIPT, "path, avg(duration_ms), count()")]
        .into_iter().map(|(id, name, regex, script, query)| {
            json!({"id":id,"name":name,"builtin":true,
                "settings":settings(&json!({"regex":regex,"script":script,"query":query})).unwrap()})
        }).collect();
    let mut checkpoint: Value =
        serde_json::from_str(include_str!("../docs/examples/pg-checkpoint.json"))
            .expect("Built-in PostgreSQL preset must be valid JSON");
    checkpoint["settings"] =
        settings(&checkpoint["settings"]).expect("Valid PostgreSQL preset settings");
    presets.push(checkpoint);
    presets
}

pub fn upsert(
    existing: &Value,
    id: &str,
    name: &str,
    value: &Value,
) -> Result<(Value, String), String> {
    let name = name.trim();
    if name.is_empty() || name.len() > 256 {
        return Err("Preset name must be 1–256 bytes".into());
    }
    let settings = settings(value)?;
    let mut rows = sanitize(existing).as_array().unwrap().clone();
    if !id.is_empty() && !rows.iter().any(|p| p["id"] == id) {
        return Err("Cannot overwrite a default or missing preset".into());
    }
    if defaults().iter().any(|p| p["name"] == name)
        || rows.iter().any(|p| p["name"] == name && p["id"] != id)
    {
        return Err("A preset with this name already exists".into());
    }
    let id = if id.is_empty() {
        if rows.len() >= MAX_PRESETS {
            return Err("Preset limit reached (100)".into());
        }
        let mut index = 1;
        while rows.iter().any(|p| p["id"] == format!("user-{index}")) {
            index += 1;
        }
        format!("user-{index}")
    } else {
        id.into()
    };
    let row = json!({"id":id,"name":name,"settings":settings});
    if let Some(index) = rows.iter().position(|p| p["id"] == id) {
        rows[index] = row;
    } else {
        rows.push(row);
    }
    Ok((json!(rows), id))
}

pub fn remove(existing: &Value, id: &str) -> Result<Value, String> {
    let mut rows = sanitize(existing).as_array().unwrap().clone();
    let index = rows
        .iter()
        .position(|p| p["id"] == id)
        .ok_or("Cannot delete a default or missing preset")?;
    rows.remove(index);
    Ok(json!(rows))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_crud_and_whitelist() {
        let defaults = defaults();
        assert_eq!(defaults.len(), 2);
        assert_eq!(defaults[0]["id"], "builtin-apache");
        assert_eq!(defaults[1]["name"], "pg_checkpoint");
        assert!(defaults.iter().all(|p| p["id"] != "builtin-access"));
        let mut settings = defaults[0]["settings"].clone();
        settings["password"] = json!("secret");
        settings["fields"] =
            json!([{"name":"total","source":"/total","type":"number","password":"secret"}]);
        let (rows, id) = upsert(&json!([]), "", " My preset ", &settings).unwrap();
        assert_eq!(rows[0]["name"], "My preset");
        assert!(!rows.to_string().contains("secret"));
        assert_eq!(sanitize(&rows), rows);
        assert!(upsert(&rows, "", "My preset", &settings).is_err());
        assert!(upsert(&rows, "", "Apache CLF + %D", &settings).is_err());
        assert!(upsert(&rows, "builtin-access", "other", &settings).is_err());
        assert!(remove(&rows, "builtin-apache").is_err());
        settings["query"] = json!("avg(total)");
        let (updated, same_id) = upsert(&rows, &id, "Renamed", &settings).unwrap();
        assert_eq!(same_id, id);
        assert_eq!(updated.as_array().unwrap().len(), 1);
        assert_eq!(updated[0]["settings"]["query"], "avg(total)");
        assert_eq!(remove(&updated, &id).unwrap(), json!([]));
    }

    #[test]
    fn invalid_and_oversized_settings_are_rejected() {
        assert_eq!(settings(&json!({})).unwrap()["mergeFirstColumn"], false);
        for value in [
            json!({"script":"x".repeat(65537)}),
            json!({"fields":"bad"}),
            json!({"query":false}),
            json!({"format":"unknown"}),
            json!({"useRoto":"yes"}),
            json!({"mergeFirstColumn":"yes"}),
        ] {
            assert!(settings(&value).is_err());
        }
        assert_eq!(
            sanitize(&json!([{"id":"builtin-access","name":"fake","settings":{}}])),
            json!([])
        );
        // Invalid executable expressions remain savable as drafts.
        assert!(settings(&json!({"script":"not valid roto","query":"avg()","regex":"["})).is_ok());
        let rows = json!(
            (0..101)
                .map(
                    |i| json!({"id":format!("user-{i}"),"name":format!("preset {i}"),"settings":{}})
                )
                .collect::<Vec<_>>()
        );
        assert_eq!(sanitize(&rows).as_array().unwrap().len(), 100);
        assert!(upsert(&rows, "", "overflow", &json!({})).is_err());
    }

    #[test]
    fn every_editor_setting_round_trips_without_running_scripts() {
        let original = json!({"format":"json", "delimiter":"\t", "filter":"checkpoint",
            "filterMode":"prefix", "ignoreCase":true, "exclude":true, "regex":"[",
            "useRoto":false, "mergeFirstColumn":true, "script":"unfinished script", "query":"avg(total), count()",
            "fields":[{"name":"total","source":"/timing/total","type":"number","required":false}]});
        assert_eq!(settings(&original).unwrap(), original);
        let (saved, _) = upsert(&json!([]), "", "JSON draft", &original).unwrap();
        let disk = serde_json::to_vec(&saved).unwrap();
        let restored: Value = serde_json::from_slice(&disk).unwrap();
        assert_eq!(sanitize(&restored)[0]["settings"], original);
    }
}
