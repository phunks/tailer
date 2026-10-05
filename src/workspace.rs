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
    if let Some(input) = value["tabs"].as_array() {
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
            clean["runUser"] = json!(tab["runUser"].as_str().unwrap_or("root"));
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
            ] {
                clean[name] = json!(tab[name].as_bool().unwrap_or(false));
            }
            clean["follow"] = json!(tab["follow"].as_bool().unwrap_or(true));
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
            tabs.push(clean);
        }
    }
    let language = value["language"].as_str().unwrap_or_default();
    json!({"version": 2, "language": if crate::i18n::LANGUAGES.contains(&language) { language } else { "" }, "connections":connections, "initialLines": bounded_int(&value["initialLines"], 50, 0, 1000000),
        "capacityMiB": bounded_int(&value["capacityMiB"], 512, 1, 10240),
        "currentIndex": bounded_int(&value["currentIndex"], 0, 0, tabs.len().saturating_sub(1) as u64),
        "width": bounded_int(&value["width"], 1100, 640, 4096), "height": bounded_int(&value["height"], 720, 480, 2160), "tabs": tabs})
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
    use super::*;
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
}
