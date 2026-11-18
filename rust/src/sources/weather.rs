//! Current weather, tomorrow's forecast and air quality from Open-Meteo (no API key), for the
//! location in the settings (a city name is looked up once with Open-Meteo's geocoding).

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Forecast {
    pub code: i64, // WMO weather code, see `icon` and `i18n::Strings::weather`
    #[serde(deserialize_with = "forecast_temp")]
    pub t_min: f64,
    #[serde(deserialize_with = "forecast_temp")]
    pub t_max: f64,
    pub rain_prob: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Weather {
    pub temp: f64, // °C
    pub feels: f64,
    pub humidity: i64,
    pub code: i64,
    pub night: bool,
    pub t_min: f64,
    pub t_max: f64,
    pub rain_prob: Option<i64>,
    pub tomorrow: Option<Forecast>,
    #[serde(with = "optional_time")]
    pub sunrise: Option<chrono::NaiveTime>,
    #[serde(with = "optional_time")]
    pub sunset: Option<chrono::NaiveTime>,
    pub pm25: Option<f64>, // µg/m³
    pub pm10: Option<f64>,
    pub aqi25: Option<f64>, // US AQI from PM2.5 / PM10
    pub aqi10: Option<f64>,
}

// Chrono's serde feature is not enabled; keep the cache's times as small strings.
mod optional_time {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(time: &Option<chrono::NaiveTime>, serializer: S) -> Result<S::Ok, S::Error> {
        time.map(|t| t.to_string()).serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<chrono::NaiveTime>, D::Error> {
        Option::<String>::deserialize(deserializer)?.map(|s| s.parse().map_err(serde::de::Error::custom)).transpose()
    }
}

// Missing tomorrow temperatures are NaN in memory and null in JSON.
fn forecast_temp<'de, D: serde::Deserializer<'de>>(deserializer: D) -> std::result::Result<f64, D::Error> {
    Ok(Option::<f64>::deserialize(deserializer)?.unwrap_or(f64::NAN))
}

const CACHE_MAX_AGE: Duration = Duration::from_secs(6 * 3600);

#[derive(Serialize, Deserialize)]
pub struct CachedWeather {
    pub weather: Weather,
    pub fetched_at: SystemTime,
}

/// A future timestamp is invalid too (for example, after the system clock changes).
pub fn cache_fresh(fetched_at: SystemTime, now: SystemTime) -> bool {
    now.duration_since(fetched_at).is_ok_and(|age| age < CACHE_MAX_AGE)
}

fn cache_path() -> PathBuf {
    PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default()).join(r"TurzxDashboard\weather.json")
}

fn load_cache(path: &Path, now: SystemTime) -> Result<Option<CachedWeather>> {
    let data = match std::fs::read(path) {
        Ok(data) => data,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let cached: CachedWeather = serde_json::from_slice(&data)?;
    Ok(cache_fresh(cached.fetched_at, now).then_some(cached))
}

impl CachedWeather {
    /// Replace only after a complete write; distinct names let the video builder fetch too.
    fn store(&self, path: &Path) -> Result<()> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let data = serde_json::to_vec(self)?;
        let tmp = path.with_extension(format!("{}.{}.tmp", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        let result = (|| {
            std::fs::write(&tmp, data)?;
            std::fs::rename(&tmp, path)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        result.context("write weather cache")
    }
}

/// Last good weather, only while it is younger than six hours.
pub fn cached() -> Option<CachedWeather> {
    load_cache(&cache_path(), SystemTime::now()).unwrap_or_else(|e| {
        log::warn!("weather cache read failed: {e:#}");
        None
    })
}

/// Air quality level, mild to severe.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Air {
    Good,
    Moderate,
    Bad,
    VeryBad,
    Hazardous,
    /// Korean scale: the concentration reaches the 주의보 (watch) / 경보 (warning) threshold;
    /// official alerts also need a 2-hour average
    Watch,
    Warning,
}

/// Korean Ministry of Environment grades (2018-03) for a PM2.5 (`fine`) or PM10 concentration.
pub fn air_korea(pm: f64, fine: bool) -> Air {
    let (good, normal, bad, watch, warn) = if fine { (15.0, 35.0, 75.0, 75.0, 150.0) } else { (30.0, 80.0, 150.0, 150.0, 300.0) };
    let pm = pm.round();
    if pm >= warn {
        Air::Warning
    } else if pm >= watch {
        Air::Watch
    } else if pm <= good {
        Air::Good
    } else if pm <= normal {
        Air::Moderate
    } else if pm <= bad {
        Air::Bad
    } else {
        Air::VeryBad
    }
}

/// US AQI categories (101-150 "unhealthy for sensitive groups" is folded into Bad).
pub fn air_us(aqi: f64) -> Air {
    match aqi.round() as i64 {
        ..=50 => Air::Good,
        51..=100 => Air::Moderate,
        101..=200 => Air::Bad,
        201..=300 => Air::VeryBad,
        _ => Air::Hazardous,
    }
}

/// Icon kind for a WMO code.
pub fn icon(code: i64, night: bool) -> &'static str {
    let day = match code {
        0 | 1 => "sun",
        2 => "partly",
        45 | 48 => "fog",
        51..=67 | 80..=82 => "rain",
        71..=77 | 85 | 86 => "snow",
        95..=99 => "storm",
        _ => "cloud",
    };
    match (day, night) {
        ("sun", true) => "moon",
        ("partly", true) => "partly_night",
        (d, _) => d,
    }
}

/// Coordinates of the configured location; a city name is looked up once.
fn coords() -> Result<(f64, f64)> {
    static FOUND: OnceLock<(f64, f64)> = OnceLock::new();
    let loc = &config::get().location;
    if let (Some(lat), Some(lon)) = (loc.latitude, loc.longitude) {
        return Ok((lat, lon));
    }
    if let Some(c) = FOUND.get() {
        return Ok(*c);
    }
    let city = loc.city.trim();
    anyhow::ensure!(!city.is_empty(), "no location: set city or latitude/longitude in the settings");
    let url = format!("https://geocoding-api.open-meteo.com/v1/search?count=1&format=json&name={}", urlencode(city));
    let j: Value = ureq::get(&url).call()?.body_mut().read_json()?;
    let r = &j["results"][0];
    let c = (r["latitude"].as_f64().with_context(|| format!("city {city:?} not found"))?, r["longitude"].as_f64().context("no longitude")?);
    log::info!("location {city:?}: {}, {} ({})", c.0, c.1, r["country"].as_str().unwrap_or("?"));
    Ok(*FOUND.get_or_init(|| c))
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Current PM2.5 / PM10 and their US AQI (CAMS model via Open-Meteo, not station measurements).
fn fetch_air((lat, lon): (f64, f64)) -> Result<[Option<f64>; 4]> {
    let url = format!(
        "https://air-quality-api.open-meteo.com/v1/air-quality?latitude={lat}&longitude={lon}&timezone=auto\
         &current=pm2_5,pm10,us_aqi_pm2_5,us_aqi_pm10"
    );
    let j: Value = ureq::get(&url).call()?.body_mut().read_json()?;
    let c = &j["current"];
    Ok(["pm2_5", "pm10", "us_aqi_pm2_5", "us_aqi_pm10"].map(|k| c[k].as_f64()))
}

/// Fetch fresh weather and remember it for startup and the off-time video.
pub fn fetch() -> Result<Weather> {
    let weather = fetch_current()?;
    let cached = CachedWeather { weather, fetched_at: SystemTime::now() };
    if let Err(e) = cached.store(&cache_path()) {
        log::warn!("weather cache write failed: {e:#}");
    }
    Ok(cached.weather)
}

fn fetch_current() -> Result<Weather> {
    let (lat, lon) = coords()?;
    let url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={lat}&longitude={lon}&timezone=auto&forecast_days=2\
         &current=temperature_2m,relative_humidity_2m,apparent_temperature,weather_code,is_day\
         &daily=weather_code,temperature_2m_max,temperature_2m_min,precipitation_probability_max,sunrise,sunset"
    );
    let j: Value = ureq::get(&url).call()?.body_mut().read_json()?;
    let c = &j["current"];
    let d = &j["daily"];
    let f = |v: &Value| v.as_f64().context("missing number");
    // "2026-10-03T06:28" (local time)
    let sun = |k: &str| d[k][0].as_str().and_then(|t| chrono::NaiveDateTime::parse_from_str(t, "%Y-%m-%dT%H:%M").ok()).map(|t| t.time());
    let tomorrow = (d["time"].as_array().map_or(0, |a| a.len()) > 1).then(|| Forecast {
        code: d["weather_code"][1].as_i64().unwrap_or(-1),
        t_min: d["temperature_2m_min"][1].as_f64().unwrap_or(f64::NAN),
        t_max: d["temperature_2m_max"][1].as_f64().unwrap_or(f64::NAN),
        rain_prob: d["precipitation_probability_max"][1].as_i64(),
    });
    let [pm25, pm10, aqi25, aqi10] = fetch_air((lat, lon)).map_err(|e| log::warn!("air quality failed: {e:#}")).unwrap_or([None; 4]);
    Ok(Weather {
        temp: f(&c["temperature_2m"])?,
        feels: f(&c["apparent_temperature"])?,
        humidity: c["relative_humidity_2m"].as_i64().unwrap_or(0),
        code: c["weather_code"].as_i64().unwrap_or(-1),
        night: c["is_day"].as_i64() == Some(0),
        t_min: f(&d["temperature_2m_min"][0])?,
        t_max: f(&d["temperature_2m_max"][0])?,
        rain_prob: d["precipitation_probability_max"][0].as_i64(),
        tomorrow,
        sunrise: sun("sunrise"),
        sunset: sun("sunset"),
        pm25,
        pm10,
        aqi25,
        aqi10,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::UNIX_EPOCH;

    struct TestCache(PathBuf);

    impl TestCache {
        fn new() -> Self {
            let unique = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
            Self(std::env::temp_dir().join(format!("turzx-weather-test-{}-{unique}", std::process::id())).join("weather.json"))
        }
    }

    impl Drop for TestCache {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
            let _ = std::fs::remove_dir(self.0.parent().unwrap());
        }
    }

    fn sample() -> CachedWeather {
        CachedWeather {
            fetched_at: UNIX_EPOCH + Duration::from_secs(1_000_000),
            weather: Weather {
                temp: 18.5, feels: 17.0, humidity: 60, code: 2, night: false,
                t_min: 12.0, t_max: 21.0, rain_prob: Some(30),
                tomorrow: Some(Forecast { code: 3, t_min: 10.0, t_max: 20.0, rain_prob: None }),
                sunrise: chrono::NaiveTime::from_hms_opt(6, 28, 0), sunset: None,
                pm25: Some(12.5), pm10: Some(25.0), aqi25: Some(50.0), aqi10: None,
            },
        }
    }

    #[test]
    fn cache_round_trip_and_replace() {
        let path = TestCache::new();
        let mut saved = sample();
        saved.store(&path.0).unwrap();
        let loaded = load_cache(&path.0, saved.fetched_at + Duration::from_secs(3600)).unwrap().unwrap();
        assert_eq!(loaded.weather, saved.weather);
        assert_eq!(loaded.fetched_at, saved.fetched_at);

        saved.weather.temp = 22.0;
        saved.fetched_at += Duration::from_secs(600);
        saved.store(&path.0).unwrap();
        let loaded = load_cache(&path.0, saved.fetched_at).unwrap().unwrap();
        assert_eq!(loaded.weather, saved.weather);
        assert_eq!(loaded.fetched_at, saved.fetched_at);
        assert_eq!(std::fs::read_dir(path.0.parent().unwrap()).unwrap().count(), 1);
    }

    #[test]
    fn cache_age_cutoff() {
        let path = TestCache::new();
        let saved = sample();
        assert!(load_cache(&path.0, saved.fetched_at).unwrap().is_none());
        saved.store(&path.0).unwrap();
        let cutoff = saved.fetched_at + CACHE_MAX_AGE;
        // Windows SystemTime has 100 ns ticks; use a representable boundary offset.
        assert!(load_cache(&path.0, cutoff - Duration::from_micros(1)).unwrap().is_some());
        assert!(load_cache(&path.0, cutoff).unwrap().is_none());
        assert!(load_cache(&path.0, cutoff + Duration::from_secs(1)).unwrap().is_none());
        assert!(load_cache(&path.0, saved.fetched_at - Duration::from_secs(1)).unwrap().is_none());
        std::fs::write(&path.0, b"broken JSON").unwrap();
        assert!(load_cache(&path.0, saved.fetched_at).is_err());
    }

    #[test]
    fn cache_preserves_missing_forecast_temperatures() {
        let path = TestCache::new();
        let mut saved = sample();
        saved.weather.tomorrow.as_mut().unwrap().t_min = f64::NAN;
        saved.weather.tomorrow.as_mut().unwrap().t_max = f64::NAN;
        saved.store(&path.0).unwrap();
        let loaded = load_cache(&path.0, saved.fetched_at).unwrap().unwrap();
        let tomorrow = loaded.weather.tomorrow.unwrap();
        assert!(tomorrow.t_min.is_nan());
        assert!(tomorrow.t_max.is_nan());
        assert_eq!(loaded.weather.temp, saved.weather.temp);
    }
}
