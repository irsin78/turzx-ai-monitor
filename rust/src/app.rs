//! Collector threads + 1 s render/send loop.

use std::io::Write;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use crate::ai::{self, Antigravity, Cache, Claude, ClaudeCli, Usage};
use crate::hw::HardwareReader;
use crate::state::{AiErr, State};
use crate::{jpeg, lcd, view, weather};

const AI_INTERVAL: f64 = 300.0; // Codex / Antigravity
const CLAUDE_INTERVAL: f64 = 900.0; // Claude's usage API rate-limits hard; 15 min worked for other monitors
const CLAUDE_CLI_INTERVAL: f64 = 300.0; // /usage panel reads while the API is unavailable
const WEATHER_INTERVAL: Duration = Duration::from_secs(600);

type Shared = Arc<Mutex<State>>;

/// Run `f` now (after `first_delay`) and then every `interval`, forever, on its own thread.
fn every(name: &'static str, interval: Duration, first_delay: Duration, mut f: impl FnMut() + Send + 'static) {
    thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            thread::sleep(first_delay);
            loop {
                let t = Instant::now();
                f();
                thread::sleep(interval.saturating_sub(t.elapsed()));
            }
        })
        .expect("spawn thread");
}

fn set_ai(st: &Shared, cache: &Cache, name: &'static str, r: anyhow::Result<Usage>) {
    let mut s = st.lock().unwrap();
    match r {
        Ok(u) => {
            log::info!("{name} usage: {u:?}");
            cache.store(name, &u);
            s.ai.insert(name, u);
            s.ai_err.remove(name);
        }
        Err(e) => {
            if e.downcast_ref::<ai::RateLimited>().is_none() {
                log::warn!("{name} usage failed: {e:#}");
            }
            s.ai_err.insert(name, AiErr::from(&e)); // keep the last value on screen, show why it is stale
        }
    }
}

/// Seconds until shortly after the next 5-hour / weekly reset of `name`: the limit refills
/// then, so read it again for the real value and the next reset time (it may already be in use).
fn until_reset(st: &Shared, name: &str) -> Option<f64> {
    const GRACE: f64 = 30.0; // the CLI shows reset times to the minute
    let now = ai::now();
    let g = st.lock().unwrap();
    let u = g.ai.get(name)?;
    [u.session_reset, u.weekly_reset].into_iter().flatten().filter(|&r| r > now).map(|r| r - now + GRACE).reduce(f64::min)
}

pub fn start_collectors(st: &Shared, ai_enabled: bool) {
    // Hardware every second (also feeds the graph history)
    let s = st.clone();
    let mut hw = HardwareReader::new();
    every("hardware", Duration::from_secs(1), Duration::ZERO, move || {
        let h = hw.read();
        let mut g = s.lock().unwrap();
        g.record(&h);
        g.hw = h;
    });

    let s = st.clone();
    every("weather", WEATHER_INTERVAL, Duration::ZERO, move || match weather::fetch() {
        Ok(w) => s.lock().unwrap().weather = Some(w),
        Err(e) => log::warn!("weather failed: {e:#}"),
    });

    if !ai_enabled {
        return; // --no-ai: test runs without touching credentials
    }
    // AI: show the last saved values right away; don't re-query sooner than the interval
    // after the previous fetch, even across restarts
    let cache = Cache::open();
    let first_delay = |name: &str, interval: f64| {
        let (last, fetched_at) = cache.last(name);
        let wait = (fetched_at + interval - ai::now()).max(0.0);
        (last, Duration::from_secs_f64(wait))
    };

    let mut claude = Claude::new(cache.clone());
    let (last, wait) = first_delay("Claude", CLAUDE_INTERVAL);
    {
        let mut g = st.lock().unwrap();
        if let Some(u) = last {
            g.ai.insert("Claude", u);
        } else if ai::now() < claude.not_before() {
            g.ai_err.insert("Claude", AiErr::RateLimited(claude.not_before()));
        }
    }
    let (s, c) = (st.clone(), cache.clone());
    thread::Builder::new()
        .name("claude".into())
        .spawn(move || {
            thread::sleep(wait);
            loop {
                // Usage API every 15 min. When it fails (rate limit, auth, network), read Claude
                // Code's /usage panel right away and then every 5 min, until the wait the server
                // asked for (Retry-After) is over, or 15 min without one; then the API again.
                let t = Instant::now();
                let api_err = match claude.fetch() {
                    Ok(u) => {
                        set_ai(&s, &c, "Claude", Ok(u));
                        let next = (CLAUDE_INTERVAL - t.elapsed().as_secs_f64()).max(0.0);
                        thread::sleep(Duration::from_secs_f64(until_reset(&s, "Claude").map_or(next, |r| r.min(next))));
                        continue;
                    }
                    Err(e) => e,
                };
                let retry_at = match api_err.downcast_ref::<ai::RateLimited>() {
                    Some(r) => r.0,
                    None => ai::now() + CLAUDE_INTERVAL,
                };
                log::info!("Claude API failed ({api_err:#}); reading /usage via CLI every 5 min, API again in {:.0} min",
                    ((retry_at - ai::now()) / 60.0).max(0.0));
                let mut api_err = Some(api_err);
                loop {
                    let r = match ClaudeCli.fetch(Duration::from_secs(45)) {
                        Ok(u) => Ok(u),
                        Err(cli_err) => {
                            log::warn!("Claude CLI read failed: {cli_err:#}");
                            // keep showing why the API is unavailable
                            Err(api_err.take().unwrap_or_else(|| cli_err))
                        }
                    };
                    set_ai(&s, &c, "Claude", r);
                    let left = retry_at - ai::now();
                    let tick = until_reset(&s, "Claude").map_or(CLAUDE_CLI_INTERVAL, |r| r.min(CLAUDE_CLI_INTERVAL));
                    if left <= tick {
                        thread::sleep(Duration::from_secs_f64(left.max(0.0)));
                        break; // API's turn
                    }
                    thread::sleep(Duration::from_secs_f64(tick));
                }
            }
        })
        .expect("spawn thread");

    let (last, wait) = first_delay("Codex", AI_INTERVAL);
    if let Some(u) = last {
        st.lock().unwrap().ai.insert("Codex", u);
    }
    let (s, c) = (st.clone(), cache.clone());
    every("codex", Duration::from_secs_f64(AI_INTERVAL), wait, move || set_ai(&s, &c, "Codex", ai::codex()));

    let (last, wait) = first_delay("Antigravity", AI_INTERVAL);
    if let Some(u) = last {
        st.lock().unwrap().ai.insert("Antigravity", u);
    }
    let (s, c) = (st.clone(), cache);
    let mut agy = Antigravity::new();
    every("antigravity", Duration::from_secs_f64(AI_INTERVAL), wait, move || set_ai(&s, &c, "Antigravity", agy.fetch()));
}

/// Windows is shutting down / restarting / logging off: the sender acknowledges once the
/// standby screen is on the panel.
type EndSession = Sender<()>;

static END_SESSION: OnceLock<Sender<EndSession>> = OnceLock::new();

/// Hidden top-level window that receives WM_ENDSESSION (a windowless process gets no notice).
fn watch_end_session() -> Receiver<EndSession> {
    use windows::core::w;
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::*;

    unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        match msg {
            WM_QUERYENDSESSION => LRESULT(1),
            WM_ENDSESSION if wp.0 != 0 => {
                // the process may be killed once this returns: wait until the frame is sent
                let (ack, done) = mpsc::channel();
                if END_SESSION.get().is_some_and(|tx| tx.send(ack).is_ok()) {
                    let _ = done.recv_timeout(Duration::from_secs(3));
                }
                LRESULT(0)
            }
            _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
        }
    }

    let (tx, rx) = mpsc::channel();
    let _ = END_SESSION.set(tx);
    thread::Builder::new()
        .name("session".into())
        .spawn(|| unsafe {
            let Ok(inst) = GetModuleHandleW(None) else { return };
            let class = w!("TurzxDashboardSession");
            let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: inst.into(), lpszClassName: class, ..Default::default() };
            RegisterClassW(&wc);
            // never shown; a message-only window would not get the broadcast
            if let Err(e) = CreateWindowExW(WINDOW_EX_STYLE::default(), class, w!("TURZX dashboard"), WS_POPUP, 0, 0, 0, 0, None, None, Some(inst.into()), None) {
                log::warn!("session window failed: {e}");
                return;
            }
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        })
        .expect("spawn thread");
    rx
}

/// Sleep for `d`, or return early with the acknowledgement when the session ends.
fn wait(rx: &Receiver<EndSession>, d: Duration) -> Option<EndSession> {
    match rx.recv_timeout(d) {
        Ok(ack) => Some(ack),
        Err(RecvTimeoutError::Timeout) => None,
        Err(RecvTimeoutError::Disconnected) => {
            thread::sleep(d);
            None
        }
    }
}

/// Show the standby screen, acknowledge, and stop drawing (Windows is about to end us).
fn end_session(panel: Option<&mut lcd::Lcd>, ack: EndSession) -> ! {
    ENDING.store(true, Ordering::Relaxed); // the watchdog must not fire now
    signal_ending(); // and the supervisor must not restart us once Windows kills us
    log::info!("session ending: standby screen");
    let mut reopened;
    let panel = match panel {
        Some(p) => Some(p),
        None => {
            // disconnected right now: one quick try, it may just have come back
            reopened = lcd::Lcd::open().ok();
            reopened.as_mut()
        }
    };
    match panel {
        Some(p) => {
            if let Err(e) = p.show_landscape(&view::standby().rgb()) {
                log::warn!("standby send failed: {e:#}");
            }
        }
        None => log::warn!("standby skipped: LCD not connected"),
    }
    let _ = ack.send(());
    loop {
        thread::sleep(Duration::from_secs(60));
    }
}

static ENDING: AtomicBool = AtomicBool::new(false);

/// A USB call that never returns would freeze the panel for good: if the render loop has not
/// come round for this long, exit and let the supervisor start a fresh worker.
const STUCK_AFTER: Duration = Duration::from_secs(60);

fn start_watchdog() -> Arc<Mutex<Instant>> {
    let beat = Arc::new(Mutex::new(Instant::now()));
    let b = beat.clone();
    every("watchdog", Duration::from_secs(5), STUCK_AFTER, move || {
        let since = b.lock().unwrap().elapsed();
        if since < STUCK_AFTER || ENDING.load(Ordering::Relaxed) {
            return;
        }
        log::error!("render loop stuck for {}s (USB call not returning): exiting for a restart", since.as_secs());
        std::process::exit(3);
    });
    beat
}

/// Panel frame interval while unlocked: 12 fps for the mascot animation (the panel takes
/// ~37 fps), or one frame a second when built without mascots.
const FRAME: Duration = if crate::mascot::ENABLED { Duration::from_millis(83) } else { Duration::from_secs(1) };
/// How far a mascot may reach beyond its strip (widest sprite: Clawd's confetti at 2x).
const MASCOT_MARGIN: f32 = 64.0;

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
    let mut was_locked = false;
    let mut lock_checked: Option<Instant> = None;
    // Frames: the dashboard is redrawn once a second into `base` and encoded whole. In between,
    // only the mascot band is restored from `base`, the mascots drawn on top and the JPEG bands
    // under it re-encoded; the rest of the frame reuses the last encoded bytes.
    let areas = view::mascot_areas();
    let band_px = (
        areas.iter().map(|a| a.x0).fold(f32::MAX, f32::min) - MASCOT_MARGIN,
        areas.iter().map(|a| a.x1).fold(0.0, f32::max) + MASCOT_MARGIN,
    );
    let band = jpeg::bands_for(band_px.0, band_px.1);
    let band_cols = band.start * 8..band.end * 8; // landscape columns
    let mut mascots = crate::mascot::Mascots::new(&areas);
    let mut enc = jpeg::RowJpeg::new();
    let mut black: Option<Vec<u8>> = None;
    let mut base: Option<(i64, view::Canvas)> = None;
    let mut work: Option<view::Canvas> = None;
    let mut last_tick = Instant::now();
    loop {
        let t = Instant::now();
        *beat.lock().unwrap() = t;
        if panel.is_none() {
            match lcd::Lcd::open() {
                Ok(p) => {
                    match lost.take() {
                        Some((since, _)) => log::info!(
                            "LCD reconnected after {:.1}s ({} attempts), firmware {}",
                            since.elapsed().as_secs_f64(), attempts + 1, p.firmware
                        ),
                        None => log::info!("LCD connected, firmware {}", p.firmware),
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
                // PC locked: black screen until unlocked (checked once a second)
                if lock_checked.is_none_or(|c| c.elapsed() >= Duration::from_secs(1)) {
                    lock_checked = Some(t);
                    let locked = session_locked();
                    if locked != was_locked {
                        log::info!("session {}", if locked { "locked: screen off" } else { "unlocked: screen on" });
                        was_locked = locked;
                    }
                }
                let dt = last_tick.elapsed().as_secs_f32();
                last_tick = t;
                let sent = if was_locked {
                    base = None;
                    let frame = black.get_or_insert_with(|| {
                        let mut e = jpeg::RowJpeg::new();
                        e.encode(&tiny_skia::Pixmap::new(view::W, view::H).unwrap(), 0..jpeg::BANDS);
                        e.frame().to_vec()
                    });
                    p.send_jpeg(frame)
                } else {
                    let now = chrono::Local::now();
                    let fresh = base.as_ref().is_none_or(|(sec, _)| *sec != now.timestamp());
                    if fresh {
                        let g = st.lock().unwrap();
                        let cv = view::render(&g, now);
                        work = Some(cv.clone());
                        base = Some((now.timestamp(), cv));
                    }
                    let (cv, (_, b)) = (work.as_mut().unwrap(), base.as_ref().unwrap());
                    if !fresh {
                        cv.restore_columns(b, band_cols.clone()); // erase last frame's mascots
                    }
                    mascots.update(dt, &areas);
                    mascots.draw(cv.pixmap_mut(), &areas);
                    enc.encode(cv.pixmap(), if fresh { 0..jpeg::BANDS } else { band.clone() });
                    p.send_jpeg(enc.frame())
                };
                match sent {
                    Ok(_) if was_locked => Duration::from_secs(1),
                    Ok(_) => FRAME,
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
        if let Some(ack) = wait(&ending, delay.saturating_sub(t.elapsed())) {
            end_session(panel.as_mut(), ack);
        }
    }
}

// --- supervisor ------------------------------------------------------------------------------
//
// The scheduled task starts the supervisor, which only runs the real dashboard (`--worker`) as
// a child and starts it again whenever it exits. A crash inside a driver DLL (nvml.dll died
// during an NVIDIA driver update) takes the whole process down, so recovery has to live in a
// process that touches no drivers.

const ENDING_EVENT: windows::core::PCWSTR = windows::core::w!("Local\\TurzxDashboardEnding");

/// Worker: Windows is ending the session (standby screen sent).
fn signal_ending() {
    use windows::Win32::System::Threading::{OpenEventW, SetEvent, EVENT_MODIFY_STATE};
    unsafe {
        if let Ok(h) = OpenEventW(EVENT_MODIFY_STATE, false, ENDING_EVENT) {
            let _ = SetEvent(h);
        }
    }
}

pub fn supervise() -> ! {
    use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
    let ending = unsafe { CreateEventW(None, true, false, ENDING_EVENT) }.ok();
    let is_ending = || ending.is_some_and(|h| unsafe { WaitForSingleObject(h, 0) }.0 == 0);
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut restarts = 0u32;
    loop {
        let started = Instant::now();
        let status = std::env::current_exe().and_then(|exe| std::process::Command::new(exe).args(&args).arg("--worker").status());
        let ran = started.elapsed();
        if is_ending() {
            log::info!("supervisor: session ending, not restarting");
            loop {
                thread::sleep(Duration::from_secs(60));
            }
        }
        let how = match &status {
            Ok(s) => match s.code() {
                Some(c) => format!("exit code {c} (0x{:08X})", c as u32),
                None => "no exit code".into(),
            },
            Err(e) => format!("could not start: {e}"),
        };
        // a worker that ran a while counts as healthy: start over with a short delay
        if ran > Duration::from_secs(120) {
            restarts = 0;
        }
        let delay = Duration::from_secs([2, 5, 10, 30][(restarts as usize).min(3)]);
        restarts += 1;
        log::error!("worker ended after {:.0}s: {how}; restarting in {}s", ran.as_secs_f64(), delay.as_secs());
        thread::sleep(delay);
    }
}

/// True while this Windows session shows the lock screen.
pub fn session_locked() -> bool {
    use windows::Win32::System::RemoteDesktop::*;
    unsafe {
        let mut buf = windows::core::PWSTR::null();
        let mut len = 0u32;
        if WTSQuerySessionInformationW(Some(WTS_CURRENT_SERVER_HANDLE), WTS_CURRENT_SESSION, WTSSessionInfoEx, &mut buf, &mut len).is_err() {
            return false;
        }
        let info = &*(buf.0 as *const WTSINFOEXW);
        // Windows 8+: SessionFlags is WTS_SESSIONSTATE_LOCK (0) or WTS_SESSIONSTATE_UNLOCK (1)
        let locked = info.Level == 1 && info.Data.WTSInfoExLevel1.SessionFlags == WTS_SESSIONSTATE_LOCK as i32;
        WTSFreeMemory(buf.0 as *mut _);
        locked
    }
}

/// Log to %LOCALAPPDATA%\TurzxDashboard\dashboard.log (and stderr when there is a console).
pub fn init_logging() {
    struct Tee(std::fs::File);
    impl Write for Tee {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            let _ = std::io::stderr().write_all(buf);
            self.0.write(buf)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.0.flush()
        }
    }
    let dir = std::path::PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_default()).join("TurzxDashboard");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("dashboard.log");
    // keep the log small: start over once it passes 1 MB
    let append = std::fs::metadata(&path).map(|m| m.len() < 1_000_000).unwrap_or(false);
    let mut b = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,tiny_skia=error"));
    if let Ok(f) = std::fs::OpenOptions::new().create(true).append(append).write(true).truncate(!append).open(&path) {
        b.target(env_logger::Target::Pipe(Box::new(Tee(f))));
    }
    b.init();
}

/// Ask Windows to run this process in efficiency mode (EcoQoS): it is scheduled on the
/// E-cores at low clocks, which costs less power for this light, steady load.
pub fn eco_mode() {
    use windows::Win32::System::Threading::{
        GetCurrentProcess, ProcessPowerThrottling, SetProcessInformation, PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        PROCESS_POWER_THROTTLING_EXECUTION_SPEED, PROCESS_POWER_THROTTLING_STATE,
    };
    let state = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
        StateMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
    };
    let r = unsafe {
        SetProcessInformation(GetCurrentProcess(), ProcessPowerThrottling, &state as *const _ as *const _, std::mem::size_of_val(&state) as u32)
    };
    match r {
        Ok(()) => log::info!("efficiency mode (EcoQoS) on"),
        Err(e) => log::warn!("efficiency mode unavailable: {e}"),
    }
}

/// Only one dashboard may drive the panel. Returns false when another instance is running.
pub fn single_instance() -> bool {
    use windows::core::w;
    use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
    use windows::Win32::System::Threading::CreateMutexW;
    unsafe {
        let h = CreateMutexW(None, true, w!("Local\\TurzxDashboard"));
        let exists = GetLastError() == ERROR_ALREADY_EXISTS;
        let ok = h.is_ok();
        std::mem::forget(h); // held for the lifetime of the process
        ok && !exists
    }
}

