//! What the panel shows while the PC is off: a video kept on the panel's TF card. At shutdown
//! the dashboard tells the panel to play it, and the panel goes on playing it by itself after
//! the PC is gone. The video in use is recorded in `%LOCALAPPDATA%\TurzxDashboard\offline_video.txt`
//! (its path on the panel, then the date it was made for) when it is uploaded.
//!
//! The video is made again every day (the calendar in it changes): a low-priority thread builds
//! it and the frame loop uploads it to the panel's TF card a megabyte at a time between frames,
//! alternating between two file names so a shutdown during the upload still has a whole video.

pub mod decode;
pub mod encode;

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::panel::lcd;

fn record() -> PathBuf {
    PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_default()).join(r"TurzxDashboard\offline_video.txt")
}

/// Path on the panel of the video to play while the PC is off, if one was uploaded.
pub fn video() -> Option<String> {
    let text = std::fs::read_to_string(record()).ok()?;
    let path = text.lines().next()?.trim();
    (path.starts_with('/') && path.ends_with(".h264")).then(|| path.to_string())
}

/// The date ("YYYY-MM-DD") the current video was made for.
fn video_date() -> Option<String> {
    std::fs::read_to_string(record()).ok()?.lines().nth(1).map(|d| d.trim().to_string())
}

/// Remember the uploaded video (its path on the panel) and the day it was made for.
fn set_video(path: &str, date: &str) -> std::io::Result<()> {
    if let Some(dir) = record().parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(record(), format!("{path}\n{date}\n"))
}

const VIDEO_DIR: &str = "/tmp/sdcard/mmcblk0p1/video";
/// One megabyte every this often: each takes about 0.3 s, during which no frame is sent.
const CHUNK_EVERY: Duration = Duration::from_secs(2);

/// A finished video on its way to the panel.
pub struct Upload {
    path: String,
    date: String,
    data: Vec<u8>,
    sent: usize,
    started: bool,
    next: Instant,
}

/// The video the builder made, waiting for the frame loop.
static PENDING: Mutex<Option<Upload>> = Mutex::new(None);
/// An upload is under way (the builder waits for it).
static BUSY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

impl Upload {
    /// The next video to send, if the builder has one.
    pub fn take() -> Option<Upload> {
        PENDING.lock().unwrap().take()
    }

    /// Send the next megabyte when it is time; true when the whole file is on the panel.
    pub fn step(&mut self, panel: &mut lcd::Lcd) -> anyhow::Result<bool> {
        if Instant::now() < self.next {
            return Ok(false);
        }
        if !self.started {
            let st = panel.storage()?;
            anyhow::ensure!(st.sd_free as usize > self.data.len() / 1024 + 1024, "no TF card or not enough space ({} KB free)", st.sd_free);
            panel.begin_file(&self.path)?;
            self.started = true;
            log::info!("offline video: uploading {} ({} KB)", self.path, self.data.len() / 1024);
        }
        let end = (self.sent + lcd::CHUNK).min(self.data.len());
        panel.file_chunk(&self.data[self.sent..end], end == self.data.len())?;
        self.sent = end;
        self.next = Instant::now() + CHUNK_EVERY;
        if self.sent < self.data.len() {
            return Ok(false);
        }
        let old = video();
        set_video(&self.path, &self.date)?;
        // a video under another name (an earlier test) is not needed any more
        if let Some(old) = old.filter(|o| !o.contains("/turzx_movies_")) {
            if let Err(e) = panel.delete(&old) {
                log::warn!("could not delete {old}: {e:#}");
            }
        }
        BUSY.store(false, std::sync::atomic::Ordering::Relaxed);
        log::info!("offline video ready: {}", self.path);
        Ok(true)
    }

    /// Give up (the builder tries again later).
    pub fn abandon(self, why: &anyhow::Error) {
        log::warn!("offline video upload of {} stopped: {why:#}", self.path);
        BUSY.store(false, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Build today's video in the background whenever there is none for today (needs the TMDB key).
pub fn start() {
    if crate::config::get().offline.tmdb_key.trim().is_empty() {
        return;
    }
    std::thread::Builder::new()
        .name("offline video".into())
        .spawn(|| {
            lower_priority();
            std::thread::sleep(Duration::from_secs(60)); // let the dashboard settle first
            loop {
                let now = chrono::Local::now();
                let today = now.format("%Y-%m-%d").to_string();
                let busy = BUSY.load(std::sync::atomic::Ordering::Relaxed);
                let wait = if busy || video_date().as_deref() == Some(today.as_str()) {
                    Duration::from_secs(600)
                } else {
                    match build(&now) {
                        Ok(data) => {
                            // the other of the two names, so the current video stays whole
                            let path = if video().is_some_and(|v| v.ends_with("_a.h264")) { "turzx_movies_b.h264" } else { "turzx_movies_a.h264" };
                            log::info!("offline video built: {} KB", data.len() / 1024);
                            BUSY.store(true, std::sync::atomic::Ordering::Relaxed);
                            *PENDING.lock().unwrap() =
                                Some(Upload { path: format!("{VIDEO_DIR}/{path}"), date: today, data, sent: 0, started: false, next: Instant::now() });
                            Duration::from_secs(600)
                        }
                        Err(e) => {
                            log::warn!("offline video: {e:#}");
                            Duration::from_secs(3600)
                        }
                    }
                };
                std::thread::sleep(wait);
            }
        })
        .expect("spawn thread");
}

/// Build at the lowest priority, in background mode (low I/O and memory priority too).
fn lower_priority() {
    use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN, THREAD_PRIORITY_LOWEST};
    unsafe {
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN);
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_LOWEST);
    }
}

/// Frames per second of the video (what the vendor app uses as well).
pub const FPS: u32 = 30;

/// Longest part of a clip that is used.
const CLIP_MAX_S: f64 = 30.0;

/// The clip folder (`[offline] videos`, or `%LOCALAPPDATA%\TurzxDashboard\videos`).
pub fn videos_dir() -> PathBuf {
    let dir = crate::config::get().offline.videos.trim().to_string();
    if dir.is_empty() {
        PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_default()).join(r"TurzxDashboard\videos")
    } else {
        PathBuf::from(dir)
    }
}

/// Details of a clip, from `<name>.json` next to `<name>.mp4`.
#[derive(serde::Deserialize, Default)]
#[serde(default)]
struct ClipInfo {
    title: String,
    release: String,
    now_playing: bool,
    rating: f32,
    overview: String,
}

/// The clips in the folder, in name order.
fn clips() -> Vec<(PathBuf, crate::ui::Clip)> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(videos_dir())
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("mp4")))
        .collect();
    files.sort();
    files
        .into_iter()
        .filter_map(|p| {
            let info: ClipInfo = serde_json::from_slice(&std::fs::read(p.with_extension("json")).ok()?).ok()?;
            Some((p, crate::ui::Clip { title: info.title, now_playing: info.now_playing, release: info.release, rating: info.rating, overview: info.overview }))
        })
        .collect()
}

/// Run `[offline] fetch` (hidden), waiting up to 15 minutes.
fn fetch() {
    let cmd = crate::config::get().offline.fetch.trim().to_string();
    if cmd.is_empty() {
        return;
    }
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let started = Instant::now();
    // /S with the whole command quoted: cmd removes only those outer quotes
    match std::process::Command::new("cmd").raw_arg(format!("/S /C \"{cmd}\"")).creation_flags(CREATE_NO_WINDOW).spawn() {
        Ok(mut child) => loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    log::info!("offline fetch: {status} after {:.0}s", started.elapsed().as_secs_f64());
                    break;
                }
                Ok(None) if started.elapsed() > Duration::from_secs(900) => {
                    let _ = child.kill();
                    log::warn!("offline fetch: stopped after 15 minutes");
                    break;
                }
                Ok(None) => std::thread::sleep(Duration::from_secs(2)),
                Err(e) => {
                    log::warn!("offline fetch: {e}");
                    break;
                }
            }
        },
        Err(e) => log::warn!("offline fetch: {e}"),
    }
}

/// Make today's video as H.264 for the panel: the date and calendar with the clips in the clip
/// folder, or else this week's movies as stills (TMDB, needs `[offline] tmdb_key`).
pub fn build(now: &chrono::DateTime<chrono::Local>) -> anyhow::Result<Vec<u8>> {
    fetch();
    // today's and tomorrow's forecast under the calendar (left out if the weather is unreachable)
    let weather = crate::sources::weather::fetch().inspect_err(|e| log::warn!("offline video weather: {e:#}")).ok();
    let clips = clips();
    if !clips.is_empty() {
        return build_clips(now, weather.as_ref(), &clips);
    }
    build_stills(now, weather.as_ref())
}

fn build_clips(
    now: &chrono::DateTime<chrono::Local>,
    weather: Option<&crate::sources::weather::Weather>,
    clips: &[(PathBuf, crate::ui::Clip)],
) -> anyhow::Result<Vec<u8>> {
    let mut enc = encode::H264::new(FPS, 4_000_000, crate::config::get().rotation == 180)?;
    let fade = crate::ui::CLIP_FADE_S as f64;
    let mut used = 0;
    for (path, info) in clips {
        let mut video = match decode::Video::open(path) {
            Ok(v) => v,
            Err(e) => {
                log::warn!("offline clip {}: {e:#}", path.display());
                continue;
            }
        };
        let len = video.duration().unwrap_or(CLIP_MAX_S).min(CLIP_MAX_S);
        let base = crate::ui::clip_base(now, weather, info);
        let Some((start, mut cur)) = video.next()? else { continue }; // start: the first frame's time stamp
        let mut ahead = video.next()?;
        let frames = (len * FPS as f64) as u32;
        for f in 0..frames {
            let t = f as f64 / FPS as f64; // seconds into the clip
            // advance to the source frame showing at t
            while let Some((nt, _)) = &ahead {
                if *nt - start > t {
                    break;
                }
                cur = ahead.take().unwrap().1;
                ahead = video.next()?;
            }
            let k = (t / fade).min((len - t) / fade).clamp(0.0, 1.0) as f32;
            enc.push(crate::ui::clip_frame(&base, &cur, k).pixmap())?;
        }
        used += 1;
    }
    anyhow::ensure!(used > 0, "no usable clips in {}", videos_dir().display());
    enc.finish()
}

fn build_stills(now: &chrono::DateTime<chrono::Local>, weather: Option<&crate::sources::weather::Weather>) -> anyhow::Result<Vec<u8>> {
    use crate::sources::movies;
    let key = crate::config::get().offline.tmdb_key.trim().to_string();
    anyhow::ensure!(!key.is_empty(), "no TMDB key in the settings");
    let slides: Vec<crate::ui::Slide> = movies::load(&key)?
        .into_iter()
        .filter_map(|m| match movies::backdrop(&m) {
            Ok(image) => Some(crate::ui::Slide { title: m.title, now_playing: m.now_playing, release: m.release, rating: m.rating, overview: m.overview, image }),
            Err(e) => {
                log::warn!("movie image: {e:#}");
                None
            }
        })
        .collect();
    anyhow::ensure!(!slides.is_empty(), "no movie images");
    let base = crate::ui::offline_base(now, weather);
    let mut enc = encode::H264::new(FPS, 4_000_000, crate::config::get().rotation == 180)?;
    let frames = (crate::ui::offline_len(slides.len()) * FPS as f32).round() as u32;
    for f in 0..frames {
        enc.push(crate::ui::offline_frame(&base, &slides, f as f32 / FPS as f32).pixmap())?;
    }
    enc.finish()
}
