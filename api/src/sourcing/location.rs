//! A posting's location, normalized from the board's raw text and hints.

/// Where a posting is. Every part is `None` when the board gave nothing usable.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Location {
    pub remote: bool,
    pub city: Option<String>,
    pub region: Option<String>,
    /// ISO 3166-1 alpha-2 when the name is known, otherwise as the board wrote it.
    pub country: Option<String>,
}

/// A structured address (Ashby's `address.postalAddress`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Address<'a> {
    pub locality: Option<&'a str>,
    pub region: Option<&'a str>,
    pub country: Option<&'a str>,
}

/// What the board says besides the raw text.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Hints<'a> {
    /// The board flags the posting as remote (Lever `workplaceType`, Ashby `isRemote`).
    pub remote: bool,
    /// An ISO-2 country code (Lever `country`); it overrides the parsed country.
    pub country_code: Option<&'a str>,
    pub address: Option<Address<'a>>,
}

/// Country names and codes, lowercase, to ISO 3166-1 alpha-2. The two-letter codes that
/// are also US state codes (CA, DE, IN, IL) are left out, so `Indianapolis, IN` stays in
/// Indiana; those countries match by name.
const COUNTRIES: &[(&str, &str)] = &[
    ("us", "US"),
    ("usa", "US"),
    ("united states", "US"),
    ("united states of america", "US"),
    ("uk", "GB"),
    ("gb", "GB"),
    ("united kingdom", "GB"),
    ("great britain", "GB"),
    ("canada", "CA"),
    ("germany", "DE"),
    ("france", "FR"),
    ("fr", "FR"),
    ("ireland", "IE"),
    ("ie", "IE"),
    ("netherlands", "NL"),
    ("the netherlands", "NL"),
    ("nl", "NL"),
    ("spain", "ES"),
    ("es", "ES"),
    ("portugal", "PT"),
    ("pt", "PT"),
    ("poland", "PL"),
    ("pl", "PL"),
    ("sweden", "SE"),
    ("se", "SE"),
    ("switzerland", "CH"),
    ("ch", "CH"),
    ("israel", "IL"),
    ("india", "IN"),
    ("singapore", "SG"),
    ("sg", "SG"),
    ("japan", "JP"),
    ("jp", "JP"),
    ("australia", "AU"),
    ("au", "AU"),
    ("brazil", "BR"),
    ("br", "BR"),
    ("mexico", "MX"),
    ("mx", "MX"),
];

/// The 50 states and DC: code and name.
const US_STATES: &[(&str, &str)] = &[
    ("AL", "Alabama"),
    ("AK", "Alaska"),
    ("AZ", "Arizona"),
    ("AR", "Arkansas"),
    ("CA", "California"),
    ("CO", "Colorado"),
    ("CT", "Connecticut"),
    ("DE", "Delaware"),
    ("DC", "District of Columbia"),
    ("FL", "Florida"),
    ("GA", "Georgia"),
    ("HI", "Hawaii"),
    ("ID", "Idaho"),
    ("IL", "Illinois"),
    ("IN", "Indiana"),
    ("IA", "Iowa"),
    ("KS", "Kansas"),
    ("KY", "Kentucky"),
    ("LA", "Louisiana"),
    ("ME", "Maine"),
    ("MD", "Maryland"),
    ("MA", "Massachusetts"),
    ("MI", "Michigan"),
    ("MN", "Minnesota"),
    ("MS", "Mississippi"),
    ("MO", "Missouri"),
    ("MT", "Montana"),
    ("NE", "Nebraska"),
    ("NV", "Nevada"),
    ("NH", "New Hampshire"),
    ("NJ", "New Jersey"),
    ("NM", "New Mexico"),
    ("NY", "New York"),
    ("NC", "North Carolina"),
    ("ND", "North Dakota"),
    ("OH", "Ohio"),
    ("OK", "Oklahoma"),
    ("OR", "Oregon"),
    ("PA", "Pennsylvania"),
    ("RI", "Rhode Island"),
    ("SC", "South Carolina"),
    ("SD", "South Dakota"),
    ("TN", "Tennessee"),
    ("TX", "Texas"),
    ("UT", "Utah"),
    ("VT", "Vermont"),
    ("VA", "Virginia"),
    ("WA", "Washington"),
    ("WV", "West Virginia"),
    ("WI", "Wisconsin"),
    ("WY", "Wyoming"),
];

/// Characters trimmed from the ends of a location part once `remote` is gone.
const SEPARATORS: &[char] = &['-', '–', '—', '/', ':', '(', ')', '[', ']', ','];

/// The ISO-2 code for a country in [`COUNTRIES`].
fn country_code(name: &str) -> Option<&'static str> {
    let name = name.trim().to_lowercase();
    COUNTRIES
        .iter()
        .find(|(known, _)| *known == name)
        .map(|(_, code)| *code)
}

/// The two-letter code for a US state given by code or name.
fn us_state(part: &str) -> Option<&'static str> {
    let part = part.trim();
    US_STATES
        .iter()
        .find(|(code, name)| code.eq_ignore_ascii_case(part) || name.eq_ignore_ascii_case(part))
        .map(|(code, _)| *code)
}

/// A country through the table, or as written (trimmed) when it isn't in it.
fn country(name: &str) -> Option<String> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    Some(country_code(name).map_or_else(|| name.to_string(), str::to_string))
}

fn present(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

/// Byte ranges of each case-insensitive whole word `remote` in `text`.
fn remote_words(text: &str) -> Vec<(usize, usize)> {
    let lower = text.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(at) = lower[from..].find("remote") {
        let start = from + at;
        let end = start + "remote".len();
        let before = start == 0 || !bytes[start - 1].is_ascii_alphanumeric();
        let after = end == bytes.len() || !bytes[end].is_ascii_alphanumeric();
        if before && after {
            found.push((start, end));
        }
        from = end;
    }
    found
}

/// The text with every `remote` word removed, along with an empty `()` it leaves.
fn without_remote(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (start, end) in remote_words(text) {
        out.push_str(&text[last..start]);
        out.push(' ');
        last = end;
    }
    out.push_str(&text[last..]);
    out.replace("()", " ").replace("( )", " ")
}

/// Normalizes a posting's location. `remote` is the board's flag or the word `remote` in
/// the raw text. A structured address with any part wins over the text. Otherwise the
/// text's first `;`/`|` segment is read without its `remote` tokens and split on commas:
/// one part is a country (when the table knows it) or a city; two parts are a city and a
/// US state (which also sets `US`), a country or a region; three or more are city, region
/// and country. A Lever country code overrides the parsed country.
pub fn normalize(raw: Option<&str>, hints: Hints<'_>) -> Location {
    let raw = raw.unwrap_or("");
    let mut location = Location {
        remote: hints.remote || !remote_words(raw).is_empty(),
        ..Location::default()
    };

    let address = hints.address.filter(|a| {
        [a.locality, a.region, a.country]
            .iter()
            .any(|part| part.is_some_and(|p| !p.trim().is_empty()))
    });
    if let Some(address) = address {
        location.city = present(address.locality);
        location.region = present(address.region);
        location.country = address.country.and_then(country);
    } else {
        let segment = raw.split([';', '|']).next().unwrap_or("");
        let cleaned = without_remote(segment);
        let parts: Vec<&str> = cleaned
            .split(',')
            .map(|part| part.trim().trim_matches(SEPARATORS).trim())
            .filter(|part| !part.is_empty())
            .collect();
        match parts.as_slice() {
            [] => {}
            [only] => match country_code(only) {
                Some(code) => location.country = Some(code.into()),
                None => location.city = Some((*only).into()),
            },
            [city, second] => {
                location.city = Some((*city).into());
                if let Some(state) = us_state(second) {
                    location.region = Some(state.into());
                    location.country = Some("US".into());
                } else if let Some(code) = country_code(second) {
                    location.country = Some(code.into());
                } else {
                    location.region = Some((*second).into());
                }
            }
            [city, region, .., last] => {
                location.city = Some((*city).into());
                location.region = Some((*region).into());
                location.country = country(last);
            }
        }
    }

    if let Some(code) = hints
        .country_code
        .map(str::trim)
        .filter(|code| !code.is_empty())
    {
        location.country = Some(code.to_uppercase());
    }
    location
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(
        remote: bool,
        city: Option<&str>,
        region: Option<&str>,
        country: Option<&str>,
    ) -> Location {
        Location {
            remote,
            city: city.map(Into::into),
            region: region.map(Into::into),
            country: country.map(Into::into),
        }
    }

    #[test]
    fn normalizes_locations() {
        let none = Hints::default();
        let flagged = Hints {
            remote: true,
            ..Hints::default()
        };
        let lever_gb = Hints {
            country_code: Some("gb"),
            ..Hints::default()
        };
        let ashby_address = Hints {
            remote: false,
            country_code: None,
            address: Some(Address {
                locality: Some("San Francisco"),
                region: Some("California"),
                country: Some("United States"),
            }),
        };
        let blank_address = Hints {
            address: Some(Address {
                locality: Some(" "),
                region: None,
                country: None,
            }),
            ..Hints::default()
        };
        let cases = [
            ("Remote", none, at(true, None, None, None)),
            ("remote", none, at(true, None, None, None)),
            ("Remote - US", none, at(true, None, None, Some("US"))),
            ("US - Remote", none, at(true, None, None, Some("US"))),
            ("Remote, Canada", none, at(true, None, None, Some("CA"))),
            (
                "New York (Remote)",
                none,
                at(true, Some("New York"), None, None),
            ),
            (
                "New York, NY",
                none,
                at(false, Some("New York"), Some("NY"), Some("US")),
            ),
            (
                "San Francisco, California",
                none,
                at(false, Some("San Francisco"), Some("CA"), Some("US")),
            ),
            (
                "London, United Kingdom",
                none,
                at(false, Some("London"), None, Some("GB")),
            ),
            ("Berlin", none, at(false, Some("Berlin"), None, None)),
            ("Germany", none, at(false, None, None, Some("DE"))),
            (
                "Toronto, ON, Canada",
                none,
                at(false, Some("Toronto"), Some("ON"), Some("CA")),
            ),
            (
                "Austin, TX; Remote",
                none,
                at(true, Some("Austin"), Some("TX"), Some("US")),
            ),
            (
                "Paris, Île-de-France | London",
                none,
                at(false, Some("Paris"), Some("Île-de-France"), None),
            ),
            (
                "Lyon, Rhône, Atlantis",
                none,
                at(false, Some("Lyon"), Some("Rhône"), Some("Atlantis")),
            ),
            (
                "Remoteville",
                none,
                at(false, Some("Remoteville"), None, None),
            ),
            ("", none, at(false, None, None, None)),
            ("", flagged, at(true, None, None, None)),
            (
                "Anywhere in the US",
                ashby_address,
                at(false, Some("San Francisco"), Some("California"), Some("US")),
            ),
            (
                "Berlin",
                blank_address,
                at(false, Some("Berlin"), None, None),
            ),
            (
                "London",
                lever_gb,
                at(false, Some("London"), None, Some("GB")),
            ),
            (
                "New York, NY",
                lever_gb,
                at(false, Some("New York"), Some("NY"), Some("GB")),
            ),
        ];
        for (raw, hints, want) in cases {
            assert_eq!(normalize(Some(raw), hints), want, "{raw:?} {hints:?}");
        }
        assert_eq!(normalize(None, none), at(false, None, None, None));
    }
}
