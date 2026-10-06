//! User settings, read from `%APPDATA%\TurzxDashboard\config.toml`. A file with the defaults is
//! written on the first run; missing keys fall back to the defaults.

use std::path::PathBuf;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    pub location: Location,
}

/// Where the weather and air quality are for.
#[derive(Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct Location {
    pub name: String,
    pub latitude: f64,
    pub longitude: f64,
}

impl Default for Config {
    fn default() -> Self {
        Config { location: Location::default() }
    }
}

impl Default for Location {
    fn default() -> Self {
        Location { name: "Seoul".into(), latitude: 37.5665, longitude: 126.978 }
    }
}

pub fn path() -> PathBuf {
    PathBuf::from(std::env::var("APPDATA").unwrap_or_default()).join(r"TurzxDashboard\config.toml")
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
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Ok(text) = toml::to_string_pretty(&c) {
                let _ = std::fs::write(&p, text);
            }
            log::info!("wrote default settings to {}", p.display());
            c
        }
    }
}

/// The settings, loaded on first use.
pub fn get() -> &'static Config {
    static C: OnceLock<Config> = OnceLock::new();
    C.get_or_init(load)
}
