//! Pure classifiers of a hit (heavily unit-tested): user agent and bots, referrer
//! and acquisition, request path, GeoIP. Data tables are ported 1:1 from
//! `lib/WebPulse/Classify/data/*.php` of 0.1.0.

pub mod bots;
pub mod geoip;
pub mod path;
pub mod referrer;
pub mod ua;
