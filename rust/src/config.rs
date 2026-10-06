//! User settings, read from `%APPDATA%\TurzxDashboard\config.toml`.
//!
//! The file is written with comments and the defaults on the first run; missing keys fall back
//! to the defaults. "auto" values follow the Windows display language and region. The dashboard
//! restarts by itself when the file is saved (see `watch`).

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    /// "auto" or a language code (see `i18n::LANGUAGES`)
    pub language: String,
    /// "12h" or "24h"
    pub clock: String,
    /// "C" or "F"
    pub temperature: String,
    pub location: Location,
    pub holidays: Holidays,
    pub air_quality: AirQuality,
    pub ai: Ai,
    pub fans: Fans,
}

/// Where the weather and air quality are for.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct Location {
    /// looked up by name when latitude/longitude are not set
    pub city: String,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct Holidays {
    /// ISO 3166 country code ("KR", "US", "JP", ...), "auto" (Windows region) or "none"
    pub country: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct AirQuality {
    /// "kr" (Korean Ministry of Environment grades), "us" (US AQI) or "auto" (kr in Korea)
    pub scale: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct Ai {
    pub claude: bool,
    pub codex: bool,
    pub antigravity: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct Fans {
    /// names of the two board fan channels and the GPU fan; empty = the language's default
    pub labels: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            language: "auto".into(),
            clock: "12h".into(),
            temperature: "C".into(),
            location: Location::default(),
            holidays: Holidays { country: "auto".into() },
            air_quality: AirQuality { scale: "auto".into() },
            ai: Ai { claude: true, codex: true, antigravity: true },
            fans: Fans { labels: Vec::new() },
        }
    }
}

impl Default for Location {
    fn default() -> Self {
        Location { city: "Seoul".into(), latitude: None, longitude: None }
    }
}

impl Default for Holidays {
    fn default() -> Self {
        Holidays { country: "auto".into() }
    }
}

impl Default for AirQuality {
    fn default() -> Self {
        AirQuality { scale: "auto".into() }
    }
}

impl Default for Ai {
    fn default() -> Self {
        Ai { claude: true, codex: true, antigravity: true }
    }
}

impl Default for Fans {
    fn default() -> Self {
        Fans { labels: Vec::new() }
    }
}

impl Config {
    pub fn hour24(&self) -> bool {
        self.clock.trim().eq_ignore_ascii_case("24h")
    }

    pub fn fahrenheit(&self) -> bool {
        self.temperature.trim().eq_ignore_ascii_case("F")
    }

    /// Two-letter country code for holidays, or None.
    pub fn holiday_country(&self) -> Option<String> {
        match self.holidays.country.trim() {
            "" => None,
            c if c.eq_ignore_ascii_case("none") => None,
            c if c.eq_ignore_ascii_case("auto") => windows_region(),
            c => Some(c.to_ascii_uppercase()),
        }
    }

    /// Korean air-quality grades (true) or US AQI (false); resolved once (settings changes
    /// restart the process).
    pub fn korean_air_scale(&self) -> bool {
        static K: OnceLock<bool> = OnceLock::new();
        *K.get_or_init(|| match self.air_quality.scale.trim().to_ascii_lowercase().as_str() {
            "kr" => true,
            "us" => false,
            _ => windows_region().as_deref() == Some("KR"),
        })
    }
}

/// Windows locale ("ko-KR", "en-US", "zh-Hant-TW", ...).
pub fn windows_locale() -> String {
    use windows::Win32::Globalization::GetUserDefaultLocaleName;
    let mut buf = [0u16; 85];
    let n = unsafe { GetUserDefaultLocaleName(&mut buf) };
    if n <= 1 {
        return "en-US".into();
    }
    String::from_utf16_lossy(&buf[..n as usize - 1])
}

/// Region part of the Windows locale ("KR" from "ko-KR").
pub fn windows_region() -> Option<String> {
    windows_locale().rsplit('-').next().filter(|r| r.len() == 2).map(str::to_ascii_uppercase)
}

pub fn dir() -> PathBuf {
    PathBuf::from(std::env::var("APPDATA").unwrap_or_default()).join("TurzxDashboard")
}

pub fn path() -> PathBuf {
    dir().join("config.toml")
}

/// The settings file written on the first run: every key, with what it does.
fn template(c: &Config) -> String {
    let opt = |v: Option<f64>| v.map_or("# not set: looked up from city".to_string(), |v| v.to_string());
    format!(
        r#"# TURZX AI monitor settings. Save the file and the dashboard restarts with them.

# Language: "auto" (Windows display language) or one of
# en, ko, ja, zh-CN, zh-TW, es, fr, de, it, pt, ru, pl, tr, nl, vi, id
language = "{lang}"

# Clock: "12h" or "24h"
clock = "{clock}"

# Temperature unit: "C" or "F"
temperature = "{temp}"

[location]
# Weather and air quality for this city (looked up by name), or set latitude/longitude
city = "{city}"
{lat_key} = {lat}
{lon_key} = {lon}

[holidays]
# Public holidays shown in red: an ISO country code ("KR", "US", "JP", "DE", ...),
# "auto" (the Windows region) or "none"
country = "{country}"

[air_quality]
# "auto" (Korean grades in Korea, US AQI elsewhere), "kr" or "us"
scale = "{scale}"

[ai]
# Which plan usages to read (each reads that tool's local login; see README)
claude = {claude}
codex = {codex}
antigravity = {agy}

[fans]
# Names for the two board fan channels and the GPU fan, e.g. ["Radiator", "Pump", "GPU"];
# empty = default names in the chosen language
labels = [{labels}]
"#,
        lang = c.language,
        clock = c.clock,
        temp = c.temperature,
        city = c.location.city,
        lat_key = if c.location.latitude.is_some() { "latitude" } else { "# latitude" },
        lon_key = if c.location.longitude.is_some() { "longitude" } else { "# longitude" },
        lat = opt(c.location.latitude),
        lon = opt(c.location.longitude),
        country = c.holidays.country,
        scale = c.air_quality.scale,
        claude = c.ai.claude,
        codex = c.ai.codex,
        agy = c.ai.antigravity,
        labels = c.fans.labels.iter().map(|l| format!("{l:?}")).collect::<Vec<_>>().join(", "),
    )
}

fn load() -> Config {
    let p = path();
    match std::fs::read_to_string(&p) {
        Ok(text) => toml::from_str(&text).unwrap_or_else(|e| {
            log::warn!("{}: {e}; using defaults", p.display());
            Config::default()
        }),
        Err(_) => {
            let c = Config::default();
            let _ = std::fs::create_dir_all(dir());
            match std::fs::write(&p, template(&c)) {
                Ok(()) => log::info!("wrote default settings to {}", p.display()),
                Err(e) => log::warn!("could not write {}: {e}", p.display()),
            }
            c
        }
    }
}

/// The settings, loaded on first use.
pub fn get() -> &'static Config {
    static C: OnceLock<Config> = OnceLock::new();
    C.get_or_init(load)
}

fn modified() -> Option<SystemTime> {
    std::fs::metadata(path()).and_then(|m| m.modified()).ok()
}

/// Exit (code 0) when the settings file changes; the supervisor starts the dashboard again
/// right away, so every setting takes effect without special handling.
pub fn watch() {
    let start = modified();
    std::thread::Builder::new()
        .name("config".into())
        .spawn(move || loop {
            std::thread::sleep(Duration::from_secs(2));
            if modified() != start {
                std::thread::sleep(Duration::from_millis(500)); // let the editor finish writing
                log::info!("settings changed: restarting");
                std::process::exit(0);
            }
        })
        .expect("spawn thread");
}
