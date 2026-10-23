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
