use qtbridge::qobject;
use serde_json::{Value, json};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub fn config_directory_for(
    os: &str,
    get: impl Fn(&str) -> Option<PathBuf>,
) -> io::Result<PathBuf> {
    let home = || {
        get("HOME")
            .filter(|p| p.is_absolute())
            .ok_or_else(|| io::Error::other("HOME is not set"))
    };
    let base = match os {
        "macos" => home()?.join("Library/Application Support"),
        "windows" => get("APPDATA")
            .filter(|p| !p.as_os_str().is_empty())
            .ok_or_else(|| io::Error::other("APPDATA is not set"))?,
        _ => match get("XDG_CONFIG_HOME").filter(|p| p.is_absolute()) {
            Some(path) => path,
            None => home()?.join(".config"),
        },
    };
    Ok(base.join("Tailer"))
}

pub fn config_directory() -> io::Result<PathBuf> {
    config_directory_for(std::env::consts::OS, |name| {
        std::env::var_os(name).map(PathBuf::from)
    })
}

pub fn cache_directory() -> io::Result<PathBuf> {
    let home = || {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| io::Error::other("HOME is not set"))
    };
    let base = if cfg!(target_os = "macos") {
        home()?.join("Library/Caches")
    } else if cfg!(target_os = "windows") {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .ok_or_else(|| io::Error::other("LOCALAPPDATA is not set"))?
    } else {
        std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or(home()?.join(".cache"))
    };
    Ok(base.join("Tailer/sessions"))
}

fn bounded_int(value: &Value, default: u64, min: u64, max: u64) -> Value {
    let number = value.as_u64().or_else(|| {
        value
            .as_f64()
            .filter(|n| n.is_finite() && *n >= 0.0)
            .map(|n| n as u64)
    });
    json!(number.unwrap_or(default).clamp(min, max))
}

/// Whitelist only non-secret configuration. Never serialize arbitrary QML data.
pub fn sanitize(value: &Value) -> Value {
    let mut groups = Vec::new();
    if let Some(input) = value["bookmarkGroups"].as_array() {
        for entry in input.iter().take(100) {
            let id = entry["id"].as_str().unwrap_or_default();
            let name = entry["name"].as_str().unwrap_or_default().trim();
            if id.is_empty()
                || id.len() > 128
                || name.is_empty()
                || groups.iter().any(|g: &Value| g["id"] == id)
            {
                continue;
            }
            groups.push(
                json!({"id":id, "name":name.chars().take(256).collect::<String>(),
                "parentId":entry["parentId"].as_str().unwrap_or_default(),
                "expanded":entry["expanded"].as_bool().unwrap_or(true)}),
            );
        }
    }
    // Only retain existing parents, and break cycles before exposing the tree to QML.
    for index in 0..groups.len() {
        let mut seen = vec![groups[index]["id"].as_str().unwrap().to_owned()];
        let mut parent = groups[index]["parentId"].as_str().unwrap().to_owned();
        while !parent.is_empty() {
            if seen.contains(&parent) {
                groups[index]["parentId"] = json!("");
                break;
            }
            seen.push(parent.clone());
            match groups.iter().find(|g| g["id"] == parent) {
                Some(group) => parent = group["parentId"].as_str().unwrap().to_owned(),
                None => {
                    groups[index]["parentId"] = json!("");
                    break;
                }
            }
        }
    }
    let mut connections = Vec::new();
    if let Some(input) = value["connections"].as_array() {
        for entry in input.iter().take(100) {
            let id = entry["id"].as_str().unwrap_or_default();
            if id.is_empty() || connections.iter().any(|c: &Value| c["id"] == id) {
                continue;
            }
            let mut clean = json!({});
            for field in ["id", "name", "host", "user", "key"] {
                clean[field] = json!(
                    entry[field]
                        .as_str()
                        .unwrap_or_default()
                        .chars()
                        .take(8192)
                        .collect::<String>()
                );
            }
            clean["port"] = bounded_int(&entry["port"], 0, 0, 65535);
            connections.push(clean);
        }
    }
    let mut tabs = Vec::new();
    let mut bookmarks = Vec::new();
    for collection in ["tabs", "bookmarks"] {
        if let Some(input) = value[collection].as_array() {
            for tab in input.iter().take(100) {
                if !tab.is_object() {
                    continue;
                }
                let mut clean = json!({});
                for name in [
                    "logPath",
                    "logTitle",
                    "host",
                    "user",
                    "key",
                    "filterText",
                    "searchText",
                    "trapText",
                ] {
                    clean[name] = json!(
                        tab[name]
                            .as_str()
                            .unwrap_or_default()
                            .chars()
                            .take(8192)
                            .collect::<String>()
                    );
                }
                clean["filterNumericJson"] = json!(
                    crate::numeric::sanitize(
                        &tab["filterNumericJson"]
                            .as_str()
                            .and_then(|s| serde_json::from_str::<Value>(s).ok())
                            .unwrap_or(Value::Null)
                    )
                    .to_string()
                );
                clean["runUser"] = json!(tab["runUser"].as_str().unwrap_or("root"));
                clean["analysisPresetId"] = json!(
                    tab["analysisPresetId"]
                        .as_str()
                        .filter(|id| !id.is_empty() && id.len() <= 128 && *id != "builtin-access")
                        .unwrap_or("builtin-apache")
                );
                // Old workspaces have no source field: migrate them as file tabs.
                clean["source"] = json!(tab["source"].as_str().unwrap_or("file"));
                let encoding = tab["encoding"].as_str().unwrap_or("UTF-8");
                clean["encoding"] = json!(if crate::encoding::ENCODINGS.contains(&encoding) {
                    encoding
                } else {
                    "UTF-8"
                });
                for name in [
                    "remote",
                    "filterRegex",
                    "filterCase",
                    "filterInvert",
                    "searchRegex",
                    "searchCase",
                    "trapCase",
                    "analysisMergeFirstColumn",
                    "syslogBadges",
                ] {
                    clean[name] = json!(tab[name].as_bool().unwrap_or(false));
                }
                clean["follow"] = json!(tab["follow"].as_bool().unwrap_or(true));
                clean["filterEnabled"] = json!(tab["filterEnabled"].as_bool().unwrap_or(true));
                // An explicitly empty list stays empty; only legacy tabs migrate.
                clean["trapRulesJson"] = json!(
                    crate::trap::sanitize_rules(
                        &tab["trapRulesJson"]
                            .as_str()
                            .and_then(|s| serde_json::from_str::<Value>(s).ok())
                            .unwrap_or_else(
                                || json!([{"text":tab["trapText"], "ignoreCase":tab["trapCase"]}])
                            )
                    )
                    .to_string()
                );
                // Notification state is transient and resets on restoration.
                clean["unread"] = json!(false);
                clean["alert"] = json!(false);
                clean["port"] = bounded_int(&tab["port"], 0, 0, 65535);
                clean["initial"] = bounded_int(&tab["initial"], 50, 0, 1000000);
                clean["capacity"] = bounded_int(&tab["capacity"], 512, 1, 10240);
                clean["elevation"] = bounded_int(&tab["elevation"], 0, 0, 2);
                let mut id = tab["connectionId"].as_str().unwrap_or_default().to_string();
                if clean["remote"] == true && id.is_empty() {
                    let existing = connections.iter().find(|c| {
                        ["host", "user", "port", "key"]
                            .iter()
                            .all(|f| c[*f] == clean[*f])
                    });
                    id = if let Some(existing) = existing {
                        existing["id"].as_str().unwrap().to_string()
                    } else {
                        let mut number = connections.len() + 1;
                        while connections
                            .iter()
                            .any(|c| c["id"] == format!("migrated-{number}"))
                        {
                            number += 1;
                        }
                        let id = format!("migrated-{number}");
                        connections.push(json!({"id":id,"name":clean["host"],"host":clean["host"],"user":clean["user"],"port":clean["port"],"key":clean["key"]}));
                        id
                    };
                }
                clean["connectionId"] = json!(id);
                for field in ["host", "user", "port", "key"] {
                    clean.as_object_mut().unwrap().remove(field);
                }
                if collection == "bookmarks" {
                    let group_id = tab["groupId"].as_str().unwrap_or_default();
                    clean["groupId"] = json!(if groups.iter().any(|g| g["id"] == group_id) {
                        group_id
                    } else {
                        ""
                    });
                    clean["bookmarkTitle"] = json!(
                        tab["bookmarkTitle"]
                            .as_str()
                            .filter(|title| !title.trim().is_empty())
                            .unwrap_or_else(|| {
                                if clean["logTitle"].as_str().unwrap().is_empty() {
                                    clean["logPath"].as_str().unwrap()
                                } else {
                                    clean["logTitle"].as_str().unwrap()
                                }
                            })
                            .chars()
                            .take(8192)
                            .collect::<String>()
                    );
                    bookmarks.push(clean);
                } else {
                    tabs.push(clean);
                }
            }
        }
    }
    let language = value["language"].as_str().unwrap_or_default();
    let appearance = value["appearance"].as_str().unwrap_or("auto");
    json!({"version": 2, "appearance": if ["auto", "light", "dark"].contains(&appearance) { appearance } else { "auto" }, "language": if crate::i18n::LANGUAGES.contains(&language) { language } else { "" }, "connections":connections, "initialLines": bounded_int(&value["initialLines"], 50, 0, 1000000),
        "capacityMiB": bounded_int(&value["capacityMiB"], 512, 1, 10240),
        "currentIndex": bounded_int(&value["currentIndex"], 0, 0, tabs.len().saturating_sub(1) as u64),
        "width": bounded_int(&value["width"], 1100, 640, 4096), "height": bounded_int(&value["height"], 720, 480, 2160), "tabs": tabs, "bookmarks": bookmarks, "bookmarkGroups":groups,
        "analysisPresets":crate::analysis_presets::sanitize(&value["analysisPresets"])})
}

fn save_file(path: &Path, value: &Value) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("Invalid save path"))?;
    fs::create_dir_all(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
    }
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    file.write_all(&serde_json::to_vec_pretty(&sanitize(value))?)?;
    file.sync_all()?;
    drop(file);
    #[cfg(windows)]
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(temporary, path)
}

pub struct Workspace {
    state: Value,
    error: String,
    path: String,
    disk_path: Option<PathBuf>,
}

impl Default for Workspace {
    fn default() -> Self {
        let mut workspace = Self {
            state: sanitize(&json!({})),
            error: String::new(),
            path: String::new(),
            disk_path: None,
        };
        match config_directory() {
            Ok(dir) => {
                let path = dir.join("workspace.json");
                workspace.path = path.display().to_string();
                match fs::read(&path) {
                    Ok(bytes) => {
                        match serde_json::from_slice::<Value>(&bytes) {
                            Ok(value) if value["version"] == 1 || value["version"] == 2 => {
                                workspace.state = sanitize(&value)
                            }
                            _ => workspace.error =
                                "Cannot read configuration format. Automatic saving is disabled."
                                    .into(),
                        }
                    }
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {
                        // One-time migration of the old two settings, never secrets.
                        if cfg!(target_os = "macos")
                            && let Some(home) = std::env::var_os("HOME")
                            && let Ok(old) = fs::read_to_string(
                                PathBuf::from(home).join("Library/Preferences/tailer.ini"),
                            )
                        {
                            for line in old.lines() {
                                if let Some((name, number)) = line.split_once('=')
                                    && ["initialLines", "capacityMiB"].contains(&name)
                                    && let Ok(number) = number.parse::<u64>()
                                {
                                    workspace.state[name] = json!(number);
                                }
                            }
                            workspace.state = sanitize(&workspace.state);
                        }
                    }
                    Err(e) => workspace.error = format!("Cannot read configuration: {e}"),
                }
                if workspace.error.is_empty() {
                    workspace.disk_path = Some(path);
                }
            }
            Err(e) => workspace.error = e.to_string(),
        }
        workspace
    }
}

#[qobject]
impl Workspace {
    qproperty!("analysisPresets", Read = analysis_presets, Notify = changed);
    fn analysis_presets(&self) -> Value {
        let mut rows = crate::analysis_presets::defaults();
        for mut row in self.state["analysisPresets"].as_array().unwrap().clone() {
            row["builtin"] = json!(false);
            rows.push(row);
        }
        json!(rows)
    }

    fn persist_analysis_presets(&mut self, presets: Value) -> Result<(), String> {
        let path = self
            .disk_path
            .as_ref()
            .ok_or("Configuration saving is unavailable")?;
        let mut next = self.state.clone();
        next["analysisPresets"] = presets;
        save_file(path, &next).map_err(|e| format!("Cannot save presets: {e}"))?;
        self.state = sanitize(&next);
        self.changed();
        Ok(())
    }

    #[qslot]
    fn save_analysis_preset(&mut self, id: String, name: String, settings: Value) -> Value {
        match crate::analysis_presets::upsert(&self.state["analysisPresets"], &id, &name, &settings)
            .and_then(|(presets, id)| self.persist_analysis_presets(presets).map(|_| id))
        {
            Ok(id) => json!({"id":id}),
            Err(error) => json!({"error":error}),
        }
    }

    #[qslot]
    fn delete_analysis_preset(&mut self, id: String) -> Value {
        match crate::analysis_presets::remove(&self.state["analysisPresets"], &id)
            .and_then(|presets| self.persist_analysis_presets(presets))
        {
            Ok(()) => json!({"ok":true}),
            Err(error) => json!({"error":error}),
        }
    }

    qproperty!("state", Member = state, Notify = changed);
    qproperty!("error", Read = translated_error, Notify = changed);
    fn translated_error(&self) -> String {
        crate::i18n::text(&self.error)
    }
    #[qslot]
    fn refresh_language(&mut self) {
        self.changed();
    }
    qproperty!("path", Member = path, Notify = changed);
    #[qsignal]
    fn changed(&mut self);
    #[qslot]
    fn save(&mut self, value: Value) {
        let Some(path) = &self.disk_path else { return };
        // The preset store owns this collection. Stale QML workspace snapshots
        // must never overwrite a preset saved immediately by the editor.
        let mut value = value;
        value["analysisPresets"] = self.state["analysisPresets"].clone();
        if let Err(error) = save_file(path, &value) {
            self.error = format!("Cannot save configuration: {error}");
        } else {
            self.error.clear();
            self.state = sanitize(&value);
        }
        self.changed();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn merge_first_column_tab_setting_round_trips_and_defaults_off() {
        let clean = sanitize(&serde_json::json!({"tabs": [
            {"analysisMergeFirstColumn": true}, {"analysisMergeFirstColumn": false},
            {}, {"analysisMergeFirstColumn": "true"}
        ]}));
        assert_eq!(clean["tabs"][0]["analysisMergeFirstColumn"], true);
        for index in 1..4 {
            assert_eq!(clean["tabs"][index]["analysisMergeFirstColumn"], false);
        }
        assert_eq!(sanitize(&clean), clean);
    }
    use super::*;
    #[test]
    fn syslog_endpoints_round_trip_in_tabs_and_bookmarks() {
        let clean = sanitize(&json!({
            "tabs": [{"source":"syslog-udp", "logPath":"127.0.0.1:1514", "remote":false, "syslogBadges":true}],
            "bookmarks": [{"source":"syslog-udp", "logPath":"[::1]:1514", "remote":false, "syslogBadges":true}]
        }));
        assert_eq!(clean["tabs"][0]["source"], "syslog-udp");
        assert_eq!(clean["tabs"][0]["logPath"], "127.0.0.1:1514");
        assert_eq!(clean["bookmarks"][0]["logPath"], "[::1]:1514");
        assert_eq!(clean["tabs"][0]["syslogBadges"], true);
        assert_eq!(clean["bookmarks"][0]["syslogBadges"], true);
        assert_eq!(sanitize(&clean), clean);
    }
    #[test]
    fn analysis_preset_selection_round_trips_per_tab_and_bookmark() {
        let clean = sanitize(&json!({
            "tabs": [{"analysisPresetId":"custom-one"}, {"analysisPresetId":"builtin-apache"},
                {}, {"analysisPresetId":""}, {"analysisPresetId":42},
                {"analysisPresetId":"x".repeat(129)}, {"analysisPresetId":"builtin-access"}],
            "bookmarks": [{"analysisPresetId":"custom-one"}, {"analysisPresetId":"builtin-access"}]
        }));
        assert_eq!(clean["tabs"][0]["analysisPresetId"], "custom-one");
        assert_eq!(clean["tabs"][1]["analysisPresetId"], "builtin-apache");
        for index in 2..7 {
            assert_eq!(clean["tabs"][index]["analysisPresetId"], "builtin-apache");
        }
        assert_eq!(clean["bookmarks"][0]["analysisPresetId"], "custom-one");
        assert_eq!(clean["bookmarks"][1]["analysisPresetId"], "builtin-apache");
        assert_eq!(sanitize(&clean), clean);
    }
    #[test]
    fn analysis_presets_survive_disk_and_unrelated_workspace_saves() {
        let directory =
            std::env::temp_dir().join(format!("tailer-preset-test-{}", std::process::id()));
        let path = directory.join("workspace.json");
        let mut workspace = Workspace {
            state: sanitize(&json!({})),
            error: String::new(),
            path: path.display().to_string(),
            disk_path: Some(path.clone()),
        };
        let mut settings = crate::analysis_presets::defaults()[0]["settings"].clone();
        settings["filter"] = json!("checkpoint complete:");
        settings["query"] = json!("avg(total), count()");
        settings["password"] = json!("secret");
        let result = workspace.save_analysis_preset("".into(), "Test".into(), settings.clone());
        assert!(result.get("error").is_none(), "{result}");
        let id = result["id"].as_str().unwrap().to_owned();
        workspace.save(json!({"tabs":[],"analysisPresets":[]}));
        let restored: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(restored["analysisPresets"].as_array().unwrap().len(), 1);
        assert_eq!(
            restored["analysisPresets"][0]["settings"]["query"],
            "avg(total), count()"
        );
        assert!(!restored.to_string().contains("secret"));
        workspace.state = sanitize(&restored);
        assert_eq!(workspace.analysis_presets().as_array().unwrap().len(), 3);
        assert!(
            workspace
                .delete_analysis_preset("builtin-apache".into())
                .get("error")
                .is_some()
        );
        assert_eq!(workspace.delete_analysis_preset(id)["ok"], true);
        fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn numeric_and_background_settings_round_trip_in_tabs_and_bookmarks() {
        let condition = json!({"capture":1,"operator":">","value":"100"});
        let tab = json!({"filterText":r".* (\d+)$", "filterRegex":true,"filterNumericJson":condition.to_string(),
            "trapRulesJson":json!([{"text":r".* (\d+)$","regex":true,"numeric":condition,
                "background":true,"opacity":25,"scope":"capture","color":"#ffff00"}]).to_string()});
        let clean = sanitize(&json!({"tabs":[tab.clone()],"bookmarks":[tab]}));
        for collection in ["tabs", "bookmarks"] {
            assert_eq!(
                serde_json::from_str::<Value>(
                    clean[collection][0]["filterNumericJson"].as_str().unwrap()
                )
                .unwrap(),
                condition
            );
            let rules: Value =
                serde_json::from_str(clean[collection][0]["trapRulesJson"].as_str().unwrap())
                    .unwrap();
            assert_eq!(rules[0]["numeric"], condition);
            assert_eq!(rules[0]["background"], true);
            assert_eq!(rules[0]["opacity"], 25);
            assert_eq!(rules[0]["scope"], "capture");
        }
        assert_eq!(sanitize(&clean), clean);
        assert_eq!(
            sanitize(&json!({"tabs":[{}]}))["tabs"][0]["filterNumericJson"],
            "null"
        );
    }
    #[test]
    fn trap_tags_migrate_round_trip_and_are_bounded() {
        let clean = sanitize(&json!({"tabs":[{"trapText":"ERROR","trapCase":true},
            {"trapText":"old", "trapRulesJson":"[]"},
            {"trapRulesJson":json!([{"text":"ERROR.*?aaaa","regex":true,"color":"#ff0000","password":"secret"}]).to_string()},
            {"trapRulesJson":json!(vec![json!({"text":"x".repeat(2000),"color":"unsafe"});12]).to_string()}
        ]}));
        let tags = |i: usize| {
            serde_json::from_str::<Value>(clean["tabs"][i]["trapRulesJson"].as_str().unwrap())
                .unwrap()
        };
        assert_eq!(
            tags(0),
            json!([{"text":"ERROR","regex":false,"ignoreCase":true,"color":"#35c66b"}])
        );
        assert_eq!(tags(1), json!([]));
        assert_eq!(tags(2)[0]["regex"], true);
        assert_eq!(tags(2)[0]["color"], "#ff0000");
        assert!(tags(2)[0].get("password").is_none());
        assert_eq!(tags(3).as_array().unwrap().len(), 10);
        assert_eq!(tags(3)[0]["text"].as_str().unwrap().len(), 1024);
        assert_eq!(tags(3)[0]["color"], "#35c66b");
        assert_eq!(sanitize(&clean), clean);
    }
    #[test]
    fn language_selection_round_trips_and_invalid_defaults_to_detection() {
        for language in crate::i18n::LANGUAGES {
            let clean = sanitize(&json!({"language":language}));
            assert_eq!(clean["language"], *language);
            assert_eq!(sanitize(&clean), clean);
        }
        assert_eq!(sanitize(&json!({}))["language"], "");
        assert_eq!(sanitize(&json!({"language":"fr"}))["language"], "");
    }
    #[test]
    fn encoding_is_saved_and_legacy_or_unknown_defaults_to_utf8() {
        let clean = sanitize(
            &json!({"tabs":[{"encoding":"CP932"},{},{"encoding":"bogus"},{"encoding":"ISO-2022-JP"}]}),
        );
        assert_eq!(clean["tabs"][0]["encoding"], "CP932");
        assert_eq!(clean["tabs"][1]["encoding"], "UTF-8");
        assert_eq!(clean["tabs"][2]["encoding"], "UTF-8");
        assert_eq!(clean["tabs"][3]["encoding"], "ISO-2022-JP");
        assert_eq!(sanitize(&clean), clean);
    }
    #[test]
    fn custom_command_and_title_round_trip_without_modification() {
        let script = "printf '%s\\n' \"$HOME\" | cat\nprintf error >&2";
        let clean = sanitize(
            &json!({"connections":[{"id":"server","name":"Server","host":"server"}],"tabs":[{"remote":true,"source":"custom","connectionId":"server","logPath":script,"logTitle":"Pod logs"}]}),
        );
        assert_eq!(clean["tabs"][0]["logPath"], script);
        assert_eq!(clean["tabs"][0]["logTitle"], "Pod logs");
        assert_eq!(clean["tabs"][0]["source"], "custom");
        assert_eq!(sanitize(&clean), clean);
    }
    #[test]
    fn legacy_remote_tabs_migrate_to_deduplicated_profiles() {
        let clean = sanitize(&json!({"version":1,"tabs":[
            {"remote":true,"host":"server","user":"alice","port":22,"key":"/key","logPath":"/a"},
            {"remote":true,"host":"server","user":"alice","port":22,"key":"/key","logPath":"/b","elevation":1},
            {"remote":true,"host":"server","user":"bob","logPath":"/c"},
            {"remote":false,"logPath":"/local"}
        ]}));
        assert_eq!(clean["version"], 2);
        assert_eq!(clean["connections"].as_array().unwrap().len(), 2);
        assert_eq!(
            clean["tabs"][0]["connectionId"],
            clean["tabs"][1]["connectionId"]
        );
        assert_ne!(
            clean["tabs"][0]["connectionId"],
            clean["tabs"][2]["connectionId"]
        );
        assert_eq!(clean["tabs"][3]["connectionId"], "");
        assert!(clean["tabs"][0].get("host").is_none());
        assert_eq!(sanitize(&clean), clean);
    }

    #[test]
    fn profiles_preserve_ids_and_exclude_secrets() {
        let clean = sanitize(&json!({"version":2,"connections":[
            {"id":"prod","name":"Production","host":"server","port":2200.0,"password":"secret","token":"secret"},
            {"id":"prod","host":"duplicate"}, {"name":"missing-id"}
        ],"tabs":[{"remote":true,"connectionId":"prod"}]}));
        assert_eq!(clean["connections"].as_array().unwrap().len(), 1);
        assert_eq!(clean["connections"][0]["port"], 2200);
        assert_eq!(clean["tabs"][0]["connectionId"], "prod");
        assert!(!clean.to_string().contains("secret"));
        assert_eq!(sanitize(&clean), clean);
    }
    #[test]
    fn standard_paths_and_xdg_fallback() {
        let get = |name: &str| match name {
            "HOME" => Some(PathBuf::from("/home/user")),
            "APPDATA" => Some(PathBuf::from("/roaming")),
            _ => None,
        };
        assert_eq!(
            config_directory_for("macos", get).unwrap(),
            PathBuf::from("/home/user/Library/Application Support/Tailer")
        );
        assert_eq!(
            config_directory_for("linux", get).unwrap(),
            PathBuf::from("/home/user/.config/Tailer")
        );
        assert_eq!(
            config_directory_for("windows", get).unwrap(),
            PathBuf::from("/roaming/Tailer")
        );
        assert_eq!(
            config_directory_for("linux", |n| if n == "XDG_CONFIG_HOME" {
                Some("/custom".into())
            } else {
                get(n)
            })
            .unwrap(),
            PathBuf::from("/custom/Tailer")
        );
        assert_eq!(
            config_directory_for("linux", |n| if n == "XDG_CONFIG_HOME" {
                Some("relative".into())
            } else {
                get(n)
            })
            .unwrap(),
            PathBuf::from("/home/user/.config/Tailer")
        );
    }
    #[test]
    fn round_trip_excludes_secrets_and_preserves_tabs() {
        let value = json!({"version":1, "initialLines":20, "password":"secret", "tabs":[{"logPath":"/tmp/log", "host":"server", "password":"hidden", "filterText":"ERROR", "elevation":2}]});
        let clean = sanitize(&value);
        assert!(!clean.to_string().contains("secret"));
        assert!(!clean.to_string().contains("hidden"));
        let dir = std::env::temp_dir().join(format!("tailer-workspace-{}", std::process::id()));
        let path = dir.join("workspace.json");
        save_file(&path, &value).unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&fs::read(&path).unwrap()).unwrap(),
            clean
        );
        save_file(&path, &clean).unwrap();
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn qml_floating_point_numbers_are_preserved() {
        let clean = sanitize(&json!({"initialLines":20.0,"currentIndex":1.0,"tabs":[{},{}]}));
        assert_eq!(clean["initialLines"], 20);
        assert_eq!(clean["currentIndex"], 1);
    }

    #[test]
    fn source_is_saved_and_old_tabs_default_to_file() {
        let value = sanitize(
            &json!({"tabs":[{"source":"docker","logPath":"web-1","trapText":"ERROR","trapCase":true,"alert":true}, {"logPath":"/tmp/log"}]}),
        );
        assert_eq!(value["tabs"][0]["trapText"], "ERROR");
        assert_eq!(value["tabs"][0]["trapCase"], true);
        assert_eq!(value["tabs"][0]["alert"], false);
        assert_eq!(value["tabs"][0]["source"], "docker");
        assert_eq!(value["tabs"][0]["logPath"], "web-1");
        assert_eq!(value["tabs"][1]["source"], "file");
    }

    #[test]
    fn bookmarks_survive_closed_tabs_and_exclude_secrets() {
        let input = json!({"tabs": [], "bookmarks": [{
            "logPath": "web", "logTitle": "Production", "source": "docker",
            "remote": true, "host": "server", "encoding": "CP932",
            "elevation": 2, "runUser": "app", "initial": 100,
            "capacity": 64, "filterText": "ERROR", "filterRegex": true,
            "searchText": "timeout", "follow": false, "unread": true,
            "alert": true, "password": "secret-password", "token": "secret-token",
            "trapRulesJson": "[{\"text\":\"ERROR\",\"color\":\"#35c66b\"}]"
        }]});
        let clean = sanitize(&input);
        assert_eq!(clean["tabs"], json!([]));
        let bookmark = &clean["bookmarks"][0];
        for field in [
            "logPath",
            "logTitle",
            "source",
            "remote",
            "encoding",
            "elevation",
            "runUser",
            "initial",
            "capacity",
            "filterText",
            "filterRegex",
            "searchText",
            "follow",
        ] {
            assert_eq!(bookmark[field], input["bookmarks"][0][field]);
        }
        assert_eq!(bookmark["unread"], false);
        assert_eq!(bookmark["alert"], false);
        assert_eq!(clean["connections"][0]["host"], "server");
        assert_eq!(bookmark["connectionId"], clean["connections"][0]["id"]);
        assert!(!clean.to_string().contains("secret-"));
        assert_eq!(sanitize(&clean), clean);
        let dir = std::env::temp_dir().join(format!("tailer-bookmarks-{}", std::process::id()));
        let path = dir.join("workspace.json");
        save_file(&path, &input).unwrap();
        let restored: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(sanitize(&restored), clean);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn bookmarks_default_empty_and_are_bounded() {
        assert_eq!(sanitize(&json!({}))["bookmarks"], json!([]));
        let clean = sanitize(&json!({"bookmarks": vec![json!({"logPath":"/tmp/log"}); 101]}));
        assert_eq!(clean["bookmarks"].as_array().unwrap().len(), 100);
        assert_eq!(
            sanitize(&json!({"bookmarks":[null, false, "invalid"]}))["bookmarks"],
            json!([])
        );
    }

    #[test]
    fn filter_and_trap_enabled_states_survive_tabs_and_bookmarks() {
        let row = json!({"filterText":"ERROR.*", "filterRegex":true, "filterEnabled":false,
            "trapRulesJson":r#"[{"text":"WARN.*","regex":true,"enabled":false}]"#});
        let clean = sanitize(&json!({"tabs":[row.clone(), {}], "bookmarks":[row]}));
        for collection in ["tabs", "bookmarks"] {
            assert_eq!(clean[collection][0]["filterEnabled"], false);
            assert_eq!(clean[collection][0]["filterText"], "ERROR.*");
            let rules: Value =
                serde_json::from_str(clean[collection][0]["trapRulesJson"].as_str().unwrap())
                    .unwrap();
            assert_eq!(rules[0]["enabled"], false);
            assert_eq!(rules[0]["text"], "WARN.*");
        }
        assert_eq!(clean["tabs"][1]["filterEnabled"], true);
        assert_eq!(sanitize(&clean), clean);
        let dir = std::env::temp_dir().join(format!("tailer-rule-enable-{}", std::process::id()));
        let path = dir.join("workspace.json");
        save_file(&path, &clean).unwrap();
        let restored: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(sanitize(&restored), clean);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn appearance_round_trips_and_defaults_to_auto() {
        for appearance in ["auto", "light", "dark"] {
            let clean = sanitize(&json!({"appearance": appearance}));
            assert_eq!(clean["appearance"], appearance);
            assert_eq!(sanitize(&clean), clean);
        }
        for input in [
            json!({}),
            json!({"appearance":"invalid"}),
            json!({"appearance":null}),
            json!({"appearance":1}),
        ] {
            assert_eq!(sanitize(&input)["appearance"], "auto");
        }
    }

    #[test]
    fn bookmark_titles_are_independent_and_legacy_titles_migrate() {
        let clean = sanitize(
            &json!({"tabs":[{"logTitle":"Tab", "bookmarkTitle":"ignored"}], "bookmarks":[
                {"logTitle":"Original", "bookmarkTitle":"本番ログ / Custom"},
                {"logTitle":"Legacy", "logPath":"/tmp/log"},
                {"logPath":"/tmp/fallback", "bookmarkTitle":"  "},
                {"bookmarkTitle":"x".repeat(9000)}
            ]}),
        );
        assert_eq!(clean["bookmarks"][0]["bookmarkTitle"], "本番ログ / Custom");
        assert_eq!(clean["bookmarks"][0]["logTitle"], "Original");
        assert_eq!(clean["bookmarks"][1]["bookmarkTitle"], "Legacy");
        assert_eq!(clean["bookmarks"][2]["bookmarkTitle"], "/tmp/fallback");
        assert_eq!(
            clean["bookmarks"][3]["bookmarkTitle"]
                .as_str()
                .unwrap()
                .len(),
            8192
        );
        assert!(clean["tabs"][0].get("bookmarkTitle").is_none());
        assert_eq!(sanitize(&clean), clean);
    }

    #[test]
    fn bookmark_groups_round_trip_and_invalid_trees_are_repaired() {
        let clean = sanitize(&json!({"bookmarkGroups":[
            {"id":"prod", "name":"Production", "expanded":false},
            {"id":"web", "name":"Web", "parentId":"prod"},
            {"id":"orphan", "name":"Orphan", "parentId":"missing"},
            {"id":"self", "name":"Self", "parentId":"self"},
            {"id":"a", "name":"A", "parentId":"b"},
            {"id":"b", "name":"B", "parentId":"a"},
            {"id":"prod", "name":"Duplicate"}, {"id":"blank", "name":" "}
        ], "bookmarks":[{"groupId":"web"}, {"groupId":"missing"}, {}]}));
        assert_eq!(clean["bookmarkGroups"].as_array().unwrap().len(), 6);
        assert_eq!(clean["bookmarkGroups"][0]["expanded"], false);
        assert_eq!(clean["bookmarkGroups"][1]["parentId"], "prod");
        for i in [2, 3, 4] {
            assert_eq!(clean["bookmarkGroups"][i]["parentId"], "");
        }
        assert_eq!(clean["bookmarks"][0]["groupId"], "web");
        for i in [1, 2] {
            assert_eq!(clean["bookmarks"][i]["groupId"], "");
        }
        assert_eq!(sanitize(&clean), clean);
        assert_eq!(sanitize(&json!({}))["bookmarkGroups"], json!([]));
        let groups: Vec<Value> = (0..101)
            .map(|i| json!({"id":i.to_string(),"name":"Group"}))
            .collect();
        assert_eq!(
            sanitize(&json!({"bookmarkGroups":groups}))["bookmarkGroups"]
                .as_array()
                .unwrap()
                .len(),
            100
        );
    }
}
