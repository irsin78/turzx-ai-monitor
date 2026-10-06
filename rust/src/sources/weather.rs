//! Current weather, tomorrow's forecast and air quality from Open-Meteo (no API key), for the
//! location in the settings (a city name is looked up once with Open-Meteo's geocoding).

use std::sync::OnceLock;

use anyhow::{Context, Result};
use serde_json::Value;

use crate::config;

#[derive(Clone, Debug)]
pub struct Forecast {
    pub code: i64, // WMO weather code, see `icon` and `i18n::Strings::weather`
    pub t_min: f64,
    pub t_max: f64,
    pub rain_prob: Option<i64>,
}

#[derive(Clone, Debug)]
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
    pub sunrise: Option<chrono::NaiveTime>,
    pub sunset: Option<chrono::NaiveTime>,
    pub pm25: Option<f64>, // µg/m³
    pub pm10: Option<f64>,
    pub aqi25: Option<f64>, // US AQI from PM2.5 / PM10
    pub aqi10: Option<f64>,
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

pub fn fetch() -> Result<Weather> {
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
