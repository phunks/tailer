//! Snapshot-based analysis. JIT compilation and calls never run on the GUI or
//! reception thread. Cancellation is cooperative between lines, not a JIT timeout.
use regex::Regex;
use roto::{FileTree, NoCtx, RotoString, Runtime, TypedFunc, Val, library};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[path = "analysis_input.rs"]
mod input;
pub use input::InputConfig;
#[path = "analysis_datetime.rs"]
mod datetime;
#[path = "analysis_query.rs"]
mod query;
use datetime::{NaiveTimestamp, ScriptDatetime, UtcTimestamp};

const MAX_FIELDS: usize = 32;
const MAX_GROUPS: usize = 2000;
const MAX_VALUE_BYTES: usize = 1024;
const MAX_LINE_BYTES: usize = 256 * 1024;
const MAX_REGEX_BYTES: usize = 16 * 1024;
const SAMPLE_COUNT: usize = 5;

#[cfg(test)]
thread_local! {
    // JIT initialization and calls run on the calling thread. Keep counts
    // isolated so concurrently running tests cannot affect this assertion.
    static REGEX_COMPILE_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub const ACCESS_REGEX: &str = r#"^(?P<time>\S+)\s+(?P<level>\w+):\s+(?P<client>\S+)\s+-\s+"(?P<method>\S+)\s+(?P<path>\S+)\s+(?P<protocol>[^"]+)"\s+(?P<status>\d{3})(?:\s+.*)?$"#;
pub const ACCESS_SCRIPT: &str = r#"fn parse(c: Captures) -> Record {
    Record.new()
        .text("path", strip_query(c.text("path")))
        .text("status", c.text("status"))
        .text("method", c.text("method"))
}
"#;
pub const APACHE_REGEX: &str = r#"^(?P<client>\S+)\s+\S+\s+\S+\s+\[(?P<time>[^\]]+)\]\s+"(?P<method>\S+)\s+(?P<path>\S+)\s+(?P<protocol>[^"]+)"\s+(?P<status>\d{3})\s+(?P<bytes>\d+|-)(?:\s+(?P<duration_us>\d+))?\s*$"#;
pub const APACHE_SCRIPT: &str = r#"// Common Log Format, optionally followed by %D (microseconds).
fn parse(c: Captures) -> Record {
    let r = Record.new()
        .text("path", strip_query(c.text("path")))
        .text("status", c.text("status"));
    if c.has("duration_us") {
        r.number("duration_ms", c.number("duration_us") / 1000.0)
    } else {
        r
    }
}
"#;

#[derive(Clone, Default, PartialEq)]
struct Captures {
    // Roto clones arguments when calling methods; share the immutable capture
    // map instead of copying every captured string for every c.text() call.
    fields: Arc<BTreeMap<String, String>>,
}

#[derive(Clone)]
struct CompiledRegex {
    // Method calls clone their receiver; share the compiled regex rather than
    // cloning the regex engine on every match.
    regex: Arc<Regex>,
}

impl PartialEq for CompiledRegex {
    fn eq(&self, other: &Self) -> bool {
        self.regex.as_str() == other.regex.as_str()
    }
}

#[derive(Clone, Default, PartialEq)]
struct Record {
    text: BTreeMap<String, String>,
    numbers: BTreeMap<String, f64>,
    skipped: bool,
    error: String,
}

impl Record {
    fn validate_field(&mut self, name: &str) -> bool {
        if name.is_empty() || name.len() > 64 {
            self.error = "Field names must be 1–64 bytes".into();
            return false;
        }
        if self.text.len() + self.numbers.len() >= MAX_FIELDS
            && !self.text.contains_key(name)
            && !self.numbers.contains_key(name)
        {
            self.error = "Too many fields (maximum 32)".into();
            return false;
        }
        true
    }
}

type ParseFn = TypedFunc<NoCtx, fn(Val<Captures>) -> Val<Record>>;

fn compile(script: &str) -> Result<ParseFn, String> {
    if script.len() > 64 * 1024 {
        return Err("Script exceeds 64 KiB".into());
    }
    let lib = library! {
        /// Convenience datetime; format returns empty text on parse/format failure.
        #[clone] type Datetime = Val<ScriptDatetime>;
        impl Val<ScriptDatetime> {
            fn parse(text: RotoString, format: RotoString) -> Val<ScriptDatetime> {
                Val(ScriptDatetime::parse(text.as_ref(), format.as_ref()))
            }
            fn parse_rfc3339(text: RotoString) -> Val<ScriptDatetime> {
                Val(ScriptDatetime::parse_rfc3339(text.as_ref()))
            }
            fn is_valid(dt: Val<ScriptDatetime>) -> bool {
                dt.0.is_valid()
            }
            fn to_local(dt: Val<ScriptDatetime>) -> Val<ScriptDatetime> {
                Val(dt.0.to_local())
            }
            fn fixed_offset_east(dt: Val<ScriptDatetime>, hours: u32) -> Val<ScriptDatetime> {
                Val(dt.0.fixed_offset(hours, true))
            }
            fn fixed_offset_west(dt: Val<ScriptDatetime>, hours: u32) -> Val<ScriptDatetime> {
                Val(dt.0.fixed_offset(hours, false))
            }
            fn floor_hours(dt: Val<ScriptDatetime>, width: u32) -> Val<ScriptDatetime> {
                Val(dt.0.floor_hours(width))
            }
            fn floor_minutes(dt: Val<ScriptDatetime>, width: u32) -> Val<ScriptDatetime> {
                Val(dt.0.floor_minutes(width))
            }
            fn floor_seconds(dt: Val<ScriptDatetime>, width: u32) -> Val<ScriptDatetime> {
                Val(dt.0.floor_seconds(width))
            }
            fn format(dt: Val<ScriptDatetime>, format: RotoString) -> RotoString {
                dt.0.format(format.as_ref())
            }
        }
        /// A datetime without timezone. Parse and format failures return None.
        #[clone] type NaiveDateTime = Val<NaiveTimestamp>;
        impl Val<NaiveTimestamp> {
            fn parse(text: RotoString, format: RotoString) -> Option<Val<NaiveTimestamp>> {
                NaiveTimestamp::parse(text.as_ref(), format.as_ref()).map(Val)
            }
            fn format(dt: Val<NaiveTimestamp>, format: RotoString) -> Option<RotoString> {
                dt.0.format(format.as_ref())
            }
        }
        /// An RFC 3339 datetime normalized to UTC, independent of local timezone.
        #[clone] type DateTime = Val<UtcTimestamp>;
        impl Val<UtcTimestamp> {
            fn parse_rfc3339(text: RotoString) -> Option<Val<UtcTimestamp>> {
                UtcTimestamp::parse_rfc3339(text.as_ref()).map(Val)
            }
            fn format(dt: Val<UtcTimestamp>, format: RotoString) -> Option<RotoString> {
                dt.0.format(format.as_ref())
            }
        }
        /// A Rust regex. Use a top-level Roto const to compile once during JIT.
        #[clone] type Regex = Val<CompiledRegex>;
        impl Val<CompiledRegex> {
            /// Invalid or oversized patterns return None; no implicit cache.
            fn compile(pattern: RotoString) -> Option<Val<CompiledRegex>> {
                #[cfg(test)]
                REGEX_COMPILE_CALLS.with(|calls| calls.set(calls.get() + 1));
                if pattern.len() > MAX_REGEX_BYTES {
                    return None;
                }
                Regex::new(pattern.as_ref()).ok().map(|regex| {
                    Val(CompiledRegex { regex: Arc::new(regex) })
                })
            }
            fn is_match(r: Val<CompiledRegex>, text: RotoString) -> bool {
                r.0.regex.is_match(text.as_ref())
            }
        }
        /// Named regular-expression captures. Numeric groups use "1", "2", etc.
        #[clone] type Captures = Val<Captures>;
        impl Val<Captures> {
            fn text(c: Val<Captures>, name: RotoString) -> RotoString {
                c.0.fields.get(name.as_ref()).cloned().unwrap_or_default().into()
            }
            fn has(c: Val<Captures>, name: RotoString) -> bool {
                c.0.fields.contains_key(name.as_ref())
            }
            /// Missing or invalid numbers become NaN; Record.number reports them.
            fn number(c: Val<Captures>, name: RotoString) -> f64 {
                c.0.fields.get(name.as_ref())
                    .and_then(|s| s.parse::<f64>().ok()).unwrap_or(f64::NAN)
            }
        }
        /// Output record consumed by Rust aggregation.
        #[clone] type Record = Val<Record>;
        impl Val<Record> {
            fn new() -> Val<Record> { Val(Record::default()) }
            fn skip() -> Val<Record> {
                Val(Record { skipped: true, ..Record::default() })
            }
            fn error(message: RotoString) -> Val<Record> {
                Val(Record { error: clipped(message.as_ref(), MAX_VALUE_BYTES), ..Record::default() })
            }
            fn text(mut r: Val<Record>, name: RotoString, value: RotoString) -> Val<Record> {
                if r.0.validate_field(name.as_ref()) {
                    if value.len() > MAX_VALUE_BYTES {
                        r.0.error = "Text value exceeds 1,024 bytes".into();
                    } else {
                        r.0.numbers.remove(name.as_ref());
                        r.0.text.insert(name.to_string(), value.to_string());
                    }
                }
                r
            }
            fn number(mut r: Val<Record>, name: RotoString, value: f64) -> Val<Record> {
                if r.0.validate_field(name.as_ref()) {
                    if !value.is_finite() {
                        r.0.error = "Numeric value must be finite".into();
                    } else {
                        r.0.text.remove(name.as_ref());
                        r.0.numbers.insert(name.to_string(), value);
                    }
                }
                r
            }
        }
        /// Remove query and fragment without decoding or changing the path.
        fn strip_query(value: RotoString) -> RotoString {
            value.as_ref().split(['?', '#']).next().unwrap_or_default().to_owned().into()
        }
        fn syslog_severity_name(value: u32) -> RotoString {
            crate::syslog::severity_name(value).into()
        }
        fn syslog_facility_name(value: u32) -> RotoString {
            crate::syslog::facility_name(value).into()
        }
        fn syslog_severity(line: RotoString) -> RotoString {
            crate::syslog::pri(line.as_ref()).map(|(pri, _)| crate::syslog::severity_name((pri % 8) as u32))
                .unwrap_or("").into()
        }
        fn syslog_facility(line: RotoString) -> RotoString {
            crate::syslog::pri(line.as_ref()).map(|(pri, _)| crate::syslog::facility_name((pri / 8) as u32))
                .unwrap_or("").into()
        }
    };
    let runtime = Runtime::from_lib(lib).map_err(|e| e.to_string())?;
    let mut package = FileTree::test_file("analysis.roto", script, 0)
        .compile(&runtime)
        .map_err(|e| {
            let mut message = String::new();
            let _ = e.write(&mut message, false);
            message
        })?;
    package
        .get_function::<fn(Val<Captures>) -> Val<Record>>("parse")
        .map_err(|e| format!("Expected fn parse(c: Captures) -> Record: {e}"))
}

fn clipped(value: &str, limit: usize) -> String {
    let mut end = value.len().min(limit);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

#[derive(Clone)]
pub struct Config {
    pub script: String,
    pub regex: String,
    pub group: String,
    pub metric: String,
    pub operation: String,
    pub input: InputConfig,
    // None preserves legacy single-group/single-metric callers.
    pub query: Option<String>,
}

pub struct Snapshot {
    pub bytes: Vec<u8>,
    pub first_line: usize,
    pub revision: u64,
    pub finished: bool,
}

struct Program {
    parse: Option<ParseFn>,
    input: input::Input,
    query: query::Query,
}

impl Program {
    fn new(config: &Config) -> Result<Self, String> {
        if config.regex.len() > MAX_REGEX_BYTES {
            return Err("Regex exceeds 16 KiB".into());
        }
        let query = match &config.query {
            Some(text) => query::Query::parse(text)?,
            None => query::Query::legacy(&config.group, &config.metric, &config.operation)?,
        };
        Ok(Self {
            query,
            input: input::Input::new(&config.input, &config.regex)?,
            parse: if config.script.trim().is_empty() {
                None
            } else {
                Some(compile(&config.script)?)
            },
        })
    }
}

fn aggregate(
    program: &Program,
    _config: &Config,
    snapshot: Snapshot,
    cancel: impl Fn() -> bool,
) -> Option<Value> {
    let started = Instant::now();
    let mut aggregator = query::Aggregator::new(&program.query);
    let mut total = 0usize;
    let mut matched = 0usize;
    let mut skipped = 0usize;
    let mut unmatched = 0usize;
    let mut errors = 0usize;
    let mut samples = Vec::new();
    let mut previews = Vec::new();
    let mut first = None;
    let mut last = None;
    let text = std::str::from_utf8(&snapshot.bytes).ok()?;
    for (index, raw) in text.split_inclusive('\n').enumerate() {
        if cancel() {
            return None;
        }
        // An unfinished line is not a record until newline/EOF arrives.
        if !raw.ends_with('\n') && !snapshot.finished {
            break;
        }
        let id = snapshot.first_line + index + 1;
        first.get_or_insert(id);
        last = Some(id);
        total += 1;
        let line = raw.trim_end_matches(['\r', '\n']);
        let previewing = previews.len() < SAMPLE_COUNT;
        let mut preview = if previewing {
            json!({"line":id, "text":clipped(line, 240)})
        } else {
            Value::Null
        };
        let accepted = line.len() <= MAX_LINE_BYTES && program.input.accepts(line);
        let extracted = if accepted {
            program.input.extract(line, program.parse.is_none())
        } else {
            Ok(None)
        };
        let failure = if line.len() > MAX_LINE_BYTES {
            errors += 1;
            Some("Line exceeds 256 KiB".to_owned())
        } else if !accepted {
            skipped += 1;
            if previewing {
                preview["status"] = json!("filtered");
            }
            None
        } else if let Err(error) = &extracted {
            errors += 1;
            Some(error.clone())
        } else if let Ok(Some(extracted)) = extracted {
            if previewing {
                preview["fields"] = json!(
                    extracted
                        .captures
                        .fields
                        .iter()
                        .take(MAX_FIELDS)
                        .map(|(k, v)| (k.clone(), clipped(v, 240)))
                        .collect::<BTreeMap<_, _>>()
                );
            }
            let record = match &program.parse {
                Some(parse) => parse.call(Val(extracted.captures)).0,
                None => extracted.record,
            };
            if !record.error.is_empty() {
                errors += 1;
                Some(record.error)
            } else if record.skipped {
                skipped += 1;
                if previewing {
                    preview["status"] = json!("skipped");
                }
                None
            } else {
                let error = aggregator.push(&record).err();
                if error.is_some() {
                    errors += 1;
                } else {
                    matched += 1;
                    if previewing {
                        preview["status"] = json!("accepted");
                    }
                }
                error
            }
        } else {
            unmatched += 1;
            if previewing {
                preview["status"] = json!("unmatched");
            }
            Some("Regex did not match".to_owned())
        };
        if previewing && let Some(reason) = &failure {
            preview["reason"] = json!(reason);
            if preview.get("status").is_none() {
                preview["status"] = json!("error");
            }
        }
        if previewing {
            previews.push(preview);
        }
        if let Some(reason) = failure
            && samples.len() < SAMPLE_COUNT
        {
            samples.push(json!({"line":id, "reason":reason, "text":clipped(line, 240)}));
        }
    }
    let (columns, rows) = aggregator.finish();
    Some(
        json!({"columns":columns, "rows":rows, "total":total, "matched":matched, "skipped":skipped,
        "unmatched":unmatched, "errors":errors, "samples":samples, "preview":previews,
        "firstLine":first, "lastLine":last, "elapsedMs":started.elapsed().as_secs_f64()*1000.0}),
    )
}

struct State {
    stop: AtomicBool,
    generation: AtomicU64,
    request: Mutex<Option<(u64, Config)>>,
    output: Mutex<Option<Value>>,
}

pub struct Worker {
    state: Arc<State>,
}

impl Worker {
    pub fn new(snapshot: impl Fn(Option<u64>) -> Option<Snapshot> + Send + 'static) -> Self {
        let state = Arc::new(State {
            stop: AtomicBool::new(false),
            generation: AtomicU64::new(0),
            request: Mutex::new(None),
            output: Mutex::new(None),
        });
        let shared = state.clone();
        std::thread::spawn(move || {
            let mut active: Option<(u64, Config, Program, f64)> = None;
            let mut revision = None;
            let mut next = Instant::now();
            while !shared.stop.load(Ordering::Relaxed) {
                let request = shared.request.lock().unwrap().take();
                if let Some((generation, config)) = request {
                    let started = Instant::now();
                    let program = Program::new(&config);
                    let compile_ms = started.elapsed().as_secs_f64() * 1000.0;
                    if shared.generation.load(Ordering::Relaxed) != generation {
                        continue;
                    }
                    match program {
                        Ok(program) => {
                            active = Some((generation, config, program, compile_ms));
                            revision = None;
                            next = Instant::now();
                        }
                        Err(error) => {
                            active = None;
                            *shared.output.lock().unwrap() = Some(
                                json!({"generation":generation, "error":error, "compileMs":compile_ms}),
                            );
                        }
                    }
                }
                if Instant::now() >= next
                    && let Some((generation, config, program, compile_ms)) = &active
                {
                    if let Some(input) = snapshot(revision) {
                        let current_revision = input.revision;
                        let cancelled = || {
                            shared.stop.load(Ordering::Relaxed)
                                || shared.generation.load(Ordering::Relaxed) != *generation
                        };
                        if let Some(mut result) = aggregate(program, config, input, cancelled)
                            && !cancelled()
                        {
                            result["generation"] = json!(generation);
                            result["compileMs"] = json!(compile_ms);
                            *shared.output.lock().unwrap() = Some(result);
                            revision = Some(current_revision);
                        }
                    }
                    next = Instant::now() + Duration::from_secs(2);
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        });
        Self { state }
    }
    pub fn submit(&self, config: Config) -> u64 {
        let generation = self.state.generation.fetch_add(1, Ordering::Relaxed) + 1;
        *self.state.request.lock().unwrap() = Some((generation, config));
        generation
    }
    pub fn take(&self) -> Option<Value> {
        self.state.output.lock().unwrap().take()
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.state.stop.store(true, Ordering::Relaxed);
        self.state.generation.fetch_add(1, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> Config {
        Config {
            script: ACCESS_SCRIPT.into(),
            regex: ACCESS_REGEX.into(),
            group: "path".into(),
            metric: String::new(),
            operation: "count".into(),
            input: InputConfig::default(),
            query: None,
        }
    }
    fn run(config: &Config, text: &str, finished: bool) -> Value {
        aggregate(
            &Program::new(config).unwrap(),
            config,
            Snapshot {
                bytes: text.as_bytes().to_vec(),
                first_line: 10,
                revision: 1,
                finished,
            },
            || false,
        )
        .unwrap()
    }

    fn mapped_config(format: &str, fields: Value) -> Config {
        let mut c = config();
        c.script.clear();
        c.group = "kind".into();
        c.metric = "total".into();
        c.operation = "sum".into();
        c.input = InputConfig::from_value(&json!({"format":format, "fields":fields})).unwrap();
        c
    }

    #[test]
    fn syslog_roto_helpers_return_names_without_pri_arithmetic() {
        let parse = compile(
            r#"fn parse(c: Captures) -> Record {
            Record.new()
                .text("emergency", syslog_severity_name(0))
                .text("kernel", syslog_facility_name(0))
                .text("severity", syslog_severity(c.text("line")))
                .text("facility", syslog_facility(c.text("line")))
                .text("invalid", syslog_severity_name(8))
        }"#,
        )
        .unwrap();
        for value in 0..192 {
            let record = parse
                .call(Val(Captures {
                    fields: Arc::new(BTreeMap::from([("line".into(), format!("<{value}>hello"))])),
                }))
                .0;
            assert_eq!(record.text["emergency"], "Emergency");
            assert_eq!(record.text["kernel"], "kernel");
            assert_eq!(
                record.text["severity"],
                crate::syslog::SEVERITIES[value % 8]
            );
            assert_eq!(
                record.text["facility"],
                crate::syslog::FACILITIES[value / 8]
            );
            assert_eq!(record.text["invalid"], "");
        }
        let record = parse.call(Val(Captures::default())).0;
        assert_eq!(record.text["severity"], "");
        assert_eq!(record.text["facility"], "");
    }

    #[test]
    fn roto_naive_datetime_groups_minutes_without_merging_dates() {
        let mut c = mapped_config(
            "delimiter",
            json!([
                {"name":"timestamp","source":"0","type":"text"},
                {"name":"elapsed","source":"1","type":"number"}
            ]),
        );
        c.query = Some("minute, avg(elapsed), max(elapsed), count()".into());
        c.script = r#"fn parse(c: Captures) -> Record {
            match NaiveDateTime.parse(c.text("timestamp"), "%Y%m%d %H:%M:%S") {
                Some(dt) => {
                    match dt.format("%Y-%m-%d %H:%M") {
                        Some(bucket) => Record.new().text("minute", bucket).number("elapsed", c.number("elapsed")),
                        None => Record.error("Invalid datetime format"),
                    }
                },
                None => Record.error("Invalid timestamp"),
            }
        }"#.into();
        let result = run(
            &c,
            "20261008 01:37:07,2\n20261008 01:37:59,4\n20261009 01:37:00,8\n20260230 01:37:00,1\n",
            true,
        );
        assert_eq!(result["matched"], 3);
        assert_eq!(result["errors"], 1);
        assert_eq!(
            result["rows"][0]["cells"],
            json!(["2026-10-08 01:37", 3.0, 4.0, 2])
        );
        assert_eq!(
            result["rows"][1]["cells"],
            json!(["2026-10-09 01:37", 8.0, 8.0, 1])
        );
    }

    #[test]
    fn roto_convenience_datetime_skips_bad_rows_without_match() {
        let mut c = mapped_config(
            "delimiter",
            json!([
                {"name":"timestamp","source":"0","type":"text"},
                {"name":"elapsed","source":"1","type":"number"}
            ]),
        );
        c.query = Some("minute, avg(elapsed), count()".into());
        c.script = r#"fn parse(c: Captures) -> Record {
            let dt = Datetime.parse(c.text("timestamp"), "%Y%m%d %H:%M:%S");
            let minute = dt.format("%Y-%m-%d %H:%M");
            if !dt.is_valid() || minute == "" {
                return Record.skip();
            }
            Record.new().text("minute", minute).number("elapsed", c.number("elapsed"))
        }"#
        .into();
        let result = run(
            &c,
            "20261008 01:37:07,2\n20261008 01:37:59,4\n20260230 01:37:00,100\ninvalid,100\n",
            true,
        );
        assert_eq!(result["matched"], 2);
        assert_eq!(result["skipped"], 2);
        assert_eq!(result["errors"], 0);
        assert_eq!(
            result["rows"][0]["cells"],
            json!(["2026-10-08 01:37", 3.0, 2])
        );
        assert!(result["samples"].as_array().unwrap().is_empty());
        c.script = c.script.replace("%Y-%m-%d %H:%M", "%Q");
        let result = run(&c, "20261008 01:37:07,2\n", true);
        assert_eq!(result["skipped"], 1);
        assert!(result["rows"].as_array().unwrap().is_empty());
    }

    #[test]
    fn roto_convenience_rfc3339_returns_utc_text_or_empty() {
        let parse = compile(
            r#"fn parse(c: Captures) -> Record {
            let dt = Datetime.parse_rfc3339(c.text("timestamp"));
            Record.new().text("minute", dt.format("%Y-%m-%d %H:%M"))
        }"#,
        )
        .unwrap();
        for (timestamp, expected) in [
            ("2026-10-08T01:37:07+09:00", "2026-10-07 16:37"),
            ("2026-10-08T01:37:07Z", "2026-10-08 01:37"),
            ("invalid", ""),
        ] {
            let record = parse
                .call(Val(Captures {
                    fields: Arc::new(BTreeMap::from([("timestamp".into(), timestamp.into())])),
                }))
                .0;
            assert_eq!(record.text["minute"], expected);
            assert!(record.error.is_empty());
        }
    }

    #[test]
    fn roto_datetime_zone_and_floor_chains_group_log_intervals() {
        let mut c = mapped_config(
            "delimiter",
            json!([
                {"name":"timestamp","source":"0","type":"text"},
                {"name":"elapsed","source":"1","type":"number"}
            ]),
        );
        c.query = Some("time, avg(elapsed), count()".into());
        c.script = r#"fn parse(c: Captures) -> Record {
            let dt = Datetime.parse_rfc3339(c.text("timestamp"))
                .fixed_offset_east(9).floor_minutes(5);
            let time = dt.format("%Y-%m-%d %H:%M %:z");
            if time == "" { return Record.skip(); }
            Record.new().text("time", time).number("elapsed", c.number("elapsed"))
        }"#
        .into();
        let result = run(
            &c,
            "2026-10-08T01:37:07Z,2\n2026-10-08T10:39:59+09:00,4\ninvalid,100\n",
            true,
        );
        assert_eq!(
            result["rows"][0]["cells"],
            json!(["2026-10-08 10:35 +09:00", 3.0, 2])
        );
        assert_eq!(result["skipped"], 1);
        let parse = compile(
            r#"fn parse(c: Captures) -> Record {
            let dt = Datetime.parse("20261008 10:37:47", "%Y%m%d %H:%M:%S")
                .fixed_offset_east(9).to_local().fixed_offset_west(9);
            Record.new()
                .text("hour", dt.floor_hours(1).format("%Y-%m-%d %H:%M %:z"))
                .text("second", dt.floor_seconds(15).format("%S"))
        }"#,
        )
        .unwrap();
        let record = parse.call(Val(Captures::default())).0;
        assert_eq!(record.text["hour"], "2026-10-07 16:00 -09:00");
        assert_eq!(record.text["second"], "45");
    }

    #[test]
    fn roto_rfc3339_datetime_groups_equivalent_offsets_in_utc() {
        let mut c = mapped_config(
            "delimiter",
            json!([
                {"name":"timestamp","source":"0","type":"text"},
                {"name":"elapsed","source":"1","type":"number"}
            ]),
        );
        c.query = Some("minute, avg(elapsed), count()".into());
        c.script = r#"fn parse(c: Captures) -> Record {
            match DateTime.parse_rfc3339(c.text("timestamp")) {
                Some(dt) => {
                    match dt.format("%Y-%m-%d %H:%M") {
                        Some(bucket) => Record.new().text("minute", bucket).number("elapsed", c.number("elapsed")),
                        None => Record.error("Invalid datetime format"),
                    }
                },
                None => Record.error("Invalid timestamp"),
            }
        }"#.into();
        let result = run(
            &c,
            "2026-10-08T01:37:07.185057405Z,2\n2026-10-08T10:37:59+09:00,4\n2026-10-08T01:37:07+09:00,8\n2026-10-08T01:37:07,1\n",
            true,
        );
        assert_eq!(result["matched"], 3);
        assert_eq!(result["errors"], 1);
        assert_eq!(
            result["rows"][0]["cells"],
            json!(["2026-10-07 16:37", 8.0, 1])
        );
        assert_eq!(
            result["rows"][1]["cells"],
            json!(["2026-10-08 01:37", 3.0, 2])
        );
    }

    #[test]
    fn roto_datetime_bad_formats_return_none_without_panicking() {
        let parse = compile(
            r#"fn parse(c: Captures) -> Record {
            match NaiveDateTime.parse("20261008 01:37:07", "%Y%m%d %H:%M:%S") {
                Some(dt) => {
                    match dt.format(c.text("format")) {
                        Some(value) => Record.new().text("value", value),
                        None => Record.skip(),
                    }
                },
                None => Record.error("Unexpected parse failure"),
            }
        }"#,
        )
        .unwrap();
        for format in ["%Q".to_owned(), "%z".to_owned(), "%Y".repeat(300)] {
            let record = parse
                .call(Val(Captures {
                    fields: Arc::new(BTreeMap::from([("format".into(), format)])),
                }))
                .0;
            assert!(record.skipped);
            assert!(record.error.is_empty());
        }
    }

    #[test]
    fn postgres_checkpoint_datetime_sample() {
        let mut c = config();
        c.input.filter = "checkpoint complete:".into();
        c.regex = r"^(?P<timestamp>\S+).*?write=(?P<write>[0-9]+(?:\.[0-9]+)?)\s+s,\s+sync=(?P<sync>[0-9]+(?:\.[0-9]+)?)\s+s,\s+total=(?P<total>[0-9]+(?:\.[0-9]+)?)\s+s".into();
        c.query = Some("minute, avg(total_ms), max(total_ms), count()".into());
        c.script = r#"fn parse(c: Captures) -> Record {
            match DateTime.parse_rfc3339(c.text("timestamp")) {
                Some(dt) => {
                    match dt.format("%Y-%m-%d %H:%M") {
                        Some(bucket) => Record.new().text("minute", bucket).number("total_ms", c.number("total") * 1000.0),
                        None => Record.error("Invalid datetime format"),
                    }
                },
                None => Record.error("Invalid RFC 3339 timestamp"),
            }
        }"#.into();
        let line = "2026-10-08T01:37:07.185057405Z 2026-10-08 01:37:07.184 UTC [28] LOG:  checkpoint complete: wrote 96 buffers (0.6%); 0 WAL file(s) added, 0 removed, 0 recycled; write=9.662 s, sync=0.008 s, total=9.708 s; sync files=27, longest=0.004 s, average=0.001 s; distance=562 kB, estimate=830 kB; lsn=0/C2D9098, redo lsn=0/C2D9060\n";
        let result = run(&c, line, true);
        assert_eq!(result["matched"], 1);
        assert_eq!(result["errors"], 0);
        assert_eq!(
            result["rows"][0]["cells"],
            json!(["2026-10-08 01:37", 9708.0, 9708.0, 1])
        );
    }

    #[test]
    fn builtin_postgres_checkpoint_template_runs_as_configured() {
        let preset = crate::analysis_presets::defaults()
            .into_iter()
            .find(|p| p["id"] == "builtin-pg-checkpoint")
            .unwrap();
        let settings = &preset["settings"];
        assert_eq!(preset["name"], "pg_checkpoint");
        let mut c = config();
        c.input = InputConfig::from_value(settings).unwrap();
        c.regex = settings["regex"].as_str().unwrap().into();
        c.script = settings["script"].as_str().unwrap().into();
        c.query = Some(settings["query"].as_str().unwrap().into());
        let line = "2026-10-08T04:17:00.236642062Z 2026-10-08 04:17:00.236 UTC [28] LOG:  checkpoint complete: wrote 2 buffers (0.0%); 1 WAL file(s) added, 0 removed, 0 recycled; write=0.133 s, sync=0.003 s, total=0.173 s; sync files=2; distance=512 kB, estimate=1024 kB\n";
        let second = line
            .replace("wrote 2", "wrote 4")
            .replace("1 WAL", "3 WAL")
            .replace("write=0.133", "write=0.333")
            .replace("sync=0.003", "sync=0.005")
            .replace("total=0.173", "total=0.373")
            .replace("distance=512", "distance=2048")
            .replace("estimate=1024", "estimate=4096");
        let result = run(&c, &format!("unrelated\n{line}{second}"), true);
        assert_eq!(result["matched"], 2);
        assert_eq!(result["skipped"], 1);
        assert_eq!(result["errors"], 0);
        assert_eq!(result["rows"].as_array().unwrap().len(), 1);
        let cells = result["rows"][0]["cells"].as_array().unwrap();
        assert_eq!(cells.len(), 9);
        let local = chrono::DateTime::parse_from_rfc3339("2026-10-08T04:17:00.236642062Z")
            .unwrap()
            .with_timezone(&chrono::Local);
        use chrono::Timelike;
        let bucket = local
            .with_minute(local.minute() / 30 * 30)
            .unwrap()
            .format("%Y-%m-%d %H:%M")
            .to_string();
        assert_eq!(cells[0], bucket);
        for (actual, expected) in cells[1..]
            .iter()
            .zip([3.0, 0.233, 0.004, 0.273, 0.373, 48.0, 2.0, 4.0])
        {
            assert!((actual.as_f64().unwrap() - expected).abs() < 1e-10);
        }
        assert_eq!(result["rows"][0]["count"], 2);
        let later = line.replace("T04:17:", "T04:47:");
        assert_eq!(
            run(&c, &format!("{line}{later}"), true)["rows"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            run(
                &c,
                &line.replace("2026-10-08T04:17:00.236642062Z", "invalid"),
                true
            )["skipped"],
            1
        );
        assert_eq!(
            run(
                &c,
                &line.replace("; distance=512 kB, estimate=1024 kB", ""),
                true
            )["unmatched"],
            1
        );
        // A wildcard decimal point must not accept malformed numeric tokens.
        assert_eq!(
            run(&c, &line.replace("write=0.133", "write=0x133"), true)["unmatched"],
            1
        );
    }

    #[test]
    fn query_dsl_aggregates_mapped_json_and_recomputes_snapshots() {
        let mut c = mapped_config(
            "json",
            json!([
                {"name":"path", "source":"/path", "type":"text"},
                {"name":"method", "source":"/method", "type":"text"},
                {"name":"status", "source":"/status", "type":"number"},
                {"name":"elapsed", "source":"/elapsed", "type":"number", "required":false},
                {"name":"bytes", "source":"/bytes", "type":"number", "required":false}
            ]),
        );
        c.query = Some(
            "avg(elapsed), path, method, status, sum(bytes), min(elapsed), max(elapsed), count()"
                .into(),
        );
        let program = Program::new(&c).unwrap();
        let first = concat!(
            "{\"path\":\"/a\",\"method\":\"GET\",\"status\":200,\"elapsed\":2,\"bytes\":10}\n",
            "{\"path\":\"/a\",\"method\":\"GET\",\"status\":200,\"elapsed\":4,\"bytes\":20}\n",
            "{\"path\":\"/a\",\"method\":\"GET\",\"status\":200,\"elapsed\":100}\n"
        );
        let result = aggregate(
            &program,
            &c,
            Snapshot {
                bytes: first.as_bytes().to_vec(),
                first_line: 0,
                revision: 1,
                finished: true,
            },
            || false,
        )
        .unwrap();
        assert_eq!(result["matched"], 2);
        assert_eq!(result["errors"], 1);
        assert_eq!(
            result["rows"][0]["cells"],
            json!([3.0, "/a", "GET", 200.0, 30.0, 2.0, 4.0, 2])
        );
        let second =
            "{\"path\":\"/b\",\"method\":\"POST\",\"status\":500,\"elapsed\":8,\"bytes\":5}\n";
        let result = aggregate(
            &program,
            &c,
            Snapshot {
                bytes: second.as_bytes().to_vec(),
                first_line: 3,
                revision: 2,
                finished: true,
            },
            || false,
        )
        .unwrap();
        assert_eq!(
            result["rows"][0]["cells"],
            json!([8.0, "/b", "POST", 500.0, 5.0, 8.0, 8.0, 1])
        );
        assert_eq!(result["rows"].as_array().unwrap().len(), 1);
        assert!(run(&c, "", true)["rows"].as_array().unwrap().is_empty());
        c.query = Some("".into());
        assert!(Program::new(&c).is_err());
    }

    #[test]
    fn ui_regex_filter_precedes_extraction_and_direct_numeric_aggregation() {
        let mut c = mapped_config(
            "regex",
            json!([
                {"name":"total", "source":"total", "type":"number"}
            ]),
        );
        c.group.clear();
        c.regex = r"write=(?P<write>[0-9.]+) s, sync=(?P<sync>[0-9.]+) s, total=(?P<total>[0-9.]+)"
            .into();
        c.input.filter = "checkpoint complete:".into();
        let line = "LOG:  checkpoint complete: wrote 54 buffers (0.3%); 0 WAL file(s) added, 0 removed, 0 recycled; write=5.399 s, sync=0.005 s, total=5.442\n";
        let result = run(
            &c,
            &format!("unrelated\n{line}LOG: checkpoint complete: malformed\n"),
            true,
        );
        assert_eq!(result["skipped"], 1);
        assert_eq!(result["matched"], 1);
        assert_eq!(result["unmatched"], 1);
        assert_eq!(result["rows"][0]["value"], 5.442);
        assert_eq!(result["preview"][0]["status"], "filtered");
        assert_eq!(result["preview"][1]["fields"]["total"], "5.442");
        assert_eq!(result["samples"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn ui_delimiter_whitespace_and_roto_share_mapped_fields() {
        let mappings = json!([
            {"name":"kind", "source":"0", "type":"text"},
            {"name":"total", "source":"2", "type":"number"}
        ]);
        let mut c = mapped_config("delimiter", mappings.clone());
        let result = run(&c, "a,,2\na,,3\nb,,bad\nmissing\n", true);
        assert_eq!(result["rows"][0]["value"], 5.0);
        assert_eq!(result["errors"], 2);
        c.input.format = "whitespace".into();
        let result = run(&c, "a   ignored\t2\na ignored 3\n", true);
        assert_eq!(result["rows"][0]["value"], 5.0);
        c.script = "fn parse(c: Captures) -> Record { Record.new().text(\"kind\", c.text(\"kind\")).number(\"total\", c.number(\"total\") * 2.0) }".into();
        assert_eq!(run(&c, "a ignored 2\n", true)["rows"][0]["value"], 4.0);
        c.script.clear();
        c.input.format = "delimiter".into();
        c.input.delimiter = "::".into();
        assert_eq!(run(&c, "a::::2\n", true)["rows"][0]["value"], 2.0);
    }

    #[test]
    fn ui_json_lines_preserve_types_and_diagnose_missing_null_and_malformed() {
        let c = mapped_config(
            "json",
            json!([
                {"name":"kind", "source":"/event", "type":"text"},
                {"name":"total", "source":"/timing/total", "type":"number"},
                {"name":"optional", "source":"/optional", "type":"text", "required":false}
            ]),
        );
        let result = run(
            &c,
            concat!(
                "{\"event\":\"checkpoint\",\"timing\":{\"total\":5.442}}\n",
                "{\"event\":\"checkpoint\",\"timing\":{\"total\":2}}\n",
                "{\"event\":\"checkpoint\",\"timing\":{\"total\":\"3\"}}\n",
                "{\"event\":\"checkpoint\",\"timing\":{\"total\":null}}\n",
                "{\"event\":\"checkpoint\"}\n",
                "invalid json\n"
            ),
            true,
        );
        assert_eq!(result["matched"], 2);
        assert_eq!(result["errors"], 4);
        assert_eq!(result["unmatched"], 0);
        assert_eq!(result["rows"][0]["value"], 7.442);
        assert_eq!(result["preview"].as_array().unwrap().len(), 5);
    }

    #[test]
    fn ui_filter_modes_validation_and_limits() {
        for (mode, pattern) in [("contains", "KEEP"), ("prefix", "KEEP"), ("regex", "^KEEP")] {
            let mut c = config();
            c.script.clear();
            c.group.clear();
            c.regex = "^(?P<line>.*)$".into();
            c.input.filter_mode = mode.into();
            c.input.filter = pattern.into();
            c.input.ignore_case = true;
            assert_eq!(run(&c, "keep this\ndrop that\n", true)["skipped"], 1);
            c.input.exclude = true;
            assert_eq!(run(&c, "keep this\ndrop that\n", true)["matched"], 1);
        }
        for settings in [
            json!({"format":"unknown"}),
            json!({"format":"delimiter", "delimiter":""}),
            json!({"format":"json", "fields":[]}),
            json!({"filterMode":"regex", "filter":"["}),
            json!({"fields":[{"name":"x", "source":"missing", "type":"text"}]}),
            json!({"format":"json", "fields":[{"name":"x", "source":"/bad~2", "type":"text"}]}),
            json!({"fields":[{"name":"x", "source":"0", "type":"text"}, {"name":"x", "source":"0", "type":"text"}]}),
        ] {
            let mut c = config();
            c.input = InputConfig::from_value(&settings).unwrap();
            assert!(Program::new(&c).is_err());
        }
        assert!(InputConfig::from_value(&json!({"fields":"bad"})).is_err());
        let mut c = mapped_config(
            "delimiter",
            json!([
                {"name":"kind", "source":"0", "type":"text"},
                {"name":"total", "source":"1", "type":"number"}
            ]),
        );
        assert_eq!(run(&c, "a,NaN\na,inf\n", true)["errors"], 2);
        c.input.filter = "keep".into();
        assert_eq!(run(&c, "not json", false)["total"], 0);
    }

    #[test]
    fn ui_json_pointer_escapes_optional_fields_and_text_limits() {
        let mut c = mapped_config(
            "json",
            json!([
                {"name":"kind", "source":"/a~1b/~0key/0", "type":"text"},
                {"name":"total", "source":"/value", "type":"number"},
                {"name":"optional", "source":"/absent", "type":"text", "required":false}
            ]),
        );
        let line = json!({"a/b":{"~key":["日本語"]}, "value":0}).to_string();
        let result = run(&c, &line, true);
        assert_eq!(result["rows"][0]["group"], "日本語");
        assert_eq!(result["rows"][0]["value"], 0.0);
        let invalid = json!({"a/b":{"~key":["日本語"]}, "value":0, "absent":false}).to_string();
        assert_eq!(run(&c, &invalid, true)["errors"], 1);
        c.input.format = "delimiter".into();
        c.input.delimiter = "、".into();
        c.input.fields[0].source = "0".into();
        c.input.fields[1].source = "1".into();
        c.input.fields[2].source = "2".into();
        assert_eq!(run(&c, "日本語、0", true)["matched"], 1);
        assert_eq!(
            run(&c, &format!("{}、0", "x".repeat(MAX_VALUE_BYTES + 1)), true)["errors"],
            1
        );
        c.input.fields[0].source = "00".into();
        assert!(Program::new(&c).is_err());
    }
    #[test]
    fn script_compiles_regex_and_reuses_it_for_matching() {
        REGEX_COMPILE_CALLS.with(|calls| calls.set(0));
        let parse = compile(
            r#"fn parse(c: Captures) -> Record {
            match Regex.compile("^日本語[0-9]+$") {
                Some(re) => {
                    if re.is_match("日本語123") && !re.is_match("日本語abc")
                        && re.is_match("日本語456") {
                        Record.new().text("result", "ok")
                    } else {
                        Record.error("Unexpected match result")
                    }
                },
                None => Record.error("Regex compilation failed"),
            }
        }"#,
        )
        .unwrap();
        assert_eq!(REGEX_COMPILE_CALLS.with(|calls| calls.get()), 0);
        for _ in 0..3 {
            let record = parse.call(Val(Captures::default())).0;
            assert!(record.error.is_empty(), "{}", record.error);
            assert_eq!(record.text["result"], "ok");
        }
        // Contrast with a const: a local call compiles once per parse invocation.
        assert_eq!(REGEX_COMPILE_CALLS.with(|calls| calls.get()), 3);
    }

    #[test]
    fn script_regex_compilation_handles_dynamic_and_invalid_patterns() {
        let parse = compile(
            r#"fn parse(c: Captures) -> Record {
            match Regex.compile(c.text("pattern")) {
                Some(re) => {
                    if re.is_match(c.text("input")) {
                        Record.new()
                    } else {
                        Record.skip()
                    }
                },
                None => Record.error("Invalid regex"),
            }
        }"#,
        )
        .unwrap();
        for (pattern, input, invalid, skipped) in [
            ("^hello$".to_owned(), "hello", false, false),
            ("^hello$".to_owned(), "goodbye", false, true),
            ("[".to_owned(), "hello", true, false),
            ("(?=hello)".to_owned(), "hello", true, false),
            ("x".repeat(MAX_REGEX_BYTES + 1), "hello", true, false),
            (String::new(), "", false, false),
        ] {
            let record = parse
                .call(Val(Captures {
                    fields: Arc::new(BTreeMap::from([
                        ("pattern".to_owned(), pattern),
                        ("input".to_owned(), input.to_owned()),
                    ])),
                }))
                .0;
            assert_eq!(!record.error.is_empty(), invalid);
            assert_eq!(record.skipped, skipped);
        }
    }

    #[test]
    fn script_const_regex_compiles_once_across_lines_and_snapshots() {
        REGEX_COMPILE_CALLS.with(|calls| calls.set(0));
        let mut c = config();
        c.regex = "^(?P<line>.*)$".into();
        c.group.clear();
        c.script = r#"const IGNORE: Regex? = Regex.compile("^DEBUG:");

        fn parse(c: Captures) -> Record {
            match IGNORE {
                Some(re) => {
                    if re.is_match(c.text("line")) {
                        Record.skip()
                    } else {
                        Record.new()
                    }
                },
                None => Record.error("Invalid regex"),
            }
        }"#
        .into();
        let program = Program::new(&c).unwrap();
        // The host function has already run before the first line is processed.
        assert_eq!(REGEX_COMPILE_CALLS.with(|calls| calls.get()), 1);
        for revision in 1..=2 {
            let result = aggregate(
                &program,
                &c,
                Snapshot {
                    bytes: b"DEBUG: ignore\nINFO: keep\n".repeat(revision as usize),
                    first_line: 0,
                    revision,
                    finished: true,
                },
                || false,
            )
            .unwrap();
            assert_eq!(result["matched"], revision);
            assert_eq!(result["skipped"], revision);
            assert_eq!(result["unmatched"], 0);
            assert_eq!(result["errors"], 0);
            assert_eq!(REGEX_COMPILE_CALLS.with(|calls| calls.get()), 1);
        }
        // Reapplying settings creates a new program and initializes its const.
        let _reapplied = Program::new(&c).unwrap();
        assert_eq!(REGEX_COMPILE_CALLS.with(|calls| calls.get()), 2);
    }

    #[test]
    fn script_const_invalid_regex_returns_none_without_recompiling() {
        REGEX_COMPILE_CALLS.with(|calls| calls.set(0));
        let parse = compile(
            r#"const INVALID: Regex? = Regex.compile("[");
            fn parse(c: Captures) -> Record {
                match INVALID {
                    Some(re) => Record.error("Unexpected successful compilation"),
                    None => Record.skip(),
                }
            }"#,
        )
        .unwrap();
        assert_eq!(REGEX_COMPILE_CALLS.with(|calls| calls.get()), 1);
        for _ in 0..3 {
            let record = parse.call(Val(Captures::default())).0;
            assert!(record.skipped);
            assert!(record.error.is_empty());
        }
        assert_eq!(REGEX_COMPILE_CALLS.with(|calls| calls.get()), 1);
    }

    #[test]
    fn access_query_grouping_and_diagnostics() {
        let line = "2026-10-07T11:37:54.236480684Z INFO:     123.123.123.123:57260 - \"HEAD /ui/policies/?page=1 HTTP/1.1\" 200 OK\n";
        let result = run(
            &config(),
            &format!("{line}{line}invalid\nunfinished"),
            false,
        );
        assert_eq!(result["total"], 3);
        assert_eq!(result["matched"], 2);
        assert_eq!(result["unmatched"], 1);
        assert_eq!(result["rows"][0]["group"], "/ui/policies/");
        assert_eq!(result["rows"][0]["value"], 2.0);
        assert_eq!(result["samples"][0]["line"], 13);
        assert_eq!(result["lastLine"], 13);
    }
    #[test]
    fn apache_numeric_operations_and_missing_values() {
        let mut c = Config {
            script: APACHE_SCRIPT.into(),
            regex: APACHE_REGEX.into(),
            group: "path".into(),
            metric: "duration_ms".into(),
            operation: "average".into(),
            input: InputConfig::default(),
            query: None,
        };
        let input = "127.0.0.1 - - [07/Oct/2026:11:37:54 +0000] \"GET /x?a=1 HTTP/1.1\" 200 42 1000\n127.0.0.1 - - [07/Oct/2026:11:37:54 +0000] \"GET /x HTTP/1.1\" 200 42 3000\n127.0.0.1 - - [07/Oct/2026:11:37:54 +0000] \"GET /x HTTP/1.1\" 200 42\n";
        for (op, expected) in [("average", 2.0), ("sum", 4.0), ("min", 1.0), ("max", 3.0)] {
            c.operation = op.into();
            let result = run(&c, input, true);
            assert_eq!(result["rows"][0]["value"], expected);
            assert_eq!(result["errors"], 1);
        }
    }
    #[test]
    fn errors_skip_eof_and_cancel() {
        assert!(compile("fn parse(c: Captures) -> bool { true }").is_err());
        assert!(compile("not valid roto").is_err());
        let mut c = config();
        c.regex = "^(?P<n>.*)$".into();
        c.script =
            "fn parse(c: Captures) -> Record { Record.new().number(\"value\", c.number(\"n\")) }"
                .into();
        c.group.clear();
        let result = run(&c, "oops\n", true);
        assert_eq!(result["errors"], 1);
        c.script = "fn parse(c: Captures) -> Record { Record.skip() }".into();
        assert_eq!(run(&c, "last", true)["skipped"], 1);
        let p = Program::new(&c).unwrap();
        assert!(
            aggregate(
                &p,
                &c,
                Snapshot {
                    bytes: b"x\n".to_vec(),
                    first_line: 0,
                    revision: 0,
                    finished: true
                },
                || true
            )
            .is_none()
        );
    }

    #[test]
    fn snapshots_follow_retention_and_worker_uses_latest_config() {
        let input = Arc::new(Mutex::new(Snapshot {
            bytes: b"a\nb\n".to_vec(),
            first_line: 0,
            revision: 1,
            finished: true,
        }));
        let shared = input.clone();
        let worker = Worker::new(move |previous| {
            let input = shared.lock().unwrap();
            (previous != Some(input.revision)).then(|| Snapshot {
                bytes: input.bytes.clone(),
                first_line: input.first_line,
                revision: input.revision,
                finished: input.finished,
            })
        });
        let mut c = config();
        c.regex = "^(?P<key>.*)$".into();
        c.script =
            "fn parse(c: Captures) -> Record { Record.new().text(\"key\", c.text(\"key\")) }"
                .into();
        c.group = "key".into();
        let wait = |expected: u64| {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                if let Some(result) = worker.take()
                    && result["generation"].as_u64() == Some(expected)
                {
                    break result;
                }
                assert!(Instant::now() < deadline, "Analysis worker timeout");
                std::thread::sleep(Duration::from_millis(20));
            }
        };
        assert_eq!(wait(worker.submit(c.clone()))["total"], 2);
        *input.lock().unwrap() = Snapshot {
            bytes: b"b\nc\n".to_vec(),
            first_line: 1,
            revision: 2,
            finished: true,
        };
        let result = wait(worker.submit(c.clone()));
        assert_eq!(result["firstLine"], 2);
        assert_eq!(result["rows"][0]["group"], "b");
        assert_eq!(result["rows"][1]["group"], "c");
        let mut invalid = c.clone();
        invalid.script = "invalid".into();
        worker.submit(invalid);
        assert_eq!(wait(worker.submit(c))["matched"], 2);
    }

    #[test]
    fn diagnostics_are_bounded_and_group_limit_is_explicit() {
        let mut c = config();
        c.regex = "^(?P<key>.*)$".into();
        c.script =
            "fn parse(c: Captures) -> Record { Record.new().text(\"key\", c.text(\"key\")) }"
                .into();
        c.group = "key".into();
        let text: String = (0..MAX_GROUPS + 7).map(|i| format!("{i}\n")).collect();
        let result = run(&c, &text, true);
        assert_eq!(result["rows"].as_array().unwrap().len(), MAX_GROUPS);
        assert_eq!(result["errors"], 7);
        assert_eq!(result["samples"].as_array().unwrap().len(), SAMPLE_COUNT);
        assert_eq!(
            run(&c, &format!("{}\n", "x".repeat(MAX_VALUE_BYTES + 1)), true)["errors"],
            1
        );
        assert_eq!(
            run(&c, &format!("{}\n", "x".repeat(MAX_LINE_BYTES + 1)), true)["errors"],
            1
        );
    }

    #[test]
    #[ignore = "manual throughput measurement; no timing assertion"]
    fn benchmark_retained_access_logs() {
        let c = config();
        let program = Program::new(&c).unwrap();
        let line = "2026-10-07T11:37:54.236480684Z INFO:     123.123.123.123:57260 - \"HEAD /ui/policies/?page=1 HTTP/1.1\" 200 OK\n";
        let bytes = line.repeat(200_000).into_bytes();
        let size = bytes.len();
        let result = aggregate(
            &program,
            &c,
            Snapshot {
                bytes,
                first_line: 0,
                revision: 1,
                finished: true,
            },
            || false,
        )
        .unwrap();
        assert_eq!(result["matched"], 200_000);
        let ms = result["elapsedMs"].as_f64().unwrap();
        eprintln!(
            "ANALYSIS_BENCH: 200000 lines, {size} bytes, {ms:.1} ms, {:.0} lines/s",
            200_000.0 / (ms / 1000.0)
        );
    }
}
