use qtbridge::qobject;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::{OnceLock, RwLock};

pub const LANGUAGES: &[&str] = &["en", "ja", "zh-CN", "zh-TW", "ko"];
static LANGUAGE: RwLock<&str> = RwLock::new("en");

fn catalog() -> &'static BTreeMap<String, Vec<String>> {
    static CATALOG: OnceLock<BTreeMap<String, Vec<String>>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        include_str!("translations.txt")
            .lines()
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(|line| {
                let values: Vec<String> = line
                    .split('|')
                    .map(|value| value.replace("\\n", "\n"))
                    .collect();
                assert_eq!(values.len(), LANGUAGES.len(), "Invalid translation: {line}");
                (values[0].clone(), values)
            })
            .collect()
    })
}

pub fn language_for_locale(locale: &str) -> &'static str {
    let locale = locale.to_ascii_lowercase().replace('_', "-");
    if locale.starts_with("ja") {
        "ja"
    } else if locale.starts_with("ko") {
        "ko"
    } else if locale.starts_with("zh") {
        if locale.contains("hant")
            || locale.starts_with("zh-tw")
            || locale.starts_with("zh-hk")
            || locale.starts_with("zh-mo")
        {
            "zh-TW"
        } else {
            "zh-CN"
        }
    } else {
        "en"
    }
}

pub fn set_language(language: &str) {
    *LANGUAGE.write().unwrap() = LANGUAGES
        .iter()
        .copied()
        .find(|&l| l == language)
        .unwrap_or("en");
}

fn template_values<'a>(text: &'a str, template: &str) -> Option<Vec<&'a str>> {
    let parts: Vec<_> = template.split("{}").collect();
    if parts.len() == 1 {
        return None;
    }
    let mut rest = text.strip_prefix(parts[0])?;
    let mut values = Vec::new();
    for (index, part) in parts.iter().enumerate().skip(1) {
        if index == parts.len() - 1 {
            values.push(rest.strip_suffix(part)?);
        } else {
            let position = rest.find(part)?;
            values.push(&rest[..position]);
            rest = &rest[position + part.len()..];
        }
    }
    Some(values)
}

fn translate(text: &str, index: usize, depth: usize) -> String {
    if index == 0 || depth > 8 {
        return text.into();
    }
    if let Some(values) = catalog().get(text) {
        return values[index].clone();
    }
    for (template, translations) in catalog() {
        if let Some(values) = template_values(text, template) {
            let mut result = String::new();
            let mut parts = translations[index].split("{}");
            result.push_str(parts.next().unwrap());
            for (value, part) in values.iter().zip(parts) {
                result.push_str(&translate(value, index, depth + 1));
                result.push_str(part);
            }
            return result;
        }
    }
    if text.contains('\n') {
        return text
            .split('\n')
            .map(|part| translate(part, index, depth + 1))
            .collect::<Vec<_>>()
            .join("\n");
    }
    if text.contains(" · ") {
        return text
            .split(" · ")
            .map(|part| translate(part, index, depth + 1))
            .collect::<Vec<_>>()
            .join(" · ");
    }
    if let Some(rest) = text.strip_prefix(' ') {
        return format!(" {}", translate(rest, index, depth + 1));
    }
    text.into() // External diagnostics and user-supplied values stay unchanged.
}

pub fn text(text: &str) -> String {
    let language = *LANGUAGE.read().unwrap();
    translate(
        text,
        LANGUAGES.iter().position(|&l| l == language).unwrap_or(0),
        0,
    )
}

#[derive(Default)]
pub struct Translations {
    language: String,
    messages: Value,
}

#[qobject]
impl Translations {
    qproperty!("language", Member = language, Notify = changed);
    qproperty!("messages", Member = messages, Notify = changed);
    #[qsignal]
    fn changed(&mut self);
    #[qslot]
    fn select(&mut self, language: String) {
        let index = LANGUAGES.iter().position(|&l| l == language).unwrap_or(0);
        self.language = LANGUAGES[index].into();
        set_language(&self.language);
        let mut messages = json!({});
        for (key, values) in catalog() {
            messages[key] = json!(values[index]);
        }
        self.messages = messages;
        self.changed();
    }
    #[qslot]
    fn detect(&self, locale: String) -> String {
        language_for_locale(&locale).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_is_complete_and_placeholders_match() {
        assert_eq!(LANGUAGES, &["en", "ja", "zh-CN", "zh-TW", "ko"]);
        assert!(catalog().len() > 100);
        let rows = include_str!("translations.txt")
            .lines()
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .count();
        assert_eq!(catalog().len(), rows, "Duplicate English translation keys");
        for (key, translations) in catalog() {
            assert_eq!(key, &translations[0]);
            assert!(
                key.is_ascii() || !key.chars().any(|c| ('\u{3000}'..='\u{9fff}').contains(&c)),
                "Non-English key: {key}"
            );
            for value in translations {
                assert!(!value.is_empty(), "{key}");
                assert_eq!(
                    key.matches("{}").count(),
                    value.matches("{}").count(),
                    "{key}: {value}"
                );
            }
        }
        for fragment in include_str!("Main.qml").split("root.tr(\"").skip(1) {
            let key = fragment.split('"').next().unwrap();
            assert!(catalog().contains_key(key), "Missing UI translation: {key}");
        }
    }
    #[test]
    fn locales_and_fallback() {
        for (locale, expected) in [
            ("ja_JP.UTF-8", "ja"),
            ("en_US", "en"),
            ("zh-Hans", "zh-CN"),
            ("zh_Hant_HK", "zh-TW"),
            ("zh_TW", "zh-TW"),
            ("ko_KR", "ko"),
            ("fr_FR", "en"),
        ] {
            assert_eq!(language_for_locale(locale), expected);
        }
    }
    #[test]
    fn formatted_messages_translate_without_translating_logs() {
        for (source, expected) in [
            (
                "接続終了: SSHチャネル要求が拒否／タイムアウトされました",
                "Connection ended: SSH channel request was rejected or timed out",
            ),
            (
                "接続終了: SSHチャネル要求が拒否されました: PTY",
                "Connection ended: SSH channel request was rejected: PTY",
            ),
            (
                "接続終了: SSHチャネル要求がタイムアウトしました: exec",
                "Connection ended: SSH channel request timed out: exec",
            ),
            (
                "接続終了: SSHチャネル要求の応答前に接続が終了しました: exec",
                "Connection ended: Connection closed before the SSH channel request reply: exec",
            ),
            (
                "接続終了 · 保存済み履歴は検索可能",
                "Connection ended · Saved history remains searchable",
            ),
        ] {
            assert_eq!(translate(expected, 0, 0), expected);
            assert_eq!(translate(expected, 1, 0), source);
        }
        // Keep the russh backend's application-owned errors in the catalog.
        let source = include_str!("ssh.rs").split("#[cfg(test)]").next().unwrap();
        for fragment in source.split("error(\"").skip(1) {
            let key = fragment.split('"').next().unwrap();
            assert!(
                catalog().contains_key(key),
                "Missing SSH translation: {key}"
            );
        }
        assert_eq!(
            translate("Following · Saved 42 KiB", 1, 0),
            "監視中 · 保存 42 KiB"
        );
        assert_eq!(
            translate("Cannot connect: Check the host name", 1, 0),
            "接続できません: ホスト名を確認してください"
        );
        assert_eq!(translate("server: Connected", 1, 0), "server: 接続中");
        assert_eq!(
            translate("Permission denied (publickey).", 1, 0),
            "Permission denied (publickey)."
        );
        assert_eq!(
            translate(
                "Filter matched 2 lines · Showing up to the last 2,000 lines / 256KiB · History search: 3 matches (last 2,000 results maximum)",
                1,
                0
            ),
            "フィルタ一致 2 行 · 表示は末尾最大2,000行 / 256KiB · 全履歴検索 3 件（結果は末尾最大2,000件）"
        );
    }
    #[test]
    fn english_keys_translate_to_all_languages() {
        for (index, expected) in [
            "Connections",
            "接続先管理",
            "连接管理",
            "連線管理",
            "연결 관리",
        ]
        .iter()
        .enumerate()
        {
            assert_eq!(translate("Connections", index, 0), *expected);
            assert_eq!(
                translate("Unknown external diagnostic", index, 0),
                "Unknown external diagnostic"
            );
        }
    }
}
