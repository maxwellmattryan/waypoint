//! Plain text from a board's HTML description.

use scraper::{ElementRef, Html, Node};

use crate::capture::text::HIDDEN;

/// Elements that start and end a paragraph: a blank line before and after.
const PARAGRAPHS: [&str; 15] = [
    "p",
    "section",
    "article",
    "header",
    "footer",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "ul",
    "ol",
    "table",
    "blockquote",
];
/// Elements that start and end a line.
const LINES: [&str; 4] = ["div", "li", "tr", "pre"];

/// How many line breaks an element puts around itself: 2 for a paragraph, 1 for a line,
/// 0 for inline elements.
fn breaks(name: &str) -> usize {
    if PARAGRAPHS.contains(&name) {
        2
    } else if LINES.contains(&name) {
        1
    } else {
        0
    }
}

/// Builds the text, holding back breaks, spaces and the list bullet until the next
/// visible character so none of them end up leading, trailing or doubled.
#[derive(Default)]
struct Writer {
    out: String,
    /// Line breaks owed before the next character: at most 2, which is one blank line.
    breaks: usize,
    /// Whitespace was seen since the last character.
    space: bool,
    /// The next character starts a list item.
    bullet: bool,
}

impl Writer {
    fn brk(&mut self, n: usize) {
        self.breaks = self.breaks.max(n);
        self.space = false;
    }

    fn text(&mut self, text: &str) {
        for ch in text.chars() {
            if ch.is_whitespace() {
                self.space = true;
                continue;
            }
            if self.breaks > 0 || self.bullet {
                if !self.out.is_empty() {
                    for _ in 0..self.breaks {
                        self.out.push('\n');
                    }
                }
                if self.bullet {
                    self.out.push_str("- ");
                }
                self.breaks = 0;
                self.bullet = false;
            } else if self.space && !self.out.is_empty() {
                self.out.push(' ');
            }
            self.space = false;
            self.out.push(ch);
        }
    }
}

/// The text of an HTML fragment, keeping its paragraphs and lists. Text inside
/// [`HIDDEN`] elements is dropped. Paragraph-level blocks are separated by one blank line,
/// `div`, `tr`, `pre` and `br` break lines, and each `li` starts a line with `- `.
/// Whitespace inside a line collapses to one space; entities are decoded by the parser.
/// The tree is walked with an explicit stack, so deep nesting can't overflow it.
pub fn html_to_text(html: &str) -> String {
    let fragment = Html::parse_fragment(html);
    let mut writer = Writer::default();
    // Each frame: the element's break count, whether it's an `li`, and its children.
    let mut stack = vec![(0, false, fragment.root_element().children())];
    while let Some(top) = stack.last_mut() {
        let Some(child) = top.2.next() else {
            let (n, li, _) = stack.pop().expect("the stack has a top");
            if n > 0 {
                writer.brk(n);
            }
            if li {
                writer.bullet = false;
            }
            continue;
        };
        match child.value() {
            Node::Text(text) => writer.text(text),
            Node::Element(el) => {
                let name = el.name();
                if HIDDEN.contains(&name) {
                    continue;
                }
                if name == "br" {
                    writer.brk((writer.breaks + 1).min(2));
                    continue;
                }
                let Some(el) = ElementRef::wrap(child) else {
                    continue;
                };
                let n = breaks(name);
                if n > 0 {
                    writer.brk(n);
                }
                let li = name == "li";
                if li {
                    writer.bullet = true;
                }
                stack.push((n, li, el.children()));
            }
            _ => {}
        }
    }
    writer.out
}

/// Decodes one level of HTML escaping: Greenhouse sends `content` as `&lt;p&gt;…`, so
/// this gives back the markup for [`html_to_text`].
pub fn unescape(escaped: &str) -> String {
    Html::parse_fragment(escaped)
        .root_element()
        .text()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_to_text() {
        let cases = [
            (
                "<p>Intro &amp; more</p><ul><li>One</li><li> Two </li></ul><p>End</p>",
                "Intro & more\n\n- One\n- Two\n\nEnd",
            ),
            (
                "<script>track()</script><style>p { color: red }</style>\
                 <noscript>Enable JS</noscript><p>Kept</p>",
                "Kept",
            ),
            ("<p>  a \n\n\t b  </p>", "a b"),
            ("<p>a <strong>bold</strong> word</p>", "a bold word"),
            (
                "line one<br>line two<br/><br>line three",
                "line one\nline two\n\nline three",
            ),
            ("<div>a</div><div>b</div>", "a\nb"),
            (
                "<h2>About</h2><p></p><p> </p><p>Body</p>\n\n<p>More</p>",
                "About\n\nBody\n\nMore",
            ),
            (
                "<ol><li><strong>Ship</strong> things</li><li></li></ol>After",
                "- Ship things\n\nAfter",
            ),
            ("<blockquote>Quote</blockquote>tail", "Quote\n\ntail"),
            ("caf&eacute;&nbsp;&#8211; ok", "café – ok"),
            ("", ""),
            ("   ", ""),
        ];
        for (html, want) in cases {
            assert_eq!(super::html_to_text(html), want, "{html:?}");
        }

        let escaped =
            "&lt;p&gt;Fish &amp;amp; chips&lt;/p&gt;&lt;ul&gt;&lt;li&gt;Hot&lt;/li&gt;&lt;/ul&gt;";
        assert_eq!(
            unescape(escaped),
            "<p>Fish &amp; chips</p><ul><li>Hot</li></ul>"
        );
        assert_eq!(
            super::html_to_text(&unescape(escaped)),
            "Fish & chips\n\n- Hot"
        );

        // Deep enough that a recursive walk would need a large stack; the parser itself is
        // slow on deep nesting, so it stays at a few thousand.
        let deep = format!("{}deep{}", "<div>".repeat(4_000), "</div>".repeat(4_000));
        assert_eq!(super::html_to_text(&deep), "deep");
    }
}
