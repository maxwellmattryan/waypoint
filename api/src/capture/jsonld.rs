//! Tier 1: a schema.org `JobPosting` in the page's JSON-LD.

use scraper::{Html, Selector};
use serde_json::Value;

use super::fields::{Extracted, Unit, clean, format_comp};

/// The first `JobPosting` with a non-blank title and company in any
/// `<script type="application/ld+json">`, searching top-level objects, arrays and `@graph`
/// arrays. Scripts whose JSON doesn't parse are skipped.
pub fn extract(html: &str) -> Option<Extracted> {
    let doc = Html::parse_document(html);
    let scripts = Selector::parse("script[type]").expect("valid selector");
    doc.select(&scripts)
        .filter(|script| {
            script.value().attr("type").is_some_and(|ty| {
                ty.trim()
                    .to_ascii_lowercase()
                    .starts_with("application/ld+json")
            })
        })
        .filter_map(|script| serde_json::from_str::<Value>(&script.text().collect::<String>()).ok())
        .find_map(|value| {
            let mut postings = Vec::new();
            collect_postings(&value, &mut postings);
            postings.into_iter().find_map(map_posting)
        })
}

/// Pushes every `JobPosting` node in `value`, its arrays and its `@graph`, in order.
fn collect_postings<'v>(value: &'v Value, out: &mut Vec<&'v Value>) {
    match value {
        Value::Array(items) => items.iter().for_each(|item| collect_postings(item, out)),
        Value::Object(map) => {
            if is_job_posting(map.get("@type")) {
                out.push(value);
            }
            if let Some(graph) = map.get("@graph") {
                collect_postings(graph, out);
            }
        }
        _ => {}
    }
}

fn is_job_posting(ty: Option<&Value>) -> bool {
    match ty {
        Some(Value::String(ty)) => ty == "JobPosting",
        Some(Value::Array(types)) => types.iter().any(|ty| ty == "JobPosting"),
        _ => false,
    }
}

fn map_posting(node: &Value) -> Option<Extracted> {
    let title = clean(node.get("title").and_then(Value::as_str))?;
    let company = clean(organization(node.get("hiringOrganization")?).as_deref())?;
    Some(Extracted {
        company: Some(company),
        title: Some(title),
        location: location(node),
        comp: comp(node),
    })
}

/// A string, an `Organization` with a `name`, or the first of an array of either.
fn organization(value: &Value) -> Option<String> {
    match value {
        Value::String(name) => Some(name.clone()),
        Value::Object(map) => map.get("name").and_then(Value::as_str).map(str::to_string),
        Value::Array(items) => items.iter().find_map(organization),
        _ => None,
    }
}

/// Every `jobLocation` as `locality, region, country`, joined with `; `, plus `Remote`
/// when `jobLocationType` is `TELECOMMUTE`.
fn location(node: &Value) -> Option<String> {
    let mut places: Vec<String> = match node.get("jobLocation") {
        Some(Value::Array(items)) => items.iter().filter_map(place).collect(),
        Some(item) => place(item).into_iter().collect(),
        None => Vec::new(),
    };
    let remote = match node.get("jobLocationType") {
        Some(Value::String(ty)) => ty.eq_ignore_ascii_case("TELECOMMUTE"),
        Some(Value::Array(types)) => types
            .iter()
            .filter_map(Value::as_str)
            .any(|ty| ty.eq_ignore_ascii_case("TELECOMMUTE")),
        _ => false,
    };
    if remote {
        places.push("Remote".into());
    }
    clean(Some(&places.join("; ")))
}

fn place(value: &Value) -> Option<String> {
    let address = value.get("address").unwrap_or(value);
    if let Some(text) = address.as_str() {
        return clean(Some(text));
    }
    let part = |key: &str| match address.get(key) {
        Some(Value::String(text)) => clean(Some(text)),
        Some(Value::Object(map)) => clean(map.get("name").and_then(Value::as_str)),
        _ => None,
    };
    let parts: Vec<String> = ["addressLocality", "addressRegion", "addressCountry"]
        .into_iter()
        .filter_map(part)
        .collect();
    (!parts.is_empty()).then(|| parts.join(", "))
}

/// `baseSalary` as a `MonetaryAmount` whose `value` is a `QuantitativeValue` (min/max or a
/// single value) or a bare number.
fn comp(node: &Value) -> Option<String> {
    let salary = match node.get("baseSalary")? {
        Value::Array(items) => items.first()?,
        salary => salary,
    };
    let currency = salary
        .get("currency")
        .or_else(|| node.get("salaryCurrency"))
        .and_then(Value::as_str);
    let value = salary.get("value").unwrap_or(salary);
    let (min, max, unit) = if value.is_object() {
        let single = number(value.get("value"));
        (
            number(value.get("minValue")).or(single),
            number(value.get("maxValue")),
            value.get("unitText").and_then(Value::as_str),
        )
    } else {
        (number(Some(value)), None, None)
    };
    let unit = unit
        .or_else(|| salary.get("unitText").and_then(Value::as_str))
        .and_then(Unit::from_schema);
    format_comp(currency, min, max, unit)
}

/// A JSON number, or a string holding one (commas allowed).
pub(crate) fn number(value: Option<&Value>) -> Option<f64> {
    match value? {
        Value::Number(n) => n.as_f64(),
        Value::String(text) => text.trim().replace(',', "").parse().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(company: &str, title: &str, location: Option<&str>, comp: Option<&str>) -> Extracted {
        Extracted {
            company: Some(company.into()),
            title: Some(title.into()),
            location: location.map(Into::into),
            comp: comp.map(Into::into),
        }
    }

    #[test]
    fn extracts_job_postings() {
        let cases = [
            (
                "basic",
                include_str!("../../tests/fixtures/capture/jsonld_basic.html"),
                Some(fields(
                    "Acme Corp",
                    "Senior Engineer",
                    Some("San Francisco, CA, US"),
                    Some("USD 180,000–210,000 per year"),
                )),
            ),
            (
                "graph, array @type, string org, location array, telecommute, string value, entities",
                include_str!("../../tests/fixtures/capture/jsonld_graph.html"),
                Some(fields(
                    "Globex & Sons",
                    "Data & ML Engineer",
                    Some("Berlin, Germany; Paris, FR; Remote"),
                    Some("EUR 52.50 per hour"),
                )),
            ),
            (
                "top-level array, skips an untitled posting, bare number salary",
                include_str!("../../tests/fixtures/capture/jsonld_array.html"),
                Some(fields("Initech", "Support Lead", None, Some("USD 90,000"))),
            ),
            (
                "malformed",
                include_str!("../../tests/fixtures/capture/jsonld_malformed.html"),
                None,
            ),
            (
                "no job posting",
                include_str!("../../tests/fixtures/capture/jsonld_none.html"),
                None,
            ),
            (
                "no title, no company",
                include_str!("../../tests/fixtures/capture/jsonld_incomplete.html"),
                None,
            ),
        ];
        for (name, html, want) in cases {
            assert_eq!(extract(html), want, "{name}");
        }
    }
}
