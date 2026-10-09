//! Shared PRI decoding for presentation and Roto; raw messages stay unchanged.
pub const SEVERITIES: [&str; 8] = [
    "Emergency",
    "Alert",
    "Critical",
    "Error",
    "Warning",
    "Notice",
    "Informational",
    "Debug",
];
pub const FACILITIES: [&str; 24] = [
    "kernel", "user", "mail", "daemon", "auth", "syslog", "lpr", "news", "uucp", "cron",
    "authpriv", "ftp", "ntp", "audit", "alert", "clock", "local0", "local1", "local2", "local3",
    "local4", "local5", "local6", "local7",
];
const SEVERITY_LABELS: [&str; 8] = [
    "emrg", "alrt", "crit", "erro", "warn", "noti", "info", "dbug",
];
const FACILITY_LABELS: [&str; 24] = [
    "kern", "user", "mail", "dmon", "auth", "sysl", "lpr ", "news", "uucp", "cron", "aprv", "ftp ",
    "ntp ", "audt", "alrt", "clck", "loc0", "loc1", "loc2", "loc3", "loc4", "loc5", "loc6", "loc7",
];

pub fn pri(line: &str) -> Option<(usize, usize)> {
    let rest = line.strip_prefix('<')?;
    let end = rest.find('>')?;
    if end == 0 || end > 3 || !rest[..end].bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let value = rest[..end].parse::<usize>().ok()?;
    (value <= 191).then_some((value, end + 2))
}

pub fn severity_name(value: u32) -> &'static str {
    SEVERITIES.get(value as usize).copied().unwrap_or("")
}
pub fn facility_name(value: u32) -> &'static str {
    FACILITIES.get(value as usize).copied().unwrap_or("")
}
pub fn badge(value: usize) -> String {
    format!(
        "[{}|{}] ",
        SEVERITY_LABELS[value % 8].to_ascii_uppercase(),
        FACILITY_LABELS[value / 8].to_ascii_uppercase()
    )
}
pub fn color(value: usize) -> &'static str {
    // Muted color families keep labels readable without overpowering the body.
    match value % 8 {
        0..=3 => "#B15956",
        4 => "#B1B146",
        5..=6 => "#3580A1",
        _ => "#888888",
    }
}

/// Context views prefix lines with their history line number.
pub fn decoration(line: &str) -> Option<(usize, usize, usize)> {
    let start = if pri(line).is_some() {
        0
    } else {
        let (number, _) = line.split_once(": ")?;
        if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        number.len() + 2
    };
    let (value, length) = pri(&line[start..])?;
    Some((start, length, value))
}

pub fn display_text(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        if let Some((start, length, value)) = decoration(line) {
            output.push_str(&line[..start]);
            output.push_str(&badge(value));
            output.push_str(&line[start + length..]);
        } else {
            output.push_str(line);
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pri_names_and_display_are_bounded() {
        assert_eq!(severity_name(0), "Emergency");
        assert_eq!(facility_name(0), "kernel");
        assert_eq!(facility_name(23), "local7");
        assert_eq!(severity_name(8), "");
        assert_eq!(facility_name(24), "");
        for value in 0..192 {
            assert_eq!(
                pri(&format!("<{value}>body")),
                Some((value, value.to_string().len() + 2))
            );
            assert_eq!(badge(value).len(), 12);
            assert!(badge(value).is_ascii());
        }
        for invalid in ["<192>x", "<-1>x", "<9999>x", "<>x", "<a>x", "<12", " x<12>"] {
            assert!(pri(invalid).is_none());
        }
        assert_eq!(
            display_text("<190>hello\n42: <0>world\n<192>bad\n"),
            "[INFO|LOC7] hello\n42: [EMRG|KERN] world\n<192>bad\n"
        );
    }
}
