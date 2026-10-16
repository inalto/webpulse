//! Static tables generated from the PHP 0.1.0 implementation.

pub mod countries;
pub mod timezones;

/// English name of a lowercase ISO country code.
pub fn country_name(code: &str) -> Option<&'static str> {
    countries::COUNTRIES
        .binary_search_by(|(c, _)| (*c).cmp(code))
        .ok()
        .map(|i| countries::COUNTRIES[i].1)
}

/// True for an identifier of PHP's `DateTimeZone::listIdentifiers()`.
pub fn is_timezone_identifier(name: &str) -> bool {
    timezones::TIMEZONES.binary_search(&name).is_ok()
}

#[cfg(test)]
mod tests {
    #[test]
    fn tables() {
        assert_eq!(super::country_name("it"), Some("Italy"));
        assert_eq!(super::country_name("zz"), None);
        assert!(super::is_timezone_identifier("Europe/Rome"));
        assert!(super::is_timezone_identifier("UTC"));
        assert!(!super::is_timezone_identifier("Mars/Olympus"));
        for w in super::countries::COUNTRIES.windows(2) {
            assert!(w[0].0 < w[1].0);
        }
        for w in super::timezones::TIMEZONES.windows(2) {
            assert!(w[0] < w[1]);
        }
        for tz in super::timezones::TIMEZONES {
            assert!(tz.parse::<chrono_tz::Tz>().is_ok(), "{tz}");
        }
    }
}
