//! Public holidays of the country in the settings, shown in red on the calendar.
//!
//! Korea uses the table below (incl. substitute holidays, 2026-2040; generated from the Python
//! `holidays` package v0.105; regenerate when temporary holidays such as election days are
//! announced). Other countries come from the Nager.Date API (date.nager.at, nationwide
//! holidays only), cached per country and year under %LOCALAPPDATA%\TurzxDashboard\holidays.
//! Rendering never waits for the network: a year is fetched in the background on first use.

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

use serde_json::Value;

use crate::config;

pub const HOLIDAYS: &[(u16, u8, u8)] = &[
    (2026, 1, 1), (2026, 2, 16), (2026, 2, 17), (2026, 2, 18), (2026, 3, 1), (2026, 3, 2), (2026, 5, 1), (2026, 5, 5),
    (2026, 5, 24), (2026, 5, 25), (2026, 6, 3), (2026, 6, 6), (2026, 7, 17), (2026, 8, 15), (2026, 8, 17), (2026, 9, 24),
    (2026, 9, 25), (2026, 9, 26), (2026, 10, 3), (2026, 10, 5), (2026, 10, 9), (2026, 12, 25), (2027, 1, 1), (2027, 2, 6),
    (2027, 2, 7), (2027, 2, 8), (2027, 2, 9), (2027, 3, 1), (2027, 5, 1), (2027, 5, 3), (2027, 5, 5), (2027, 5, 13),
    (2027, 6, 6), (2027, 7, 17), (2027, 7, 19), (2027, 8, 15), (2027, 8, 16), (2027, 9, 14), (2027, 9, 15), (2027, 9, 16),
    (2027, 10, 3), (2027, 10, 4), (2027, 10, 9), (2027, 10, 11), (2027, 12, 25), (2027, 12, 27), (2028, 1, 1), (2028, 1, 26),
    (2028, 1, 27), (2028, 1, 28), (2028, 3, 1), (2028, 4, 12), (2028, 5, 1), (2028, 5, 2), (2028, 5, 5), (2028, 6, 6),
    (2028, 7, 17), (2028, 8, 15), (2028, 10, 2), (2028, 10, 3), (2028, 10, 4), (2028, 10, 5), (2028, 10, 9), (2028, 12, 25),
    (2029, 1, 1), (2029, 2, 12), (2029, 2, 13), (2029, 2, 14), (2029, 3, 1), (2029, 5, 1), (2029, 5, 5), (2029, 5, 7),
    (2029, 5, 20), (2029, 5, 21), (2029, 6, 6), (2029, 7, 17), (2029, 8, 15), (2029, 9, 21), (2029, 9, 22), (2029, 9, 23),
    (2029, 9, 24), (2029, 10, 3), (2029, 10, 9), (2029, 12, 25), (2030, 1, 1), (2030, 2, 2), (2030, 2, 3), (2030, 2, 4),
    (2030, 2, 5), (2030, 3, 1), (2030, 4, 3), (2030, 5, 1), (2030, 5, 5), (2030, 5, 6), (2030, 5, 9), (2030, 6, 6),
    (2030, 6, 12), (2030, 7, 17), (2030, 8, 15), (2030, 9, 11), (2030, 9, 12), (2030, 9, 13), (2030, 10, 3), (2030, 10, 9),
    (2030, 12, 25), (2031, 1, 1), (2031, 1, 22), (2031, 1, 23), (2031, 1, 24), (2031, 3, 1), (2031, 3, 3), (2031, 5, 1),
    (2031, 5, 5), (2031, 5, 28), (2031, 6, 6), (2031, 7, 17), (2031, 8, 15), (2031, 9, 30), (2031, 10, 1), (2031, 10, 2),
    (2031, 10, 3), (2031, 10, 9), (2031, 12, 25), (2032, 1, 1), (2032, 2, 10), (2032, 2, 11), (2032, 2, 12), (2032, 3, 1),
    (2032, 4, 14), (2032, 5, 1), (2032, 5, 3), (2032, 5, 5), (2032, 5, 16), (2032, 5, 17), (2032, 6, 6), (2032, 7, 17),
    (2032, 7, 19), (2032, 8, 15), (2032, 8, 16), (2032, 9, 18), (2032, 9, 19), (2032, 9, 20), (2032, 9, 21), (2032, 10, 3),
    (2032, 10, 4), (2032, 10, 9), (2032, 10, 11), (2032, 12, 25), (2032, 12, 27), (2033, 1, 1), (2033, 1, 30), (2033, 1, 31),
    (2033, 2, 1), (2033, 2, 2), (2033, 3, 1), (2033, 5, 1), (2033, 5, 2), (2033, 5, 5), (2033, 5, 6), (2033, 6, 6),
    (2033, 7, 17), (2033, 7, 18), (2033, 8, 15), (2033, 9, 7), (2033, 9, 8), (2033, 9, 9), (2033, 10, 3), (2033, 10, 9),
    (2033, 10, 10), (2033, 12, 25), (2033, 12, 26), (2034, 1, 1), (2034, 2, 18), (2034, 2, 19), (2034, 2, 20), (2034, 2, 21),
    (2034, 3, 1), (2034, 5, 1), (2034, 5, 5), (2034, 5, 25), (2034, 6, 6), (2034, 6, 14), (2034, 7, 17), (2034, 8, 15),
    (2034, 9, 26), (2034, 9, 27), (2034, 9, 28), (2034, 10, 3), (2034, 10, 9), (2034, 12, 25), (2035, 1, 1), (2035, 2, 7),
    (2035, 2, 8), (2035, 2, 9), (2035, 3, 1), (2035, 4, 4), (2035, 5, 1), (2035, 5, 5), (2035, 5, 7), (2035, 5, 15),
    (2035, 6, 6), (2035, 7, 17), (2035, 8, 15), (2035, 9, 15), (2035, 9, 16), (2035, 9, 17), (2035, 9, 18), (2035, 10, 3),
    (2035, 10, 9), (2035, 12, 25), (2036, 1, 1), (2036, 1, 27), (2036, 1, 28), (2036, 1, 29), (2036, 1, 30), (2036, 3, 1),
    (2036, 3, 3), (2036, 4, 9), (2036, 5, 1), (2036, 5, 3), (2036, 5, 5), (2036, 5, 6), (2036, 6, 6), (2036, 7, 17),
    (2036, 8, 15), (2036, 10, 3), (2036, 10, 4), (2036, 10, 5), (2036, 10, 6), (2036, 10, 7), (2036, 10, 9), (2036, 12, 25),
    (2037, 1, 1), (2037, 2, 14), (2037, 2, 15), (2037, 2, 16), (2037, 2, 17), (2037, 3, 1), (2037, 3, 2), (2037, 5, 1),
    (2037, 5, 5), (2037, 5, 22), (2037, 6, 6), (2037, 7, 17), (2037, 8, 15), (2037, 8, 17), (2037, 9, 23), (2037, 9, 24),
    (2037, 9, 25), (2037, 10, 3), (2037, 10, 5), (2037, 10, 9), (2037, 12, 25), (2038, 1, 1), (2038, 2, 3), (2038, 2, 4),
    (2038, 2, 5), (2038, 3, 1), (2038, 5, 1), (2038, 5, 3), (2038, 5, 5), (2038, 5, 11), (2038, 6, 2), (2038, 6, 6),
    (2038, 7, 17), (2038, 7, 19), (2038, 8, 15), (2038, 8, 16), (2038, 9, 12), (2038, 9, 13), (2038, 9, 14), (2038, 9, 15),
    (2038, 10, 3), (2038, 10, 4), (2038, 10, 9), (2038, 10, 11), (2038, 12, 25), (2038, 12, 27), (2039, 1, 1), (2039, 1, 23),
    (2039, 1, 24), (2039, 1, 25), (2039, 1, 26), (2039, 3, 1), (2039, 4, 30), (2039, 5, 1), (2039, 5, 2), (2039, 5, 3),
    (2039, 5, 5), (2039, 6, 6), (2039, 7, 17), (2039, 7, 18), (2039, 8, 15), (2039, 10, 1), (2039, 10, 2), (2039, 10, 3),
    (2039, 10, 4), (2039, 10, 5), (2039, 10, 9), (2039, 10, 10), (2039, 12, 25), (2039, 12, 26), (2040, 1, 1), (2040, 2, 11),
    (2040, 2, 12), (2040, 2, 13), (2040, 2, 14), (2040, 3, 1), (2040, 4, 4), (2040, 4, 11), (2040, 5, 1), (2040, 5, 5),
    (2040, 5, 7), (2040, 5, 18), (2040, 6, 6), (2040, 7, 17), (2040, 8, 15), (2040, 9, 20), (2040, 9, 21), (2040, 9, 22),
    (2040, 10, 3), (2040, 10, 9), (2040, 12, 25),
];

fn korea(y: i32, m: u32, d: u32) -> bool {
    HOLIDAYS.binary_search(&(y as u16, m as u8, d as u8)).is_ok()
}

/// One country-year: loading, the holiday dates, or a failure with a time to try again.
enum Year {
    Loading,
    Ready(HashSet<(u32, u32)>),
    Failed(std::time::Instant),
}

type Years = HashMap<i32, Year>;

fn years() -> &'static Mutex<Years> {
    static Y: OnceLock<Mutex<Years>> = OnceLock::new();
    Y.get_or_init(|| Mutex::new(HashMap::new()))
}

fn country() -> Option<&'static str> {
    static C: OnceLock<Option<String>> = OnceLock::new();
    C.get_or_init(|| config::get().holiday_country()).as_deref()
}

fn cache_file(country: &str, year: i32) -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_default())
        .join(r"TurzxDashboard\holidays")
        .join(format!("{country}-{year}.json"))
}

fn parse(json: &str) -> Option<HashSet<(u32, u32)>> {
    let v: Value = serde_json::from_str(json).ok()?;
    Some(
        v.as_array()?
            .iter()
            .filter(|h| h["global"].as_bool().unwrap_or(true))
            .filter_map(|h| {
                let d = h["date"].as_str()?; // "2026-01-19"
                Some((d.get(5..7)?.parse().ok()?, d.get(8..10)?.parse().ok()?))
            })
            .collect(),
    )
}

/// Load one year from the cache file, or download it (on a background thread).
fn fetch(country: &'static str, year: i32) {
    std::thread::spawn(move || {
        let file = cache_file(country, year);
        let days = std::fs::read_to_string(&file).ok().and_then(|t| parse(&t)).or_else(|| {
            let url = format!("https://date.nager.at/api/v3/PublicHolidays/{year}/{country}");
            let text = ureq::get(&url).call().ok()?.body_mut().read_to_string().ok()?;
            let days = parse(&text)?;
            let _ = std::fs::create_dir_all(file.parent()?);
            let _ = std::fs::write(&file, &text);
            log::info!("holidays {country} {year}: {} days", days.len());
            Some(days)
        });
        let state = match days {
            Some(d) => Year::Ready(d),
            None => {
                log::warn!("holidays {country} {year}: not available, trying again later");
                Year::Failed(std::time::Instant::now())
            }
        };
        years().lock().unwrap().insert(year, state);
    });
}

/// Holidays of a month as a bit set: bit `d` is day `d` (1..=31).
pub fn month(y: i32, m: u32) -> u32 {
    let Some(country) = country() else { return 0 };
    if country == "KR" {
        return (1..=31).filter(|&d| korea(y, m, d)).fold(0, |acc, d| acc | 1 << d);
    }
    const RETRY: std::time::Duration = std::time::Duration::from_secs(600);
    let mut map = years().lock().unwrap();
    match map.get(&y) {
        Some(Year::Ready(days)) => days.iter().filter(|(mm, _)| *mm == m).fold(0, |acc, (_, d)| acc | 1 << d),
        Some(Year::Loading) => 0,
        Some(Year::Failed(at)) if at.elapsed() < RETRY => 0,
        _ => {
            map.insert(y, Year::Loading);
            drop(map);
            fetch(country, y);
            0
        }
    }
}
