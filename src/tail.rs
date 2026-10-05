use std::fs::{File, Metadata};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::PathBuf;

#[cfg(test)]
const MAX_TEXT: usize = 256 * 1024;
#[cfg(test)]
const MAX_LINES: usize = 2000;
const READ_CHUNK: u64 = 64 * 1024;

/// Raw-byte, name-following reader. Decoding belongs to the session pipeline.
pub struct FileFollower {
    path: PathBuf,
    file: File,
    identity: Metadata,
    offset: u64,
}

impl FileFollower {
    pub fn open(path: PathBuf, initial: u32, encoding: &str) -> io::Result<Self> {
        let mut file = File::open(&path)?;
        let identity = file.metadata()?;
        if !identity.is_file() {
            return Err(io::Error::other("Select a regular file"));
        }
        let end = identity.len();
        let offset = initial_offset(&mut file, end, initial, encoding)?;
        file.seek(SeekFrom::Start(offset))?;
        Ok(Self {
            path,
            file,
            identity,
            offset,
        })
    }

    /// A reset marks a new stream (replacement or truncation). A missing path
    /// is transient: drain the old handle and retry the name on the next poll.
    pub fn poll(&mut self, buffer: &mut [u8]) -> io::Result<(usize, bool)> {
        let mut reset = false;
        match std::fs::metadata(&self.path) {
            Ok(metadata) => {
                if !same_file(&self.identity, &metadata) {
                    match File::open(&self.path) {
                        Ok(file) => {
                            let identity = file.metadata()?;
                            if !identity.is_file() {
                                return Err(io::Error::other("Select a regular file"));
                            }
                            self.file = file;
                            self.identity = identity;
                            self.offset = 0;
                            reset = true;
                        }
                        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                        Err(error) => return Err(error),
                    }
                } else if metadata.len() < self.offset {
                    self.file.seek(SeekFrom::Start(0))?;
                    self.offset = 0;
                    reset = true;
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let count = self.file.read(buffer)?;
        self.offset += count as u64;
        Ok((count, reset))
    }
}

// Scan backwards with constant memory, ignoring a final line delimiter.
fn initial_offset(file: &mut File, end: u64, initial: u32, encoding: &str) -> io::Result<u64> {
    if initial == 0 {
        return Ok(end);
    }
    let (delimiter, width): (&[u8], usize) = match encoding {
        "UTF-16LE" => (&[10, 0], 2),
        "UTF-16BE" => (&[0, 10], 2),
        _ => (&[10], 1),
    };
    let mut cursor = end - end % width as u64;
    let mut remaining = initial;
    let mut buffer = [0; READ_CHUNK as usize];
    while cursor > 0 {
        let start = cursor.saturating_sub(buffer.len() as u64);
        let size = (cursor - start) as usize;
        file.seek(SeekFrom::Start(start))?;
        file.read_exact(&mut buffer[..size])?;
        for index in (0..size).step_by(width).rev() {
            let after = start + index as u64 + width as u64;
            if &buffer[index..index + width] == delimiter && after != end {
                remaining -= 1;
                if remaining == 0 {
                    return Ok(after);
                }
            }
        }
        cursor = start;
    }
    Ok(0)
}

#[cfg(test)]
/// Bounded, incremental UTF-8 reader. Independent of the Qt event loop.
pub struct TailReader {
    path: PathBuf,
    file: File,
    identity: Metadata,
    offset: u64,
    pending: Vec<u8>,
    pub text: String,
}

#[cfg(test)]
impl TailReader {
    pub fn open(path: PathBuf) -> io::Result<Self> {
        let mut file = File::open(&path)?;
        let identity = file.metadata()?;
        if !identity.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Select a regular file",
            ));
        }
        let offset = identity.len().saturating_sub(MAX_TEXT as u64);
        file.seek(SeekFrom::Start(offset))?;
        let mut reader = Self {
            path,
            file,
            identity,
            offset,
            pending: Vec::new(),
            text: String::new(),
        };
        // Initial preview is bounded even when opening a very large log.
        reader.read_bytes(MAX_TEXT as u64)?;
        Ok(reader)
    }

    pub fn poll(&mut self) -> io::Result<bool> {
        let metadata = std::fs::metadata(&self.path)?;
        let reset = !same_file(&self.identity, &metadata) || metadata.len() < self.offset;
        if reset {
            let file = File::open(&self.path)?;
            let identity = file.metadata()?;
            self.file = file;
            self.identity = identity;
            self.offset = 0;
            self.pending.clear();
            self.text.clear();
        }
        Ok(self.read_bytes(READ_CHUNK)? || reset)
    }

    fn read_bytes(&mut self, limit: u64) -> io::Result<bool> {
        let mut bytes = Vec::new();
        (&mut self.file).take(limit).read_to_end(&mut bytes)?;
        if bytes.is_empty() {
            return Ok(false);
        }
        self.offset += bytes.len() as u64;
        self.pending.extend(bytes);
        let mut consumed = 0;
        while consumed < self.pending.len() {
            match std::str::from_utf8(&self.pending[consumed..]) {
                Ok(text) => {
                    self.text.push_str(text);
                    consumed = self.pending.len();
                }
                Err(error) => {
                    let valid_end = consumed + error.valid_up_to();
                    self.text
                        .push_str(std::str::from_utf8(&self.pending[consumed..valid_end]).unwrap());
                    consumed = valid_end;
                    match error.error_len() {
                        Some(length) => {
                            self.text.push('\u{fffd}');
                            consumed += length;
                        }
                        None => break, // Keep a split UTF-8 character for the next poll.
                    }
                }
            }
        }
        self.pending.drain(..consumed);
        self.trim();
        Ok(true)
    }

    fn trim(&mut self) {
        let mut start = self.text.len().saturating_sub(MAX_TEXT);
        while !self.text.is_char_boundary(start) {
            start += 1;
        }
        let excess = self.text.lines().count().saturating_sub(MAX_LINES);
        if excess > 0
            && let Some((position, _)) = self.text.match_indices('\n').nth(excess - 1)
        {
            start = start.max(position + 1);
        }
        self.text.drain(..start);
    }
}

#[cfg(unix)]
fn same_file(a: &Metadata, b: &Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    a.dev() == b.dev() && a.ino() == b.ino()
}

#[cfg(not(unix))]
fn same_file(a: &Metadata, b: &Metadata) -> bool {
    a.created().ok() == b.created().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Fixture(PathBuf);
    impl Fixture {
        fn new(bytes: &[u8]) -> Self {
            static ID: AtomicU64 = AtomicU64::new(0);
            let dir = std::env::temp_dir().join(format!(
                "tailer-{}-{}",
                std::process::id(),
                ID.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("test.log");
            std::fs::write(&path, bytes).unwrap();
            Self(path)
        }
        fn append(&self, bytes: &[u8]) {
            std::fs::OpenOptions::new()
                .append(true)
                .open(&self.0)
                .unwrap()
                .write_all(bytes)
                .unwrap();
        }
        fn reader(&self) -> TailReader {
            TailReader::open(self.0.clone()).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(self.0.parent().unwrap());
        }
    }

    #[test]
    fn follower_initial_lines_and_raw_bytes() {
        for contents in [&b"one\ntwo\nthree\n"[..], &b"one\ntwo\nthree"[..]] {
            let fixture = Fixture::new(contents);
            for initial in [0, 1, 2, 10] {
                let mut follower = FileFollower::open(fixture.0.clone(), initial, "UTF-8").unwrap();
                let mut buffer = [0; 128];
                let (count, reset) = follower.poll(&mut buffer).unwrap();
                let expected = match initial {
                    0 => &b""[..],
                    1 => &contents[8..],
                    2 => &contents[4..],
                    _ => contents,
                };
                assert_eq!(&buffer[..count], expected);
                assert!(!reset);
            }
        }
        let fixture = Fixture::new(&[0x93, 0xfa]);
        let mut follower = FileFollower::open(fixture.0.clone(), 1, "CP932").unwrap();
        let mut buffer = [0; 1];
        assert_eq!(follower.poll(&mut buffer).unwrap(), (1, false));
        assert_eq!(buffer, [0x93]);
        assert_eq!(follower.poll(&mut buffer).unwrap(), (1, false));
        assert_eq!(buffer, [0xfa]);
    }

    #[test]
    fn follower_utf16_initial_lines_and_large_backward_scan() {
        for (encoding, little) in [("UTF-16LE", true), ("UTF-16BE", false)] {
            let contents: Vec<u8> = "前\n日本語\n"
                .encode_utf16()
                .flat_map(|unit| {
                    if little {
                        unit.to_le_bytes()
                    } else {
                        unit.to_be_bytes()
                    }
                })
                .collect();
            let fixture = Fixture::new(&contents);
            let mut follower = FileFollower::open(fixture.0.clone(), 1, encoding).unwrap();
            let mut buffer = [0; 128];
            let (count, _) = follower.poll(&mut buffer).unwrap();
            assert_eq!(&buffer[..count], &contents[4..]);
        }
        let contents = format!("old\n{}\nlast\n", "x".repeat(READ_CHUNK as usize * 2));
        let fixture = Fixture::new(contents.as_bytes());
        let mut follower = FileFollower::open(fixture.0.clone(), 2, "UTF-8").unwrap();
        assert_eq!(follower.offset, 4);
        let mut buffer = [0; 16];
        assert_eq!(follower.poll(&mut buffer).unwrap(), (16, false));
    }

    #[test]
    fn follower_append_truncation_and_missing_rotation() {
        let fixture = Fixture::new(b"original\n");
        let mut follower = FileFollower::open(fixture.0.clone(), 0, "UTF-8").unwrap();
        let mut buffer = [0; 128];
        assert_eq!(follower.poll(&mut buffer).unwrap(), (0, false));
        fixture.append(b"append\n");
        let (count, reset) = follower.poll(&mut buffer).unwrap();
        assert_eq!(&buffer[..count], b"append\n");
        assert!(!reset);
        std::fs::write(&fixture.0, b"new\n").unwrap();
        let (count, reset) = follower.poll(&mut buffer).unwrap();
        assert_eq!(&buffer[..count], b"new\n");
        assert!(reset);
        std::fs::rename(&fixture.0, fixture.0.with_extension("old")).unwrap();
        assert_eq!(follower.poll(&mut buffer).unwrap(), (0, false));
        std::fs::write(&fixture.0, b"replacement\n").unwrap();
        let (count, reset) = follower.poll(&mut buffer).unwrap();
        assert_eq!(&buffer[..count], b"replacement\n");
        assert!(reset);
    }

    #[test]
    fn reads_initial_and_appended_text() {
        let file = Fixture::new(b"first\n");
        let mut reader = file.reader();
        assert_eq!(reader.text, "first\n");
        assert!(!reader.poll().unwrap());
        file.append(b"second\n");
        assert!(reader.poll().unwrap());
        assert_eq!(reader.text, "first\nsecond\n");
    }

    #[test]
    fn preserves_split_utf8() {
        let file = Fixture::new(&[0xe3]);
        let mut reader = file.reader();
        assert_eq!(reader.text, "");
        file.append(&[0x81, 0x82]);
        reader.poll().unwrap();
        assert_eq!(reader.text, "あ");
    }

    #[test]
    fn replaces_invalid_utf8() {
        let file = Fixture::new(b"ok\xff\n");
        assert_eq!(file.reader().text, "ok\u{fffd}\n");
    }

    #[test]
    fn follows_truncation() {
        let file = Fixture::new(b"original text\n");
        let mut reader = file.reader();
        std::fs::write(&file.0, b"new\n").unwrap();
        assert!(reader.poll().unwrap());
        assert_eq!(reader.text, "new\n");
    }

    #[cfg(unix)]
    #[test]
    fn follows_rotation_and_recovers_missing_path() {
        let file = Fixture::new(b"old\n");
        let mut reader = file.reader();
        std::fs::rename(&file.0, file.0.with_extension("old")).unwrap();
        assert!(reader.poll().is_err());
        std::fs::write(&file.0, b"replacement\n").unwrap();
        reader.poll().unwrap();
        assert_eq!(reader.text, "replacement\n");
    }

    #[test]
    fn bounds_preview_and_retained_lines() {
        let file = Fixture::new("line\n".repeat(100_000).as_bytes());
        let reader = file.reader();
        assert!(reader.text.len() <= MAX_TEXT);
        assert!(reader.text.lines().count() <= MAX_LINES);
    }

    #[test]
    fn bounds_lines_without_trailing_newline() {
        let file = Fixture::new(format!("{}last", "line\n".repeat(MAX_LINES)).as_bytes());
        assert_eq!(file.reader().text.lines().count(), MAX_LINES);
    }

    #[test]
    fn bounds_each_poll() {
        let file = Fixture::new(b"");
        let mut reader = file.reader();
        file.append(&vec![b'x'; READ_CHUNK as usize * 2]);
        reader.poll().unwrap();
        assert_eq!(reader.text.len(), READ_CHUNK as usize);
        reader.poll().unwrap();
        assert_eq!(reader.text.len(), READ_CHUNK as usize * 2);
    }
}
