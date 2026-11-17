use jiff::SignedDuration;
use thiserror::Error;

use crate::datetime::Timestamp;

const MAX_REPEAT_SECONDS: u64 = 31_557_600;

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ScheduleTimeError {
    #[error("time value is empty")]
    Empty,
    #[error("time value is not a valid duration, Unix timestamp or RFC 3339 timestamp")]
    Invalid,
    #[error("duration contains an unsupported unit; use s, m, h, d or w")]
    UnsupportedUnit,
    #[error("duration is too large")]
    Overflow,
    #[error("scheduled time must be at least {minimum_seconds} seconds in the future")]
    TooSoon { minimum_seconds: u32 },
    #[error("scheduled time cannot be more than {maximum_seconds} seconds in the future")]
    TooFar { maximum_seconds: u64 },
    #[error("repeat interval must be between 60 seconds and one year")]
    InvalidRepeat,
}

pub fn parse_schedule_time(
    value: &str,
    now: Timestamp,
    minimum_delay_seconds: u32,
    maximum_delay_seconds: u64,
) -> Result<Timestamp, ScheduleTimeError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(ScheduleTimeError::Empty);
    }

    let without_prefix = value
        .strip_prefix("in ")
        .or_else(|| value.strip_prefix("za "))
        .unwrap_or(value)
        .trim();

    let target = match parse_duration_seconds(without_prefix) {
        Ok(seconds) => {
            if seconds > maximum_delay_seconds {
                return Err(ScheduleTimeError::TooFar {
                    maximum_seconds: maximum_delay_seconds,
                });
            }
            if seconds < u64::from(minimum_delay_seconds) {
                return Err(ScheduleTimeError::TooSoon {
                    minimum_seconds: minimum_delay_seconds,
                });
            }
            let seconds = i64::try_from(seconds).map_err(|_| ScheduleTimeError::Overflow)?;
            now.checked_add(SignedDuration::from_secs(seconds))
                .map_err(|_| ScheduleTimeError::Overflow)?
        }
        Err(ScheduleTimeError::Overflow) => {
            return Err(ScheduleTimeError::TooFar {
                maximum_seconds: maximum_delay_seconds,
            });
        }
        Err(ScheduleTimeError::UnsupportedUnit) => {
            return parse_absolute(value, now, minimum_delay_seconds, maximum_delay_seconds);
        }
        Err(_) => parse_absolute_timestamp(value)?,
    };

    validate_horizon(target, now, minimum_delay_seconds, maximum_delay_seconds)
}

pub fn parse_repeat_interval(value: &str) -> Result<i64, ScheduleTimeError> {
    let seconds = parse_duration_seconds(value)?;
    if !(60..=MAX_REPEAT_SECONDS).contains(&seconds) {
        return Err(ScheduleTimeError::InvalidRepeat);
    }
    i64::try_from(seconds).map_err(|_| ScheduleTimeError::Overflow)
}

pub fn parse_duration_seconds(value: &str) -> Result<u64, ScheduleTimeError> {
    let value = value.trim().to_ascii_lowercase();
    if value.is_empty() {
        return Err(ScheduleTimeError::Empty);
    }

    let bytes = value.as_bytes();
    let mut index = 0;
    let mut total = 0_u64;
    let mut segments = 0_u32;

    while index < bytes.len() {
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index >= bytes.len() {
            break;
        }

        let number_start = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
        if number_start == index || index >= bytes.len() {
            return Err(ScheduleTimeError::Invalid);
        }

        let amount = value[number_start..index]
            .parse::<u64>()
            .map_err(|_| ScheduleTimeError::Overflow)?;
        let multiplier = match bytes[index] {
            b's' => 1,
            b'm' => 60,
            b'h' => 3_600,
            b'd' => 86_400,
            b'w' => 604_800,
            _ => return Err(ScheduleTimeError::UnsupportedUnit),
        };
        index += 1;
        segments = segments.saturating_add(1);
        total = total
            .checked_add(
                amount
                    .checked_mul(multiplier)
                    .ok_or(ScheduleTimeError::Overflow)?,
            )
            .ok_or(ScheduleTimeError::Overflow)?;
    }

    if segments == 0 || total == 0 {
        return Err(ScheduleTimeError::Invalid);
    }
    Ok(total)
}

fn parse_absolute(
    value: &str,
    now: Timestamp,
    minimum_delay_seconds: u32,
    maximum_delay_seconds: u64,
) -> Result<Timestamp, ScheduleTimeError> {
    let target = parse_absolute_timestamp(value)?;
    validate_horizon(target, now, minimum_delay_seconds, maximum_delay_seconds)
}

fn parse_absolute_timestamp(value: &str) -> Result<Timestamp, ScheduleTimeError> {
    if value.bytes().all(|byte| byte.is_ascii_digit()) {
        let timestamp = value
            .parse::<i64>()
            .map_err(|_| ScheduleTimeError::Invalid)?;
        return Timestamp::from_second(timestamp).map_err(|_| ScheduleTimeError::Invalid);
    }

    Timestamp::parse_rfc3339(value).map_err(|_| ScheduleTimeError::Invalid)
}

fn validate_horizon(
    target: Timestamp,
    now: Timestamp,
    minimum_delay_seconds: u32,
    maximum_delay_seconds: u64,
) -> Result<Timestamp, ScheduleTimeError> {
    let delay = target.duration_since(now).as_secs();
    if delay < i64::from(minimum_delay_seconds) {
        return Err(ScheduleTimeError::TooSoon {
            minimum_seconds: minimum_delay_seconds,
        });
    }

    let maximum = i64::try_from(maximum_delay_seconds).map_err(|_| ScheduleTimeError::Overflow)?;
    if delay > maximum {
        return Err(ScheduleTimeError::TooFar {
            maximum_seconds: maximum_delay_seconds,
        });
    }

    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::{
        ScheduleTimeError, parse_duration_seconds, parse_repeat_interval, parse_schedule_time,
    };

    #[test]
    fn parses_compound_relative_duration() {
        assert_eq!(parse_duration_seconds("1d 2h 30m"), Ok(95_400));
    }

    #[test]
    fn parses_rfc3339_and_unix_timestamps() {
        let now = "2026-09-03T10:00:00Z".parse().unwrap();
        let rfc = parse_schedule_time("2026-09-03T12:00:00Z", now, 10, 86_400)
            .expect("RFC 3339 should parse");
        let unix = parse_schedule_time(&rfc.as_second().to_string(), now, 10, 86_400)
            .expect("Unix timestamp should parse");
        let offset = parse_schedule_time("2026-09-03T14:00:00+02:00", now, 10, 86_400)
            .expect("an explicit UTC offset should parse");
        assert_eq!(rfc, unix);
        assert_eq!(rfc, offset);
    }

    #[test]
    fn rejects_non_rfc3339_absolute_times() {
        let now = "2026-09-03T10:00:00Z".parse().unwrap();
        for value in [
            "2026-09-03T12:00Z",
            "2026-09-03T14:00:00+02",
            "2026-09-03T14:00:00+0200",
            "2026-09-03T14:00:00+02:00[Europe/Warsaw]",
            "2026-09-03T12:00:00",
            "2026-09-04T12:00:00+24:00",
        ] {
            assert_eq!(
                parse_schedule_time(value, now, 10, 86_400),
                Err(ScheduleTimeError::Invalid),
                "{value}"
            );
        }
    }

    #[test]
    fn enforces_schedule_horizon() {
        let now = "2026-09-03T10:00:00Z".parse().unwrap();
        assert!(parse_schedule_time("5s", now, 10, 86_400).is_err());
        assert!(parse_schedule_time("2d", now, 10, 86_400).is_err());
    }

    #[test]
    fn checks_absolute_horizon_in_whole_seconds() {
        let now = "2026-09-03T10:00:00.500Z".parse().unwrap();
        assert_eq!(
            parse_schedule_time("2026-09-03T10:00:10.499Z", now, 10, 60),
            Err(ScheduleTimeError::TooSoon {
                minimum_seconds: 10
            })
        );
        assert!(parse_schedule_time("2026-09-03T10:00:10.500Z", now, 10, 60).is_ok());
        assert!(parse_schedule_time("2026-09-03T10:01:01.499Z", now, 10, 60).is_ok());
        assert_eq!(
            parse_schedule_time("2026-09-03T10:01:01.500Z", now, 10, 60),
            Err(ScheduleTimeError::TooFar {
                maximum_seconds: 60
            })
        );
    }

    #[test]
    fn repeat_interval_is_bounded() {
        assert_eq!(parse_repeat_interval("1h"), Ok(3_600));
        assert!(parse_repeat_interval("30s").is_err());
    }

    #[test]
    fn extreme_duration_does_not_panic_and_enforces_boundaries() {
        let now = "2026-09-03T10:00:00Z".parse().unwrap();

        // Exact maximum delay succeeds
        let exact_max = parse_schedule_time("86400s", now, 10, 86_400);
        assert!(exact_max.is_ok());

        // Maximum + 1 second returns TooFar
        assert_eq!(
            parse_schedule_time("86401s", now, 10, 86_400),
            Err(ScheduleTimeError::TooFar {
                maximum_seconds: 86_400
            })
        );

        // Extreme durations still produce a horizon error instead of overflowing.
        assert_eq!(
            parse_schedule_time("9223372036854775807s", now, 10, 86_400),
            Err(ScheduleTimeError::TooFar {
                maximum_seconds: 86_400
            })
        );
        assert_eq!(
            parse_schedule_time("18446744073709551615s", now, 10, 86_400),
            Err(ScheduleTimeError::TooFar {
                maximum_seconds: 86_400
            })
        );
        assert_eq!(
            parse_schedule_time("999999999999999999999999999999s", now, 10, 86_400),
            Err(ScheduleTimeError::TooFar {
                maximum_seconds: 86_400
            })
        );

        // Multiple units with large numbers
        assert_eq!(
            parse_schedule_time("999999w 999999d 999999h", now, 10, 86_400),
            Err(ScheduleTimeError::TooFar {
                maximum_seconds: 86_400
            })
        );
    }
}
