use regex::{Regex, RegexBuilder};
use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

pub const DISPLAY_BYTES: usize = 256 * 1024;
pub const DISPLAY_LINES: usize = 2000;

#[derive(Clone, Default)]
pub struct Pattern {
    pub text: String,
    pub regex: bool,
    pub ignore_case: bool,
    pub invert: bool,
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

pub fn scan(
    path: &Path,
    pattern: &Pattern,
    stop: &AtomicBool,
    generation: &AtomicU64,
    expected: u64,
) -> Result<Matches, String> {
    let matcher = compile(pattern)?;
    let file = File::open(path).map_err(|error| error.to_string())?;
    let mut matches = Matches::default();
    let mut bytes = 0;
    let mut number = 0;
    read_lines(file, |line| {
        if stop.load(Ordering::Relaxed) || generation.load(Ordering::Relaxed) != expected {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Search cancelled",
            ));
        }
        number += 1;
        if matcher.is_match(&line) != pattern.invert {
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

pub fn context(path: &Path, target: usize) -> io::Result<String> {
    let mut index = 0;
    let mut text = String::new();
    read_lines(File::open(path)?, |line| {
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
    let mut start = text.len().saturating_sub(DISPLAY_BYTES);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    let excess = text.lines().count().saturating_sub(DISPLAY_LINES);
    if excess > 0
        && let Some((pos, _)) = text.match_indices('\n').nth(excess - 1)
    {
        start = start.max(pos + 1);
    }
    text.drain(..start);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;

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
