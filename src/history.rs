use regex::{Regex, RegexBuilder};
use std::collections::VecDeque;
#[cfg(test)]
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read};
#[cfg(test)]
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

pub const DISPLAY_BYTES: usize = 256 * 1024;
pub const DISPLAY_LINES: usize = 2000;
pub const RETAIN_BYTES: usize = 25 * 1024 * 1024;
pub const RETAIN_LINES: usize = 200_000;

/// Lazily allocated byte ring with an incremental line-boundary index.
#[derive(Default)]
pub struct Ring {
    data: VecDeque<u8>,
    ends: VecDeque<usize>,
    start: usize,
}

impl Ring {
    pub fn len(&self) -> usize {
        self.data.len()
    }
    pub fn lines(&self) -> usize {
        self.ends.len()
            + usize::from(self.ends.back().copied().unwrap_or(self.start) < self.start + self.len())
    }
    pub fn reader(&self) -> impl Read + '_ {
        let (a, b) = self.data.as_slices();
        a.chain(b)
    }
    pub fn write_to(&self, output: &mut impl io::Write) -> io::Result<()> {
        let (a, b) = self.data.as_slices();
        output.write_all(a)?;
        output.write_all(b)
    }
    pub fn preview(&self) -> String {
        let start = self.len().saturating_sub(DISPLAY_BYTES + 4);
        let bytes: Vec<_> = self.data.iter().skip(start).copied().collect();
        let mut text = String::from_utf8_lossy(&bytes).into_owned();
        trim_text(&mut text);
        text
    }
    /// Return evicted bytes and complete lines. UTF-8 tails remain valid;
    /// UTF-16 original-byte windows retain code-unit alignment.
    pub fn append(
        &mut self,
        input: &[u8],
        limit: usize,
        lines: usize,
        encoding: &str,
    ) -> (usize, usize) {
        let before = self.start + self.len();
        let previous = self.data.back().copied();
        self.data.extend(input);
        let width = if encoding.starts_with("UTF-16") { 2 } else { 1 };
        for (i, &byte) in input.iter().enumerate() {
            let position = before + i;
            let newline = if width == 1 {
                byte == b'\n'
            } else if position % 2 == 1 {
                let first = if i == 0 { previous } else { Some(input[i - 1]) };
                if encoding == "UTF-16LE" {
                    first == Some(10) && byte == 0
                } else {
                    first == Some(0) && byte == 10
                }
            } else {
                false
            };
            if newline {
                self.ends.push_back(position + 1);
            }
        }
        let mut cut = self.start + self.len().saturating_sub(limit);
        let excess = self.lines().saturating_sub(lines);
        if excess > 0 {
            cut = cut.max(self.ends[excess - 1]);
        }
        if cut > self.start {
            if let Some(&end) = self
                .ends
                .iter()
                .find(|&&end| end >= cut && end < self.start + self.len())
            {
                cut = end;
            }
            if width == 2 {
                cut = cut.div_ceil(2) * 2;
            }
            if encoding == "UTF-8" {
                while let Some(&byte) = self.data.get(cut - self.start) {
                    if byte & 0xc0 != 0x80 {
                        break;
                    }
                    cut += 1;
                }
            }
        }
        let mut removed_lines = 0;
        while self.ends.front().is_some_and(|&end| end <= cut) {
            self.ends.pop_front();
            removed_lines += 1;
        }
        let removed = cut - self.start;
        self.data.drain(..removed);
        self.start = cut;
        (removed, removed_lines)
    }
}

#[derive(Clone, Default)]
pub struct Pattern {
    pub text: String,
    pub regex: bool,
    pub ignore_case: bool,
    pub invert: bool,
    pub numeric: serde_json::Value,
}

#[derive(Clone, Default)]
pub struct Query {
    pub generation: u64,
    pub filter: Pattern,
    pub search: Pattern,
    pub context: Option<usize>,
}

#[derive(Debug, Default)]
pub struct Matches {
    pub lines: VecDeque<(usize, String)>,
    pub count: usize,
}

fn compile(pattern: &Pattern) -> Result<Regex, String> {
    let expression = if pattern.regex {
        pattern.text.clone()
    } else {
        regex::escape(&pattern.text)
    };
    RegexBuilder::new(&expression)
        .case_insensitive(pattern.ignore_case)
        .build()
        .map_err(|error| error.to_string())
}

#[cfg(test)]
pub fn validate(pattern: &Pattern) -> Result<(), String> {
    compile(pattern).map(|_| ())
}

/// Bounded line decoder. Even a single enormous log line cannot exhaust RAM.
pub fn read_lines<R: Read>(
    reader: R,
    mut visit: impl FnMut(String) -> io::Result<()>,
) -> io::Result<()> {
    let mut reader = BufReader::new(reader);
    let mut pending = Vec::new();
    let mut truncated = false;
    loop {
        let buf = reader.fill_buf()?;
        if buf.is_empty() {
            if !pending.is_empty() || truncated {
                let mut text = String::from_utf8_lossy(&pending).into_owned();
                if truncated {
                    text.push_str(&crate::i18n::text(" …[long line truncated]"));
                }
                visit(text)?;
            }
            return Ok(());
        }
        let newline = buf.iter().position(|&b| b == b'\n');
        let used = newline.map_or(buf.len(), |i| i + 1);
        let data = &buf[..newline.unwrap_or(buf.len())];
        let available = DISPLAY_BYTES.saturating_sub(pending.len());
        pending.extend_from_slice(&data[..data.len().min(available)]);
        truncated |= data.len() > available;
        reader.consume(used);
        if newline.is_some() {
            let mut text = String::from_utf8_lossy(&pending).into_owned();
            if truncated {
                text.push_str(&crate::i18n::text(" …[long line truncated]"));
            }
            visit(text)?;
            pending.clear();
            truncated = false;
        }
    }
}

#[cfg(test)]
pub fn scan(
    path: &Path,
    pattern: &Pattern,
    stop: &AtomicBool,
    generation: &AtomicU64,
    expected: u64,
) -> Result<Matches, String> {
    scan_from(path, pattern, stop, generation, expected, 0)
}

#[cfg(test)]
pub fn scan_from(
    path: &Path,
    pattern: &Pattern,
    stop: &AtomicBool,
    generation: &AtomicU64,
    expected: u64,
    first_line: usize,
) -> Result<Matches, String> {
    scan_reader(
        File::open(path).map_err(|error| error.to_string())?,
        pattern,
        stop,
        generation,
        expected,
        first_line,
    )
}

pub fn scan_reader(
    reader: impl Read,
    pattern: &Pattern,
    stop: &AtomicBool,
    generation: &AtomicU64,
    expected: u64,
    first_line: usize,
) -> Result<Matches, String> {
    let matcher = compile(pattern)?;
    let numeric = crate::numeric::Condition::compile(
        &pattern.numeric,
        if pattern.regex { Some(&matcher) } else { None },
    )?;
    let mut matches = Matches::default();
    let mut bytes = 0;
    let mut number = first_line;
    read_lines(reader, |line| {
        if stop.load(Ordering::Relaxed) || generation.load(Ordering::Relaxed) != expected {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Search cancelled",
            ));
        }
        number += 1;
        let matched = match &numeric {
            Some(condition) => matcher
                .captures_iter(&line)
                .any(|captures| condition.matches(&captures)),
            None => matcher.is_match(&line),
        };
        if matched != pattern.invert {
            matches.count += 1;
            bytes += line.len();
            matches.lines.push_back((number, line));
            while bytes > DISPLAY_BYTES || matches.lines.len() > DISPLAY_LINES {
                bytes -= matches.lines.pop_front().unwrap().1.len();
            }
        }
        Ok(())
    })
    .map_err(|error| error.to_string())?;
    Ok(matches)
}

#[cfg(test)]
pub fn context(path: &Path, target: usize) -> io::Result<String> {
    context_from(path, target, 0)
}

#[cfg(test)]
pub fn context_from(path: &Path, target: usize, first_line: usize) -> io::Result<String> {
    context_reader(File::open(path)?, target, first_line)
}

pub fn context_reader(reader: impl Read, target: usize, first_line: usize) -> io::Result<String> {
    let mut index = first_line;
    let mut text = String::new();
    read_lines(reader, |line| {
        index += 1;
        if index >= target.saturating_sub(10).max(1)
            && index <= target.saturating_add(10)
            && text.len() < DISPLAY_BYTES
        {
            text.push_str(&format!("{index}: {line}\n"));
        }
        if index > target.saturating_add(10) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "Completed"));
        }
        Ok(())
    })
    .or_else(|e| {
        if e.kind() == io::ErrorKind::Interrupted {
            Ok(())
        } else {
            Err(e)
        }
    })?;
    trim_text(&mut text);
    Ok(text)
}

pub fn trim_text(text: &mut String) {
    trim_to_limit(text, DISPLAY_BYTES);
}

/// Keep only the latest displayable window; return the number of evicted lines.
pub fn trim_to_limit(text: &mut String, limit: usize) -> usize {
    let mut start = text.len().saturating_sub(limit);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    let excess = text.lines().count().saturating_sub(DISPLAY_LINES);
    if excess > 0
        && let Some((pos, _)) = text.match_indices('\n').nth(excess - 1)
    {
        start = start.max(pos + 1);
    }
    // Prefer whole lines when the byte limit falls inside an older line.
    // A single oversized line retains its UTF-8-safe suffix instead.
    if start > 0
        && text.as_bytes()[start - 1] != b'\n'
        && let Some(pos) = text[start..].find('\n')
        && start + pos + 1 < text.len()
    {
        start += pos + 1;
    }
    let removed = text[..start].bytes().filter(|&b| b == b'\n').count();
    text.drain(..start);
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;

    #[test]
    fn retention_ring_is_lazy_and_bounds_lines_independently_of_display() {
        let mut ring = Ring::default();
        assert_eq!(ring.data.capacity(), 0);
        assert_eq!(ring.ends.capacity(), 0);
        let input = "line\n".repeat(RETAIN_LINES + 10);
        assert_eq!(
            ring.append(input.as_bytes(), RETAIN_BYTES, RETAIN_LINES, "UTF-8"),
            (50, 10)
        );
        assert_eq!(ring.lines(), RETAIN_LINES);
        assert_eq!(ring.preview().lines().count(), DISPLAY_LINES);
        assert_eq!(
            ring.append(b"last", RETAIN_BYTES, RETAIN_LINES, "UTF-8"),
            (5, 1)
        );
        assert_eq!(ring.lines(), RETAIN_LINES);
        assert!(ring.preview().ends_with("last"));
    }

    #[test]
    fn retention_ring_handles_byte_overflow_unicode_and_split_utf16() {
        let mut ring = Ring::default();
        ring.append("日本語\n".as_bytes(), 5, RETAIN_LINES, "UTF-8");
        let mut bytes = Vec::new();
        ring.reader().read_to_end(&mut bytes).unwrap();
        assert_eq!(String::from_utf8(bytes).unwrap(), "語\n");
        for (encoding, little) in [("UTF-16LE", true), ("UTF-16BE", false)] {
            let mut ring = Ring::default();
            for unit in "古\n新\n末".encode_utf16() {
                for byte in if little {
                    unit.to_le_bytes()
                } else {
                    unit.to_be_bytes()
                } {
                    ring.append(&[byte], 100, 2, encoding);
                }
            }
            assert_eq!(ring.lines(), 2);
            let mut bytes = Vec::new();
            ring.reader().read_to_end(&mut bytes).unwrap();
            let mut decoder = crate::encoding::Decoder::new(encoding).unwrap();
            assert_eq!(decoder.decode(&bytes, true).unwrap(), "新\n末");
        }
        let mut ring = Ring::default();
        ring.append(
            &vec![b'x'; RETAIN_BYTES + 100],
            RETAIN_BYTES,
            RETAIN_LINES,
            "UTF-8",
        );
        assert_eq!(ring.len(), RETAIN_BYTES);
        assert!(ring.preview().len() <= DISPLAY_BYTES);
    }

    #[test]
    fn ring_trims_whole_lines_and_unicode_suffixes() {
        let mut text = "old\nnew\n".to_owned();
        assert_eq!(trim_to_limit(&mut text, 5), 1);
        assert_eq!(text, "new\n");
        let mut text = "日本語\n".to_owned();
        assert_eq!(trim_to_limit(&mut text, 5), 0);
        assert_eq!(text, "語\n");
        let mut text = format!("{}last", "line\n".repeat(DISPLAY_LINES));
        assert_eq!(trim_to_limit(&mut text, DISPLAY_BYTES), 1);
        assert_eq!(text.lines().count(), DISPLAY_LINES);
    }

    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn new(text: &str) -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "tailer-search-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::write(&path, text).unwrap();
            Self(path)
        }
        fn scan(&self, pattern: Pattern) -> Matches {
            scan(
                &self.0,
                &pattern,
                &AtomicBool::new(false),
                &AtomicU64::new(0),
                0,
            )
            .unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn scans_literal_regex_unicode_and_inverted_lines() {
        let fixture = Fixture::new(
            "[error]\nÄERROR\n\"GET /ui/ HTTP/1.1\" 403 0\n\"GET /ui/ HTTP/1.1\" 200 403\nlast",
        );
        let literal = fixture.scan(Pattern {
            text: "[error]".into(),
            ..Pattern::default()
        });
        assert_eq!(literal.lines[0], (1, "[error]".into()));
        let unicode = fixture.scan(Pattern {
            text: "äerror".into(),
            ignore_case: true,
            ..Pattern::default()
        });
        assert_eq!(unicode.lines[0].0, 2);
        let access = fixture.scan(Pattern {
            text: r#""GET [^"]*" (403|500)(\s|$)"#.into(),
            regex: true,
            ..Pattern::default()
        });
        assert_eq!(access.count, 1);
        assert_eq!(access.lines[0].0, 3);
        let inverted = fixture.scan(Pattern {
            text: "GET".into(),
            invert: true,
            ..Pattern::default()
        });
        assert_eq!(inverted.count, 3);
        assert_eq!(inverted.lines.back().unwrap(), &(5, "last".into()));
        assert_eq!(fixture.scan(Pattern::default()).count, 5);
        assert_eq!(
            fixture
                .scan(Pattern {
                    invert: true,
                    ..Pattern::default()
                })
                .count,
            0
        );
    }

    #[test]
    fn scan_bounds_results_and_cancels_even_without_matches() {
        let fixture = Fixture::new(&"line\n".repeat(DISPLAY_LINES + 20));
        let matches = fixture.scan(Pattern::default());
        assert_eq!(matches.count, DISPLAY_LINES + 20);
        assert_eq!(matches.lines.len(), DISPLAY_LINES);
        assert_eq!(matches.lines[0].0, 21);
        for (stop, generation) in [(true, 0), (false, 1)] {
            assert!(
                scan(
                    &fixture.0,
                    &Pattern {
                        text: "absent".into(),
                        ..Pattern::default()
                    },
                    &AtomicBool::new(stop),
                    &AtomicU64::new(generation),
                    0
                )
                .unwrap_err()
                .contains("cancelled")
            );
        }
    }

    #[test]
    fn invalid_regex_is_reported() {
        assert!(
            validate(&Pattern {
                text: "[".into(),
                regex: true,
                ..Pattern::default()
            })
            .is_err()
        );
        assert!(
            validate(&Pattern {
                text: "[".into(),
                ..Pattern::default()
            })
            .is_ok()
        );
    }

    #[test]
    fn numeric_filters_scan_all_history_and_support_inversion() {
        let fixture = Fixture::new(
            "xxxxxx 200 8 128\nxxxxxx 200 8 100\nxxxxxx 200 8 -1.5\nxxxxxx 200 8 bad\n",
        );
        let pattern = Pattern {
            text: r".* (\S+)$".into(),
            regex: true,
            numeric: serde_json::json!({"capture":1,"operator":">","value":"100"}),
            ..Pattern::default()
        };
        let matched = fixture.scan(pattern.clone());
        assert_eq!(matched.count, 1);
        assert_eq!(matched.lines[0].0, 1);
        let inverted = fixture.scan(Pattern {
            invert: true,
            ..pattern.clone()
        });
        assert_eq!(inverted.count, 3);
        let fractional = fixture.scan(Pattern {
            numeric: serde_json::json!({"capture":1,"operator":"<","value":"0"}),
            ..pattern
        });
        assert_eq!(fractional.lines[0].0, 3);
    }
    #[test]
    fn decoder_handles_split_unicode_and_long_lines() {
        let input = format!("あ\n{}\nlast", "x".repeat(DISPLAY_BYTES * 2));
        let mut lines = Vec::new();
        read_lines(input.as_bytes(), |line| {
            lines.push(line);
            Ok(())
        })
        .unwrap();
        assert_eq!(lines[0], "あ");
        assert!(lines[1].len() < DISPLAY_BYTES + 100);
        assert_eq!(lines[2], "last");
    }
}
