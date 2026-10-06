//! Background threads that keep the shared state fresh: hardware every second, weather every
//! 10 minutes, AI plan usage on its own schedule per service.

use std::thread;
use std::time::{Duration, Instant};

use super::Shared;
use crate::sensors::HardwareReader;
use crate::sources::ai::{self, Antigravity, Cache, Claude, ClaudeCli, Usage};
use crate::sources::weather;
use crate::state::AiErr;

const AI_INTERVAL: f64 = 300.0; // Codex / Antigravity
const CLAUDE_INTERVAL: f64 = 900.0; // Claude's usage API rate-limits hard; 15 min worked for other monitors
const CLAUDE_CLI_INTERVAL: f64 = 300.0; // /usage panel reads while the API is unavailable
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
