//! Raw RFC 822 bytes to [`ParsedEmail`]. `mail-parser` does the MIME structure, transfer
//! and charset decoding, RFC 2047 encoded words, addresses and dates; this module only
//! picks the parts and fields Waypoint keeps. HTML is returned as HTML: converting it is
//! [`super::html`]'s job, never `mail-parser`'s.

use std::fmt;

use chrono::{DateTime, Utc};
use mail_parser::{HeaderValue, Message, MessageParser, MessagePart, MimeHeaders, PartType};

/// One mailbox, its address lowercased.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Address {
    pub address: String,
    pub name: Option<String>,
}

/// The fields of a message Waypoint keeps. Message ids are stored without angle brackets,
/// and header values are RFC 2047-decoded and kept in their original order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ParsedEmail {
    pub rfc_message_id: Option<String>,
    pub in_reply_to: Vec<String>,
    pub references: Vec<String>,
    pub from: Option<Address>,
    pub sender: Option<Address>,
    pub reply_to: Vec<Address>,
    pub return_path: Option<String>,
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    pub subject: String,
    pub sent_at: Option<DateTime<Utc>>,
    pub headers: Vec<(String, String)>,
    /// The first inline text/plain body part, decoded to UTF-8.
    pub text_part: Option<String>,
    /// The first inline text/html body part, decoded to UTF-8.
    pub html_part: Option<String>,
}

impl ParsedEmail {
    /// The first value of header `name` (case-insensitive).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// The parser found no message in the bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError(pub String);

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cannot parse the message: {}", self.0)
    }
}

impl std::error::Error for ParseError {}

/// Parses one raw message. Fails only when no header can be read at all.
pub fn parse(raw: &[u8]) -> Result<ParsedEmail, ParseError> {
    let message = MessageParser::default()
        .parse(raw)
        .ok_or_else(|| ParseError("no headers found".into()))?;
    if message.root_part().headers.is_empty() {
        return Err(ParseError("no headers found".into()));
    }

    let headers = message
        .root_part()
        .headers
        .iter()
        .map(|h| {
            let raw_value = raw
                .get(h.offset_start as usize..h.offset_end as usize)
                .map(String::from_utf8_lossy)
                .unwrap_or_default();
            (h.name().to_string(), header_text(&h.value, &raw_value))
        })
        .collect();

    Ok(ParsedEmail {
        rfc_message_id: message
            .message_id()
            .map(clean_id)
            .filter(|id| !id.is_empty()),
        in_reply_to: ids(message.in_reply_to()),
        references: ids(message.references()),
        from: message.from().and_then(|a| a.iter().find_map(address)),
        sender: message.sender().and_then(|a| a.iter().find_map(address)),
        reply_to: addresses(message.reply_to()),
        return_path: message
            .return_address()
            .map(|a| clean_id(a).to_lowercase())
            .filter(|a| !a.is_empty()),
        to: addresses(message.to()),
        cc: addresses(message.cc()),
        subject: message.subject().map(collapse).unwrap_or_default(),
        sent_at: message
            .date()
            .filter(|d| d.is_valid())
            .and_then(|d| DateTime::from_timestamp(d.to_timestamp(), 0)),
        headers,
        text_part: body_part(&message, &message.text_body, "plain"),
        html_part: body_part(&message, &message.html_body, "html"),
    })
}

/// The first part among `ids` that is an inline `text/<subtype>` (a part with no
/// Content-Type counts as text/plain). Attachments, calendars and binaries never match.
fn body_part(message: &Message<'_>, ids: &[u32], subtype: &str) -> Option<String> {
    ids.iter()
        .filter_map(|id| message.part(*id))
        .find(|part| is_body(part, subtype))
        .and_then(|part| match &part.body {
            PartType::Text(text) | PartType::Html(text) => Some(text.to_string()),
            _ => None,
        })
}

fn is_body(part: &MessagePart<'_>, subtype: &str) -> bool {
    if part
        .content_disposition()
        .is_some_and(|d| d.ctype().eq_ignore_ascii_case("attachment"))
    {
        return false;
    }
    let (ctype, sub) = match part.content_type() {
        Some(ct) => (ct.ctype(), ct.subtype().unwrap_or("plain")),
        None => ("text", "plain"),
    };
    let kind_ok = matches!(
        (&part.body, subtype),
        (PartType::Text(_), "plain") | (PartType::Html(_), "html")
    );
    kind_ok && ctype.eq_ignore_ascii_case("text") && sub.eq_ignore_ascii_case(subtype)
}

fn address(addr: &mail_parser::Addr<'_>) -> Option<Address> {
    let address = addr.address()?.trim().to_lowercase();
    if address.is_empty() {
        return None;
    }
    Some(Address {
        address,
        name: addr.name().map(collapse).filter(|n| !n.is_empty()),
    })
}

fn addresses(list: Option<&mail_parser::Address<'_>>) -> Vec<Address> {
    list.map(|a| a.iter().filter_map(address).collect())
        .unwrap_or_default()
}

/// Message ids from an `In-Reply-To` or `References` value, brackets removed.
fn ids(value: &HeaderValue<'_>) -> Vec<String> {
    let raw: Vec<&str> = match value {
        HeaderValue::Text(t) => vec![t.as_ref()],
        HeaderValue::TextList(list) => list.iter().map(|t| t.as_ref()).collect(),
        _ => Vec::new(),
    };
    raw.into_iter()
        .flat_map(|t| t.split_whitespace())
        .map(clean_id)
        .filter(|id| !id.is_empty())
        .collect()
}

fn clean_id(id: &str) -> String {
    id.trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .trim()
        .to_string()
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A header's value as display text: decoded for text and address headers, the unfolded
/// raw value for the rest (dates, content types, received lines).
fn header_text(value: &HeaderValue<'_>, raw_value: &str) -> String {
    match value {
        HeaderValue::Text(t) => collapse(t),
        HeaderValue::TextList(list) => list
            .iter()
            .map(|t| collapse(t))
            .collect::<Vec<_>>()
            .join(", "),
        HeaderValue::Address(list) => list
            .iter()
            .map(|a| match (a.name(), a.address()) {
                (Some(name), Some(addr)) => format!("{} <{addr}>", collapse(name)),
                (None, Some(addr)) => addr.to_string(),
                (Some(name), None) => collapse(name),
                (None, None) => String::new(),
            })
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(", "),
        _ => collapse(raw_value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_str(raw: &str) -> ParsedEmail {
        parse(raw.replace('\n', "\r\n").as_bytes()).expect("parses")
    }

    #[test]
    fn multipart_alternative_prefers_text_plain() {
        let p = parse_str(
            "From: Ana Recruiter <Ana@Acme.example>\n\
             To: me@home.example, Bo <bo@home.example>\n\
             Cc: cc@home.example\n\
             Subject: Hello\n\
             Message-ID: <abc@acme.example>\n\
             In-Reply-To: <parent@acme.example>\n\
             References: <root@acme.example> <parent@acme.example>\n\
             Date: Tue, 06 Oct 2026 14:03:00 -0400\n\
             MIME-Version: 1.0\n\
             Content-Type: multipart/alternative; boundary=\"b1\"\n\
             \n\
             --b1\n\
             Content-Type: text/plain; charset=utf-8\n\
             \n\
             Plain body\n\
             --b1\n\
             Content-Type: text/html; charset=utf-8\n\
             \n\
             <p>HTML body</p>\n\
             --b1--\n",
        );
        assert_eq!(p.text_part.as_deref().map(str::trim), Some("Plain body"));
        assert_eq!(
            p.html_part.as_deref().map(str::trim),
            Some("<p>HTML body</p>")
        );
        let from = p.from.clone().expect("from");
        assert_eq!(from.address, "ana@acme.example");
        assert_eq!(from.name.as_deref(), Some("Ana Recruiter"));
        let to: Vec<_> = p.to.iter().map(|a| a.address.as_str()).collect();
        assert_eq!(to, ["me@home.example", "bo@home.example"]);
        assert_eq!(p.cc[0].address, "cc@home.example");
        assert_eq!(p.rfc_message_id.as_deref(), Some("abc@acme.example"));
        assert_eq!(p.in_reply_to, ["parent@acme.example"]);
        assert_eq!(p.references, ["root@acme.example", "parent@acme.example"]);
        assert_eq!(
            p.sent_at.map(|d| d.to_rfc3339()).as_deref(),
            Some("2026-10-06T18:03:00+00:00")
        );
        assert_eq!(p.header("subject"), Some("Hello"));
        assert_eq!(p.headers[0].0, "From");
    }

    #[test]
    fn html_only_has_no_text_part() {
        let p = parse_str(
            "From: a@b.example\nSubject: x\nContent-Type: text/html; charset=utf-8\n\n<p>Hi</p>\n",
        );
        assert_eq!(p.text_part, None);
        assert_eq!(p.html_part.as_deref().map(str::trim), Some("<p>Hi</p>"));
    }

    #[test]
    fn blank_text_plain_still_returns_both_parts() {
        // normalize falls back to HTML when the text part is blank; the parser keeps both.
        let p = parse_str(
            "From: a@b.example\nContent-Type: multipart/alternative; boundary=b\n\n\
             --b\nContent-Type: text/plain\n\n   \n--b\nContent-Type: text/html\n\n<b>Real</b>\n--b--\n",
        );
        assert_eq!(p.text_part.as_deref().map(str::trim), Some(""));
        assert_eq!(p.html_part.as_deref().map(str::trim), Some("<b>Real</b>"));
    }

    #[test]
    fn nested_mixed_ignores_attachment_and_calendar() {
        let p = parse_str(
            "From: a@b.example\nContent-Type: multipart/mixed; boundary=outer\n\n\
             --outer\nContent-Type: multipart/alternative; boundary=inner\n\n\
             --inner\nContent-Type: text/plain\n\nBody text\n\
             --inner\nContent-Type: text/html\n\n<p>Body html</p>\n--inner--\n\
             --outer\nContent-Type: text/calendar; method=REQUEST\n\nBEGIN:VCALENDAR\nEND:VCALENDAR\n\
             --outer\nContent-Type: application/pdf; name=cv.pdf\n\
             Content-Disposition: attachment; filename=cv.pdf\n\
             Content-Transfer-Encoding: base64\n\nJVBERi0xLjQK\n--outer--\n",
        );
        assert_eq!(p.text_part.as_deref().map(str::trim), Some("Body text"));
        assert_eq!(
            p.html_part.as_deref().map(str::trim),
            Some("<p>Body html</p>")
        );
    }

    #[test]
    fn calendar_only_has_no_body_part() {
        let p = parse_str(
            "From: a@b.example\nContent-Type: text/calendar\n\nBEGIN:VCALENDAR\nEND:VCALENDAR\n",
        );
        assert_eq!((p.text_part, p.html_part), (None, None));
    }

    #[test]
    fn base64_and_quoted_printable_decode() {
        let b64 = parse_str(
            "From: a@b.example\nContent-Type: text/plain; charset=utf-8\n\
             Content-Transfer-Encoding: base64\n\nSGVsbG8sIHdvcmxkIOKCrA==\n",
        );
        assert_eq!(
            b64.text_part.as_deref().map(str::trim),
            Some("Hello, world €")
        );
        let qp = parse_str(
            "From: a@b.example\nContent-Type: text/plain; charset=utf-8\n\
             Content-Transfer-Encoding: quoted-printable\n\n\
             Caf=C3=A9 and a soft=\n break\n",
        );
        assert_eq!(
            qp.text_part.as_deref().map(str::trim),
            Some("Café and a soft break")
        );
    }

    #[test]
    fn legacy_charsets_decode_to_utf8() {
        let mut win =
            b"From: a@b.example\r\nContent-Type: text/plain; charset=windows-1252\r\n\r\n".to_vec();
        win.extend_from_slice(b"\x93Quoted\x94 \x80100\r\n");
        let p = parse(&win).expect("parses");
        assert_eq!(
            p.text_part.as_deref().map(str::trim),
            Some("\u{201c}Quoted\u{201d} €100")
        );

        let mut latin =
            b"From: a@b.example\r\nContent-Type: text/plain; charset=ISO-8859-1\r\n\r\n".to_vec();
        latin.extend_from_slice(b"Ingenier\xeda\r\n");
        let p = parse(&latin).expect("parses");
        assert_eq!(p.text_part.as_deref().map(str::trim), Some("Ingeniería"));
    }

    #[test]
    fn rfc2047_subject_and_display_names_decode() {
        let p = parse_str(
            "From: =?UTF-8?Q?Mar=C3=ADa_G=C3=B3mez?= <maria@acme.example>\n\
             To: =?ISO-8859-1?Q?Jos=E9?= <jose@home.example>\n\
             Subject: =?UTF-8?B?RW50cmV2aXN0YSDigJMgSW5nZW5pZXLDrWE=?=\n\n.\n",
        );
        assert_eq!(p.subject, "Entrevista – Ingeniería");
        assert_eq!(
            p.from.as_ref().and_then(|a| a.name.as_deref()),
            Some("María Gómez")
        );
        assert_eq!(p.to[0].name.as_deref(), Some("José"));
        assert_eq!(p.header("Subject"), Some("Entrevista – Ingeniería"));
    }

    #[test]
    fn missing_or_bad_date_is_none() {
        assert_eq!(parse_str("From: a@b.example\n\nx\n").sent_at, None);
        assert_eq!(
            parse_str("From: a@b.example\nDate: not a date at all\n\nx\n").sent_at,
            None
        );
    }

    #[test]
    fn no_headers_is_a_parse_error() {
        assert!(parse(b"").is_err());
    }
}
