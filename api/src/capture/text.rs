//! A page's visible text, which the LLM tier reads.

use scraper::{ElementRef, Html, Node, Selector};

/// Elements whose text a reader never sees.
pub(crate) const HIDDEN: [&str; 5] = ["script", "style", "noscript", "template", "svg"];

/// The page's `<title>`, its `meta name=description` content, then every text node in
/// `<body>` outside [`HIDDEN`] elements. Each piece has its whitespace collapsed, empty
/// ones are dropped and the rest are joined with `\n`. At most `max_chars` characters.
pub fn visible_text(html: &str, max_chars: usize) -> String {
    let doc = Html::parse_document(html);
    let mut pieces = Vec::new();

    let title = Selector::parse("head title").expect("valid selector");
    if let Some(title) = doc.select(&title).next() {
        pieces.push(title.text().collect::<String>());
    }
    let description = Selector::parse("meta[name]").expect("valid selector");
    for meta in doc.select(&description) {
        let el = meta.value();
        if el
            .attr("name")
            .is_some_and(|name| name.trim().eq_ignore_ascii_case("description"))
        {
            if let Some(content) = el.attr("content") {
                pieces.push(content.to_string());
            }
            break;
        }
    }

    let body = Selector::parse("body").expect("valid selector");
    if let Some(body) = doc.select(&body).next() {
        for node in body.descendants() {
            let Node::Text(text) = node.value() else {
                continue;
            };
            let hidden = node.ancestors().any(|ancestor| {
                ElementRef::wrap(ancestor).is_some_and(|el| HIDDEN.contains(&el.value().name()))
            });
            if !hidden {
                pieces.push(text.to_string());
            }
        }
    }

    let joined = pieces
        .iter()
        .map(|piece| piece.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|piece| !piece.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    joined.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn visible_text() {
        let cases = [
            (
                "<html><head><title> Senior  Engineer </title>\
                 <meta name=\"description\" content=\"Join   Acme\"></head>\
                 <body><h1>Role</h1>\n<p>Build &amp; ship</p></body></html>",
                "Senior Engineer\nJoin Acme\nRole\nBuild & ship",
            ),
            (
                "<body><script>var x = 1;</script><style>p{}</style>\
                 <noscript>Enable JS</noscript><template><p>tpl</p></template>\
                 <svg><title>icon</title><text>svg text</text></svg><p>Kept</p></body>",
                "Kept",
            ),
            ("<body><script>only()</script></body>", ""),
            ("<p>  a \n\n b  </p><div>   </div><p>c</p>", "a b\nc"),
        ];
        for (html, want) in cases {
            assert_eq!(super::visible_text(html, 30_000), want, "{html}");
        }
        let long = format!("<p>{}</p>", "é".repeat(40_000));
        assert_eq!(super::visible_text(&long, 30_000).chars().count(), 30_000);
    }
}
