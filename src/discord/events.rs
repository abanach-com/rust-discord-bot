use chrono::Utc;

use crate::storage::ScheduledEvent;

pub fn generate_icalendar_event(event: &ScheduledEvent) -> String {
    let now = Utc::now().format("%Y%m%dT%H%M%SZ");
    let dtstart = event.start_time.format("%Y%m%dT%H%M%SZ");
    let dtend = event
        .end_time
        .map(|e| e.format("%Y%m%dT%H%M%SZ").to_string())
        .unwrap_or_else(|| {
            (event.start_time + chrono::Duration::hours(1))
                .format("%Y%m%dT%H%M%SZ")
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
        TZID:{tz}\r\n\
        STATUS:CONFIRMED\r\n\
        END:VEVENT\r\n\
        END:VCALENDAR\r\n",
        id = event.id,
        tz = event.timezone,
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
    use chrono::DateTime;
    use uuid::Uuid;

    #[test]
    fn test_generate_icalendar_event() {
        let ev = ScheduledEvent {
            id: Uuid::new_v4(),
            guild_id: "123".to_owned(),
            title: "Community Game Night, & More!".to_owned(),
            description: "Join us for fun\nBring friends!".to_owned(),
            start_time: DateTime::parse_from_rfc3339("2026-12-01T19:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            end_time: Some(
                DateTime::parse_from_rfc3339("2026-12-01T21:00:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
            ),
            timezone: "Europe/Warsaw".to_owned(),
            max_participants: Some(10),
            created_at: Utc::now(),
        };

        let ics = generate_icalendar_event(&ev);
        assert!(ics.contains("BEGIN:VCALENDAR"));
        assert!(ics.contains("SUMMARY:Community Game Night\\, & More!"));
        assert!(ics.contains("DESCRIPTION:Join us for fun\\nBring friends!"));
        assert!(ics.contains("TZID:Europe/Warsaw"));
        assert!(ics.contains("END:VCALENDAR"));
    }
}
