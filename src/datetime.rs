use std::fmt;
use std::ops::{Add, Sub};
use std::str::FromStr;

use jiff::{SignedDuration, civil::Date, tz::TimeZone};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// UTC instant shared by the application, its JSON APIs and PostgreSQL.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, sqlx::Type)]
#[sqlx(transparent)]
pub struct Timestamp(jiff_sqlx::Timestamp);

#[derive(Clone, Copy, Debug, thiserror::Error)]
#[error("invalid RFC 3339 timestamp")]
pub struct ParseTimestampError;

impl Timestamp {
    pub fn unix_epoch() -> Self {
        Self::default()
    }

    pub fn now() -> Self {
        Self(jiff::Timestamp::now().into())
    }

    pub fn from_second(seconds: i64) -> Result<Self, jiff::Error> {
        jiff::Timestamp::from_second(seconds).map(Self::from_jiff)
    }

    pub fn from_millisecond(milliseconds: i64) -> Result<Self, jiff::Error> {
        jiff::Timestamp::from_millisecond(milliseconds).map(Self::from_jiff)
    }

    fn from_jiff(timestamp: jiff::Timestamp) -> Self {
        Self(timestamp.into())
    }

    pub fn to_jiff(self) -> jiff::Timestamp {
        self.0.to_jiff()
    }

    pub fn as_second(self) -> i64 {
        i64::try_from(self.to_jiff().as_nanosecond().div_euclid(1_000_000_000))
            .expect("timestamp seconds fit in i64")
    }

    pub fn as_millisecond(self) -> i64 {
        i64::try_from(self.to_jiff().as_nanosecond().div_euclid(1_000_000))
            .expect("timestamp milliseconds fit in i64")
    }

    pub fn date(self) -> Date {
        TimeZone::UTC.to_datetime(self.to_jiff()).date()
    }

    pub fn checked_add(self, duration: SignedDuration) -> Result<Self, jiff::Error> {
        self.to_jiff().checked_add(duration).map(Self::from_jiff)
    }

    pub(crate) fn saturating_add(self, duration: SignedDuration) -> Self {
        self.checked_add(duration).unwrap_or_else(|_| {
            Self::from_jiff(if duration.is_negative() {
                jiff::Timestamp::MIN
            } else {
                jiff::Timestamp::MAX
            })
        })
    }

    pub fn duration_since(self, other: Self) -> SignedDuration {
        self.to_jiff().duration_since(other.to_jiff())
    }

    pub fn parse_rfc3339(value: &str) -> Result<Self, ParseTimestampError> {
        // Jiff's ISO parser also accepts incomplete times and zone annotations.
        // Keep the RFC 3339 shape required by schedules and persisted cursors.
        let [
            y1,
            y2,
            y3,
            y4,
            b'-',
            m1,
            m2,
            b'-',
            d1,
            d2,
            b'T' | b't' | b' ',
            h1,
            h2,
            b':',
            min1,
            min2,
            b':',
            s1,
            s2,
            zone @ ..,
        ] = value.as_bytes()
        else {
            return Err(ParseTimestampError);
        };
        if ![y1, y2, y3, y4, m1, m2, d1, d2, h1, h2, min1, min2, s1, s2]
            .into_iter()
            .all(u8::is_ascii_digit)
        {
            return Err(ParseTimestampError);
        }
        let (offset, fraction_digits) = match zone {
            [b'.', rest @ ..] => {
                let digits = rest
                    .iter()
                    .take_while(|&byte| byte.is_ascii_digit())
                    .count();
                if digits == 0 {
                    return Err(ParseTimestampError);
                }
                (&rest[digits..], digits)
            }
            offset => (offset, 0),
        };
        let offset = std::str::from_utf8(offset).map_err(|_| ParseTimestampError)?;
        let offset_start = 19
            + if fraction_digits == 0 {
                0
            } else {
                1 + fraction_digits
            };
        let normalized_offset = match offset.as_bytes() {
            [b'Z' | b'z'] => None,
            _ => Some(parse_offset(offset, false)?),
        };
        if fraction_digits > 9 || offset.starts_with('−') {
            let civil = &value[..offset_start.min(29)];
            let offset = normalized_offset.as_deref().unwrap_or(offset);
            return Self::parse_iso(&format!("{civil}{offset}"));
        }
        Self::parse_iso(value)
    }

    fn parse_iso(value: &str) -> Result<Self, ParseTimestampError> {
        value
            .parse::<jiff::Timestamp>()
            .map(Self::from_jiff)
            .map_err(|_| ParseTimestampError)
    }

    pub fn to_rfc3339(self) -> String {
        self.format_utc("+00:00")
    }

    fn format_utc(self, suffix: &str) -> String {
        let utc = TimeZone::UTC.to_datetime(self.to_jiff());
        let precision = fraction_precision(utc.subsec_nanosecond());
        format!("{utc:.precision$}{suffix}")
    }
}

impl FromStr for Timestamp {
    type Err = ParseTimestampError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let value = input.trim();
        if let Ok(timestamp) = Self::parse_rfc3339(value) {
            return Ok(timestamp);
        }

        // Older API payloads also use an unpadded civil time and compact offsets.
        let (civil, offset) = if value.ends_with(['Z', 'z']) {
            (&value[..value.len() - 1], "Z".to_owned())
        } else if value
            .get(value.len().saturating_sub(3)..)
            .is_some_and(|suffix| suffix.eq_ignore_ascii_case("UTC"))
        {
            (&value[..value.len() - 3], "Z".to_owned())
        } else {
            let time_start = value.find(['T', 't', ' ']).ok_or(ParseTimestampError)?;
            let offset_start = time_start
                + value[time_start..]
                    .rfind(['+', '-', '−'])
                    .ok_or(ParseTimestampError)?;
            let (civil, offset) = value.split_at(offset_start);
            (civil, parse_offset(offset, true)?)
        };
        let civil = civil.trim_end().replace(['t', 'T'], " ");
        let civil = match civil.split_once('.') {
            Some((whole, fraction))
                if fraction.len() > 9 && fraction.bytes().all(|byte| byte.is_ascii_digit()) =>
            {
                format!("{whole}.{}", &fraction[..9])
            }
            _ => civil,
        };
        let civil = jiff::civil::DateTime::strptime("%Y-%m-%d %H:%M:%S%.f", &civil)
            .map_err(|_| ParseTimestampError)?;
        Self::parse_iso(&format!("{civil}{offset}"))
    }
}

fn parse_offset(offset: &str, relaxed: bool) -> Result<String, ParseTimestampError> {
    let (sign, rest) = if let Some(rest) = offset.strip_prefix('+') {
        ('+', rest)
    } else if let Some(rest) = offset.strip_prefix(['-', '−']) {
        ('-', rest)
    } else {
        return Err(ParseTimestampError);
    };
    let hours = rest.get(..2).ok_or(ParseTimestampError)?;
    let minutes = if relaxed {
        rest[2..].trim_start_matches(|ch: char| ch == ':' || ch.is_whitespace())
    } else {
        rest[2..].strip_prefix(':').ok_or(ParseTimestampError)?
    };
    if !hours.bytes().all(|byte| byte.is_ascii_digit())
        || minutes.len() != 2
        || !minutes.bytes().all(|byte| byte.is_ascii_digit())
        || hours > "23"
        || minutes > "59"
    {
        return Err(ParseTimestampError);
    }
    Ok(format!("{sign}{hours}:{minutes}"))
}

impl Default for Timestamp {
    fn default() -> Self {
        Self::from_jiff(jiff::Timestamp::UNIX_EPOCH)
    }
}

fn fraction_precision(nanoseconds: i32) -> usize {
    if nanoseconds == 0 {
        0
    } else if nanoseconds % 1_000_000 == 0 {
        3
    } else if nanoseconds % 1_000 == 0 {
        6
    } else {
        9
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let utc = TimeZone::UTC.to_datetime(self.to_jiff());
        // Human-facing messages previously used the datetime's Display rather
        // than its RFC 3339 form.
        let precision = fraction_precision(utc.subsec_nanosecond());
        write!(formatter, "{} {:.precision$} UTC", utc.date(), utc.time())
    }
}

impl Serialize for Timestamp {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.format_utc("Z"))
    }
}

impl<'de> Deserialize<'de> for Timestamp {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

impl Add<SignedDuration> for Timestamp {
    type Output = Self;

    fn add(self, duration: SignedDuration) -> Self {
        Self::from_jiff(self.to_jiff() + duration)
    }
}

impl Sub<SignedDuration> for Timestamp {
    type Output = Self;

    fn sub(self, duration: SignedDuration) -> Self {
        Self::from_jiff(self.to_jiff() - duration)
    }
}

impl Sub for Timestamp {
    type Output = SignedDuration;

    fn sub(self, other: Self) -> SignedDuration {
        self.duration_since(other)
    }
}

/// A day of elapsed time (24 hours), for UTC timestamps and retention windows.
pub fn days(count: i64) -> SignedDuration {
    SignedDuration::from_hours(count.checked_mul(24).expect("duration hours overflow"))
}

#[cfg(test)]
mod tests {
    use super::Timestamp;

    #[test]
    fn timestamps_preserve_utc_json_and_cursor_format_at_each_precision() {
        for (input, utc) in [
            ("2026-01-02T04:04:05+01:00", "2026-01-02T03:04:05"),
            ("2026-01-02T04:04:05.12+01:00", "2026-01-02T03:04:05.120"),
            (
                "2026-01-02T04:04:05.12345+01:00",
                "2026-01-02T03:04:05.123450",
            ),
            (
                "2026-01-02T04:04:05.123456780123+01:00",
                "2026-01-02T03:04:05.123456780",
            ),
        ] {
            let timestamp = Timestamp::parse_rfc3339(input).unwrap();
            assert_eq!(timestamp.to_rfc3339(), format!("{utc}+00:00"));
            let json = format!("{utc}Z");
            assert_eq!(serde_json::to_value(timestamp).unwrap(), json);
            assert_eq!(
                serde_json::from_value::<Timestamp>(json.into()).unwrap(),
                timestamp
            );
        }
    }

    #[test]
    fn unix_values_floor_fractional_pre_epoch_instants() {
        let timestamp: Timestamp = "1969-12-31T23:59:59.9999Z".parse().unwrap();
        assert_eq!(timestamp.as_second(), -1);
        assert_eq!(timestamp.as_millisecond(), -1);
    }

    #[test]
    fn rfc3339_requires_complete_seconds_and_a_minute_precision_offset() {
        for input in [
            "2026-01-02T04:04+01:00",
            "2026-01-02T04:04:05+01",
            "2026-01-02T04:04:05+01:00:01",
            "2026-01-02T04:04:05,12Z",
            "2026-01-02T04:04:05Z[UTC]",
            "2026-01-02T04:04:05+0100",
        ] {
            assert!(Timestamp::parse_rfc3339(input).is_err(), "{input}");
        }
        for legacy in ["2026-1-2 3:4:5 UTC", "2026-01-02T04:04:05+0100"] {
            assert_eq!(
                legacy.parse::<Timestamp>().unwrap(),
                "2026-01-02T03:04:05Z".parse().unwrap(),
            );
        }
    }
}
