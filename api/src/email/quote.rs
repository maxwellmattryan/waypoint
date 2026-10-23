//! Removes quoted replies from a plain-text body and splits off its signature. Forwarded
//! content is kept: it is often the only copy of what the recruiter wrote.

/// A body with its quoted replies removed and its signature split off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stripped {
    pub text: String,
    pub signature: Option<String>,
}

/// Strips `text` (lines separated by `\n`):
///
/// 1. Cuts from the first reply marker to the end: Gmail "On … wrote:" (also wrapped over
///    two lines), an Outlook `From:` block (`Sent:`/`Date:` then `To:`/`Subject:` within
///    the next 4 lines, with the line of underscores above it if there is one), or
///    "-----Original Message-----". Markers after a forward marker
///    ("---------- Forwarded message ---------", "Begin forwarded message:") don't count.
/// 2. Removes the remaining lines that start with `>`.
/// 3. Moves a `-- ` signature block, and trailing "Sent from my …" or "Get Outlook for …"
///    lines, into `signature`. Only the part before a forward marker is searched.
/// 4. If that leaves the text blank, returns the unstripped text and no signature.
pub fn strip(text: &str) -> Stripped {
    let lines: Vec<&str> = text.lines().collect();
    let forward = lines.iter().position(|l| is_forward_marker(l));
    let scan_end = forward.unwrap_or(lines.len());
    let cut = (0..scan_end)
        .find_map(|i| reply_marker_at(&lines, i))
        .unwrap_or(lines.len());

    let kept: Vec<&str> = lines[..cut]
        .iter()
        .copied()
        .filter(|l| !l.trim_start().starts_with('>'))
        .collect();
    // Where the forward marker sits in `kept` (cutting stops before it, and it isn't a `>`
    // line, so it survives whenever it was before the cut).
    let own_end = kept
        .iter()
        .position(|l| is_forward_marker(l))
        .unwrap_or(kept.len());

    let (body_end, sig_start) = signature_span(&kept[..own_end]);
    let mut body: Vec<&str> = kept[..body_end].to_vec();
    let signature = join_trimmed(&kept[sig_start..own_end]);
    body.extend_from_slice(&kept[own_end..]);

    let body = join_trimmed(&body);
    match body {
        Some(text) => Stripped { text, signature },
        None => Stripped {
            text: text.trim_matches('\n').to_string(),
            signature: None,
        },
    }
}

fn is_forward_marker(line: &str) -> bool {
    let l = line.trim();
    (l.starts_with("-----") && l.to_ascii_lowercase().contains("forwarded message"))
        || l.eq_ignore_ascii_case("Begin forwarded message:")
}

/// The line a reply marker starts at, if one starts at line `i`.
fn reply_marker_at(lines: &[&str], i: usize) -> Option<usize> {
    let line = lines[i].trim();
    // Gmail: "On Tue, Oct 6, 2026 at 2:03 PM Ana <ana@acme.example> wrote:", possibly wrapped.
    if line.starts_with("On ") {
        if line.ends_with("wrote:") {
            return Some(i);
        }
        if lines
            .get(i + 1)
            .is_some_and(|next| next.trim().ends_with("wrote:"))
        {
            return Some(i);
        }
    }
    // "-----Original Message-----"
    let lower = line.to_ascii_lowercase();
    if lower.starts_with("--") && lower.contains("original message") {
        return Some(i);
    }
    // Outlook: From: then Sent:/Date: then To:/Subject: within the next 4 lines.
    if strip_bold(line).starts_with("From:") {
        let window = &lines[i + 1..lines.len().min(i + 5)];
        let sent = window.iter().position(|l| {
            let l = strip_bold(l.trim());
            l.starts_with("Sent:") || l.starts_with("Date:")
        });
        if let Some(sent) = sent {
            let addressed = window[sent + 1..].iter().any(|l| {
                let l = strip_bold(l.trim());
                l.starts_with("To:") || l.starts_with("Subject:")
            });
            if addressed {
                let underscores = i > 0 && {
                    let above = lines[i - 1].trim();
                    above.len() >= 3 && above.chars().all(|c| c == '_')
                };
                return Some(if underscores { i - 1 } else { i });
            }
        }
    }
    None
}

/// Outlook plain text sometimes keeps the bold markers: `*From:* Ana`.
fn strip_bold(line: &str) -> String {
    line.replacen("*", "", 2)
}

/// `(end of body, start of signature)` within `lines`: a `-- ` delimiter (trailing
/// whitespace may already be trimmed), else trailing "Sent from my …" lines.
fn signature_span(lines: &[&str]) -> (usize, usize) {
    if let Some(delim) = lines.iter().position(|l| *l == "-- " || *l == "--") {
        return (delim, delim + 1);
    }
    let mut start = lines.len();
    for (i, line) in lines.iter().enumerate().rev() {
        let l = line.trim();
        if l.is_empty() {
            continue;
        }
        if is_device_line(l) {
            start = i;
        } else {
            break;
        }
    }
    (start, start)
}

fn is_device_line(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    lower.starts_with("sent from my ") || lower.starts_with("get outlook for ")
}

/// Joins lines, trimming blank lines at both ends; `None` if nothing is left.
fn join_trimmed(lines: &[&str]) -> Option<String> {
    let start = lines.iter().position(|l| !l.trim().is_empty())?;
    let end = lines.iter().rposition(|l| !l.trim().is_empty())? + 1;
    Some(lines[start..end].join("\n"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn strip() {
        // (case, input, text, signature)
        let cases: &[(&str, &str, &str, Option<&str>)] = &[
            (
                "plain text is unchanged",
                "Hi there\n\nThanks",
                "Hi there\n\nThanks",
                None,
            ),
            (
                "gmail on-wrote",
                "Sounds good.\n\nOn Tue, Oct 6, 2026 at 2:03 PM Ana <ana@acme.example> wrote:\n> Are you free?\n",
                "Sounds good.",
                None,
            ),
            (
                "gmail on-wrote wrapped over two lines",
                "Yes!\n\nOn Tue, Oct 6, 2026 at 2:03 PM Ana Recruiter\n<ana@acme.example> wrote:\nold text",
                "Yes!",
                None,
            ),
            (
                "stray > lines are removed",
                "Answer one\n> quoted bit\nAnswer two",
                "Answer one\nAnswer two",
                None,
            ),
            (
                "outlook from-sent-to block",
                "Thursday works.\n\nFrom: Ana <ana@acme.example>\nSent: Monday, October 5, 2026 9:00 AM\nTo: Me <me@home.example>\nSubject: Intro\n\nOriginal",
                "Thursday works.",
                None,
            ),
            (
                "outlook block with underscores and Date",
                "Thanks\n________________________________\nFrom: Ana\nDate: Monday\nSubject: Intro\nOriginal",
                "Thanks",
                None,
            ),
            (
                "outlook bold markers",
                "Ok\n*From:* Ana\n*Sent:* Monday\n*To:* Me\nOriginal",
                "Ok",
                None,
            ),
            (
                "a From: line alone is not a marker",
                "From: the team at Acme\nWelcome aboard",
                "From: the team at Acme\nWelcome aboard",
                None,
            ),
            (
                "original message",
                "Got it\n-----Original Message-----\nFrom: Ana\nolder",
                "Got it",
                None,
            ),
            (
                "gmail forward is kept",
                "FYI\n\n---------- Forwarded message ---------\nFrom: Ana <ana@acme.example>\nDate: Mon, Oct 5, 2026\nSubject: Role\nTo: <me@home.example>\n\nWe'd love to talk.",
                "FYI\n\n---------- Forwarded message ---------\nFrom: Ana <ana@acme.example>\nDate: Mon, Oct 5, 2026\nSubject: Role\nTo: <me@home.example>\n\nWe'd love to talk.",
                None,
            ),
            (
                "apple forward is kept",
                "See below\n\nBegin forwarded message:\n\nFrom: Ana\nSubject: Role\nDate: Mon\nTo: Me\n\nHello",
                "See below\n\nBegin forwarded message:\n\nFrom: Ana\nSubject: Role\nDate: Mon\nTo: Me\n\nHello",
                None,
            ),
            (
                "dash-dash signature",
                "Let's talk.\n\n-- \nAna Gomez\nSenior Technical Recruiter",
                "Let's talk.",
                Some("Ana Gomez\nSenior Technical Recruiter"),
            ),
            (
                "signature delimiter already trimmed",
                "Hi\n--\nAna",
                "Hi",
                Some("Ana"),
            ),
            (
                "sent from my iphone",
                "Sure thing\n\nSent from my iPhone",
                "Sure thing",
                Some("Sent from my iPhone"),
            ),
            (
                "signature before a forward",
                "FYI\n-- \nMe\n---------- Forwarded message ---------\nHello -- from Ana",
                "FYI\n---------- Forwarded message ---------\nHello -- from Ana",
                Some("Me"),
            ),
            (
                "blank result falls back to the unstripped text",
                "> only a quote\n> nothing else",
                "> only a quote\n> nothing else",
                None,
            ),
        ];
        for (name, input, text, signature) in cases {
            let got = super::strip(input);
            assert_eq!(got.text, *text, "{name}");
            assert_eq!(got.signature.as_deref(), *signature, "{name}");
        }
    }
}
