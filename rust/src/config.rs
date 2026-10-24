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
    /// 0 or 180: turn everything on the panel upside down (panel mounted the other way)
    pub rotation: u16,
    pub location: Location,
    pub holidays: Holidays,
    pub air_quality: AirQuality,
    pub ai: Ai,
    pub plan_labels: PlanLabels,
    pub fans: Fans,
    pub disks: Disks,
    pub offline: Offline,
    pub debug: Debug,
}

/// What the panel shows while the PC is off.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Offline {
    /// TMDB API key (v3 key or v4 read token) for the movie video; empty = no movies
    pub tmdb_key: String,
    /// folder of video clips to use instead of movie stills (`<name>.mp4` with `<name>.json`);
    /// empty = `%LOCALAPPDATA%\TurzxDashboard\videos`
    pub videos: String,
    /// command run before the video is made (e.g. something that fills the clip folder)
    pub fetch: String,
}

/// Names shown next to the AI names, keyed by the plan value the service reports; values that
/// are not listed get a readable form of the raw value ("max_5x" -> "Max 5x").
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct PlanLabels {
    pub claude: std::collections::HashMap<String, String>,
    pub codex: std::collections::HashMap<String, String>,
    pub antigravity: std::collections::HashMap<String, String>,
}

impl PlanLabels {
    pub fn lookup<'a>(labels: &'a std::collections::HashMap<String, String>, raw: &str) -> Option<&'a str> {
        labels.get(raw).map(|s| s.trim()).filter(|s| !s.is_empty())
    }
}

/// "max_5x" / "free-tier" -> "Max 5x" / "Free Tier".
pub fn readable_plan(raw: &str) -> String {
    raw.split(['_', '-'])
        .flat_map(str::split_whitespace)
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map(|c| c.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Diagnostics (not in the written template).
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Debug {
    /// log frame timing every 10 s
    pub perf: bool,
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

/// SSD temperatures in the network row.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Disks {
    /// drive letters of up to two drives, e.g. ["C", "D"]; empty = the first two internal drives
    pub show: Vec<String>,
    /// names to show for them; empty = the drive model, shortened
    pub labels: Vec<String>,
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
            rotation: 0,
            location: Location::default(),
            holidays: Holidays { country: "auto".into() },
            air_quality: AirQuality { scale: "auto".into() },
            ai: Ai { claude: true, codex: true, antigravity: true },
            plan_labels: PlanLabels::default(),
            fans: Fans { labels: Vec::new() },
            disks: Disks::default(),
            offline: Offline::default(),
            debug: Debug::default(),
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
pub fn template(c: &Config) -> String {
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

# Panel rotation: 0, or 180 when the panel is mounted upside down
rotation = {rotation}

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

[plan_labels]
# Name next to each AI service, by the plan value it reports (claude: rateLimitTier, then
# subscriptionType; codex: plan_type; antigravity: tier id). Unlisted values are shown in a
# readable form; the services do not report prices.
# codex.pro = "Pro 200"
# claude.default_claude_max_5x = "Max 5x"

[fans]
# Names for the two board fan channels and the GPU fan, e.g. ["Radiator", "Pump", "GPU"];
# empty = default names in the chosen language
labels = [{labels}]

[disks]
# SSD temperatures next to the network speeds: drive letters of up to two drives,
# e.g. ["C", "D"]; empty = the first two internal drives
show = [{disks}]
# Names for them, e.g. ["System", "Games"]; empty = the drive model ("990 PRO")
labels = [{disk_labels}]

[offline]
# While the PC is off the panel plays a video of upcoming movies, made from TMDB
# (themoviedb.org, free API key for personal use; also in the settings window)
tmdb_key = "{tmdb_key}"
"#,
        lang = c.language,
        clock = c.clock,
        temp = c.temperature,
        rotation = c.rotation,
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
        disks = c.disks.show.iter().map(|l| format!("{l:?}")).collect::<Vec<_>>().join(", "),
        disk_labels = c.disks.labels.iter().map(|l| format!("{l:?}")).collect::<Vec<_>>().join(", "),
        tmdb_key = c.offline.tmdb_key,
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
