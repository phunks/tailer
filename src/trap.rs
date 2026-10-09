use regex::{Regex, RegexBuilder};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::ops::Range;

pub const MAX_RULES: usize = 10;
pub const DEFAULT_COLOR: &str = "#35c66b";
const CARRY_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug)]
pub struct Rule {
    pub enabled: bool,
    pub text: String,
    pub regex: bool,
    pub ignore_case: bool,
    pub color: String,
    pub background: bool,
    pub opacity: u32,
    pub scope: String,
}

pub fn valid_color(color: &str) -> bool {
    color.len() == 7 && color.starts_with('#') && color[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

pub fn sanitize_rules(value: &Value) -> Value {
    json!(
        value
            .as_array()
            .into_iter()
            .flatten()
            .filter(|v| v.is_object())
            .filter_map(|v| {
                let text: String = v["text"]
                    .as_str()
                    .unwrap_or_default()
                    .chars()
                    .take(1024)
                    .collect();
                if text.is_empty() {
                    return None;
                }
                let color = v["color"]
                    .as_str()
                    .filter(|s| valid_color(s))
                    .unwrap_or(DEFAULT_COLOR);
                let mut clean = json!({"text":text, "regex":v["regex"].as_bool().unwrap_or(false),
                    "ignoreCase":v["ignoreCase"].as_bool().unwrap_or(false), "color":color});
                if v["enabled"].as_bool() == Some(false) {
                    clean["enabled"] = json!(false);
                }
                if !v["numeric"].is_null() {
                    clean["numeric"] = crate::numeric::sanitize(&v["numeric"]);
                }
                if v["background"].as_bool().unwrap_or(false) {
                    clean["background"] = json!(true);
                }
                if !v["opacity"].is_null() {
                    clean["opacity"] =
                        json!(v["opacity"].as_f64().unwrap_or(25.0).clamp(0.0, 100.0) as u32);
                }
                if let Some(scope) = v["scope"]
                    .as_str()
                    .filter(|s| ["capture", "match", "line"].contains(s))
                {
                    clean["scope"] = json!(scope);
                }
                Some(clean)
            })
            .take(MAX_RULES)
            .collect::<Vec<_>>()
    )
}

#[derive(Clone)]
pub struct CompiledRule {
    pub rule: Rule,
    expression: Option<Regex>,
    numeric: Option<crate::numeric::Condition>,
}

impl CompiledRule {
    fn ranges(&self, text: &str) -> Vec<Range<usize>> {
        if !self.rule.enabled {
            return Vec::new();
        }
        if self.numeric.is_some() || self.rule.scope == "line" || self.rule.scope == "capture" {
            let mut result = Vec::new();
            let mut offset = 0;
            for line in text.split_inclusive('\n') {
                let content = line.trim_end_matches(['\r', '\n']);
                if let Some(expression) = &self.expression {
                    for captures in expression.captures_iter(content) {
                        if self
                            .numeric
                            .as_ref()
                            .is_some_and(|condition| !condition.matches(&captures))
                        {
                            continue;
                        }
                        let Some(matched) = captures.get(if self.rule.scope == "capture" {
                            self.numeric.as_ref().map_or(1, |n| n.capture)
                        } else {
                            0
                        }) else {
                            continue;
                        };
                        let range = if self.rule.scope == "line" {
                            offset..offset + content.len()
                        } else {
                            offset + matched.start()..offset + matched.end()
                        };
                        if !range.is_empty() {
                            result.push(range);
                        }
                        if self.rule.scope == "line" {
                            break;
                        }
                    }
                } else if !ranges(content, &self.rule.text, self.rule.ignore_case).is_empty() {
                    if !content.is_empty() {
                        result.push(offset..offset + content.len());
                    }
                }
                offset += line.len();
            }
            return result;
        }
        match &self.expression {
            Some(expression) => expression
                .find_iter(text)
                .filter(|m| !m.is_empty())
                .map(|m| m.start()..m.end())
                .collect(),
            None => ranges(text, &self.rule.text, self.rule.ignore_case),
        }
    }
}

pub fn compile_rules(value: &Value) -> Result<Vec<CompiledRule>, String> {
    let input = value.as_array().ok_or("Invalid trap rules")?;
    if input.len() > MAX_RULES {
        return Err("A maximum of 10 tags is allowed.".into());
    }
    let mut compiled = Vec::new();
    for entry in input {
        let text = entry["text"].as_str().unwrap_or_default();
        if text.is_empty() {
            return Err("Enter text to detect.".into());
        }
        if text.chars().count() > 1024 {
            return Err("Trap text must be at most 1,024 characters.".into());
        }
        let color = entry["color"].as_str().unwrap_or(DEFAULT_COLOR);
        if !valid_color(color) {
            return Err("Choose a color in #RRGGBB format.".into());
        }
        let rule = Rule {
            enabled: entry["enabled"].as_bool().unwrap_or(true),
            text: text.into(),
            regex: entry["regex"].as_bool().unwrap_or(false),
            ignore_case: entry["ignoreCase"].as_bool().unwrap_or(false),
            color: color.into(),
            background: entry["background"].as_bool().unwrap_or(false),
            opacity: entry["opacity"].as_f64().unwrap_or(25.0).clamp(0.0, 100.0) as u32,
            scope: entry["scope"].as_str().unwrap_or("match").into(),
        };
        let expression = if rule.regex {
            Some(
                RegexBuilder::new(text)
                    .case_insensitive(rule.ignore_case)
                    .build()
                    .map_err(|e| format!("Invalid regular expression: {e}"))?,
            )
        } else {
            None
        };
        let numeric = crate::numeric::Condition::compile(&entry["numeric"], expression.as_ref())?;
        if !["capture", "match", "line"].contains(&rule.scope.as_str()) {
            return Err("Invalid highlight scope.".into());
        }
        if rule.scope == "capture"
            && expression
                .as_ref()
                .is_none_or(|r| r.captures_len() <= numeric.as_ref().map_or(1, |n| n.capture))
        {
            return Err("The selected capture group does not exist.".into());
        }
        compiled.push(CompiledRule {
            rule,
            expression,
            numeric,
        });
    }
    Ok(compiled)
}

#[derive(Default)]
pub struct Trap {
    rules: Vec<CompiledRule>,
    carry: Vec<u8>,
    processed: usize,
    base: usize,
    seen: Vec<HashSet<usize>>,
    pub hits: u64,
    syslog_labels: bool,
    pending_line: Vec<u8>,
    oversized_line: bool,
}

pub fn ranges(text: &str, needle: &str, ignore_case: bool) -> Vec<Range<usize>> {
    if needle.is_empty() {
        return Vec::new();
    }
    if !ignore_case {
        return text
            .match_indices(needle)
            .map(|(i, s)| i..i + s.len())
            .collect();
    }
    let needle = needle.to_lowercase();
    let mut folded = String::new();
    let mut offsets = Vec::new();
    for (start, c) in text.char_indices() {
        for lower in c.to_lowercase() {
            folded.push(lower);
            offsets.extend(std::iter::repeat_n(
                (start, start + c.len_utf8()),
                lower.len_utf8(),
            ));
        }
    }
    folded
        .match_indices(&needle)
        .map(|(i, s)| offsets[i].0..offsets[i + s.len() - 1].1)
        .collect()
}

impl Trap {
    #[cfg(test)]
    pub fn configure(&mut self, text: String, ignore_case: bool) {
        let value = sanitize_rules(&json!([{"text":text,"ignoreCase":ignore_case}]));
        self.configure_rules(compile_rules(&value).unwrap());
    }
    pub fn configure_rules(&mut self, rules: Vec<CompiledRule>) {
        self.rules = rules;
        self.reset();
    }
    pub fn reset(&mut self) {
        self.pending_line.clear();
        self.oversized_line = false;
        self.carry.clear();
        self.processed = 0;
        self.base = 0;
        self.seen = vec![HashSet::new(); self.rules.len()];
    }
    pub fn receive(&mut self, data: &[u8]) {
        if self.rules.is_empty() || data.is_empty() {
            return;
        }
        if !self.syslog_labels {
            self.receive_text(data);
            return;
        }
        // Convert completed lines once, never a partial PRI. Buffering is bounded;
        // oversized lines are skipped for notification and recover at the newline.
        for part in data.split_inclusive(|&byte| byte == b'\n') {
            if self.pending_line.len() + part.len() > CARRY_BYTES {
                self.pending_line.clear();
                self.oversized_line = true;
            }
            if !self.oversized_line {
                self.pending_line.extend_from_slice(part);
            }
            if part.ends_with(b"\n") {
                if !self.oversized_line {
                    let line = String::from_utf8_lossy(&self.pending_line);
                    let display = crate::syslog::display_text(&line);
                    // Each Syslog line is a separate record: ^/$ must apply to
                    // every message, not only the first retained carry buffer.
                    self.carry.clear();
                    self.processed = 0;
                    self.base = 0;
                    for seen in &mut self.seen {
                        seen.clear();
                    }
                    self.receive_text(display.as_bytes());
                } else {
                    // Prevent a match spanning a discarded line.
                    self.carry.clear();
                    self.processed = 0;
                    self.base = 0;
                    for seen in &mut self.seen {
                        seen.clear();
                    }
                }
                self.pending_line.clear();
                self.oversized_line = false;
            }
        }
    }
    pub fn set_syslog_labels(&mut self, enabled: bool) {
        if self.syslog_labels != enabled {
            self.syslog_labels = enabled;
            // Switching presentation is not new reception. Keep cumulative hits.
            self.reset();
        }
    }
    fn receive_text(&mut self, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        if self.rules.is_empty() {
            return;
        }
        let previous = self.processed;
        self.carry.extend_from_slice(data);
        // Keep incomplete UTF-8 until the next receive, rather than matching a
        // replacement character introduced solely by transport fragmentation.
        let mut complete = self.carry.len();
        if let Err(error) = std::str::from_utf8(&self.carry)
            && error.error_len().is_none()
        {
            complete = error.valid_up_to();
        }
        let decoded = String::from_utf8_lossy(&self.carry[..complete]);
        let old = String::from_utf8_lossy(&self.carry[..previous.min(complete)]).len();
        for (rule, seen) in self.rules.iter().zip(&mut self.seen) {
            // Numeric conditions are evaluated only on complete lines: a partial "12"
            // may grow into "128", changing both the numeric value and an end anchor.
            let input = if rule.numeric.is_some() {
                &decoded[..decoded.rfind('\n').map_or(0, |i| i + 1)]
            } else if self.syslog_labels {
                decoded.trim_end_matches(['\r', '\n'])
            } else {
                &decoded
            };
            for range in rule.ranges(input) {
                // A growing greedy regex match must not alert again at the same start.
                if (rule.numeric.is_some() || range.end > old)
                    && seen.insert(self.base + range.start)
                {
                    self.hits += 1;
                }
            }
        }
        let keep = if self.rules.iter().any(|r| r.rule.regex) {
            CARRY_BYTES
        } else {
            self.rules
                .iter()
                .map(|r| r.rule.text.chars().count() * 4 + 4)
                .max()
                .unwrap_or(4)
        };
        let mut start = complete.saturating_sub(keep);
        while start > 0 && self.carry[start] & 0xc0 == 0x80 {
            start -= 1;
        }
        self.base += String::from_utf8_lossy(&self.carry[..start]).len();
        for seen in &mut self.seen {
            seen.retain(|&position| position >= self.base);
        }
        self.carry.drain(..start);
        self.processed = complete - start;
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
pub fn highlight(text: &str, needle: &str, ignore_case: bool) -> String {
    let value = sanitize_rules(&json!([{"text":needle,"ignoreCase":ignore_case}]));
    highlight_rules(text, &compile_rules(&value).unwrap())
}

pub fn highlight_rules(text: &str, rules: &[CompiledRule]) -> String {
    highlight_syslog(text, rules, false)
}

pub fn highlight_syslog(text: &str, rules: &[CompiledRule], badges: bool) -> String {
    let display;
    let raw = text;
    let text = if badges {
        display = crate::syslog::display_text(raw);
        display.as_str()
    } else {
        raw
    };
    // Qt Quick can carry a translucent character background over a literal
    // newline in <pre>, painting the following row twice. Explicit line breaks
    // preserve the same plain text and line metrics without that format bleed.
    let explicit_breaks = rules.iter().any(|compiled| compiled.rule.background);
    let escaped = |text: &str| {
        let html = escape(text);
        if explicit_breaks {
            html.replace('\n', "<br/>")
        } else {
            html
        }
    };
    // Earlier tags win overlapping ranges. Keep non-overlapping parts of later tags.
    let mut spans: Vec<(Range<usize>, &Rule)> = Vec::new();
    for compiled in rules {
        let mut additions = Vec::new();
        let ranges = if badges {
            let mut ranges = Vec::new();
            let mut offset = 0;
            for line in text.split_inclusive('\n') {
                ranges.extend(
                    compiled
                        .ranges(line.trim_end_matches(['\r', '\n']))
                        .into_iter()
                        .map(|range| offset + range.start..offset + range.end),
                );
                offset += line.len();
            }
            ranges
        } else {
            compiled.ranges(text)
        };
        for range in ranges {
            let mut start = range.start;
            let first = spans.partition_point(|(taken, _)| taken.end <= start);
            for (taken, _) in &spans[first..] {
                if taken.start >= range.end {
                    break;
                }
                if start < taken.start {
                    additions.push((start..taken.start, &compiled.rule));
                }
                start = start.max(taken.end);
                if start >= range.end {
                    break;
                }
            }
            if start < range.end {
                additions.push((start..range.end, &compiled.rule));
            }
        }
        spans.extend(additions);
        spans.sort_by_key(|(range, _)| range.start);
    }
    spans.sort_by_key(|(range, _)| range.start);
    let mut decorations = Vec::new();
    if badges {
        let mut offset = 0;
        for line in raw.split_inclusive('\n') {
            if let Some((start, length, value)) = crate::syslog::decoration(line) {
                let badge_len = crate::syslog::badge(value).len();
                decorations.push((offset + start..offset + start + badge_len, value));
                offset += line.len() + badge_len - length;
            } else {
                offset += line.len();
            }
        }
    }
    let mut output = String::from("<pre>");
    let mut previous = 0;
    // Explicit trap styles take precedence over default label colors, including
    // capture/line/background styles. Split decorations around matched spans.
    let mut rendered = Vec::new();
    for (range, value) in decorations {
        let mut start = range.start;
        let first = spans.partition_point(|(hidden, _)| hidden.end <= start);
        for (hidden, _) in &spans[first..] {
            if hidden.start >= range.end {
                break;
            }
            if hidden.end <= start || hidden.start >= range.end {
                continue;
            }
            if start < hidden.start {
                rendered.push((start..hidden.start, None, Some(value)));
            }
            start = start.max(hidden.end);
        }
        if start < range.end {
            rendered.push((start..range.end, None, Some(value)));
        }
    }
    for (range, rule) in spans {
        rendered.push((range, Some(rule), None));
    }
    rendered.sort_by_key(|(range, _, _)| range.start);
    for (range, rule, badge) in rendered {
        if range.start < previous {
            continue;
        }
        output.push_str(&escaped(&text[previous..range.start]));
        if let Some(value) = badge {
            output.push_str(&format!(
                "<span style=\"color:{};font-weight:bold\">{}</span>",
                crate::syslog::color(value),
                escaped(&text[range.clone()])
            ));
            previous = range.end;
            continue;
        }
        let rule = rule.unwrap();
        if rule.background {
            let alpha = (rule.opacity * 255 + 50) / 100;
            output.push_str(&format!(
                "<span style=\"background-color:#{alpha:02x}{}\">",
                &rule.color[1..]
            ));
        } else {
            output.push_str(&format!(
                "<span style=\"color:{};font-weight:bold\">",
                rule.color
            ));
        }
        output.push_str(&escaped(&text[range.clone()]));
        output.push_str("</span>");
        previous = range.end;
    }
    output.push_str(&escaped(&text[previous..]));
    output.push_str("</pre>");
    output
}

pub fn context_position(text: &str, selected: Option<usize>) -> i32 {
    let Some(selected) = selected else { return -1 };
    let prefix = format!("{selected}: ");
    let mut position = 0;
    for line in text.split_inclusive('\n') {
        if line.starts_with(&prefix) {
            return position;
        }
        position += line.encode_utf16().count() as i32;
    }
    -1
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn syslog_badges_match_display_and_escape_body() {
        let rules =
            compile_rules(&json!([{"text":r"\[INFO\|LOC7\].*", "regex":true, "color":"#ef5350"}]))
                .unwrap();
        let raw = "<190>hello <script>&\n42: <0>panic\n<192>unchanged\n";
        let html = highlight_syslog(raw, &rules, true);
        assert!(html.contains("[INFO|LOC7]"));
        assert!(html.contains("[EMRG|KERN]"));
        assert!(html.contains("hello &lt;script&gt;&amp;"));
        assert!(html.contains("color:#ef5350;font-weight:bold\">[INFO|LOC7] hello"));
        assert!(!html.contains("&lt;190&gt;"));
        assert!(html.contains("&lt;192&gt;unchanged"));
        assert_eq!(
            highlight_syslog(raw, &rules, false),
            highlight_rules(raw, &rules)
        );
    }
    #[test]
    fn syslog_traps_handle_fragmentation_toggle_and_bounded_lines() {
        let rules = compile_rules(&json!([
            {"text":r"^\[WARN\|LOC7\].* DROP (\d+)$", "regex":true,
             "numeric":{"capture":1,"operator":">","value":"10"}},
            {"text":"[WARN|LOC7]", "ignoreCase":true},
            {"text":"<188>"}
        ]))
        .unwrap();
        let mut trap = Trap::default();
        trap.configure_rules(rules);
        trap.set_syslog_labels(true);
        for byte in "<188>日本語 DROP 12\n".as_bytes() {
            trap.receive(&[*byte]);
        }
        assert_eq!(trap.hits, 2);
        trap.receive(b"");
        assert_eq!(trap.hits, 2);
        trap.set_syslog_labels(false);
        assert_eq!(
            trap.hits, 2,
            "Changing presentation must not replay history"
        );
        trap.receive(b"<188>raw\n");
        assert_eq!(trap.hits, 3);
        trap.set_syslog_labels(true);
        trap.receive(b"<18");
        trap.configure_rules(compile_rules(&json!([{"text":"[WARN|LOC7]"}])).unwrap());
        trap.receive(b"8>discard partial\n");
        assert_eq!(trap.hits, 3);
        trap.receive(b"<188>next\n");
        assert_eq!(trap.hits, 4);
        trap.receive(&vec![b'x'; CARRY_BYTES + 100]);
        assert!(trap.pending_line.len() <= CARRY_BYTES);
        trap.receive(b"\n<188>recovered\n");
        assert_eq!(trap.hits, 5);
        trap.configure_rules(
            compile_rules(&json!([
                {"text":r"^\[WARN\|LOC7\].* DROP$", "regex":true}
            ]))
            .unwrap(),
        );
        trap.receive(b"<188>first DROP\n<188>second DROP\n");
        assert_eq!(trap.hits, 7);
    }

    #[test]
    fn syslog_highlights_support_label_captures_and_backgrounds() {
        let rules = compile_rules(&json!([
            {"text":r"\[(INFO)\|LOC7\]", "regex":true, "scope":"capture", "color":"#abcdef"}
        ]))
        .unwrap();
        let html = highlight_syslog("<190>body\n", &rules, true);
        assert!(html.contains("color:#abcdef;font-weight:bold\">INFO</span>"));
        assert!(html.contains("|LOC7] "));
        let rules = compile_rules(&json!([
            {"text":r"^\[INFO\|LOC7\].* (\d+)$", "regex":true, "scope":"line",
             "background":true, "color":"#abcdef", "opacity":25,
             "numeric":{"capture":1,"operator":">","value":"10"}}
        ]))
        .unwrap();
        let html = highlight_syslog("<190>body 12\n<190>body 2\n", &rules, true);
        assert!(html.contains("background-color:#40abcdef\">[INFO|LOC7] body 12</span>"));
        assert!(!html.contains("<190>"));
        assert!(!highlight_syslog("<190>body 12\n", &rules, false).contains("background-color"));
        let rules = compile_rules(&json!([
            {"text":r"^\[INFO\|LOC7\].* DROP$", "regex":true, "color":"#abcdef"}
        ]))
        .unwrap();
        let html = highlight_syslog("<190>first DROP\n<190>second DROP\n", &rules, true);
        assert_eq!(html.matches("color:#abcdef").count(), 2);
    }
    #[test]
    fn disabled_rules_preserve_conditions_without_detection_or_highlighting() {
        let mut value = json!([{"text":"ERROR.*", "regex":true, "enabled":false}]);
        let clean = sanitize_rules(&value);
        assert_eq!(clean[0]["enabled"], false);
        assert_eq!(sanitize_rules(&clean), clean);
        let mut trap = Trap::default();
        let rules = compile_rules(&value).unwrap();
        assert_eq!(
            highlight_rules("ERROR test", &rules),
            "<pre>ERROR test</pre>"
        );
        trap.configure_rules(rules);
        trap.receive(b"ERROR test\n");
        assert_eq!(trap.hits, 0);
        value[0]["enabled"] = json!(true);
        let rules = compile_rules(&value).unwrap();
        assert!(highlight_rules("ERROR test", &rules).contains("color:"));
        trap.configure_rules(rules);
        trap.receive(b"ERROR new\n");
        assert_eq!(trap.hits, 1);
        assert!(
            compile_rules(&json!([{"text":"legacy"}])).unwrap()[0]
                .rule
                .enabled
        );
    }
    #[test]
    fn numeric_traps_wait_for_newlines_and_highlight_scopes() {
        let rule = json!({"text":r".* (\d+)$", "regex":true,
            "numeric":{"capture":1,"operator":">","value":"100"},
            "background":true,"color":"#ff0000","opacity":25,"scope":"capture"});
        let compiled = compile_rules(&json!([rule.clone()])).unwrap();
        let mut trap = Trap::default();
        trap.configure_rules(compiled.clone());
        trap.receive(b"xxxxxx 200 8 1");
        trap.receive(b"28");
        assert_eq!(trap.hits, 0);
        trap.receive(b"\nxxxxxx 200 8 100\n");
        assert_eq!(trap.hits, 1);
        trap.receive(b"xxxxxx 200 8 129\n");
        assert_eq!(trap.hits, 2);
        assert_eq!(
            highlight_rules("xxxxxx 200 8 128\nxxxxxx 200 8 100\n", &compiled),
            "<pre>xxxxxx 200 8 <span style=\"background-color:#40ff0000\">128</span><br/>xxxxxx 200 8 100<br/></pre>"
        );
        for scope in ["match", "line"] {
            let mut rule = rule.clone();
            rule["scope"] = json!(scope);
            assert_eq!(
                highlight_rules(
                    "xxxxxx 200 8 128\n",
                    &compile_rules(&json!([rule])).unwrap()
                ),
                "<pre><span style=\"background-color:#40ff0000\">xxxxxx 200 8 128</span><br/></pre>"
            );
        }
        let clean = sanitize_rules(&json!([rule]));
        assert_eq!(sanitize_rules(&clean), clean);
        assert!(compile_rules(&json!([{"text":"x", "numeric":{"value":"100"}}])).is_err());
    }
    #[test]
    fn multiple_rules_regex_fragments_and_growing_matches() {
        let rules = compile_rules(&json!([
            {"text":"ERROR.*?aaaa", "regex":true, "color":"#ff0000"},
            {"text":"warn", "ignoreCase":true},
            {"text":"障害"}
        ]))
        .unwrap();
        let mut trap = Trap::default();
        trap.configure_rules(rules);
        trap.receive(b"ERROR xyz aa");
        assert_eq!(trap.hits, 0);
        trap.receive("aa WARN 障害".as_bytes());
        assert_eq!(trap.hits, 3);
        trap.receive(b" unrelated");
        assert_eq!(trap.hits, 3);
        trap.configure_rules(compile_rules(&json!([{"text":"ERROR.*", "regex":true}])).unwrap());
        trap.receive(b"ERROR");
        trap.receive(b" extended");
        assert_eq!(trap.hits, 4);
        trap.receive(b"\nERROR again");
        assert_eq!(trap.hits, 5);
        trap.reset();
        trap.receive(b"ERROR");
        assert_eq!(trap.hits, 6);
    }

    #[test]
    fn colors_overlap_priority_and_html_are_safe() {
        let rules = compile_rules(&json!([
            {"text":"RR", "color":"#ff0000"},
            {"text":"ERROR.*?aaaa", "regex":true, "color":"#0000ff"}
        ]))
        .unwrap();
        let html = highlight_rules("<ERROR & aaaa>", &rules);
        assert!(html.contains("color:#ff0000;font-weight:bold\">RR</span>"));
        assert!(html.contains("color:#0000ff;font-weight:bold\">E</span>"));
        assert!(html.contains("color:#0000ff;font-weight:bold\">OR &amp; aaaa</span>"));
        assert!(html.starts_with("<pre>&lt;"));
        assert!(html.ends_with("&gt;</pre>"));
    }

    #[test]
    fn validation_limits_and_zero_width_regex() {
        assert!(compile_rules(&json!([{"text":"[", "regex":true}])).is_err());
        assert!(compile_rules(&json!([{"text":"x", "color":"red\" onclick=\"bad"}])).is_err());
        assert!(compile_rules(&json!([{"text":""}])).is_err());
        assert!(compile_rules(&json!(vec![json!({"text":"x"}); 11])).is_err());
        assert!(compile_rules(&json!([{"text":"x".repeat(1025)}])).is_err());
        let rules = compile_rules(&json!([{"text":"^|$", "regex":true}])).unwrap();
        assert_eq!(highlight_rules("abc", &rules), "<pre>abc</pre>");
        let mut trap = Trap::default();
        trap.configure_rules(rules);
        trap.receive(b"abc");
        assert_eq!(trap.hits, 0);
    }

    #[test]
    fn regex_carry_is_bounded_and_utf8_safe() {
        let mut trap = Trap::default();
        trap.configure_rules(compile_rules(&json!([{"text":"障害.*aaaa", "regex":true}])).unwrap());
        trap.receive("日".repeat(CARRY_BYTES).as_bytes());
        assert!(trap.carry.len() <= CARRY_BYTES + 3);
        for byte in "障害aaaa".bytes() {
            trap.receive(&[byte]);
        }
        assert_eq!(trap.hits, 1);
        trap.receive(b"more");
        assert_eq!(trap.hits, 1);
    }
    #[test]
    fn context_position_is_exact_and_uses_qt_utf16_offsets() {
        let text = "6: 日本語😀\n7: target\n8: next\n";
        assert_eq!(
            context_position(text, Some(7)),
            "6: 日本語😀\n".encode_utf16().count() as i32
        );
        assert_eq!(
            context_position(text, Some(8)),
            "6: 日本語😀\n7: target\n".encode_utf16().count() as i32
        );
        assert_eq!(context_position(text, None), -1);
        assert_eq!(context_position(text, Some(1)), -1);
    }
    #[test]
    fn split_receives_and_old_overlap_do_not_retrigger() {
        let mut trap = Trap::default();
        trap.configure("ERROR".into(), true);
        trap.receive(b"Er");
        assert_eq!(trap.hits, 0);
        trap.receive(b"ror");
        assert_eq!(trap.hits, 1);
        trap.receive(b" unrelated");
        assert_eq!(trap.hits, 1);
        trap.configure("new".into(), false);
        trap.receive(b"new");
        assert_eq!(trap.hits, 2);
        trap.configure(String::new(), false);
        trap.receive(b"new");
        assert_eq!(trap.hits, 2);
    }
    #[test]
    fn unicode_fragmentation_and_html_escape() {
        let mut trap = Trap::default();
        trap.configure("障害".into(), false);
        let bytes = "障害".as_bytes();
        for byte in bytes {
            trap.receive(&[*byte]);
        }
        assert_eq!(trap.hits, 1);
        let html = highlight("<script>ÄERROR&", "äerror", true);
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains(">ÄERROR</span>&amp;"));
    }
}
