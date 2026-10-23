//! A parsed message's clean body text: the part to read, converted and stripped.

use super::mime::ParsedEmail;
use super::{BodySource, html, quote};

/// Stored in `emails.normalizer_version`. Bump it when the output of [`normalize`]
/// changes, so stored rows can be found and normalized again from `emails.raw`.
pub const NORMALIZER_VERSION: i32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Normalized {
    pub body_text: String,
    pub signature: Option<String>,
    pub body_source: BodySource,
}

/// Picks the text/plain part when it isn't blank, else the HTML part converted with
/// [`html::to_text`], else nothing (`body_source = none`, empty text). Line endings become
/// `\n` and trailing whitespace is trimmed on each line, then [`quote::strip`] removes
/// quoted replies and splits off the signature.
pub fn normalize(parsed: &ParsedEmail) -> Normalized {
    let (text, body_source) = match (&parsed.text_part, &parsed.html_part) {
        (Some(text), _) if !text.trim().is_empty() => (text.clone(), BodySource::Plain),
        (_, Some(html)) => (html::to_text(html), BodySource::Html),
        _ => {
            return Normalized {
                body_text: String::new(),
                signature: None,
                body_source: BodySource::None,
            };
        }
    };
    let clean = text
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n");
    let stripped = quote::strip(&clean);
    Normalized {
        body_text: stripped.text,
        signature: stripped.signature,
        body_source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(text: Option<&str>, html: Option<&str>) -> ParsedEmail {
        ParsedEmail {
            text_part: text.map(Into::into),
            html_part: html.map(Into::into),
            ..Default::default()
        }
    }

    #[test]
    fn normalize() {
        let cases = [
            (
                parsed(Some("Hi  \r\nthere\r\n-- \r\nAna"), Some("<p>x</p>")),
                ("Hi\nthere", Some("Ana"), BodySource::Plain),
            ),
            (
                parsed(Some(" \r\n "), Some("<p>From HTML</p>")),
                ("From HTML", None, BodySource::Html),
            ),
            (
                parsed(None, Some("<p>Only HTML</p>")),
                ("Only HTML", None, BodySource::Html),
            ),
            (parsed(None, None), ("", None, BodySource::None)),
        ];
        for (input, (text, signature, source)) in cases {
            let got = super::normalize(&input);
            assert_eq!(
                (
                    got.body_text.as_str(),
                    got.signature.as_deref(),
                    got.body_source
                ),
                (text, signature, source)
            );
        }
    }
}
