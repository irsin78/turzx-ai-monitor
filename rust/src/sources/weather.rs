//! Current weather and tomorrow's forecast from Open-Meteo with a MET Norway fallback,
//! plus Open-Meteo air quality (no API keys). A city is looked up once with Open-Meteo.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
use chrono::{NaiveDate, TimeZone, Timelike};
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

fn forecast_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15)))
        .user_agent(concat!("turzx-ai-monitor/", env!("CARGO_PKG_VERSION"), " github.com/irsin78/turzx-ai-monitor"))
        .build()
        .into()
}

/// Diagnostic MET forecast and sunrise/sunset only; no air quality or cache writes.
pub fn fetch_met_only() -> Result<Weather> {
    fetch_met(&forecast_agent(), coords()?)
}

fn fetch_current() -> Result<Weather> {
    let coords = coords()?;
    let agent = forecast_agent();
    let mut weather = forecast_with_fallback(|| fetch_open_meteo(&agent, coords), || fetch_met(&agent, coords))?;
    let [pm25, pm10, aqi25, aqi10] = fetch_air(coords).map_err(|e| log::warn!("air quality failed: {e:#}")).unwrap_or([None; 4]);
    weather.pm25 = pm25;
    weather.pm10 = pm10;
    weather.aqi25 = aqi25;
    weather.aqi10 = aqi10;
    Ok(weather)
}

/// Only a failed primary forecast triggers MET; air quality remains independent.
fn forecast_with_fallback(primary: impl FnOnce() -> Result<Weather>, backup: impl FnOnce() -> Result<Weather>) -> Result<Weather> {
    match primary() {
        Ok(weather) => Ok(weather),
        Err(primary_error) => {
            let weather = backup().map_err(|e| anyhow::anyhow!("Open-Meteo: {primary_error:#}; MET Norway: {e:#}"))?;
            log::info!("weather from MET Norway (Open-Meteo: {primary_error:#})");
            Ok(weather)
        }
    }
}

fn fetch_open_meteo(agent: &ureq::Agent, (lat, lon): (f64, f64)) -> Result<Weather> {
    let url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={lat}&longitude={lon}&timezone=auto&forecast_days=2\
         &current=temperature_2m,relative_humidity_2m,apparent_temperature,weather_code,is_day\
         &daily=weather_code,temperature_2m_max,temperature_2m_min,precipitation_probability_max,sunrise,sunset"
    );
    let j: Value = agent.get(&url).call()?.body_mut().read_json()?;
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
        pm25: None,
        pm10: None,
        aqi25: None,
        aqi10: None,
    })
}

fn fetch_met(agent: &ureq::Agent, (lat, lon): (f64, f64)) -> Result<Weather> {
    let now = chrono::Local::now();
    let url = format!("https://api.met.no/weatherapi/locationforecast/2.0/complete?lat={lat:.4}&lon={lon:.4}");
    let j: Value = agent.get(&url).call()?.body_mut().read_json()?;
    let mut weather = parse_met(&j, now.date_naive(), &chrono::Local)?;
    let url = format!(
        "https://api.met.no/weatherapi/sunrise/3.0/sun?lat={lat:.4}&lon={lon:.4}&date={}&offset={}",
        now.date_naive(), urlencode(&now.format("%:z").to_string())
    );
    match agent.get(&url).call().and_then(|mut r| r.body_mut().read_json::<Value>()) {
        Ok(sun) => set_met_sun(&mut weather, &sun, &chrono::Local),
        Err(e) => log::debug!("MET Norway sunrise/sunset failed: {e}"),
    }
    Ok(weather)
}

fn met_symbol(data: &Value) -> Option<&str> {
    data["next_1_hours"]["summary"]["symbol_code"].as_str()
        .or_else(|| data["next_6_hours"]["summary"]["symbol_code"].as_str())
}

/// MET's day/night suffix affects the icon, not the WMO condition code.
fn met_code(symbol: &str) -> i64 {
    let base = ["_day", "_night", "_polartwilight"].iter().find_map(|suffix| symbol.strip_suffix(suffix)).unwrap_or(symbol);
    if base.contains("thunder") {
        return 95;
    }
    match base {
        "clearsky" => 0,
        "fair" => 1,
        "partlycloudy" => 2,
        "cloudy" => 3,
        "fog" => 45,
        "lightrain" => 61,
        "rain" => 63,
        "heavyrain" => 65,
        "lightrainshowers" => 80,
        "rainshowers" => 81,
        "heavyrainshowers" => 82,
        "lightsleet" | "lightsleetshowers" => 66,
        "sleet" | "heavysleet" | "sleetshowers" | "heavysleetshowers" => 67,
        "lightsnow" => 71,
        "snow" => 73,
        "heavysnow" => 75,
        "lightsnowshowers" | "snowshowers" => 85,
        "heavysnowshowers" => 86,
        _ => {
            log::debug!("unknown MET Norway symbol: {symbol}");
            3
        }
    }
}

/// Remaining hourly values and six-hour extrema are assigned to their local start date.
/// A six-hour interval can cross midnight; its full extrema still count for that start date.
fn met_day<T: TimeZone>(series: &[Value], date: NaiveDate, zone: &T) -> Option<Forecast> {
    let mut t_min = f64::INFINITY;
    let mut t_max = f64::NEG_INFINITY;
    let mut rain_prob = None;
    let mut codes = Vec::<(i64, usize)>::new();
    let mut noon = None;
    for entry in series {
        let time = chrono::DateTime::parse_from_rfc3339(entry["time"].as_str()?).ok()?.with_timezone(zone);
        if time.date_naive() != date {
            continue;
        }
        let data = &entry["data"];
        for temp in [
            data["instant"]["details"]["air_temperature"].as_f64(),
            data["next_6_hours"]["details"]["air_temperature_min"].as_f64(),
            data["next_6_hours"]["details"]["air_temperature_max"].as_f64(),
        ].into_iter().flatten() {
            t_min = t_min.min(temp);
            t_max = t_max.max(temp);
        }
        for period in ["next_1_hours", "next_6_hours"] {
            if let Some(prob) = data[period]["details"]["probability_of_precipitation"].as_f64() {
                let prob = prob.round() as i64;
                rain_prob = Some(rain_prob.map_or(prob, |previous: i64| previous.max(prob)));
            }
        }
        if let Some(symbol) = met_symbol(data) {
            let code = met_code(symbol);
            if let Some((_, count)) = codes.iter_mut().find(|(c, _)| *c == code) {
                *count += 1;
            } else {
                codes.push((code, 1));
            }
            // Prefer the symbol nearest noon within 11:00..=13:00, otherwise the mode.
            let distance = (i64::from(time.num_seconds_from_midnight()) - 12 * 3600).abs();
            if distance <= 3600 && noon.is_none_or(|(best, _)| distance < best) {
                noon = Some((distance, code));
            }
        }
    }
    (t_min.is_finite() && t_max.is_finite()).then(|| Forecast {
        code: noon.map(|(_, code)| code).or_else(|| codes.iter().max_by_key(|(_, count)| *count).map(|(code, _)| *code)).unwrap_or(3),
        t_min, t_max, rain_prob,
    })
}

/// The timezone is explicit so fixtures do not depend on the test machine's location.
fn parse_met<T: TimeZone>(j: &Value, today: NaiveDate, zone: &T) -> Result<Weather> {
    let series = j["properties"]["timeseries"].as_array().context("MET Norway: missing timeseries")?;
    let data = &series.first().context("MET Norway: empty timeseries")?["data"];
    let current = &data["instant"]["details"];
    let temp = current["air_temperature"].as_f64().context("MET Norway: missing temperature")?;
    let symbol = met_symbol(data).context("MET Norway: missing current symbol")?;
    let day = met_day(series, today, zone).context("MET Norway: missing today's temperatures or invalid time")?;
    Ok(Weather {
        temp,
        feels: current["apparent_air_temperature"].as_f64().unwrap_or(temp),
        humidity: current["relative_humidity"].as_f64().context("MET Norway: missing humidity")?.round() as i64,
        code: met_code(symbol),
        night: symbol.ends_with("_night"),
        t_min: day.t_min,
        t_max: day.t_max,
        rain_prob: day.rain_prob,
        tomorrow: today.succ_opt().and_then(|date| met_day(series, date, zone)),
        sunrise: None,
        sunset: None,
        pm25: None,
        pm10: None,
        aqi25: None,
        aqi10: None,
    })
}

fn set_met_sun<T: TimeZone>(weather: &mut Weather, j: &Value, zone: &T) {
    let sun = |key: &str| j["properties"][key]["time"].as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s)
            .or_else(|_| chrono::DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M%:z")).ok())
        .map(|t| t.with_timezone(zone).time());
    weather.sunrise = sun("sunrise");
    weather.sunset = sun("sunset");
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

    // UTC timestamps deliberately cross local midnight in Asia/Seoul.
    const MET_FORECAST: &str = r#"{"properties":{"timeseries":[
        {"time":"2026-11-17T15:00:00Z","data":{
            "instant":{"details":{"air_temperature":3.5,"apparent_air_temperature":2.25,"relative_humidity":65.6}},
            "next_1_hours":{"summary":{"symbol_code":"clearsky_night"}},
            "next_6_hours":{"details":{"air_temperature_min":1,"air_temperature_max":6}}}},
        {"time":"2026-11-18T03:00:00Z","data":{
            "instant":{"details":{"air_temperature":8}},
            "next_1_hours":{"summary":{"symbol_code":"fair_day"}},
            "next_6_hours":{"details":{"air_temperature_min":7,"air_temperature_max":12}}}},
        {"time":"2026-11-18T14:00:00Z","data":{
            "instant":{"details":{"air_temperature":2}},
            "next_1_hours":{"summary":{"symbol_code":"cloudy"}},
            "next_6_hours":{"details":{"air_temperature_min":-2,"air_temperature_max":4}}}},
        {"time":"2026-11-18T15:00:00Z","data":{
            "instant":{"details":{"air_temperature":4}},
            "next_1_hours":{"summary":{"symbol_code":"cloudy"},"details":{"probability_of_precipitation":10}},
            "next_6_hours":{"details":{"air_temperature_min":0,"air_temperature_max":6}}}},
        {"time":"2026-11-19T00:00:00Z","data":{
            "instant":{"details":{"air_temperature":9}},
            "next_1_hours":{"summary":{"symbol_code":"clearsky_day"},"details":{"probability_of_precipitation":20}}}},
        {"time":"2026-11-19T03:00:00Z","data":{
            "instant":{"details":{"air_temperature":15}},
            "next_6_hours":{"summary":{"symbol_code":"heavyrainshowers_day"},
                "details":{"air_temperature_min":11,"air_temperature_max":17,"probability_of_precipitation":80}}}},
        {"time":"2026-11-19T06:00:00Z","data":{
            "instant":{"details":{"air_temperature":12}},
            "next_1_hours":{"summary":{"symbol_code":"cloudy"}},
            "next_6_hours":{"details":{"air_temperature_min":10,"air_temperature_max":14}}}},
        {"time":"2026-11-19T15:00:00Z","data":{
            "instant":{"details":{"air_temperature":99}},
            "next_1_hours":{"summary":{"symbol_code":"snow"},"details":{"probability_of_precipitation":100}}}}
    ]}}"#;

    const MET_SUN: &str = r#"{"properties":{
        "sunrise":{"time":"2026-11-18T07:14+09:00"},
        "sunset":{"time":"2026-11-18T17:20+09:00"}
    }}"#;

    #[test]
    fn met_forecast_and_sun_fixtures() {
        let j: Value = serde_json::from_str(MET_FORECAST).unwrap();
        let today = NaiveDate::from_ymd_opt(2026, 11, 18).unwrap();
        let zone = chrono_tz::Asia::Seoul;
        let mut weather = parse_met(&j, today, &zone).unwrap();
        assert_eq!((weather.temp, weather.feels, weather.humidity), (3.5, 2.25, 66));
        assert_eq!((weather.code, weather.night), (0, true));
        assert_eq!((weather.t_min, weather.t_max, weather.rain_prob), (-2.0, 12.0, None));
        assert_eq!(weather.tomorrow, Some(Forecast { code: 82, t_min: 0.0, t_max: 17.0, rain_prob: Some(80) }));
        assert_eq!((weather.pm25, weather.pm10, weather.aqi25, weather.aqi10), (None, None, None, None));
        assert_eq!((weather.sunrise, weather.sunset), (None, None));

        let sun: Value = serde_json::from_str(MET_SUN).unwrap();
        set_met_sun(&mut weather, &sun, &zone);
        assert_eq!(weather.sunrise, chrono::NaiveTime::from_hms_opt(7, 14, 0));
        assert_eq!(weather.sunset, chrono::NaiveTime::from_hms_opt(17, 20, 0));
        set_met_sun(&mut weather, &sun, &chrono::Utc);
        assert_eq!(weather.sunrise, chrono::NaiveTime::from_hms_opt(22, 14, 0));
        set_met_sun(&mut weather, &serde_json::json!({"properties":{"sunrise":{"time":null}}}), &zone);
        assert_eq!((weather.sunrise, weather.sunset), (None, None));
    }

    #[test]
    fn met_optional_values_and_modal_symbol() {
        let mut j: Value = serde_json::from_str(MET_FORECAST).unwrap();
        let series = j["properties"]["timeseries"].as_array_mut().unwrap();
        let data = &mut series[0]["data"];
        data["instant"]["details"]["apparent_air_temperature"] = Value::Null;
        data["next_1_hours"]["summary"]["symbol_code"] = "fair_polartwilight".into();
        data["next_1_hours"]["details"] = serde_json::json!({"probability_of_precipitation":30});
        data["next_6_hours"]["details"]["probability_of_precipitation"] = 60.into();
        // No noon symbol: cloudy is the most common remaining tomorrow symbol.
        series.remove(5);
        let today = NaiveDate::from_ymd_opt(2026, 11, 18).unwrap();
        let weather = parse_met(&j, today, &chrono_tz::Asia::Seoul).unwrap();
        assert_eq!(weather.feels, weather.temp);
        assert_eq!((weather.code, weather.night, weather.rain_prob), (1, false, Some(60)));
        assert_eq!(weather.tomorrow, Some(Forecast { code: 3, t_min: 0.0, t_max: 14.0, rain_prob: Some(20) }));

        // A six-hour summary also works for the current condition.
        let first = &mut j["properties"]["timeseries"][0]["data"];
        first["next_1_hours"]["summary"]["symbol_code"] = Value::Null;
        first["next_6_hours"]["summary"] = serde_json::json!({"symbol_code":"snow_night"});
        let weather = parse_met(&j, today, &chrono_tz::Asia::Seoul).unwrap();
        assert_eq!((weather.code, weather.night), (73, true));
    }

    #[test]
    fn met_symbol_mapping_uses_supported_wmo_codes() {
        for (symbol, expected) in [
            ("clearsky_day", 0), ("fair_night", 1), ("partlycloudy_polartwilight", 2),
            ("cloudy", 3), ("fog", 45), ("lightrain", 61), ("rain", 63), ("heavyrain", 65),
            ("lightrainshowers_day", 80), ("rainshowers_night", 81), ("heavyrainshowers_day", 82),
            ("lightsleet", 66), ("sleet", 67), ("heavysleetshowers_day", 67),
            ("lightsnow", 71), ("snow", 73), ("heavysnow", 75),
            ("lightsnowshowers_night", 85), ("snowshowers_day", 85), ("heavysnowshowers_day", 86),
            ("rainandthunder", 95), ("lightsleetshowersandthunder_night", 95), ("unknown_day", 3),
        ] {
            let code = met_code(symbol);
            assert_eq!(code, expected, "{symbol}");
            assert!(crate::i18n::weather_index(code).is_some(), "{symbol}: {code}");
        }
    }

    #[test]
    fn forecast_selects_backup_only_after_primary_error() {
        let expected = sample().weather;
        let actual = forecast_with_fallback(|| Ok(expected.clone()), || panic!("backup must not run")).unwrap();
        assert_eq!(actual, expected);
        for primary_error in ["timeout", "invalid JSON", "HTTP 503"] {
            let actual = forecast_with_fallback(|| anyhow::bail!(primary_error), || Ok(expected.clone())).unwrap();
            assert_eq!(actual, expected);
        }
        let error = forecast_with_fallback(|| anyhow::bail!("primary timeout"), || anyhow::bail!("backup bad JSON")).unwrap_err().to_string();
        assert!(error.contains("Open-Meteo: primary timeout"));
        assert!(error.contains("MET Norway: backup bad JSON"));
    }

    #[test]
    fn met_rejects_missing_forecast_data() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 18).unwrap();
        for j in [serde_json::json!({}), serde_json::json!({"properties":{"timeseries":[]}})] {
            assert!(parse_met(&j, today, &chrono::Utc).is_err());
        }
        let mut j: Value = serde_json::from_str(MET_FORECAST).unwrap();
        j["properties"]["timeseries"][0]["time"] = "bad time".into();
        assert!(parse_met(&j, today, &chrono_tz::Asia::Seoul).is_err());
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
