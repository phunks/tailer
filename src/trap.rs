use regex::{Regex, RegexBuilder};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::ops::Range;

pub const MAX_RULES: usize = 10;
pub const DEFAULT_COLOR: &str = "#35c66b";
const CARRY_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug)]
pub struct Rule {
    pub text: String,
    pub regex: bool,
    pub ignore_case: bool,
    pub color: String,
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
                Some(
                    json!({"text":text, "regex":v["regex"].as_bool().unwrap_or(false),
                "ignoreCase":v["ignoreCase"].as_bool().unwrap_or(false), "color":color}),
                )
            })
            .take(MAX_RULES)
            .collect::<Vec<_>>()
    )
}

#[derive(Clone)]
pub struct CompiledRule {
    pub rule: Rule,
    expression: Option<Regex>,
}

impl CompiledRule {
    fn ranges(&self, text: &str) -> Vec<Range<usize>> {
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
            text: text.into(),
            regex: entry["regex"].as_bool().unwrap_or(false),
            ignore_case: entry["ignoreCase"].as_bool().unwrap_or(false),
            color: color.into(),
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
        compiled.push(CompiledRule { rule, expression });
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
        self.carry.clear();
        self.processed = 0;
        self.base = 0;
        self.seen = vec![HashSet::new(); self.rules.len()];
    }
    pub fn receive(&mut self, data: &[u8]) {
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
            for range in rule.ranges(&decoded) {
                // A growing greedy regex match must not alert again at the same start.
                if range.end > old && seen.insert(self.base + range.start) {
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
    // Earlier tags win overlapping ranges. Keep non-overlapping parts of later tags.
    let mut spans: Vec<(Range<usize>, &str)> = Vec::new();
    for compiled in rules {
        let mut additions = Vec::new();
        for range in compiled.ranges(text) {
            let mut start = range.start;
            let first = spans.partition_point(|(taken, _)| taken.end <= start);
            for (taken, _) in &spans[first..] {
                if taken.start >= range.end {
                    break;
                }
                if start < taken.start {
                    additions.push((start..taken.start, compiled.rule.color.as_str()));
                }
                start = start.max(taken.end);
                if start >= range.end {
                    break;
                }
            }
            if start < range.end {
                additions.push((start..range.end, compiled.rule.color.as_str()));
            }
        }
        spans.extend(additions);
        spans.sort_by_key(|(range, _)| range.start);
    }
    spans.sort_by_key(|(range, _)| range.start);
    let mut output = String::from("<pre>");
    let mut previous = 0;
    for (range, color) in spans {
        if range.start < previous {
            continue;
        }
        output.push_str(&escape(&text[previous..range.start]));
        output.push_str(&format!("<span style=\"color:{color};font-weight:bold\">"));
        output.push_str(&escape(&text[range.clone()]));
        output.push_str("</span>");
        previous = range.end;
    }
    output.push_str(&escape(&text[previous..]));
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
