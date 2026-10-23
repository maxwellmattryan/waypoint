//! Cheap sender heuristics that decide whether a message is worth a model call. Every rule
//! that fires is recorded by its code, in a fixed order, so a verdict can be explained and
//! re-scored later (docs/email.md lists the rules and the decision order).

use std::collections::HashSet;

use super::mime::{Address, ParsedEmail};
use super::normalize::Normalized;
use super::{Ats, VerdictKind};

/// Stored in `emails.heuristics_version`. Bump it when a rule or the decision changes.
pub const HEURISTICS_VERSION: i32 = 1;

/// ATS sender domains.
pub(crate) const ATS_SENDERS: [(&str, Ats); 7] = [
    ("greenhouse.io", Ats::Greenhouse),
    ("greenhouse-mail.io", Ats::Greenhouse),
    ("lever.co", Ats::Lever),
    ("ashbyhq.com", Ats::Ashby),
    ("myworkday.com", Ats::Workday),
    ("workday.com", Ats::Workday),
    ("myworkdayjobs.com", Ats::Workday),
];
/// ATS job-board hosts that show up as links in the body.
const ATS_LINKS: [(&str, Ats); 5] = [
    ("boards.greenhouse.io", Ats::Greenhouse),
    ("job-boards.greenhouse.io", Ats::Greenhouse),
    ("jobs.lever.co", Ats::Lever),
    ("jobs.ashbyhq.com", Ats::Ashby),
    ("myworkdayjobs.com", Ats::Workday),
];
const RECRUITER_TITLES: [&str; 6] = [
    "recruiter",
    "recruiting",
    "talent acquisition",
    "talent partner",
    "sourcer",
    "hiring manager",
];
const RECRUITING_KEYWORDS: [&str; 14] = [
    "your application",
    "application for",
    "thank you for applying",
    "thanks for applying",
    "interview",
    "phone screen",
    "availability",
    "next steps",
    "offer",
    "unfortunately",
    "move forward",
    "moving forward",
    "the role",
    "the position",
];
const JOB_BOARDS: [&str; 6] = [
    "linkedin.com",
    "indeed.com",
    "glassdoor.com",
    "ziprecruiter.com",
    "monster.com",
    "wellfound.com",
];
const DIGEST_LOCAL_PARTS: [&str; 3] = ["alert", "jobs-noreply", "digest"];
const DIGEST_SUBJECTS: [&str; 5] = [
    "job alert",
    "jobs for you",
    "new jobs",
    "recommended jobs",
    "jobs you may be interested in",
];
const NEWSLETTER_WORDS: [&str; 3] = ["newsletter", "digest", "weekly roundup"];
const BULK_PRECEDENCE: [&str; 3] = ["bulk", "list", "junk"];

/// What the rules read: the sender fields and headers of the parsed message, and the
/// quote-stripped body **with** its signature (recruiter titles live in signatures).
#[derive(Debug, Clone, Default)]
pub struct HeuristicInput {
    pub from: Option<Address>,
    pub sender: Option<Address>,
    pub reply_to: Vec<Address>,
    pub return_path: Option<String>,
    pub subject: String,
    pub body: String,
    pub headers: Vec<(String, String)>,
}

impl HeuristicInput {
    pub fn new(parsed: &ParsedEmail, normalized: &Normalized) -> Self {
        let mut body = normalized.body_text.clone();
        if let Some(signature) = &normalized.signature {
            body.push('\n');
            body.push_str(signature);
        }
        Self {
            from: parsed.from.clone(),
            sender: parsed.sender.clone(),
            reply_to: parsed.reply_to.clone(),
            return_path: parsed.return_path.clone(),
            subject: parsed.subject.clone(),
            body,
            headers: parsed.headers.clone(),
        }
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.trim())
    }
}

/// Company keys from `applications.company`: lowercased, non-alphanumerics removed, and
/// only kept when at least 3 characters long.
#[derive(Debug, Clone, Default)]
pub struct KnownCompanies(HashSet<String>);

impl KnownCompanies {
    pub fn new<'a>(companies: impl IntoIterator<Item = &'a str>) -> Self {
        Self(
            companies
                .into_iter()
                .map(company_key)
                .filter(|key| key.chars().count() >= 3)
                .collect(),
        )
    }

    fn contains(&self, key: &str) -> bool {
        self.0.contains(key)
    }
}

/// `s` lowercased with every non-alphanumeric character removed.
pub(crate) fn company_key(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub verdict: VerdictKind,
    pub ats: Option<Ats>,
    /// Every rule code that fired, in rule order; `no_signal` last when nothing decided.
    pub reasons: Vec<String>,
}

/// Runs every rule, then decides (first match wins):
///
/// 1. `job_alert_digest` → skip.
/// 2. `ats_sender` or `known_company` → candidate.
/// 3. `auto_reply` → skip.
/// 4. `newsletter`/`list_unsubscribe`/`list_id`/`precedence_bulk` → candidate only if both
///    `recruiter_title` and `recruiting_keywords` fired, else skip.
/// 5. `ats_link`, `recruiter_title` or `recruiting_keywords` → candidate.
/// 6. Otherwise skip, with `no_signal` appended.
pub fn score(input: &HeuristicInput, known: &KnownCompanies) -> Verdict {
    let mut reasons = Vec::new();
    let mut ats = None;

    // 1. ats_sender
    let sender_domains = input
        .from
        .iter()
        .chain(&input.sender)
        .chain(&input.reply_to)
        .map(|a| a.address.as_str())
        .chain(input.return_path.as_deref())
        .filter_map(domain);
    let ats_sender = sender_domains
        .flat_map(|d| {
            ATS_SENDERS
                .iter()
                .filter(move |(s, _)| domain_matches(d, s))
        })
        .map(|(_, a)| *a)
        .next();
    if let Some(found) = ats_sender {
        reasons.push(format!("ats_sender:{found}"));
        ats = Some(found);
    }

    // 2. ats_link
    let body = input.body.to_lowercase();
    if let Some((_, found)) = ATS_LINKS.iter().find(|(host, _)| body.contains(host)) {
        reasons.push(format!("ats_link:{found}"));
        ats.get_or_insert(*found);
    }

    // 3. known_company
    let from_domain = input.from.as_ref().and_then(|a| domain(&a.address));
    let from_name = input.from.as_ref().and_then(|a| a.name.as_deref());
    let known_company = from_domain.is_some_and(|d| {
        let labels: Vec<&str> = d.split('.').collect();
        labels[..labels.len().saturating_sub(1)]
            .iter()
            .any(|label| known.contains(&company_key(label)))
    }) || from_name.is_some_and(|n| known.contains(&company_key(n)));
    if known_company {
        reasons.push("known_company".into());
    }

    // 4. recruiter_title
    let name_lower = from_name.unwrap_or("").to_lowercase();
    let recruiter_title = RECRUITER_TITLES
        .iter()
        .any(|t| name_lower.contains(t) || body.contains(t));
    if recruiter_title {
        reasons.push("recruiter_title".into());
    }

    // 5. recruiting_keywords
    let subject = input.subject.to_lowercase();
    let in_body = RECRUITING_KEYWORDS
        .iter()
        .filter(|k| body.contains(*k))
        .count();
    let recruiting_keywords =
        RECRUITING_KEYWORDS.iter().any(|k| subject.contains(k)) || in_body >= 2;
    if recruiting_keywords {
        reasons.push("recruiting_keywords".into());
    }

    // 6. job_alert_digest
    let from_local = input
        .from
        .as_ref()
        .and_then(|a| a.address.split_once('@'))
        .map(|(local, _)| local);
    let board_digest = from_domain.is_some_and(|d| JOB_BOARDS.iter().any(|b| domain_matches(d, b)))
        && from_local.is_some_and(|l| DIGEST_LOCAL_PARTS.iter().any(|p| l.contains(p)));
    let job_alert_digest = board_digest || DIGEST_SUBJECTS.iter().any(|s| subject.contains(s));
    if job_alert_digest {
        reasons.push("job_alert_digest".into());
    }

    // 7. newsletter
    let newsletter = NEWSLETTER_WORDS
        .iter()
        .any(|w| subject.contains(w) || name_lower.contains(w));
    if newsletter {
        reasons.push("newsletter".into());
    }

    // 8. list_unsubscribe, list_id, precedence_bulk
    let list_unsubscribe = input.header("List-Unsubscribe").is_some();
    if list_unsubscribe {
        reasons.push("list_unsubscribe".into());
    }
    let list_id = input.header("List-Id").is_some();
    if list_id {
        reasons.push("list_id".into());
    }
    let precedence_bulk = input
        .header("Precedence")
        .is_some_and(|p| BULK_PRECEDENCE.iter().any(|b| p.eq_ignore_ascii_case(b)));
    if precedence_bulk {
        reasons.push("precedence_bulk".into());
    }

    // 9. auto_reply
    let auto_reply = input
        .header("Auto-Submitted")
        .is_some_and(|v| !v.eq_ignore_ascii_case("no"))
        || input.header("X-Autoreply").is_some()
        || input.header("X-Autorespond").is_some();
    if auto_reply {
        reasons.push("auto_reply".into());
    }

    let bulk = newsletter || list_unsubscribe || list_id || precedence_bulk;
    let verdict = if job_alert_digest {
        VerdictKind::Skip
    } else if ats_sender.is_some() || known_company {
        VerdictKind::Candidate
    } else if auto_reply {
        VerdictKind::Skip
    } else if bulk {
        if recruiter_title && recruiting_keywords {
            VerdictKind::Candidate
        } else {
            VerdictKind::Skip
        }
    } else if reasons.is_empty() {
        reasons.push("no_signal".into());
        VerdictKind::Skip
    } else {
        // Only ats_link, recruiter_title or recruiting_keywords can be left here.
        VerdictKind::Candidate
    };
    Verdict {
        verdict,
        ats,
        reasons,
    }
}

/// The part of `address` after its last `@`, unless that is empty.
pub(crate) fn domain(address: &str) -> Option<&str> {
    address
        .rsplit_once('@')
        .map(|(_, d)| d)
        .filter(|d| !d.is_empty())
}

/// `domain` is `target` or a subdomain of it.
pub(crate) fn domain_matches(domain: &str, target: &str) -> bool {
    domain == target
        || domain
            .strip_suffix(target)
            .is_some_and(|rest| rest.ends_with('.'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::email::VerdictKind::{Candidate, Skip};

    fn addr(s: &str) -> Address {
        match s.split_once(" <") {
            Some((name, rest)) => Address {
                address: rest.trim_end_matches('>').into(),
                name: Some(name.into()),
            },
            None => Address {
                address: s.into(),
                name: None,
            },
        }
    }

    fn from(s: &str) -> HeuristicInput {
        HeuristicInput {
            from: Some(addr(s)),
            ..Default::default()
        }
    }

    fn with(mut input: HeuristicInput, f: impl FnOnce(&mut HeuristicInput)) -> HeuristicInput {
        f(&mut input);
        input
    }

    fn header(name: &str, value: &str) -> (String, String) {
        (name.into(), value.into())
    }

    /// (case, input, verdict, ats, reasons)
    type Case = (
        &'static str,
        HeuristicInput,
        VerdictKind,
        Option<Ats>,
        &'static [&'static str],
    );

    #[test]
    fn score() {
        let known = KnownCompanies::new(["Acme Corp", "Initech", "AB", "Q.A"]);
        let friend = || from("Pat <pat@friends.example>");
        let cases: Vec<Case> = vec![
            // Rule 1: ats_sender, through each sender field.
            (
                "ats sender in From (subdomain)",
                from("no-reply@us.greenhouse-mail.io"),
                Candidate,
                Some(Ats::Greenhouse),
                &["ats_sender:greenhouse"],
            ),
            (
                "ats sender in Sender",
                with(friend(), |i| i.sender = Some(addr("x@hire.lever.co"))),
                Candidate,
                Some(Ats::Lever),
                &["ats_sender:lever"],
            ),
            (
                "ats sender in Reply-To",
                with(friend(), |i| i.reply_to = vec![addr("r@ashbyhq.com")]),
                Candidate,
                Some(Ats::Ashby),
                &["ats_sender:ashby"],
            ),
            (
                "ats sender in Return-Path",
                with(friend(), |i| {
                    i.return_path = Some("bounce@myworkday.com".into())
                }),
                Candidate,
                Some(Ats::Workday),
                &["ats_sender:workday"],
            ),
            (
                "near miss: lookalike domain is no ats sender",
                from("x@notlever.co"),
                Skip,
                None,
                &["no_signal"],
            ),
            // Rule 2: ats_link.
            (
                "ats link in body",
                with(friend(), |i| {
                    i.body = "Apply at https://jobs.lever.co/acme/123".into()
                }),
                Candidate,
                Some(Ats::Lever),
                &["ats_link:lever"],
            ),
            (
                "near miss: greenhouse.io without a board host",
                with(friend(), |i| {
                    i.body = "We use greenhouse.io for hiring".into()
                }),
                Skip,
                None,
                &["no_signal"],
            ),
            (
                "ats sender sets ats before an ats link",
                with(from("x@greenhouse.io"), |i| {
                    i.body = "https://jobs.ashbyhq.com/acme".into()
                }),
                Candidate,
                Some(Ats::Greenhouse),
                &["ats_sender:greenhouse", "ats_link:ashby"],
            ),
            // Rule 3: known_company.
            (
                "known company by domain label",
                from("ana@careers.acmecorp.example"),
                Candidate,
                None,
                &["known_company"],
            ),
            (
                "known company by display name",
                from("Initech <hr@mail.example>"),
                Candidate,
                None,
                &["known_company"],
            ),
            (
                "near miss: short keys are dropped, TLD is ignored",
                from("x@ab.qa"),
                Skip,
                None,
                &["no_signal"],
            ),
            // Rule 4: recruiter_title.
            (
                "recruiter title in display name",
                from("Ana, Technical Recruiter <ana@mail.example>"),
                Candidate,
                None,
                &["recruiter_title"],
            ),
            (
                "recruiter title in signature",
                with(friend(), |i| {
                    i.body = "Hi\nAna\nTalent Acquisition Partner".into()
                }),
                Candidate,
                None,
                &["recruiter_title"],
            ),
            (
                "near miss: recruit alone is no title",
                with(friend(), |i| i.body = "We recruit volunteers".into()),
                Skip,
                None,
                &["no_signal"],
            ),
            // Rule 5: recruiting_keywords.
            (
                "keyword in subject",
                with(friend(), |i| i.subject = "Your application to Acme".into()),
                Candidate,
                None,
                &["recruiting_keywords"],
            ),
            (
                "two keywords in body",
                with(friend(), |i| {
                    i.body = "Share your availability for an interview".into()
                }),
                Candidate,
                None,
                &["recruiting_keywords"],
            ),
            (
                "near miss: one keyword in body",
                with(friend(), |i| i.body = "What is your availability?".into()),
                Skip,
                None,
                &["no_signal"],
            ),
            // Rule 6: job_alert_digest.
            (
                "job board alert sender",
                from("jobalerts-noreply@linkedin.com"),
                Skip,
                None,
                &["job_alert_digest"],
            ),
            (
                "digest subject",
                with(friend(), |i| i.subject = "30 new jobs for you".into()),
                Skip,
                None,
                &["job_alert_digest"],
            ),
            (
                "near miss: a person at a job board",
                from("ana@linkedin.com"),
                Skip,
                None,
                &["no_signal"],
            ),
            // Rule 7: newsletter.
            (
                "newsletter subject",
                with(friend(), |i| i.subject = "The October newsletter".into()),
                Skip,
                None,
                &["newsletter"],
            ),
            (
                "newsletter display name",
                from("Weekly Roundup <news@blog.example>"),
                Skip,
                None,
                &["newsletter"],
            ),
            // Rule 8: list headers and precedence.
            (
                "list-unsubscribe",
                with(friend(), |i| {
                    i.headers = vec![header("List-Unsubscribe", "<mailto:u@x.example>")]
                }),
                Skip,
                None,
                &["list_unsubscribe"],
            ),
            (
                "list-id",
                with(friend(), |i| {
                    i.headers = vec![header("List-Id", "<news.blog.example>")]
                }),
                Skip,
                None,
                &["list_id"],
            ),
            (
                "precedence bulk",
                with(friend(), |i| i.headers = vec![header("Precedence", "Bulk")]),
                Skip,
                None,
                &["precedence_bulk"],
            ),
            (
                "near miss: precedence first-class",
                with(friend(), |i| {
                    i.headers = vec![header("Precedence", "first-class")]
                }),
                Skip,
                None,
                &["no_signal"],
            ),
            // Rule 9: auto_reply.
            (
                "auto-submitted",
                with(friend(), |i| {
                    i.headers = vec![header("Auto-Submitted", "auto-replied")]
                }),
                Skip,
                None,
                &["auto_reply"],
            ),
            (
                "x-autoreply",
                with(friend(), |i| i.headers = vec![header("X-Autoreply", "yes")]),
                Skip,
                None,
                &["auto_reply"],
            ),
            (
                "near miss: auto-submitted no",
                with(friend(), |i| {
                    i.headers = vec![header("Auto-Submitted", "no")]
                }),
                Skip,
                None,
                &["no_signal"],
            ),
            // Precedence.
            (
                "a digest is skipped even with recruiting keywords",
                with(from("jobs-noreply@indeed.com"), |i| {
                    i.subject = "Interview tips and new jobs".into()
                }),
                Skip,
                None,
                &["recruiting_keywords", "job_alert_digest"],
            ),
            (
                "an ats sender beats list-unsubscribe",
                with(from("no-reply@ashbyhq.com"), |i| {
                    i.headers = vec![header("List-Unsubscribe", "<https://x.example>")]
                }),
                Candidate,
                Some(Ats::Ashby),
                &["ats_sender:ashby", "list_unsubscribe"],
            ),
            (
                "a known company beats an auto reply",
                with(from("hr@initech.example"), |i| {
                    i.headers = vec![header("Auto-Submitted", "auto-generated")]
                }),
                Candidate,
                None,
                &["known_company", "auto_reply"],
            ),
            (
                "an auto reply beats keywords",
                with(friend(), |i| {
                    i.subject = "Out of office: Re: your application".into();
                    i.headers = vec![header("Auto-Submitted", "auto-replied")];
                }),
                Skip,
                None,
                &["recruiting_keywords", "auto_reply"],
            ),
            (
                "bulk mail with a title and keywords is a candidate",
                with(from("Ana Recruiter <ana@mail.example>"), |i| {
                    i.subject = "An interview for the role".into();
                    i.headers = vec![header("List-Unsubscribe", "<mailto:u@x.example>")];
                }),
                Candidate,
                None,
                &["recruiter_title", "recruiting_keywords", "list_unsubscribe"],
            ),
            (
                "bulk mail with keywords but no title is skipped",
                with(friend(), |i| {
                    i.subject = "Interview prep newsletter".into();
                }),
                Skip,
                None,
                &["recruiting_keywords", "newsletter"],
            ),
            (
                "bulk mail with a title but no keywords is skipped",
                with(from("Recruiting Team <team@mail.example>"), |i| {
                    i.headers = vec![header("List-Id", "<team.mail.example>")];
                }),
                Skip,
                None,
                &["recruiter_title", "list_id"],
            ),
            (
                "no signal",
                with(friend(), |i| i.subject = "Dinner on Friday?".into()),
                Skip,
                None,
                &["no_signal"],
            ),
        ];
        for (name, input, verdict, ats, reasons) in cases {
            let got = super::score(&input, &known);
            assert_eq!(got.verdict, verdict, "{name}: {:?}", got.reasons);
            assert_eq!(got.ats, ats, "{name}");
            assert_eq!(got.reasons, reasons, "{name}");
        }
    }

    #[test]
    fn known_company_keys() {
        let known = KnownCompanies::new(["Acme, Inc.", "X", "  "]);
        assert!(known.contains("acmeinc"));
        assert_eq!(known.0.len(), 1);
    }
}
