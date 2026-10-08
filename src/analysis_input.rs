//! UI-configured filtering and extraction, compiled on the analysis worker.
use super::{Captures, MAX_FIELDS, MAX_LINE_BYTES, MAX_REGEX_BYTES, MAX_VALUE_BYTES, Record};
use regex::Regex;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

#[derive(Clone, Default)]
pub struct InputConfig {
    pub format: String,
    pub delimiter: String,
    pub filter: String,
    pub filter_mode: String,
    pub ignore_case: bool,
    pub exclude: bool,
    pub fields: Vec<Field>,
}

#[derive(Clone)]
pub struct Field {
    pub name: String,
    pub source: String,
    pub numeric: bool,
    pub required: bool,
}

impl InputConfig {
    pub fn from_value(value: &Value) -> Result<Self, String> {
        let object = value
            .as_object()
            .ok_or("Input settings must be an object")?;
        let text = |key: &str, default: &str| -> Result<String, String> {
            match object.get(key) {
                None => Ok(default.into()),
                Some(Value::String(s)) => Ok(s.clone()),
                _ => Err(format!("{key} must be a string")),
            }
        };
        let flag = |key: &str| -> Result<bool, String> {
            match object.get(key) {
                None => Ok(false),
                Some(Value::Bool(b)) => Ok(*b),
                _ => Err(format!("{key} must be a boolean")),
            }
        };
        let mut fields = Vec::new();
        if let Some(value) = object.get("fields") {
            let rows = value.as_array().ok_or("fields must be an array")?;
            if rows.len() > MAX_FIELDS {
                return Err("Too many field mappings (maximum 32)".into());
            }
            for row in rows {
                let name = row
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or("Field name is required")?;
                let source = row
                    .get("source")
                    .and_then(Value::as_str)
                    .ok_or("Field source is required")?;
                let kind = row
                    .get("type")
                    .and_then(Value::as_str)
                    .ok_or("Field type is required")?;
                if !["text", "number"].contains(&kind) {
                    return Err("Field type must be text or number".into());
                }
                let required = match row.get("required") {
                    None => true,
                    Some(Value::Bool(b)) => *b,
                    _ => return Err("required must be a boolean".into()),
                };
                fields.push(Field {
                    name: name.into(),
                    source: source.into(),
                    numeric: kind == "number",
                    required,
                });
            }
        }
        Ok(Self {
            format: text("format", "regex")?,
            delimiter: text("delimiter", ",")?,
            filter: text("filter", "")?,
            filter_mode: text("filterMode", "contains")?,
            ignore_case: flag("ignoreCase")?,
            exclude: flag("exclude")?,
            fields,
        })
    }
}

enum Source {
    Regex(Regex),
    Delimiter(String),
    Whitespace,
    Json,
}

pub(super) struct Input {
    config: InputConfig,
    source: Source,
    filter_regex: Option<Regex>,
    filter_text: String,
    columns: BTreeSet<usize>,
}

pub(super) struct Extracted {
    pub captures: Captures,
    pub record: Record,
}

impl Input {
    pub fn new(config: &InputConfig, expression: &str) -> Result<Self, String> {
        if config.filter.len() > MAX_REGEX_BYTES {
            return Err("Line filter exceeds 16 KiB".into());
        }
        if !["", "contains", "prefix", "regex"].contains(&config.filter_mode.as_str()) {
            return Err("Unknown line filter mode".into());
        }
        let filter_regex = if !config.filter.is_empty() && config.filter_mode == "regex" {
            Some(
                regex::RegexBuilder::new(&config.filter)
                    .case_insensitive(config.ignore_case)
                    .build()
                    .map_err(|e| format!("Line filter: {e}"))?,
            )
        } else {
            None
        };
        let source = match config.format.as_str() {
            "" | "regex" => {
                if expression.len() > MAX_REGEX_BYTES {
                    return Err("Regex exceeds 16 KiB".into());
                }
                Source::Regex(Regex::new(expression).map_err(|e| e.to_string())?)
            }
            "delimiter" => {
                if config.delimiter.is_empty() || config.delimiter.len() > 128 {
                    return Err("Delimiter must be 1–128 bytes".into());
                }
                Source::Delimiter(config.delimiter.clone())
            }
            "whitespace" => Source::Whitespace,
            "json" => Source::Json,
            _ => return Err("Unknown extraction format".into()),
        };
        if config.fields.len() > MAX_FIELDS {
            return Err("Too many field mappings (maximum 32)".into());
        }
        let mut names = BTreeSet::new();
        for field in &config.fields {
            if field.name.is_empty() || field.name.len() > 64 || !names.insert(&field.name) {
                return Err("Field names must be unique and 1–64 bytes".into());
            }
            match &source {
                Source::Regex(re) => {
                    let exists = re.capture_names().enumerate().any(|(i, name)| {
                        name.map(str::to_owned).unwrap_or_else(|| i.to_string()) == field.source
                    });
                    if !exists {
                        return Err(format!("Unknown capture: {}", field.source));
                    }
                }
                Source::Delimiter(_) | Source::Whitespace => {
                    let index = field
                        .source
                        .parse::<usize>()
                        .map_err(|_| "Column must be a zero-based integer")?;
                    if index > MAX_LINE_BYTES {
                        return Err("Column index is too large".into());
                    }
                    if index.to_string() != field.source {
                        return Err(
                            "Column index must use canonical decimal notation (e.g. 0, 1, 2)"
                                .into(),
                        );
                    }
                }
                Source::Json => {
                    if field.source.len() > 1024
                        || (!field.source.is_empty() && !field.source.starts_with('/'))
                    {
                        return Err(
                            "JSON source must be a JSON Pointer (e.g. /timing/total)".into()
                        );
                    }
                    let bytes = field.source.as_bytes();
                    for (i, b) in bytes.iter().enumerate() {
                        if *b == b'~' && !matches!(bytes.get(i + 1), Some(b'0' | b'1')) {
                            return Err("Invalid JSON Pointer escape".into());
                        }
                    }
                }
            }
        }
        if !matches!(source, Source::Regex(_)) && config.fields.is_empty() {
            return Err("Add at least one field mapping".into());
        }
        Ok(Self {
            config: config.clone(),
            source,
            filter_regex,
            filter_text: if config.ignore_case {
                config.filter.to_lowercase()
            } else {
                config.filter.clone()
            },
            columns: config
                .fields
                .iter()
                .filter_map(|f| f.source.parse().ok())
                .collect(),
        })
    }

    pub fn accepts(&self, line: &str) -> bool {
        if self.config.filter.is_empty() {
            return true;
        }
        let matched = if let Some(re) = &self.filter_regex {
            re.is_match(line)
        } else {
            let normalized;
            let line = if self.config.ignore_case {
                normalized = line.to_lowercase();
                &normalized
            } else {
                line
            };
            if self.config.filter_mode == "prefix" {
                line.starts_with(&self.filter_text)
            } else {
                line.contains(&self.filter_text)
            }
        };
        matched != self.config.exclude
    }

    // None is a regex mismatch; Err is a malformed document or invalid field.
    pub fn extract(&self, line: &str, direct: bool) -> Result<Option<Extracted>, String> {
        let mut raw = BTreeMap::new();
        let mut document = None;
        match &self.source {
            Source::Regex(re) => {
                let Some(captures) = re.captures(line) else {
                    return Ok(None);
                };
                for (i, name) in re.capture_names().enumerate() {
                    if let Some(value) = captures.get(i) {
                        raw.insert(
                            name.map(str::to_owned).unwrap_or_else(|| i.to_string()),
                            value.as_str().to_owned(),
                        );
                    }
                }
            }
            Source::Delimiter(separator) => {
                // Only retain configured columns, including empty columns.
                self.columns(line.split(separator), &mut raw);
            }
            Source::Whitespace => self.columns(line.split_whitespace(), &mut raw),
            Source::Json => {
                document = Some(
                    serde_json::from_str::<Value>(line)
                        .map_err(|e| format!("Invalid JSON: {e}"))?,
                );
            }
        }
        let mut record = Record::default();
        let mut fields = BTreeMap::new();
        if self.config.fields.is_empty() {
            // Legacy Roto gets all captures, including the whole match. Direct
            // output omits capture 0 unless explicitly mapped.
            fields = raw;
            for (name, value) in &fields {
                if direct && name != "0" {
                    Self::set(&mut record, name, value, false)?;
                }
            }
        } else {
            for field in &self.config.fields {
                let value = if let Some(document) = &document {
                    match document.pointer(&field.source) {
                        None | Some(Value::Null) => None,
                        Some(Value::Number(n)) if field.numeric => Some(n.to_string()),
                        Some(Value::String(s)) if !field.numeric => Some(s.clone()),
                        Some(_) => {
                            return Err(format!("Wrong JSON type for field: {}", field.name));
                        }
                    }
                } else {
                    raw.get(&field.source).cloned()
                };
                let Some(value) = value else {
                    if field.required {
                        return Err(format!("Missing field: {}", field.name));
                    }
                    continue;
                };
                Self::set(&mut record, &field.name, &value, field.numeric)?;
                fields.insert(field.name.clone(), value);
            }
        }
        Ok(Some(Extracted {
            captures: Captures {
                fields: Arc::new(fields),
            },
            record,
        }))
    }

    fn columns<'a>(
        &self,
        columns: impl Iterator<Item = &'a str>,
        raw: &mut BTreeMap<String, String>,
    ) {
        let last = self.columns.last().copied().unwrap_or(0);
        for (index, value) in columns.take(last + 1).enumerate() {
            if self.columns.contains(&index) {
                raw.insert(index.to_string(), value.into());
            }
        }
    }

    fn set(record: &mut Record, name: &str, value: &str, numeric: bool) -> Result<(), String> {
        if !record.validate_field(name) {
            return Err(record.error.clone());
        }
        if numeric {
            let number = value
                .parse::<f64>()
                .ok()
                .filter(|n| n.is_finite())
                .ok_or_else(|| format!("Invalid finite number: {name}"))?;
            record.numbers.insert(name.into(), number);
        } else {
            if value.len() > MAX_VALUE_BYTES {
                return Err(format!("Text value exceeds 1,024 bytes: {name}"));
            }
            record.text.insert(name.into(), value.into());
        }
        Ok(())
    }
}
