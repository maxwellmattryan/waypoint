//! `propose_interview`: turns the interview times of a linked recruiter email into Google
//! Calendar events, each created only after the user approves it (see docs/email.md,
//! "Calendar proposals").
//!
//! **Slots** ([`select_slots`]). Only `interview_invite` and `availability_request`
//! classifications have slots. A proposed time is kept when its zone is known
//! (`start_utc` is set) and it starts after now; its end is `end_utc`, or 30 minutes after
//! the start. A repeated start keeps its first time, and at most 5 are kept, in email order.
//! The slot's `time_zone` is the time's zone when it is `UTC` or an IANA name, and `null`
//! for a `±HH:MM` offset (Google needs a name; the instant is exact either way).
//!
//! **Request** ([`build_request`]). Each slot's `create_calendar_event` input is built once
//! and stored on its `interview_events` row: `Interview: <company> — <stage or title>`, a
//! description naming the role, stage, interviewer, proposed time and email, and the UTC
//! start and end. Nothing else: no attendees, location, conference data or email body.

use chrono::{DateTime, NaiveDateTime, SecondsFormat, TimeDelta, Utc};
use serde::{Deserialize, Serialize};

use crate::workflows::classify_email::{EmailClass, NormalizedTime, Zone, parse_zone};

/// The workflow's name.
pub const WORKFLOW: &str = "propose_interview";
/// Its one step.
pub const STEP: &str = "create";
/// The `external` tool the step calls.
pub const TOOL: &str = "create_calendar_event";
/// Most slots proposed from one email.
pub const MAX_SLOTS: usize = 5;
/// An event's length when the email states no end, in minutes.
pub const DEFAULT_MINUTES: i64 = 30;
/// Most characters of an event's summary.
pub const MAX_SUMMARY: usize = 200;
/// Most characters of an event's description.
pub const MAX_DESCRIPTION: usize = 2_000;

/// One interview time to propose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slot {
    /// 1-based, in email order.
    pub index: usize,
    /// How many slots the email has.
    pub count: usize,
    /// The time as the email wrote it.
    pub text: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    /// `UTC` or an IANA name; `None` for an offset.
    pub time_zone: Option<String>,
}

fn instant(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text.trim())
        .ok()
        .map(|at| at.with_timezone(&Utc))
}

/// The normalized name of `UTC` or an IANA zone; `None` for an offset or an unknown zone.
pub(crate) fn zone_name(text: &str) -> Option<String> {
    match parse_zone(text.trim())? {
        (Zone::Utc | Zone::Named(_), name) => Some(name),
        (Zone::Fixed(_), _) => None,
    }
}

/// The slots of an email of `class` proposing `times`, as of `now` (see the module docs).
pub fn select_slots(class: EmailClass, times: &[NormalizedTime], now: DateTime<Utc>) -> Vec<Slot> {
    if !matches!(
        class,
        EmailClass::InterviewInvite | EmailClass::AvailabilityRequest
    ) {
        return Vec::new();
    }
    let mut kept: Vec<Slot> = Vec::new();
    for time in times {
        if kept.len() == MAX_SLOTS {
            break;
        }
        let Some(start) = time.start_utc.as_deref().and_then(instant) else {
            continue;
        };
        if start <= now || kept.iter().any(|slot| slot.start == start) {
            continue;
        }
        let end = time
            .end_utc
            .as_deref()
            .and_then(instant)
            .filter(|end| *end > start)
            .unwrap_or(start + TimeDelta::minutes(DEFAULT_MINUTES));
        kept.push(Slot {
            index: kept.len() + 1,
            count: 0,
            text: time.text.clone(),
            start,
            end,
            time_zone: time.time_zone.as_deref().and_then(zone_name),
        });
    }
    let count = kept.len();
    for slot in &mut kept {
        slot.count = count;
    }
    kept
}

/// The `create_calendar_event` input: the event to create.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventRequest {
    pub summary: String,
    pub description: String,
    /// RFC 3339.
    pub start: String,
    /// RFC 3339.
    pub end: String,
    /// `UTC` or an IANA name; `null` shows the event in the calendar's own zone.
    pub time_zone: Option<String>,
}

/// What a proposal reads from the application, the classification and the email.
#[derive(Debug, Clone, Copy, Default)]
pub struct Facts<'a> {
    /// The application's company.
    pub company: &'a str,
    /// The application's title.
    pub title: &'a str,
    /// The classification's stage.
    pub stage: Option<&'a str>,
    /// The classification's interviewer.
    pub interviewer: Option<&'a str>,
    /// The email's subject.
    pub subject: &'a str,
    pub from_name: Option<&'a str>,
    pub from_address: Option<&'a str>,
}

/// `text` on one line: newlines become spaces, then it is trimmed.
fn flat(text: &str) -> String {
    text.replace(['\r', '\n'], " ").trim().to_owned()
}

/// `text` flattened, or `None` when it is absent or blank.
fn present(text: Option<&str>) -> Option<String> {
    text.map(flat).filter(|text| !text.is_empty())
}

fn first(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

fn utc_text(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// The tool input for `slot` (see the module docs and docs/email.md).
pub fn build_request(facts: &Facts<'_>, slot: &Slot) -> EventRequest {
    let company = flat(facts.company);
    let title = present(Some(facts.title));
    let stage = present(facts.stage);
    let summary = match stage.as_ref().or(title.as_ref()) {
        Some(what) => format!("Interview: {company} — {what}"),
        None => format!("Interview: {company}"),
    };

    let mut lines = Vec::new();
    if let Some(title) = &title {
        lines.push(format!("Role: {title}"));
    }
    if let Some(stage) = &stage {
        lines.push(format!("Stage: {stage}"));
    }
    if let Some(interviewer) = present(facts.interviewer) {
        lines.push(format!("Interviewer: {interviewer}"));
    }
    if let Some(text) = present(Some(&slot.text)) {
        lines.push(if slot.count > 1 {
            format!("Proposed time {} of {}: \"{text}\"", slot.index, slot.count)
        } else {
            format!("Proposed time: \"{text}\"")
        });
    }
    let subject = present(Some(facts.subject)).unwrap_or_else(|| "(no subject)".into());
    let mut from = format!("From email: \"{subject}\"");
    if let Some(sender) = present(facts.from_name).or_else(|| present(facts.from_address)) {
        from.push_str(" · ");
        from.push_str(&sender);
    }
    lines.push(from);
    lines.push("Added by Waypoint after your approval.".into());

    EventRequest {
        summary: first(&summary, MAX_SUMMARY),
        description: first(&lines.join("\n"), MAX_DESCRIPTION),
        start: utc_text(slot.start),
        end: utc_text(slot.end),
        time_zone: slot.time_zone.clone(),
    }
}

/// The `interview_scheduled` event's body: `<summary> · <date> <HH:MM>–<HH:MM> <zone>` in
/// `time_zone` (UTC when it is `None` or unknown), or `<date> <HH:MM> – <date> <HH:MM>`
/// when the event ends on another local date.
pub fn timeline_body(
    summary: &str,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    time_zone: Option<&str>,
) -> String {
    let (start, end, zone): (NaiveDateTime, NaiveDateTime, String) =
        match time_zone.and_then(|zone| parse_zone(zone.trim())) {
            Some((Zone::Named(tz), name)) => (
                start.with_timezone(&tz).naive_local(),
                end.with_timezone(&tz).naive_local(),
                name,
            ),
            Some((Zone::Fixed(offset), name)) => (
                start.with_timezone(&offset).naive_local(),
                end.with_timezone(&offset).naive_local(),
                name,
            ),
            Some((Zone::Utc, _)) | None => (start.naive_utc(), end.naive_utc(), "UTC".into()),
        };
    let summary = flat(summary);
    if start.date() == end.date() {
        format!(
            "{summary} · {} {}–{} {zone}",
            start.format("%Y-%m-%d"),
            start.format("%H:%M"),
            end.format("%H:%M")
        )
    } else {
        format!(
            "{summary} · {} – {} {zone}",
            start.format("%Y-%m-%d %H:%M"),
            end.format("%Y-%m-%d %H:%M")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> DateTime<Utc> {
        instant(text).expect("an RFC 3339 instant")
    }

    fn time(
        text: &str,
        zone: Option<&str>,
        start: Option<&str>,
        end: Option<&str>,
    ) -> NormalizedTime {
        NormalizedTime {
            text: text.into(),
            start_local: "2030-11-03T14:00:00".into(),
            end_local: None,
            time_zone: zone.map(str::to_owned),
            start_utc: start.map(str::to_owned),
            end_utc: end.map(str::to_owned),
        }
    }

    fn slot(
        index: usize,
        count: usize,
        text: &str,
        start: &str,
        end: &str,
        zone: Option<&str>,
    ) -> Slot {
        Slot {
            index,
            count,
            text: text.into(),
            start: at(start),
            end: at(end),
            time_zone: zone.map(str::to_owned),
        }
    }

    #[test]
    fn select_slots_cases() {
        let now = at("2030-11-01T12:00:00Z");
        let ny = Some("America/New_York");
        let future = time(
            "Tue 2pm ET",
            ny,
            Some("2030-11-03T19:00:00Z"),
            Some("2030-11-03T19:45:00Z"),
        );
        let five: Vec<NormalizedTime> = (0..7)
            .map(|day| {
                let start = format!("2030-11-{:02}T15:00:00Z", 10 + day);
                time(&format!("day {day}"), Some("UTC"), Some(&start), None)
            })
            .collect();
        let cases: Vec<(&str, EmailClass, Vec<NormalizedTime>, Vec<Slot>)> = vec![
            (
                "a future time with an end and a named zone",
                EmailClass::InterviewInvite,
                vec![future.clone()],
                vec![slot(
                    1,
                    1,
                    "Tue 2pm ET",
                    "2030-11-03T19:00:00Z",
                    "2030-11-03T19:45:00Z",
                    ny,
                )],
            ),
            (
                "availability requests have slots too",
                EmailClass::AvailabilityRequest,
                vec![future.clone()],
                vec![slot(
                    1,
                    1,
                    "Tue 2pm ET",
                    "2030-11-03T19:00:00Z",
                    "2030-11-03T19:45:00Z",
                    ny,
                )],
            ),
            (
                "rejections have none",
                EmailClass::Rejection,
                vec![future.clone()],
                vec![],
            ),
            (
                "offers have none",
                EmailClass::Offer,
                vec![future.clone()],
                vec![],
            ),
            (
                "other has none",
                EmailClass::Other,
                vec![future.clone()],
                vec![],
            ),
            (
                "past, now, zone-less and future times",
                EmailClass::InterviewInvite,
                vec![
                    time("last week", Some("UTC"), Some("2030-10-25T15:00:00Z"), None),
                    time("right now", Some("UTC"), Some("2030-11-01T12:00:00Z"), None),
                    time("Tuesday 2pm", None, None, None),
                    time("Wed 9:00", Some("UTC"), Some("2030-11-06T09:00:00Z"), None),
                ],
                vec![slot(
                    1,
                    1,
                    "Wed 9:00",
                    "2030-11-06T09:00:00Z",
                    "2030-11-06T09:30:00Z",
                    Some("UTC"),
                )],
            ),
            (
                "an offset zone keeps the instant and drops the name",
                EmailClass::InterviewInvite,
                vec![time(
                    "2pm IST",
                    Some("+05:30"),
                    Some("2030-11-03T08:30:00Z"),
                    Some("2030-11-03T09:30:00Z"),
                )],
                vec![slot(
                    1,
                    1,
                    "2pm IST",
                    "2030-11-03T08:30:00Z",
                    "2030-11-03T09:30:00Z",
                    None,
                )],
            ),
            (
                "a repeated start keeps the first",
                EmailClass::InterviewInvite,
                vec![
                    future.clone(),
                    time("again", Some("UTC"), Some("2030-11-03T19:00:00Z"), None),
                    time(
                        "Thu",
                        Some("Europe/Berlin"),
                        Some("2030-11-07T09:00:00Z"),
                        None,
                    ),
                ],
                vec![
                    slot(
                        1,
                        2,
                        "Tue 2pm ET",
                        "2030-11-03T19:00:00Z",
                        "2030-11-03T19:45:00Z",
                        ny,
                    ),
                    slot(
                        2,
                        2,
                        "Thu",
                        "2030-11-07T09:00:00Z",
                        "2030-11-07T09:30:00Z",
                        Some("Europe/Berlin"),
                    ),
                ],
            ),
            (
                "at most five, in email order",
                EmailClass::InterviewInvite,
                five,
                (0..5)
                    .map(|day| {
                        slot(
                            day + 1,
                            5,
                            &format!("day {day}"),
                            &format!("2030-11-{:02}T15:00:00Z", 10 + day),
                            &format!("2030-11-{:02}T15:30:00Z", 10 + day),
                            Some("UTC"),
                        )
                    })
                    .collect(),
            ),
            ("no times", EmailClass::InterviewInvite, vec![], vec![]),
        ];
        for (name, class, times, expected) in cases {
            assert_eq!(select_slots(class, &times, now), expected, "{name}");
        }
    }

    #[test]
    fn build_request_cases() {
        let facts = Facts {
            company: "Hooli",
            title: "Backend Engineer",
            stage: Some("Technical interview"),
            interviewer: Some("Gavin Belson"),
            subject: "Interview availability",
            from_name: Some("Hooli Recruiting"),
            from_address: Some("recruiting@hooli.example"),
        };
        let ny = Some("America/New_York");
        let second = slot(
            2,
            3,
            "Tue Nov 3, 2pm ET",
            "2030-11-03T19:00:00Z",
            "2030-11-03T19:45:00Z",
            ny,
        );
        let only = slot(
            1,
            1,
            "Tue Nov 3, 2pm ET",
            "2030-11-03T19:00:00Z",
            "2030-11-03T19:45:00Z",
            None,
        );
        let long = "x".repeat(300);
        let longer = "y".repeat(2500);
        let cases: Vec<(&str, Facts<'_>, &Slot, EventRequest)> = vec![
            (
                "the documented example",
                facts,
                &second,
                EventRequest {
                    summary: "Interview: Hooli — Technical interview".into(),
                    description: "Role: Backend Engineer\nStage: Technical interview\n\
                                  Interviewer: Gavin Belson\n\
                                  Proposed time 2 of 3: \"Tue Nov 3, 2pm ET\"\n\
                                  From email: \"Interview availability\" · Hooli Recruiting\n\
                                  Added by Waypoint after your approval."
                        .into(),
                    start: "2030-11-03T19:00:00Z".into(),
                    end: "2030-11-03T19:45:00Z".into(),
                    time_zone: Some("America/New_York".into()),
                },
            ),
            (
                "a blank stage uses the title; one slot has no count; no interviewer",
                Facts {
                    stage: Some("  "),
                    interviewer: None,
                    ..facts
                },
                &only,
                EventRequest {
                    summary: "Interview: Hooli — Backend Engineer".into(),
                    description: "Role: Backend Engineer\n\
                                  Proposed time: \"Tue Nov 3, 2pm ET\"\n\
                                  From email: \"Interview availability\" · Hooli Recruiting\n\
                                  Added by Waypoint after your approval."
                        .into(),
                    start: "2030-11-03T19:00:00Z".into(),
                    end: "2030-11-03T19:45:00Z".into(),
                    time_zone: None,
                },
            ),
            (
                "no subject, the address as sender, newlines flattened",
                Facts {
                    company: " Hooli\n",
                    stage: Some("Onsite\r\nloop"),
                    interviewer: Some(" \n"),
                    subject: "  ",
                    from_name: Some(" "),
                    ..facts
                },
                &only,
                EventRequest {
                    summary: "Interview: Hooli — Onsite  loop".into(),
                    description: "Role: Backend Engineer\nStage: Onsite  loop\n\
                                  Proposed time: \"Tue Nov 3, 2pm ET\"\n\
                                  From email: \"(no subject)\" · recruiting@hooli.example\n\
                                  Added by Waypoint after your approval."
                        .into(),
                    start: "2030-11-03T19:00:00Z".into(),
                    end: "2030-11-03T19:45:00Z".into(),
                    time_zone: None,
                },
            ),
            (
                "no sender at all",
                Facts {
                    stage: None,
                    interviewer: None,
                    from_name: None,
                    from_address: None,
                    ..facts
                },
                &only,
                EventRequest {
                    summary: "Interview: Hooli — Backend Engineer".into(),
                    description: "Role: Backend Engineer\n\
                                  Proposed time: \"Tue Nov 3, 2pm ET\"\n\
                                  From email: \"Interview availability\"\n\
                                  Added by Waypoint after your approval."
                        .into(),
                    start: "2030-11-03T19:00:00Z".into(),
                    end: "2030-11-03T19:45:00Z".into(),
                    time_zone: None,
                },
            ),
        ];
        for (name, facts, slot, expected) in cases {
            assert_eq!(build_request(&facts, slot), expected, "{name}");
        }

        // The limits: 200 characters of summary, 2000 of description.
        let capped = build_request(
            &Facts {
                stage: Some(&long),
                interviewer: Some(&longer),
                ..facts
            },
            &only,
        );
        assert_eq!(capped.summary.chars().count(), MAX_SUMMARY);
        assert!(capped.summary.starts_with("Interview: Hooli — xxx"));
        assert_eq!(capped.description.chars().count(), MAX_DESCRIPTION);
        assert!(
            capped
                .description
                .starts_with("Role: Backend Engineer\nStage: xxx")
        );
    }

    #[test]
    fn timeline_body_cases() {
        let summary = "Interview: Hooli — Technical interview";
        let cases = [
            (
                "2030-11-03T19:00:00Z",
                "2030-11-03T19:45:00Z",
                Some("America/New_York"),
                "Interview: Hooli — Technical interview · 2030-11-03 14:00–14:45 America/New_York",
            ),
            (
                "2030-11-03T19:00:00Z",
                "2030-11-03T19:45:00Z",
                None,
                "Interview: Hooli — Technical interview · 2030-11-03 19:00–19:45 UTC",
            ),
            (
                "2030-11-03T19:00:00Z",
                "2030-11-03T19:45:00Z",
                Some("UTC"),
                "Interview: Hooli — Technical interview · 2030-11-03 19:00–19:45 UTC",
            ),
            (
                "2030-11-03T23:30:00Z",
                "2030-11-04T00:30:00Z",
                None,
                "Interview: Hooli — Technical interview · 2030-11-03 23:30 – 2030-11-04 00:30 UTC",
            ),
            (
                "2030-11-04T03:30:00Z",
                "2030-11-04T05:30:00Z",
                Some("America/Los_Angeles"),
                "Interview: Hooli — Technical interview · 2030-11-03 19:30–21:30 America/Los_Angeles",
            ),
            (
                "2030-11-03T22:30:00Z",
                "2030-11-03T23:30:00Z",
                Some("Europe/Berlin"),
                "Interview: Hooli — Technical interview · 2030-11-03 23:30 – 2030-11-04 00:30 Europe/Berlin",
            ),
        ];
        for (start, end, zone, expected) in cases {
            assert_eq!(timeline_body(summary, at(start), at(end), zone), expected);
        }
    }
}
