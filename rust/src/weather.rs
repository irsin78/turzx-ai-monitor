//! Current weather and tomorrow's forecast from Open-Meteo (no API key), for the location in
//! the settings.

use anyhow::{Context, Result};
use serde_json::Value;

fn loc() -> &'static crate::config::Location {
    &crate::config::get().location
}

#[derive(Clone, Debug)]
pub struct Forecast {
    pub label: &'static str,
    pub icon: &'static str,
    pub t_min: f64,
    pub t_max: f64,
    pub rain_prob: Option<i64>,
}

#[derive(Clone, Debug)]
pub struct Weather {
    pub temp: f64,
    pub feels: f64,
    pub humidity: i64,
    pub label: &'static str,
    pub icon: &'static str,
    pub t_min: f64,
    pub t_max: f64,
    pub rain_prob: Option<i64>,
    pub tomorrow: Option<Forecast>,
    pub sunrise: Option<chrono::NaiveTime>,
    pub sunset: Option<chrono::NaiveTime>,
    pub pm25: Option<f64>, // µg/m³
    pub pm10: Option<f64>,
}

/// Korean air-quality grade (환경부 기준, 2018-03) and the alert level the concentration
/// reaches (주의보/경보 concentration thresholds; official alerts also need a 2-hour average).
pub fn air_grade(pm: f64, fine: bool) -> (&'static str, u8) {
    let (good, normal, bad, watch, warn) = if fine { (15.0, 35.0, 75.0, 75.0, 150.0) } else { (30.0, 80.0, 150.0, 150.0, 300.0) };
    let pm = pm.round();
    if pm >= warn {
        ("경보", 4)
    } else if pm >= watch {
        ("주의보", 4)
    } else if pm <= good {
        ("좋음", 0)
    } else if pm <= normal {
        ("보통", 1)
    } else if pm <= bad {
        ("나쁨", 2)
    } else {
        ("매우나쁨", 3)
    }
}

/// Current PM2.5 / PM10 (CAMS model via Open-Meteo, not station measurements).
fn fetch_air() -> Result<(Option<f64>, Option<f64>)> {
    let url = format!(
        "https://air-quality-api.open-meteo.com/v1/air-quality?latitude={}&longitude={}&timezone=auto&current=pm2_5,pm10",
        loc().latitude,
        loc().longitude
    );
    let j: Value = ureq::get(&url).call()?.body_mut().read_json()?;
    Ok((j["current"]["pm2_5"].as_f64(), j["current"]["pm10"].as_f64()))
}

/// WMO weather code -> (Korean label, icon kind)
fn wmo(code: i64) -> (&'static str, &'static str) {
    match code {
        0 => ("맑음", "sun"),
        1 => ("대체로 맑음", "sun"),
        2 => ("구름 조금", "partly"),
        3 => ("흐림", "cloud"),
        45 | 48 => ("안개", "fog"),
        51 | 53 | 55 => ("이슬비", "rain"),
        56 | 57 | 66 | 67 => ("어는 비", "rain"),
        61 | 63 => ("비", "rain"),
        65 => ("강한 비", "rain"),
        71 | 73 => ("눈", "snow"),
        75 => ("많은 눈", "snow"),
        77 => ("싸락눈", "snow"),
        80 | 81 => ("소나기", "rain"),
        82 => ("강한 소나기", "rain"),
        85 | 86 => ("눈 소나기", "snow"),
        95 | 96 | 99 => ("뇌우", "storm"),
        _ => ("-", "cloud"),
    }
}

pub fn fetch() -> Result<Weather> {
    let url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={}&longitude={}&timezone=auto&forecast_days=2\
         &current=temperature_2m,relative_humidity_2m,apparent_temperature,weather_code,is_day\
         &daily=weather_code,temperature_2m_max,temperature_2m_min,precipitation_probability_max,sunrise,sunset",
        loc().latitude,
        loc().longitude
    );
    let j: Value = ureq::get(&url).call()?.body_mut().read_json()?;
    let c = &j["current"];
    let d = &j["daily"];
    let f = |v: &Value| v.as_f64().context("missing number");
    let (label, mut icon) = wmo(c["weather_code"].as_i64().unwrap_or(-1));
    if c["is_day"].as_i64() == Some(0) {
        icon = match icon {
            "sun" => "moon",
            "partly" => "partly_night",
            other => other,
        };
    }
    // "2026-10-03T06:28" (local time)
    let sun = |k: &str| d[k][0].as_str().and_then(|t| chrono::NaiveDateTime::parse_from_str(t, "%Y-%m-%dT%H:%M").ok()).map(|t| t.time());
    let tomorrow = (d["time"].as_array().map_or(0, |a| a.len()) > 1).then(|| {
        let (label, icon) = wmo(d["weather_code"][1].as_i64().unwrap_or(-1));
        Forecast {
            label,
            icon,
            t_min: d["temperature_2m_min"][1].as_f64().unwrap_or(f64::NAN),
            t_max: d["temperature_2m_max"][1].as_f64().unwrap_or(f64::NAN),
            rain_prob: d["precipitation_probability_max"][1].as_i64(),
        }
    });
    let (pm25, pm10) = fetch_air().map_err(|e| log::warn!("air quality failed: {e:#}")).unwrap_or((None, None));
    Ok(Weather {
        temp: f(&c["temperature_2m"])?,
        feels: f(&c["apparent_temperature"])?,
        humidity: c["relative_humidity_2m"].as_i64().unwrap_or(0),
        label,
        icon,
        t_min: f(&d["temperature_2m_min"][0])?,
        t_max: f(&d["temperature_2m_max"][0])?,
        rain_prob: d["precipitation_probability_max"][0].as_i64(),
        tomorrow,
        sunrise: sun("sunrise"),
        sunset: sun("sunset"),
        pm25,
        pm10,
    })
}
