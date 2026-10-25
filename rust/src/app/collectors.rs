//! Background threads that keep the shared state fresh: hardware every second, weather every
//! 10 minutes, AI plan usage on its own schedule per service.

use std::thread;
use std::time::{Duration, Instant};

use super::Shared;
use crate::sensors::HardwareReader;
use crate::sources::ai::{self, Antigravity, Cache, ClaudeCli, Usage};
use crate::sources::weather;
use crate::state::AiErr;

const AI_INTERVAL: f64 = 300.0; // Codex / Antigravity
const CLAUDE_INTERVAL: f64 = 300.0; // Claude Code's /usage panel (about 3 s per read)
const WEATHER_INTERVAL: Duration = Duration::from_secs(600);

/// Run `f` now (after `first_delay`) and then every `interval`, forever, on its own thread.
pub(super) fn every(name: &'static str, interval: Duration, first_delay: Duration, mut f: impl FnMut() + Send + 'static) {
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

    let on = &crate::config::get().ai;
    if on.claude {
        let (last, wait) = first_delay("Claude", CLAUDE_INTERVAL);
        if let Some(u) = last {
            st.lock().unwrap().ai.insert("Claude", u);
        }
        let (s, c) = (st.clone(), cache.clone());
        thread::Builder::new()
            .name("claude".into())
            .spawn(move || {
                thread::sleep(wait);
                loop {
                    // Claude Code's own /usage panel: the numbers Claude Code shows (the OAuth
                    // usage API went on answering with hours-old numbers)
                    let r = ClaudeCli.fetch(Duration::from_secs(45));
                    if let Err(e) = &r {
                        log::warn!("Claude /usage read failed: {e:#}");
                    }
                    set_ai(&s, &c, "Claude", r);
                    let tick = until_reset(&s, "Claude").map_or(CLAUDE_INTERVAL, |r| r.min(CLAUDE_INTERVAL));
                    thread::sleep(Duration::from_secs_f64(tick));
                }
            })
            .expect("spawn thread");
    }

    if on.codex {
        let (last, wait) = first_delay("Codex", AI_INTERVAL);
        if let Some(u) = last {
            st.lock().unwrap().ai.insert("Codex", u);
        }
        let (s, c) = (st.clone(), cache.clone());
        every("codex", Duration::from_secs_f64(AI_INTERVAL), wait, move || set_ai(&s, &c, "Codex", ai::codex()));
    }

    if on.antigravity {
        let (last, wait) = first_delay("Antigravity", AI_INTERVAL);
        if let Some(u) = last {
            st.lock().unwrap().ai.insert("Antigravity", u);
        }
        let (s, c) = (st.clone(), cache.clone());
        let mut agy = Antigravity::new();
        every("antigravity", Duration::from_secs_f64(AI_INTERVAL), wait, move || set_ai(&s, &c, "Antigravity", agy.fetch()));
    }
}
