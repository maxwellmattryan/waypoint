//! Normalisation shared by every tier: entity decoding, whitespace, blanks, lengths, and
//! the compensation and slug formats.

use scraper::Html;

use crate::applications::MAX_SHORT;

/// The four fields a tier extracts. The URL comes from the request, not the page.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Extracted {
    pub company: Option<String>,
    pub title: Option<String>,
    pub location: Option<String>,
    pub comp: Option<String>,
}

/// Decodes HTML entities, collapses whitespace to single spaces and trims. A blank value
/// becomes `None`, and a long one is cut to [`MAX_SHORT`] characters so the confirm form
/// never starts out invalid.
pub fn clean(value: Option<&str>) -> Option<String> {
    let value = value?;
    let decoded = if value.contains('&') {
        // Escaping `<` first means the parser only decodes entities and never sees a tag.
        let fragment = Html::parse_fragment(&value.replace('<', "&lt;"));
        fragment.root_element().text().collect::<String>()
    } else {
        value.to_string()
    };
    let collapsed = decoded.split_whitespace().collect::<Vec<_>>().join(" ");
    let cut: String = collapsed.chars().take(MAX_SHORT).collect();
    let cut = cut.trim_end();
    (!cut.is_empty()).then(|| cut.to_string())
}

/// The period a pay figure covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Hour,
    Day,
    Week,
    Month,
    Year,
}

impl Unit {
    /// A schema.org `unitText`: `HOUR`, `DAY`, `WEEK`, `MONTH` or `YEAR`.
    pub fn from_schema(text: &str) -> Option<Self> {
        match text.trim().to_ascii_uppercase().as_str() {
            "HOUR" => Some(Self::Hour),
            "DAY" => Some(Self::Day),
            "WEEK" => Some(Self::Week),
            "MONTH" => Some(Self::Month),
            "YEAR" => Some(Self::Year),
            _ => None,
        }
    }

    /// A Lever `salaryRange.interval`, such as `per-year-salary`.
    pub fn from_lever(interval: &str) -> Option<Self> {
        match interval.trim() {
            "per-hour-wage" => Some(Self::Hour),
            "per-day-wage" => Some(Self::Day),
            "per-week-salary" => Some(Self::Week),
            "per-month-salary" => Some(Self::Month),
            "per-year-salary" => Some(Self::Year),
            _ => None,
        }
    }

    fn word(self) -> &'static str {
        match self {
            Self::Hour => "hour",
            Self::Day => "day",
            Self::Week => "week",
            Self::Month => "month",
            Self::Year => "year",
        }
    }
}

/// Formats pay as `{currency} {min}–{max} per {unit}`, leaving out the parts that are
/// missing. Whole numbers get thousands separators, fractional ones two decimals. No
/// number at all gives `None`.
pub fn format_comp(
    currency: Option<&str>,
    min: Option<f64>,
    max: Option<f64>,
    unit: Option<Unit>,
) -> Option<String> {
    let finite = |n: Option<f64>| n.filter(|n| n.is_finite());
    let amount = match (finite(min), finite(max)) {
        (Some(min), Some(max)) if min != max => format!("{}–{}", number(min), number(max)),
        (Some(n), _) | (None, Some(n)) => number(n),
        (None, None) => return None,
    };
    let mut out = String::new();
    if let Some(currency) = currency.map(str::trim).filter(|c| !c.is_empty()) {
        out.push_str(currency);
        out.push(' ');
    }
    out.push_str(&amount);
    if let Some(unit) = unit {
        out.push_str(" per ");
        out.push_str(unit.word());
    }
    clean(Some(&out))
}

/// `180000` → `180,000`; `52.5` → `52.50`.
fn number(n: f64) -> String {
    let fixed = if n.fract() == 0.0 {
        format!("{n:.0}")
    } else {
        format!("{n:.2}")
    };
    let (sign, fixed) = match fixed.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", fixed.as_str()),
    };
    let (whole, fraction) = match fixed.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (fixed, None),
    };
    let mut grouped = String::new();
    for (i, digit) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    match fraction {
        Some(fraction) => format!("{sign}{grouped}.{fraction}"),
        None => format!("{sign}{grouped}"),
    }
}

/// `acme-corp` → `Acme Corp`: split on `-` and `_`, uppercase each word's first letter.
pub fn humanise(slug: &str) -> String {
    slug.split(['-', '_'])
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_uppercase().chain(chars).collect::<String>()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Currency, min, max, unit, and the expected text.
    type CompCase = (
        Option<&'static str>,
        Option<f64>,
        Option<f64>,
        Option<Unit>,
        Option<&'static str>,
    );

    #[test]
    fn formats_comp() {
        let cases: [CompCase; 9] = [
            (
                Some("USD"),
                Some(180_000.0),
                Some(210_000.0),
                Some(Unit::Year),
                Some("USD 180,000–210,000 per year"),
            ),
            (
                Some("EUR"),
                Some(52.5),
                None,
                Some(Unit::Hour),
                Some("EUR 52.50 per hour"),
            ),
            (None, None, Some(90_000.0), None, Some("90,000")),
            (
                Some("GBP"),
                Some(1_000.0),
                Some(1_000.0),
                None,
                Some("GBP 1,000"),
            ),
            (
                Some(" "),
                Some(999.0),
                None,
                Some(Unit::Month),
                Some("999 per month"),
            ),
            (None, Some(1_234_567.25), None, None, Some("1,234,567.25")),
            (Some("USD"), None, None, Some(Unit::Year), None),
            (None, Some(f64::NAN), None, None, None),
            (
                Some("CAD"),
                Some(40.0),
                Some(45.75),
                Some(Unit::Day),
                Some("CAD 40–45.75 per day"),
            ),
        ];
        for (currency, min, max, unit, want) in cases {
            assert_eq!(
                format_comp(currency, min, max, unit).as_deref(),
                want,
                "{currency:?} {min:?} {max:?} {unit:?}"
            );
        }
        assert_eq!(Unit::from_schema("year"), Some(Unit::Year));
        assert_eq!(Unit::from_schema("FORTNIGHT"), None);
        assert_eq!(Unit::from_lever("per-hour-wage"), Some(Unit::Hour));
        assert_eq!(Unit::from_lever("one-time"), None);
    }

    #[test]
    fn clean_normalises() {
        let long = "x".repeat(250);
        let cases: [(Option<&str>, Option<String>); 7] = [
            (None, None),
            (Some("   "), None),
            (Some("  Acme \n\t Corp "), Some("Acme Corp".into())),
            (
                Some("R&amp;D &lt;Lab&gt; &#8211; Café"),
                Some("R&D <Lab> – Café".into()),
            ),
            (Some("a < b & c"), Some("a < b & c".into())),
            (Some(long.as_str()), Some("x".repeat(200))),
            (Some("&nbsp;"), None),
        ];
        for (value, want) in cases {
            assert_eq!(clean(value), want, "{value:?}");
        }
    }

    #[test]
    fn humanises_slugs() {
        assert_eq!(humanise("acme-corp"), "Acme Corp");
        assert_eq!(humanise("acme_labs-inc"), "Acme Labs Inc");
        assert_eq!(humanise("acme"), "Acme");
        assert_eq!(humanise("--x--"), "X");
    }
}
