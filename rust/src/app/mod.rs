//! The dashboard process: collectors fill the shared state, the render loop draws it and
//! sends frames to the panel, and the session watcher shows the standby screen at shutdown.

mod collectors;
mod mmcss;
mod perf;
mod session;
pub mod supervisor;


use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::panel::{jpeg, lcd};
use crate::state::State;
use crate::ui;
use collectors::start_collectors;
use session::{end_session, wait, watch_end_session};
use supervisor::start_watchdog;

type Shared = Arc<Mutex<State>>;

/// Panel frame interval while unlocked: 12 fps for the mascot animation (the panel takes
/// ~37 fps), or one frame a second when built without mascots.
const FRAME: Duration = if crate::mascot::ENABLED { Duration::from_millis(83) } else { Duration::from_secs(1) };
/// How far a mascot may reach beyond its strip (widest sprite: Clawd's confetti at 2x).
const MASCOT_MARGIN: f32 = 64.0;

/// The first slot after `finished` on the grid `scheduled + k * period`.
fn advance_tick(scheduled: Instant, finished: Instant, period: Duration) -> Instant {
    let elapsed = finished.saturating_duration_since(scheduled).as_nanos();
    scheduled + period * ((elapsed / period.as_nanos()) as u32 + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slow_frames_drop_only_elapsed_slots_and_keep_the_phase() {
        let t0 = Instant::now();
        let p = Duration::from_millis(83);
        assert_eq!(advance_tick(t0, t0 + Duration::from_millis(10), p), t0 + p); // on time
        assert_eq!(advance_tick(t0, t0 + Duration::from_millis(100), p), t0 + 2 * p); // one slot late
        assert_eq!(advance_tick(t0, t0 + Duration::from_millis(250), p), t0 + 4 * p); // phase kept
    }
}

/// Reconnect delay after `attempts` failed opens: quick at first (a knock usually re-enumerates
/// within a second or two), then every 5 s while it stays unplugged.
fn backoff(attempts: u32) -> Duration {
    const STEPS_MS: [u64; 6] = [300, 500, 1000, 2000, 3000, 5000];
    Duration::from_millis(STEPS_MS[(attempts as usize).min(STEPS_MS.len() - 1)])
}

/// Render and send a frame every second; reconnect quickly when the panel drops out.
pub fn run(ai_enabled: bool) -> ! {
    let st: Shared = Arc::new(Mutex::new(State::new()));
    start_collectors(&st, ai_enabled);
    let ending = watch_end_session();
    let beat = start_watchdog();
    let mut panel: Option<lcd::Lcd> = None;
    let mut lost: Option<(Instant, String)> = None; // when the panel went away and the last reason
    let mut attempts = 0u32;
    let mut blank = false; // locked, monitors off or screen saver: black panel
    let mut blank_checked: Option<Instant> = None;
    // Frames: the dashboard is redrawn once a second into `base` and encoded whole. In between,
    // only the mascot band is restored from `base`, the mascots drawn on top and the JPEG bands
    // under it re-encoded; the rest of the frame reuses the last encoded bytes.
    let areas = ui::mascot_areas();
    let band_px = (
        areas.iter().map(|a| a.x0).fold(f32::MAX, f32::min) - MASCOT_MARGIN,
        areas.iter().map(|a| a.x1).fold(0.0, f32::max) + MASCOT_MARGIN,
    );
    let band = jpeg::bands_for(band_px.0, band_px.1);
    let band_cols = band.start * 8..band.end * 8; // landscape columns
    let mut mascots = crate::mascot::Mascots::new(&areas);
    let mut enc = jpeg::RowJpeg::new();
    let mut black: Option<Vec<u8>> = None;
    let mut drawn_at: Option<i64> = None; // second of the last full redraw
    let mut work: Option<ui::Canvas> = None;
    let mut band_backup: Vec<u8> = Vec::new(); // the mascot band as redrawn, without mascots
    let mut last_tick = Instant::now();
    // `[debug] perf = true`: frame timing in the log every 10 s
    let perf_on = crate::config::get().debug.perf;
    // frame pacing: this thread gets multimedia scheduling (EcoQoS stays on for the rest) and
    // 1 ms timer resolution, so 83 ms waits do not round up to the 15.6 ms default tick
    let _mmcss = mmcss::Multimedia::register();
    unsafe {
        windows::Win32::Media::timeBeginPeriod(1);
    }
    let mut next_tick = Instant::now(); // frame slots on a fixed 83 ms grid
    let mut perf = perf::Perf::default();
    let mut intended: Option<Instant> = None; // when this frame was meant to start
    loop {
        let t = Instant::now();
        *beat.lock().unwrap() = t;
        if let Some(i) = intended.take() {
            perf.tick(t, i);
        }
        if panel.is_none() {
            match lcd::Lcd::open() {
                Ok(p) => {
                    match lost.take() {
                        Some((since, _)) => log::info!(
                            "LCD reconnected after {:.1}s ({} attempts), firmware {}, USB {} speed",
                            since.elapsed().as_secs_f64(), attempts + 1, p.firmware, p.speed
                        ),
                        None => log::info!("LCD connected, firmware {}, USB {} speed", p.firmware, p.speed),
                    }
                    panel = Some(p);
                    attempts = 0;
                }
                Err(e) => {
                    // log when the reason changes, not on every retry
                    let reason = format!("{e:#}");
                    match &mut lost {
                        Some((_, last)) if *last == reason => {}
                        Some((_, last)) => {
                            log::warn!("LCD still unavailable: {reason}");
                            *last = reason;
                        }
                        None => {
                            log::warn!("LCD unavailable: {reason}");
                            lost = Some((Instant::now(), reason));
                        }
                    }
                    attempts += 1;
                }
            }
        }
        let delay = match panel.as_mut() {
            Some(p) => {
                // PC locked, monitors off or screen saver: black screen until it ends (checked once a second)
                if blank_checked.is_none_or(|c| c.elapsed() >= Duration::from_secs(1)) {
                    blank_checked = Some(t);
                    let reason = session::blank_reason();
                    if reason.is_some() != blank {
                        match reason {
                            Some(r) => log::info!("{r}: screen off"),
                            None => log::info!("screen on"),
                        }
                        blank = reason.is_some();
                    }
                }
                let dt = last_tick.elapsed().as_secs_f32();
                last_tick = t;
                let sent = if blank {
                    drawn_at = None;
                    let frame = black.get_or_insert_with(|| {
                        let mut e = jpeg::RowJpeg::new();
                        e.encode(&tiny_skia::Pixmap::new(ui::W, ui::H).unwrap(), 0..jpeg::BANDS);
                        e.frame().to_vec()
                    });
                    p.send_jpeg(frame)
                } else {
                    let now = chrono::Local::now();
                    let fresh = drawn_at != Some(now.timestamp());
                    if fresh {
                        let snapshot = st.lock().unwrap().clone(); // render without holding the lock
                        let cv = work.insert(ui::render(&snapshot, now));
                        cv.save_columns(band_cols.clone(), &mut band_backup);
                        drawn_at = Some(now.timestamp());
                    }
                    let cv = work.as_mut().unwrap();
                    if !fresh {
                        cv.load_columns(band_cols.clone(), &band_backup); // erase last frame's mascots
                    }
                    mascots.update(dt, &areas);
                    mascots.draw(cv.pixmap_mut(), &areas);
                    enc.encode(cv.pixmap(), if fresh { 0..jpeg::BANDS } else { band.clone() });
                    perf.render(fresh, t.elapsed());
                    let frame = enc.frame();
                    perf.bytes(frame.len());
                    let s0 = Instant::now();
                    let r = p.send_jpeg(frame);
                    perf.send(fresh, s0.elapsed());
                    r
                };
                match sent {
                    Ok(_) if blank => Duration::from_secs(1),
                    Ok(_) => {
                        // next free slot on the grid; slots already past are dropped, not queued
                        let now = Instant::now();
                        let advanced = advance_tick(next_tick, now, FRAME);
                        perf.skipped(perf::skipped_slots(next_tick + FRAME, advanced, FRAME));
                        next_tick = advanced;
                        next_tick.saturating_duration_since(t)
                    }
                    Err(e) => {
                        log::warn!("LCD lost: {e:#}");
                        panel = None; // drop the handle before reopening
                        lost = Some((Instant::now(), format!("{e:#}")));
                        Duration::from_millis(200)
                    }
                }
            }
            None => backoff(attempts.saturating_sub(1)),
        };
        if panel.is_none() || blank {
            next_tick = t + delay; // off the grid: start a new one when frames resume
        }
        perf.active(perf_on && panel.is_some() && !blank);
        perf.report();
        if perf_on && panel.is_some() && !blank {
            intended = Some(next_tick);
        }
        if let Some(ack) = wait(&ending, delay.saturating_sub(t.elapsed())) {
            end_session(panel.as_mut(), ack);
        }
    }
}
