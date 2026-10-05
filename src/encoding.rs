use encoding_rs::{DecoderResult, Encoding};
use std::io;

pub const ENCODINGS: &[&str] = &[
    "UTF-8",
    "CP932",
    "EUC-JP",
    "ISO-2022-JP",
    "UTF-16LE",
    "UTF-16BE",
    "GB18030",
    "BIG5",
    "CP949",
    "WINDOWS-1252",
    "ISO-8859-1",
];

/// One Rust decoder per stream; incomplete bytes and shift state survive reads.
pub struct Decoder {
    inner: Option<encoding_rs::Decoder>,
    finished: bool,
    pub errors: u64,
}

impl Decoder {
    pub fn new(encoding: &str) -> io::Result<Self> {
        if !ENCODINGS.contains(&encoding) {
            return Err(io::Error::other(format!(
                "Unsupported encoding: {encoding}"
            )));
        }
        // WHATWG labels alias ISO-8859-1 to Windows-1252. Keep our explicit
        // Latin-1 option exact, including C1 controls, using a byte-to-char map.
        let inner = if encoding == "ISO-8859-1" {
            None
        } else {
            let label = match encoding {
                "CP932" => "shift_jis",
                "CP949" => "euc-kr",
                label => label,
            };
            Some(
                Encoding::for_label(label.as_bytes())
                    .ok_or_else(|| io::Error::other(format!("Unsupported encoding: {encoding}")))?
                    .new_decoder_without_bom_handling(),
            )
        };
        Ok(Self {
            inner,
            finished: false,
            errors: 0,
        })
    }

    pub fn decode(&mut self, input: &[u8], final_chunk: bool) -> io::Result<String> {
        if self.finished {
            return Err(io::Error::other("Encoding conversion has finished"));
        }
        let Some(decoder) = &mut self.inner else {
            self.finished = final_chunk;
            return Ok(input.iter().map(|&byte| char::from(byte)).collect());
        };
        let mut output = String::with_capacity(8192);
        let mut consumed = 0;
        loop {
            let (result, read) = decoder.decode_to_string_without_replacement(
                &input[consumed..],
                &mut output,
                final_chunk,
            );
            consumed += read;
            match result {
                DecoderResult::InputEmpty => break,
                DecoderResult::OutputFull => output.reserve(8192),
                DecoderResult::Malformed(_, _) => {
                    output.push('�');
                    self.errors += 1;
                }
            }
        }
        self.finished = final_chunk;
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_supported_encoding_opens() {
        for encoding in ENCODINGS {
            Decoder::new(encoding).unwrap();
        }
        assert!(Decoder::new("unknown").is_err());
    }
    #[test]
    fn split_cp932_and_utf8() {
        for (encoding, bytes) in [
            ("CP932", &b"\x93\xfa\x96\x7b\x8c\xea\n"[..]),
            ("UTF-8", "日本語\n".as_bytes()),
        ] {
            let mut decoder = Decoder::new(encoding).unwrap();
            let mut text = String::new();
            for byte in bytes {
                text.push_str(&decoder.decode(&[*byte], false).unwrap());
            }
            text.push_str(&decoder.decode(&[], true).unwrap());
            assert_eq!(text, "日本語\n");
            assert_eq!(decoder.errors, 0);
        }
    }
    #[test]
    fn iso2022_state_and_utf16_split_units() {
        let mut decoder = Decoder::new("ISO-2022-JP").unwrap();
        let mut text = String::new();
        for byte in b"\x1b$BF|K\x5c8l\x1b(B\n" {
            text.push_str(&decoder.decode(&[*byte], false).unwrap());
        }
        assert_eq!(text, "日本語\n");
        for (encoding, little) in [("UTF-16LE", true), ("UTF-16BE", false)] {
            let mut decoder = Decoder::new(encoding).unwrap();
            let mut text = String::new();
            for unit in "日本語😀\n".encode_utf16() {
                for byte in if little {
                    unit.to_le_bytes()
                } else {
                    unit.to_be_bytes()
                } {
                    text.push_str(&decoder.decode(&[byte], false).unwrap());
                }
            }
            assert_eq!(text, "日本語😀\n");
        }
    }
    #[test]
    fn malformed_bytes_and_incomplete_eof_are_replaced() {
        let mut decoder = Decoder::new("UTF-8").unwrap();
        assert_eq!(decoder.decode(b"a\xffb\xe3", false).unwrap(), "a�b");
        assert_eq!(decoder.decode(&[], true).unwrap(), "�");
        assert_eq!(decoder.errors, 2);
        assert_eq!(
            Decoder::new("UTF-8")
                .unwrap()
                .decode("あ".repeat(10000).as_bytes(), true)
                .unwrap(),
            "あ".repeat(10000)
        );
    }

    #[test]
    fn latin1_and_windows1252_are_distinct() {
        assert_eq!(
            Decoder::new("ISO-8859-1")
                .unwrap()
                .decode(&[0x80, 0xe9], true)
                .unwrap(),
            "\u{80}é"
        );
        assert_eq!(
            Decoder::new("WINDOWS-1252")
                .unwrap()
                .decode(&[0x80, 0xe9], true)
                .unwrap(),
            "€é"
        );
    }

    #[test]
    fn east_asian_mappings_decode_across_single_byte_reads() {
        for (encoding, bytes, expected) in [
            ("CP932", &b"\xfa\x40"[..], "ⅰ"),
            ("CP949", &b"\xb0\xa1"[..], "가"),
            ("GB18030", &b"\xd6\xd0"[..], "中"),
            ("BIG5", &b"\xa4\xa4"[..], "中"),
            ("EUC-JP", &b"\xc6\xfc"[..], "日"),
        ] {
            let mut decoder = Decoder::new(encoding).unwrap();
            let mut output = String::new();
            for byte in bytes {
                output.push_str(&decoder.decode(&[*byte], false).unwrap());
            }
            output.push_str(&decoder.decode(&[], true).unwrap());
            assert_eq!(output, expected, "{encoding}");
            assert_eq!(decoder.errors, 0);
        }
    }

    #[test]
    fn replacement_count_excludes_literal_replacement_character() {
        let mut decoder = Decoder::new("UTF-8").unwrap();
        assert_eq!(
            decoder.decode(b"\xef\xbf\xbd\xff\xff", true).unwrap(),
            "���"
        );
        assert_eq!(decoder.errors, 2);
        assert!(decoder.decode(b"more", false).is_err());
    }
}
