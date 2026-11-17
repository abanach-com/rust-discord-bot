use crate::datetime::Timestamp;
use jiff::SignedDuration;

use crate::storage::ScheduledEvent;

pub fn generate_icalendar_event(event: &ScheduledEvent) -> String {
    let now = Timestamp::now().to_jiff().strftime("%Y%m%dT%H%M%SZ");
    let dtstart = event.start_time.to_jiff().strftime("%Y%m%dT%H%M%SZ");
    let dtend = event
        .end_time
        .map(|e| e.to_jiff().strftime("%Y%m%dT%H%M%SZ").to_string())
        .unwrap_or_else(|| {
            (jiff::tz::TimeZone::UTC.to_datetime(event.start_time.to_jiff())
                + SignedDuration::from_hours(1))
            .strftime("%Y%m%dT%H%M%SZ")
            .to_string()
        });

    let summary = escape_ics_text(&event.title);
    let description = escape_ics_text(&event.description);

    format!(
        "BEGIN:VCALENDAR\r\n\
        VERSION:2.0\r\n\
        PRODID:-//ZuckerBot//Discord Events//EN\r\n\
        CALSCALE:GREGORIAN\r\n\
        METHOD:PUBLISH\r\n\
        BEGIN:VEVENT\r\n\
        UID:{id}@zuckerbot.discord\r\n\
        DTSTAMP:{now}\r\n\
        DTSTART:{dtstart}\r\n\
        DTEND:{dtend}\r\n\
        SUMMARY:{summary}\r\n\
        DESCRIPTION:{description}\r\n\
        STATUS:CONFIRMED\r\n\
        END:VEVENT\r\n\
        END:VCALENDAR\r\n",
        id = event.id,
    )
}

fn escape_ics_text(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace(';', "\\;")
        .replace(',', "\\,")
        .replace('\n', "\\n")
        .replace('\r', "")
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn test_generate_icalendar_event() {
        let ev = ScheduledEvent {
            id: Uuid::new_v4(),
            guild_id: "123".to_owned(),
            title: "Community Game Night, & More!".to_owned(),
            description: "Join us for fun\nBring friends!".to_owned(),
            start_time: Timestamp::parse_rfc3339("2026-12-01T19:00:00Z").unwrap(),
            end_time: Some(Timestamp::parse_rfc3339("2026-12-01T21:00:00Z").unwrap()),
            timezone: "Europe/Warsaw".to_owned(),
            max_participants: Some(10),
            created_at: Timestamp::now(),
        };

        let ics = generate_icalendar_event(&ev);
        assert!(ics.contains("BEGIN:VCALENDAR"));
        assert!(ics.contains("SUMMARY:Community Game Night\\, & More!"));
        assert!(ics.contains("DESCRIPTION:Join us for fun\\nBring friends!"));
        assert!(!ics.contains("TZID:"));
        assert!(ics.contains("DTSTART:20261201T190000Z"));
        assert!(ics.contains("DTEND:20261201T210000Z"));
        assert!(ics.contains("END:VCALENDAR"));
    }

    #[test]
    fn test_implicit_icalendar_end_near_timestamp_limit() {
        let start_time = Timestamp::parse_rfc3339("9999-12-30T21:45:00Z").unwrap();
        let mut event = ScheduledEvent {
            id: Uuid::new_v4(),
            guild_id: "123".to_owned(),
            title: "Future meeting".to_owned(),
            description: String::new(),
            start_time,
            end_time: None,
            timezone: "UTC".to_owned(),
            max_participants: None,
            created_at: Timestamp::now(),
        };

        assert!(generate_icalendar_event(&event).contains("DTEND:99991230T224500Z"));
        assert!(event.registration_deadline() > start_time);

        event.end_time = Some(Timestamp::parse_rfc3339("9999-12-30T21:55:00Z").unwrap());
        assert_eq!(Some(event.registration_deadline()), event.end_time);
    }

    #[test]
    fn test_generate_icalendar_event_with_malicious_crlf_timezone() {
        let ev = ScheduledEvent {
            id: Uuid::new_v4(),
            guild_id: "123".to_owned(),
            title: "Safe Meeting".to_owned(),
            description: "Safe description".to_owned(),
            start_time: Timestamp::parse_rfc3339("2026-12-01T19:00:00Z").unwrap(),
            end_time: None,
            timezone: "UTC\r\nSUMMARY:INJECTED_COLLISION\r\nBEGIN:VEVENT".to_owned(),
            max_participants: None,
            created_at: Timestamp::now(),
        };

        let ics = generate_icalendar_event(&ev);
        assert!(!ics.contains("INJECTED_COLLISION"));
        assert_eq!(ics.matches("SUMMARY:").count(), 1);
        assert_eq!(ics.matches("BEGIN:VEVENT").count(), 1);
        assert_eq!(ics.matches("END:VEVENT").count(), 1);
        assert!(!ics.contains("TZID:"));
    }
}
