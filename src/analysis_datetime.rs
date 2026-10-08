//! Explicit timezone policy and bounded, fallible datetime formatting for JIT calls.
use chrono::format::{Parsed, StrftimeItems, parse};
use chrono::{DateTime, FixedOffset, Local, NaiveDateTime, TimeDelta, TimeZone, Timelike, Utc};
use roto::RotoString;
use std::fmt::{self, Write};

const MAX_FORMAT_BYTES: usize = 1024;

#[derive(Clone, PartialEq)]
pub(super) struct NaiveTimestamp(pub NaiveDateTime);

#[derive(Clone, PartialEq)]
pub(super) struct UtcTimestamp(pub DateTime<Utc>);

#[derive(Clone, PartialEq)]
enum TimestampValue {
    Naive(NaiveTimestamp),
    Utc(UtcTimestamp),
    Fixed(DateTime<FixedOffset>),
    Local(DateTime<Local>),
}

/// Script-friendly datetime: failed parses are retained, never replaced by a
/// sentinel timestamp. Formatting failures yield an empty string.
#[derive(Clone, PartialEq)]
pub(super) struct ScriptDatetime(Option<TimestampValue>);

impl ScriptDatetime {
    pub fn parse(text: &str, format: &str) -> Self {
        Self(NaiveTimestamp::parse(text, format).map(TimestampValue::Naive))
    }

    pub fn parse_rfc3339(text: &str) -> Self {
        Self(UtcTimestamp::parse_rfc3339(text).map(TimestampValue::Utc))
    }

    pub fn is_valid(&self) -> bool {
        self.0.is_some()
    }

    pub fn fixed_offset(&self, hours: u32, east: bool) -> Self {
        let value = (|| {
            let seconds = i32::try_from(hours.checked_mul(3600)?).ok()?;
            let offset = if east {
                FixedOffset::east_opt(seconds)?
            } else {
                FixedOffset::west_opt(seconds)?
            };
            let dt = match self.0.as_ref()? {
                TimestampValue::Naive(dt) => offset.from_local_datetime(&dt.0).single()?,
                value => {
                    let utc = value.utc()?;
                    // Guard the local representation at Chrono's date bounds.
                    utc.naive_utc()
                        .checked_add_signed(TimeDelta::seconds(offset.local_minus_utc().into()))?;
                    utc.with_timezone(&offset)
                }
            };
            Some(TimestampValue::Fixed(dt))
        })();
        Self(value)
    }

    pub fn to_local(&self) -> Self {
        let value = (|| {
            let utc = self.0.as_ref()?.utc()?;
            let local = utc.with_timezone(&Local);
            utc.naive_utc()
                .checked_add_signed(TimeDelta::seconds(local.offset().local_minus_utc().into()))?;
            Some(TimestampValue::Local(local))
        })();
        Self(value)
    }

    pub fn floor_hours(&self, width: u32) -> Self {
        self.floor(width, 24, 0)
    }
    pub fn floor_minutes(&self, width: u32) -> Self {
        self.floor(width, 60, 1)
    }
    pub fn floor_seconds(&self, width: u32) -> Self {
        self.floor(width, 60, 2)
    }

    fn floor(&self, width: u32, base: u32, unit: u8) -> Self {
        let value = (|| {
            if width == 0 || width > base || base % width != 0 {
                return None;
            }
            let value = self.0.as_ref()?;
            let naive = match value {
                TimestampValue::Naive(dt) => dt.0,
                TimestampValue::Utc(dt) => dt.0.naive_utc(),
                TimestampValue::Fixed(dt) => dt.naive_local(),
                TimestampValue::Local(dt) => dt.naive_local(),
            };
            let (hour, minute, second) = match unit {
                0 => (naive.hour() / width * width, 0, 0),
                1 => (naive.hour(), naive.minute() / width * width, 0),
                _ => (naive.hour(), naive.minute(), naive.second() / width * width),
            };
            let bucket = naive.date().and_hms_opt(hour, minute, second)?;
            Some(match value {
                TimestampValue::Naive(_) => TimestampValue::Naive(NaiveTimestamp(bucket)),
                TimestampValue::Utc(_) => TimestampValue::Utc(UtcTimestamp(bucket.and_utc())),
                TimestampValue::Fixed(dt) => {
                    TimestampValue::Fixed(dt.offset().from_local_datetime(&bucket).single()?)
                }
                // Do not guess which occurrence of an ambiguous DST wall time
                // was intended, or invent a nonexistent bucket boundary.
                TimestampValue::Local(_) => {
                    let resolved = Local.from_local_datetime(&bucket).single()?;
                    // Some OS timezone implementations normalize a gap rather
                    // than rejecting it. Require an exact wall-clock round trip.
                    let checked = resolved.with_timezone(&Utc).with_timezone(&Local);
                    if checked.naive_local() != bucket || checked.offset() != resolved.offset() {
                        return None;
                    }
                    TimestampValue::Local(resolved)
                }
            })
        })();
        Self(value)
    }

    pub fn format(&self, format: &str) -> RotoString {
        match &self.0 {
            Some(TimestampValue::Naive(dt)) => dt.format(format),
            Some(TimestampValue::Utc(dt)) => dt.format(format),
            Some(TimestampValue::Fixed(dt)) => format_zoned(dt, format),
            Some(TimestampValue::Local(dt)) => format_zoned(dt, format),
            None => None,
        }
        .unwrap_or_else(|| "".into())
    }
}

impl TimestampValue {
    fn utc(&self) -> Option<DateTime<Utc>> {
        match self {
            Self::Naive(_) => None,
            Self::Utc(dt) => Some(dt.0),
            Self::Fixed(dt) => Some(dt.with_timezone(&Utc)),
            Self::Local(dt) => Some(dt.with_timezone(&Utc)),
        }
    }
}

fn format_zoned<T: TimeZone>(dt: &DateTime<T>, format: &str) -> Option<RotoString>
where
    T::Offset: fmt::Display,
{
    if format.len() > MAX_FORMAT_BYTES {
        return None;
    }
    let mut output = BoundedOutput(String::new());
    dt.format(format).write_to(&mut output).ok()?;
    Some(output.0.into())
}

struct BoundedOutput(String);

impl Write for BoundedOutput {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        if self.0.len() + text.len() > super::MAX_VALUE_BYTES {
            return Err(fmt::Error);
        }
        self.0.push_str(text);
        Ok(())
    }
}

impl NaiveTimestamp {
    pub fn parse(text: &str, format: &str) -> Option<Self> {
        if text.len() > super::MAX_VALUE_BYTES || format.len() > MAX_FORMAT_BYTES {
            return None;
        }
        let mut parsed = Parsed::new();
        parse(&mut parsed, text, StrftimeItems::new(format)).ok()?;
        // Never silently discard a timezone supplied by a naive format.
        if parsed.offset.is_some() {
            return None;
        }
        parsed.to_naive_datetime_with_offset(0).ok().map(Self)
    }

    pub fn format(&self, format: &str) -> Option<RotoString> {
        if format.len() > MAX_FORMAT_BYTES {
            return None;
        }
        let mut output = BoundedOutput(String::new());
        // write_to propagates unsupported directives rather than panicking as
        // Display::to_string can. Naive timezone directives also fail here.
        self.0.format(format).write_to(&mut output).ok()?;
        Some(output.0.into())
    }
}

impl UtcTimestamp {
    pub fn parse_rfc3339(text: &str) -> Option<Self> {
        if text.len() > super::MAX_VALUE_BYTES {
            return None;
        }
        DateTime::parse_from_rfc3339(text)
            .ok()
            .map(|dt| Self(dt.with_timezone(&Utc)))
    }

    pub fn format(&self, format: &str) -> Option<RotoString> {
        if format.len() > MAX_FORMAT_BYTES {
            return None;
        }
        let mut output = BoundedOutput(String::new());
        self.0.format(format).write_to(&mut output).ok()?;
        Some(output.0.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_offsets_attach_to_naive_but_convert_aware_instants() {
        let naive = ScriptDatetime::parse("20261008 10:37:07", "%Y%m%d %H:%M:%S");
        assert!(!naive.to_local().is_valid());
        let attached = naive.fixed_offset(9, true);
        assert_eq!(
            attached.format("%Y-%m-%d %H:%M:%S %:z").as_ref(),
            "2026-10-08 10:37:07 +09:00"
        );
        let utc = ScriptDatetime::parse_rfc3339("2026-10-08T01:37:07Z");
        assert_eq!(
            utc.fixed_offset(9, true).format("%+").as_ref(),
            attached.format("%+").as_ref()
        );
        assert_eq!(
            attached.fixed_offset(9, true).format("%+").as_ref(),
            attached.format("%+").as_ref()
        );
        assert_eq!(
            attached
                .fixed_offset(9, false)
                .format("%Y-%m-%d %H:%M %:z")
                .as_ref(),
            "2026-10-07 16:37 -09:00"
        );
        assert!(!utc.fixed_offset(24, true).is_valid());
        assert!(!utc.fixed_offset(u32::MAX, false).is_valid());
        assert!(utc.fixed_offset(0, false).is_valid());
        assert!(
            !ScriptDatetime::parse_rfc3339("bad")
                .fixed_offset(9, true)
                .to_local()
                .is_valid()
        );
    }

    #[test]
    fn floors_use_wall_clock_boundaries_and_clear_lower_units() {
        let dt =
            ScriptDatetime::parse_rfc3339("2026-10-08T01:37:47.185057405Z").fixed_offset(9, true);
        for (width, expected) in [
            (1, "10:37:00"),
            (5, "10:35:00"),
            (10, "10:30:00"),
            (15, "10:30:00"),
            (30, "10:30:00"),
            (60, "10:00:00"),
        ] {
            assert_eq!(
                dt.floor_minutes(width).format("%H:%M:%S").as_ref(),
                expected
            );
        }
        assert_eq!(
            dt.floor_hours(1).format("%H:%M:%S%.9f").as_ref(),
            "10:00:00.000000000"
        );
        assert_eq!(dt.floor_hours(3).format("%H:%M:%S").as_ref(), "09:00:00");
        assert_eq!(dt.floor_hours(24).format("%H:%M:%S").as_ref(), "00:00:00");
        assert_eq!(
            dt.floor_seconds(1).format("%H:%M:%S%.9f").as_ref(),
            "10:37:47.000000000"
        );
        assert_eq!(dt.floor_seconds(15).format("%H:%M:%S").as_ref(), "10:37:45");
        assert_eq!(dt.floor_minutes(5).format("%:z").as_ref(), "+09:00");
        for width in [0, 7, 61, u32::MAX] {
            assert!(!dt.floor_minutes(width).is_valid());
            assert!(!dt.floor_seconds(width).is_valid());
        }
        assert!(!dt.floor_hours(5).is_valid());
        let naive = ScriptDatetime::parse("19691231 23:59:59", "%Y%m%d %H:%M:%S");
        assert_eq!(
            naive.floor_minutes(15).format("%Y-%m-%d %H:%M:%S").as_ref(),
            "1969-12-31 23:45:00"
        );
        assert!(!naive.floor_minutes(0).fixed_offset(9, true).is_valid());
    }

    #[test]
    fn local_conversion_preserves_instant_and_uses_os_timezone() {
        let utc = DateTime::parse_from_rfc3339("2026-10-08T01:37:07Z")
            .unwrap()
            .with_timezone(&Utc);
        let dt = ScriptDatetime::parse_rfc3339("2026-10-08T01:37:07Z").to_local();
        assert!(dt.is_valid());
        assert_eq!(
            dt.format("%Y-%m-%d %H:%M:%S %:z").as_ref(),
            utc.with_timezone(&Local)
                .format("%Y-%m-%d %H:%M:%S %:z")
                .to_string()
        );
        assert_eq!(
            dt.fixed_offset(0, true).format("%+").as_ref(),
            utc.format("%+").to_string()
        );
        assert_eq!(
            dt.floor_minutes(5).format("%M:%S").as_ref(),
            format!("{:02}:00", utc.with_timezone(&Local).minute() / 5 * 5)
        );
        // Additional deterministic coverage when run in a separate TZ process.
        if std::env::var("TZ").as_deref() == Ok("America/New_York") {
            let fold = ScriptDatetime::parse_rfc3339("2026-11-01T05:37:07Z").to_local();
            assert!(fold.is_valid());
            assert!(!fold.floor_minutes(5).is_valid());
            let after_gap = ScriptDatetime::parse_rfc3339("2026-03-08T07:37:07Z").to_local();
            assert!(!after_gap.floor_hours(2).is_valid());
        }
    }

    #[test]
    fn script_datetime_retains_parse_validity_and_formats_without_option() {
        let dt = ScriptDatetime::parse("20261008 01:37:07", "%Y%m%d %H:%M:%S");
        assert!(dt.is_valid());
        assert_eq!(dt.format("%H:%M").as_ref(), "01:37");
        assert!(dt.format("%Q").is_empty());
        assert!(dt.is_valid()); // Invalid output format does not invalidate the input.
        assert!(dt.format(&"%Y".repeat(300)).is_empty());
        let invalid = ScriptDatetime::parse("20260230 01:37:07", "%Y%m%d %H:%M:%S");
        assert!(!invalid.is_valid());
        assert!(invalid.format("%Y-%m-%d %H:%M").is_empty());
        let dt = ScriptDatetime::parse_rfc3339("2026-10-08T01:37:07+09:00");
        assert!(dt.is_valid());
        assert_eq!(dt.format("%Y-%m-%d %H:%M").as_ref(), "2026-10-07 16:37");
        let invalid = ScriptDatetime::parse_rfc3339("invalid");
        assert!(!invalid.is_valid());
        assert!(invalid.format("%H:%M").is_empty());
    }

    #[test]
    fn naive_formats_are_fallible_and_bounded() {
        let dt = NaiveTimestamp::parse("20261008 01:37:07", "%Y%m%d %H:%M:%S").unwrap();
        assert_eq!(dt.format("%H:%M").unwrap().as_ref(), "01:37");
        assert!(dt.format("%Q").is_none());
        assert!(dt.format("%z").is_none());
        assert!(dt.format("%+").is_none());
        assert!(dt.format(&"%Y".repeat(300)).is_none());
        assert!(dt.format(&"x".repeat(MAX_FORMAT_BYTES + 1)).is_none());
        assert!(NaiveTimestamp::parse("20260230 01:37:07", "%Y%m%d %H:%M:%S").is_none());
        assert!(NaiveTimestamp::parse("20261008 01:37:07 trailing", "%Y%m%d %H:%M:%S").is_none());
        assert!(NaiveTimestamp::parse("20261008 01:37:07", "%Q").is_none());
        assert!(
            NaiveTimestamp::parse("2026-10-08T01:37:07+09:00", "%Y-%m-%dT%H:%M:%S%:z").is_none()
        );
    }

    #[test]
    fn rfc3339_normalizes_offsets_and_preserves_nanoseconds() {
        let dt = UtcTimestamp::parse_rfc3339("2026-10-08T01:37:07.185057405Z").unwrap();
        assert_eq!(
            dt.format("%Y-%m-%dT%H:%M:%S%.9f%:z").unwrap().as_ref(),
            "2026-10-08T01:37:07.185057405+00:00"
        );
        let dt = UtcTimestamp::parse_rfc3339("2026-10-08T01:37:07+09:00").unwrap();
        assert_eq!(
            dt.format("%Y-%m-%d %H:%M").unwrap().as_ref(),
            "2026-10-07 16:37"
        );
        assert!(dt.format("%Q").is_none());
        assert!(UtcTimestamp::parse_rfc3339("2026-10-08T01:37:07").is_none());
        assert!(UtcTimestamp::parse_rfc3339("invalid").is_none());
    }
}
