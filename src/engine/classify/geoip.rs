//! Country / city lookups (`WebPulse\Classify\GeoIp`) through the `maxminddb` crate,
//! which bounds the decoding of hostile files (pointer chains and fan-out budgets).
//! Without a database every lookup returns `None` (the collector records
//! `(unknown)`). Countries are lowercase ISO codes; IPv4-mapped IPv6 addresses
//! (`::ffff:a.b.c.d`) are looked up as IPv4.

use std::collections::HashMap;
use std::net::IpAddr;
use std::path::Path;
use std::sync::Mutex;

use indexmap::IndexMap;
use maxminddb::Reader;
use serde::Deserialize;
use serde::de::{self, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde_json::{Value, json};

use crate::log::{Level, Log};

/// A string field that tolerates any other type (decoded as `None`).
#[derive(Debug, Clone, Default)]
struct AnyStr(Option<String>);

impl<'de> Deserialize<'de> for AnyStr {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = AnyStr;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("any value")
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<AnyStr, E> {
                Ok(AnyStr(Some(v.to_string())))
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> Result<AnyStr, E> {
                Ok(AnyStr(None))
            }
            fn visit_i64<E: de::Error>(self, _: i64) -> Result<AnyStr, E> {
                Ok(AnyStr(None))
            }
            fn visit_u64<E: de::Error>(self, _: u64) -> Result<AnyStr, E> {
                Ok(AnyStr(None))
            }
            fn visit_i128<E: de::Error>(self, _: i128) -> Result<AnyStr, E> {
                Ok(AnyStr(None))
            }
            fn visit_u128<E: de::Error>(self, _: u128) -> Result<AnyStr, E> {
                Ok(AnyStr(None))
            }
            fn visit_f64<E: de::Error>(self, _: f64) -> Result<AnyStr, E> {
                Ok(AnyStr(None))
            }
            fn visit_bytes<E: de::Error>(self, _: &[u8]) -> Result<AnyStr, E> {
                Ok(AnyStr(None))
            }
            fn visit_unit<E: de::Error>(self) -> Result<AnyStr, E> {
                Ok(AnyStr(None))
            }
            fn visit_none<E: de::Error>(self) -> Result<AnyStr, E> {
                Ok(AnyStr(None))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<AnyStr, A::Error> {
                while a.next_element::<IgnoredAny>()?.is_some() {}
                Ok(AnyStr(None))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> Result<AnyStr, A::Error> {
                while a.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
                Ok(AnyStr(None))
            }
        }
        d.deserialize_any(V)
    }
}

/// A float field that tolerates any other type.
#[derive(Debug, Clone, Default)]
struct AnyF64(Option<f64>);

impl<'de> Deserialize<'de> for AnyF64 {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = AnyF64;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("any value")
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<AnyF64, E> {
                Ok(AnyF64(Some(v)))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<AnyF64, E> {
                Ok(AnyF64(Some(v as f64)))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<AnyF64, E> {
                Ok(AnyF64(Some(v as f64)))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<AnyF64, E> {
                Ok(AnyF64(Some(v.trim().parse().unwrap_or(0.0))))
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> Result<AnyF64, E> {
                Ok(AnyF64(Some(if v { 1.0 } else { 0.0 })))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<AnyF64, A::Error> {
                while a.next_element::<IgnoredAny>()?.is_some() {}
                Ok(AnyF64(None))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> Result<AnyF64, A::Error> {
                while a.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
                Ok(AnyF64(None))
            }
        }
        d.deserialize_any(V)
    }
}

#[derive(Debug, Default, Deserialize)]
struct Iso {
    iso_code: Option<AnyStr>,
}

#[derive(Debug, Default, Deserialize)]
struct Names {
    names: Option<IndexMap<String, AnyStr>>,
}

#[derive(Debug, Default, Deserialize)]
struct Location {
    latitude: Option<AnyF64>,
    longitude: Option<AnyF64>,
}

#[derive(Debug, Default, Deserialize)]
struct Record {
    country: Option<Iso>,
    registered_country: Option<Iso>,
    represented_country: Option<Iso>,
    city: Option<Names>,
    subdivisions: Option<Vec<Names>>,
    location: Option<Location>,
}

impl Record {
    fn iso(&self) -> Option<String> {
        for c in [
            &self.country,
            &self.registered_country,
            &self.represented_country,
        ]
        .into_iter()
        .flatten()
        {
            if let Some(code) = c.iso_code.as_ref().and_then(|a| a.0.as_ref())
                && code.len() == 2
                && code.bytes().all(|b| b.is_ascii_alphabetic())
            {
                return Some(code.to_ascii_lowercase());
            }
        }
        None
    }
}

/// A city lookup result.
#[derive(Debug, Clone, PartialEq)]
pub struct City {
    /// City name (English when available).
    pub name: String,
    /// Lowercase ISO country.
    pub country: Option<String>,
    /// First subdivision (English name).
    pub region: Option<String>,
    /// Latitude.
    pub lat: Option<f64>,
    /// Longitude.
    pub lon: Option<f64>,
}

/// Country / city lookups with per-address caches.
pub struct GeoIp {
    country: Option<Reader<Vec<u8>>>,
    city: Option<Reader<Vec<u8>>>,
    country_cache: Mutex<HashMap<String, Option<String>>>,
    city_cache: Mutex<HashMap<String, Option<City>>>,
}

impl std::fmt::Debug for GeoIp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GeoIp")
            .field("country", &self.country.is_some())
            .field("city", &self.city.is_some())
            .finish()
    }
}

/// Opens one MaxMind DB file (errors for missing or invalid files).
pub fn open_reader(file: &Path) -> anyhow::Result<Reader<Vec<u8>>> {
    Reader::open_readfile(file).map_err(|e| anyhow::anyhow!("{}: {e}", file.display()))
}

fn addr(ip: &str) -> Option<IpAddr> {
    let a = crate::util::ip::parse_ip(ip)?;
    Some(match a {
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => IpAddr::V6(v6),
        },
        v4 => v4,
    })
}

fn lookup(r: &Reader<Vec<u8>>, ip: &str) -> Option<Record> {
    let a = addr(ip)?;
    if matches!(a, IpAddr::V6(_)) && r.metadata().ip_version == 4 {
        return None;
    }
    let res = r.lookup(a).ok()?;
    res.decode::<Record>().ok().flatten()
}

impl GeoIp {
    /// Opens the given databases; unusable files are logged and ignored.
    pub fn new(country_db: Option<&Path>, city_db: Option<&Path>, log: Option<&Log>) -> GeoIp {
        let open = |p: Option<&Path>| -> Option<Reader<Vec<u8>>> {
            let p = p.filter(|p| !p.as_os_str().is_empty() && p.is_file())?;
            match open_reader(p) {
                Ok(r) => Some(r),
                Err(e) => {
                    if let Some(l) = log {
                        l.log_ctx(
                            Level::Warn,
                            "geoip: cannot use database",
                            &json!({"file": p.display().to_string(), "error": e.to_string()}),
                        );
                    }
                    None
                }
            }
        };
        GeoIp {
            country: open(country_db),
            city: open(city_db),
            country_cache: Mutex::new(HashMap::new()),
            city_cache: Mutex::new(HashMap::new()),
        }
    }

    /// A GeoIp without databases.
    pub fn none() -> GeoIp {
        GeoIp::new(None, None, None)
    }

    /// True when at least one database is loaded.
    pub fn available(&self) -> bool {
        self.country.is_some() || self.city.is_some()
    }

    /// True when a city database is loaded.
    pub fn has_city(&self) -> bool {
        self.city.is_some()
    }

    /// Lowercase ISO-3166 alpha-2 code, or `None`.
    pub fn country(&self, ip: &str) -> Option<String> {
        let mut cache = self.country_cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(v) = cache.get(ip) {
            return v.clone();
        }
        if cache.len() > 50000 {
            cache.clear();
        }
        let code = self
            .country
            .as_ref()
            .or(self.city.as_ref())
            .and_then(|r| lookup(r, ip))
            .and_then(|rec| rec.iso());
        cache.insert(ip.to_string(), code.clone());
        code
    }

    /// City record, or `None` without a city database or name.
    pub fn city(&self, ip: &str) -> Option<City> {
        let r = self.city.as_ref()?;
        let mut cache = self.city_cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(v) = cache.get(ip) {
            return v.clone();
        }
        if cache.len() > 50000 {
            cache.clear();
        }
        let out = lookup(r, ip).and_then(|rec| {
            let names = rec.city.as_ref()?.names.as_ref()?;
            let name = match names.get("en") {
                Some(v) => v.0.clone(),
                None => names.values().next().and_then(|v| v.0.clone()),
            }?;
            if name.is_empty() {
                return None;
            }
            let region = rec
                .subdivisions
                .as_ref()
                .and_then(|s| s.first())
                .and_then(|s| s.names.as_ref())
                .and_then(|n| n.get("en"))
                .and_then(|v| v.0.clone());
            Some(City {
                name,
                country: rec.iso(),
                region,
                lat: rec
                    .location
                    .as_ref()
                    .and_then(|l| l.latitude.as_ref())
                    .and_then(|v| v.0),
                lon: rec
                    .location
                    .as_ref()
                    .and_then(|l| l.longitude.as_ref())
                    .and_then(|v| v.0),
            })
        });
        cache.insert(ip.to_string(), out.clone());
        out
    }

    /// Metadata of the country (or city) database as JSON, `None` without one.
    pub fn metadata(&self) -> Option<Value> {
        let r = self.country.as_ref().or(self.city.as_ref())?;
        Some(metadata_json(r.metadata()))
    }
}

/// MaxMind metadata as a JSON object (keys of the file format).
pub fn metadata_json(m: &maxminddb::Metadata) -> Value {
    json!({
        "binary_format_major_version": m.binary_format_major_version,
        "binary_format_minor_version": m.binary_format_minor_version,
        "build_epoch": m.build_epoch,
        "database_type": m.database_type,
        "description": m.description,
        "ip_version": m.ip_version,
        "languages": m.languages,
        "node_count": m.node_count,
        "record_size": m.record_size,
    })
}
