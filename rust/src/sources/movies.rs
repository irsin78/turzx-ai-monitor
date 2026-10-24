//! Movies for the video shown while the PC is off: upcoming releases and what is in theaters,
//! from TMDB (themoviedb.org; free API key for non-commercial use, credited in the video).
//!
//! The list is cached for a week in `%LOCALAPPDATA%\TurzxDashboard\movies\` with the backdrop
//! images, so the daily video (the calendar changes) needs no network.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tiny_skia::Pixmap;

const API: &str = "https://api.themoviedb.org/3";
const IMAGES: &str = "https://image.tmdb.org/t/p/w1280";
const KEEP: Duration = Duration::from_secs(7 * 24 * 3600);
const UPCOMING: usize = 5;
const NOW_PLAYING: usize = 3;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Movie {
    pub title: String,
    /// "YYYY-MM-DD"
    pub release: String,
    pub now_playing: bool,
    pub rating: f32,
    pub overview: String,
    /// file name of the cached backdrop
    pub image: String,
}

fn dir() -> PathBuf {
    PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_default()).join(r"TurzxDashboard\movies")
}

/// TMDB language and region for the dashboard language ("ko-KR", "KR").
fn locale() -> (String, String) {
    let lang = crate::i18n::get().code;
    let region = crate::config::get().holiday_country().unwrap_or_else(|| "US".into());
    let tag = match lang {
        "zh-CN" | "zh-TW" | "pt" => lang.replace("pt", "pt-BR"),
        l => format!("{l}-{region}"),
    };
    (tag, region)
}

fn get(key: &str, path: &str) -> Result<Value> {
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(20))).build().into();
    let url = format!("{API}{path}");
    // a v4 read token is a JWT ("eyJ..."); otherwise the v3 key goes in the query
    let req = if key.starts_with("eyJ") {
        agent.get(&url).header("Authorization", format!("Bearer {key}"))
    } else {
        agent.get(&format!("{url}&api_key={key}"))
    };
    Ok(req.header("Accept", "application/json").call()?.body_mut().read_json()?)
}

fn movies(list: &Value, now_playing: bool) -> Vec<(Movie, String)> {
    list["results"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let backdrop = m["backdrop_path"].as_str()?.to_string();
            let title = m["title"].as_str().filter(|t| !t.is_empty())?.to_string();
            let movie = Movie {
                title,
                release: m["release_date"].as_str().unwrap_or("").to_string(),
                now_playing,
                rating: m["vote_average"].as_f64().unwrap_or(0.0) as f32,
                overview: m["overview"].as_str().unwrap_or("").trim().to_string(),
                image: backdrop.trim_start_matches('/').to_string(),
            };
            Some((movie, backdrop))
        })
        .collect()
}

/// Fetch the list and the backdrops (network).
fn fetch(key: &str) -> Result<Vec<Movie>> {
    let (lang, region) = locale();
    let q = format!("?language={lang}&region={region}&page=1");
    let mut upcoming = movies(&get(key, &format!("/movie/upcoming{q}"))?, false);
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    upcoming.retain(|(m, _)| m.release >= today);
    upcoming.sort_by(|a, b| a.0.release.cmp(&b.0.release));
    let playing = movies(&get(key, &format!("/movie/now_playing{q}"))?, true); // already by popularity
    let mut picked: Vec<(Movie, String)> = upcoming.into_iter().take(UPCOMING).collect();
    for p in playing {
        if picked.iter().filter(|(m, _)| m.now_playing).count() >= NOW_PLAYING {
            break;
        }
        if !picked.iter().any(|(m, _)| m.title == p.0.title) {
            picked.push(p);
        }
    }
    if picked.is_empty() {
        bail!("TMDB returned no movies");
    }
    std::fs::create_dir_all(dir())?;
    for (m, backdrop) in &picked {
        let file = dir().join(&m.image);
        if !file.exists() {
            let mut body = ureq::get(&format!("{IMAGES}{backdrop}")).call()?.into_body();
            std::fs::write(&file, body.read_to_vec()?).with_context(|| format!("save {}", file.display()))?;
        }
    }
    let list: Vec<Movie> = picked.into_iter().map(|(m, _)| m).collect();
    std::fs::write(dir().join("movies.json"), serde_json::to_vec_pretty(&list)?)?;
    Ok(list)
}

/// The movies, from the cache when it is less than a week old (or TMDB is unreachable).
pub fn load(key: &str) -> Result<Vec<Movie>> {
    let cache = dir().join("movies.json");
    let fresh = std::fs::metadata(&cache).and_then(|m| m.modified()).is_ok_and(|t| t.elapsed().unwrap_or(KEEP) < KEEP);
    let cached = || -> Option<Vec<Movie>> { serde_json::from_slice(&std::fs::read(&cache).ok()?).ok() };
    if fresh {
        if let Some(list) = cached() {
            return Ok(list);
        }
    }
    match fetch(key) {
        Ok(list) => Ok(list),
        Err(e) => cached().filter(|l| !l.is_empty()).ok_or(e),
    }
}

/// A movie's backdrop (decoded from the cache).
pub fn backdrop(m: &Movie) -> Result<Pixmap> {
    let bytes = std::fs::read(dir().join(&m.image))?;
    let mut dec = zune_jpeg::JpegDecoder::new(std::io::Cursor::new(bytes));
    let rgb = dec.decode().map_err(|e| anyhow::anyhow!("{}: {e:?}", m.image))?;
    let info = dec.info().context("jpeg size")?;
    let (w, h) = (info.width as u32, info.height as u32);
    anyhow::ensure!(rgb.len() == (w * h * 3) as usize, "{}: not RGB", m.image);
    let mut pm = Pixmap::new(w, h).context("image size")?;
    for (d, s) in pm.data_mut().chunks_exact_mut(4).zip(rgb.chunks_exact(3)) {
        d.copy_from_slice(&[s[0], s[1], s[2], 255]);
    }
    Ok(pm)
}
