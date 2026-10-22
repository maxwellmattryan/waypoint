//! Rocket configuration. The API is local-only, so the bind address is forced to
//! `127.0.0.1` after every other source has been merged.

use std::net::Ipv4Addr;

use rocket::figment::Figment;

/// Port used when `WAYPOINT_API_PORT` is unset.
pub const DEFAULT_PORT: u16 = 8000;

/// Parses a `WAYPOINT_API_PORT` value, falling back to [`DEFAULT_PORT`] when it is absent.
pub fn parse_port(value: Option<&str>) -> Result<u16, String> {
    match value.map(str::trim) {
        None | Some("") => Ok(DEFAULT_PORT),
        Some(raw) => raw
            .parse::<u16>()
            .map_err(|_| format!("WAYPOINT_API_PORT must be a port number (0-65535), got {raw:?}")),
    }
}

/// Layers the port and the forced localhost address over `base`. The address is
/// merged last so no `ROCKET_ADDRESS` or `Rocket.toml` value can override it.
pub fn figment(base: Figment, port: u16) -> Figment {
    base.merge(("port", port))
        .merge(("address", Ipv4Addr::LOCALHOST))
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;

    use super::*;

    #[test]
    fn address_is_forced_to_localhost() {
        let base = Figment::from(rocket::Config::default()).merge(("address", "0.0.0.0"));
        let config: rocket::Config = figment(base, DEFAULT_PORT).extract().unwrap();
        assert_eq!(config.address, IpAddr::V4(Ipv4Addr::LOCALHOST));
    }

    #[test]
    fn port_defaults_to_8000() {
        let port = parse_port(None).unwrap();
        assert_eq!(port, 8000);
        let config: rocket::Config = figment(Figment::from(rocket::Config::default()), port)
            .extract()
            .unwrap();
        assert_eq!(config.port, 8000);
    }

    #[test]
    fn port_reads_value_and_rejects_garbage() {
        assert_eq!(parse_port(Some("9123")), Ok(9123));
        let err = parse_port(Some("nope")).unwrap_err();
        assert!(err.contains("WAYPOINT_API_PORT"));
    }
}
