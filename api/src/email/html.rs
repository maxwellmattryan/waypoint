//! An email's HTML part as plain text. Unlike [`crate::capture::text`], this is
//! block-aware (paragraphs, list items and table rows keep their own lines), keeps link
//! targets, and drops quoted replies before converting.

use scraper::{ElementRef, Html, Node, Selector};

/// Elements whose content a reader never sees.
const HIDDEN: [&str; 6] = ["script", "style", "head", "title", "noscript", "template"];
/// Elements that start and end on their own line.
const LINE_BLOCKS: [&str; 16] = [
    "div", "li", "tr", "table", "tbody", "thead", "tfoot", "ul", "ol", "section", "article",
    "header", "footer", "center", "hr", "pre",
];
/// Elements set off by a blank line.
const PARAGRAPHS: [&str; 7] = ["p", "h1", "h2", "h3", "h4", "h5", "h6"];
/// Quoted-reply containers removed before converting (Gmail, Apple Mail, Outlook).
const QUOTES: &str = "div.gmail_quote, blockquote, #divRplyFwdMsg";
/// Text that marks a quote container as a forward, which is kept.
const FORWARD_MARKERS: [&str; 2] = ["Forwarded message", "Begin forwarded message"];

/// Converts an email's HTML to text: drops `script`/`style`/`head`, puts block elements
/// on their own lines, prefixes list items with `- `, writes a link whose text differs
/// from its http(s) href as `text <href>`, turns `&nbsp;` into a space, collapses
/// whitespace and keeps at most one blank line in a row. Quoted replies
/// (`div.gmail_quote`, `blockquote`, `#divRplyFwdMsg`, and `#appendonsend` with everything
/// after it) are removed first; a Gmail quote that holds a forwarded message is kept.
pub fn to_text(html: &str) -> String {
    let mut doc = Html::parse_document(html);
    remove_quotes(&mut doc);

    let mut out = String::new();
    walk(doc.root_element(), &mut out);
    tidy(&out)
}

fn remove_quotes(doc: &mut Html) {
    let quotes = Selector::parse(QUOTES).expect("valid selector");
    let mut doomed: Vec<_> = doc
        .select(&quotes)
        .filter(|el| {
            let text: String = el.text().collect();
            !FORWARD_MARKERS.iter().any(|m| text.contains(m))
        })
        .map(|el| el.id())
        .collect();

    // Outlook's `#appendonsend` precedes the quoted original: drop it and everything after
    // it, up to the body.
    let marker = Selector::parse("#appendonsend").expect("valid selector");
    if let Some(start) = doc.select(&marker).next() {
        let mut node = Some(*start);
        while let Some(current) = node {
            if ElementRef::wrap(current).is_some_and(|el| el.value().name() == "body") {
                break;
            }
            doomed.push(current.id());
            let mut sibling = current.next_sibling();
            while let Some(s) = sibling {
                doomed.push(s.id());
                sibling = s.next_sibling();
            }
            node = current.parent();
        }
        // The ancestors themselves stay; only the marker and what follows go.
        let ancestors: Vec<_> = start.ancestors().map(|a| a.id()).collect();
        doomed.retain(|id| !ancestors.contains(id));
    }

    for id in doomed {
        if let Some(mut node) = doc.tree.get_mut(id) {
            node.detach();
        }
    }
}

fn walk(el: ElementRef<'_>, out: &mut String) {
    for child in el.children() {
        match child.value() {
            // Source line breaks are spaces in HTML; `&nbsp;` (U+00A0) is too.
            Node::Text(text) => out.extend(
                text.chars()
                    .map(|c| if c.is_whitespace() { ' ' } else { c }),
            ),
            Node::Element(_) => {
                let Some(child) = ElementRef::wrap(child) else {
                    continue;
                };
                element(child, out);
            }
            _ => {}
        }
    }
}

fn element(el: ElementRef<'_>, out: &mut String) {
    let name = el.value().name();
    if HIDDEN.contains(&name) {
        return;
    }
    match name {
        "br" => out.push('\n'),
        "li" => {
            breaks(out, 1);
            out.push_str("- ");
            walk(el, out);
            breaks(out, 1);
        }
        "td" | "th" => {
            walk(el, out);
            out.push(' ');
        }
        "a" => {
            walk(el, out);
            let href = el.value().attr("href").map(str::trim).unwrap_or("");
            let text = collapse(&el.text().collect::<String>().replace('\u{a0}', " "));
            let web = href.starts_with("http://") || href.starts_with("https://");
            if web && !text.is_empty() && text != href {
                out.push_str(" <");
                out.push_str(href);
                out.push('>');
            }
        }
        _ if PARAGRAPHS.contains(&name) => {
            breaks(out, 2);
            walk(el, out);
            breaks(out, 2);
        }
        _ if LINE_BLOCKS.contains(&name) => {
            breaks(out, 1);
            walk(el, out);
            breaks(out, 1);
        }
        _ => walk(el, out),
    }
}

/// Ends `out` with at least `n` line breaks (none at the very start), ignoring trailing
/// spaces and tabs, so nested and adjacent blocks don't stack up blank lines.
fn breaks(out: &mut String, n: usize) {
    let kept = out.trim_end_matches([' ', '\t']).len();
    out.truncate(kept);
    if out.trim().is_empty() {
        return;
    }
    let have = out.len() - out.trim_end_matches('\n').len();
    for _ in have..n {
        out.push('\n');
    }
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Collapses whitespace on each line, keeps at most one blank line in a row and trims
/// blank lines at both ends.
fn tidy(text: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for line in text.lines().map(collapse) {
        if line.is_empty() && lines.last().is_none_or(|l| l.is_empty()) {
            continue;
        }
        lines.push(line);
    }
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    #[test]
    fn to_text() {
        let cases = [
            (
                "drops script, style and head",
                "<html><head><title>T</title><style>p{color:red}</style></head>\
                 <body><script>x()</script><p>Kept</p></body></html>",
                "Kept",
            ),
            (
                "block elements get their own lines",
                "<div>One</div><div>Two<br>Three</div><table><tr><td>A</td><td>B</td></tr>\
                 <tr><td>C</td></tr></table>",
                "One\nTwo\nThree\nA B\nC",
            ),
            (
                "paragraphs and headings are separated by one blank line",
                "<h1>Title</h1><p>First</p><p>Second</p>",
                "Title\n\nFirst\n\nSecond",
            ),
            (
                "list items are prefixed",
                "<ul><li>Alpha</li><li>Beta <b>bold</b></li></ul>",
                "- Alpha\n- Beta bold",
            ),
            (
                "links keep a differing href",
                "<p><a href=\"https://jobs.lever.co/acme/1\">Pick a time</a> or \
                 <a href=\"https://acme.example\">https://acme.example</a> or \
                 <a href=\"mailto:a@acme.example\">mail</a></p>",
                "Pick a time <https://jobs.lever.co/acme/1> or https://acme.example or mail",
            ),
            (
                "nbsp is a space and whitespace collapses",
                "<p>a&nbsp;&nbsp;b \n\t c</p>",
                "a b c",
            ),
            (
                "at most one blank line in a row",
                "<p>a</p><p></p><p>&nbsp;</p><br><br><br><p>b</p>",
                "a\n\nb",
            ),
            (
                "gmail quote and blockquote are removed",
                "<div>Thanks!</div><div class=\"gmail_quote\">On Mon wrote:\
                 <blockquote>old</blockquote></div><blockquote>older</blockquote>",
                "Thanks!",
            ),
            (
                "a gmail forward is kept",
                "<div>FYI</div><div class=\"gmail_quote\">---------- Forwarded message \
                 ---------<br>Hello from Ana</div>",
                "FYI\n---------- Forwarded message ---------\nHello from Ana",
            ),
            (
                "outlook reply header and what follows are removed",
                "<div>Sounds good.</div><div id=\"appendonsend\"></div><hr>\
                 <div id=\"divRplyFwdMsg\">From: Ana</div><div>Original text</div>",
                "Sounds good.",
            ),
        ];
        for (name, html, want) in cases {
            assert_eq!(super::to_text(html), want, "{name}");
        }
    }
}
