//! Background threads that keep the shared state fresh: hardware every second, weather every
//! 10 minutes, AI plan usage on its own schedule per service.

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use super::Shared;
use crate::agent_status::Source;
use crate::sensors::HardwareReader;
use crate::sources::ai::{self, Antigravity, Cache, ClaudeCli, Usage};
use crate::sources::api_cost;
use crate::sources::weather;
use crate::state::AiErr;

const AI_IDLE: Duration = Duration::from_secs(300);
const AI_ACTIVE: Duration = Duration::from_secs(30);
const AI_MIN: Duration = Duration::from_secs(15);
const WEATHER_INTERVAL: Duration = Duration::from_secs(600);

#[derive(Default)]
struct WeatherRetry {
    failures: usize,
}

impl WeatherRetry {
    fn after(&mut self, success: bool) -> Duration {
        if success {
            self.failures = 0;
            return WEATHER_INTERVAL;
        }
        self.failures = self.failures.saturating_add(1);
        match self.failures {
            1 => Duration::from_secs(60),
            2 => Duration::from_secs(120),
            3 => Duration::from_secs(300),
            _ => WEATHER_INTERVAL,
        }
    }
}

/// Restore immediately, retry failed requests sooner, and expire even during a stalled fetch.
fn weather_collector(st: &Shared) {
    let cached = weather::cached();
    let fetched_at = Arc::new(Mutex::new(cached.as_ref().map(|c| c.fetched_at)));
    if let Some(cached) = cached {
        st.lock().unwrap().weather = Some(cached.weather);
    }
    let (s, last) = (st.clone(), fetched_at.clone());
    every("weather expiry", Duration::from_secs(1), Duration::ZERO, move || {
        let mut last = last.lock().unwrap();
        if last.is_some_and(|at| !weather::cache_fresh(at, SystemTime::now())) {
            s.lock().unwrap().weather = None;
            *last = None;
        }
    });
    let s = st.clone();
    thread::Builder::new().name("weather".into()).spawn(move || {
        let mut retry = WeatherRetry::default();
        loop {
            let success = match weather::fetch() {
                Ok(w) => {
                    if retry.failures > 0 {
                        log::info!("weather back after {} failures", retry.failures);
                    }
                    // Same lock order as expiry; a new value cannot be cleared as an old one.
                    let mut last = fetched_at.lock().unwrap();
                    *last = Some(SystemTime::now());
                    s.lock().unwrap().weather = Some(w);
                    true
                }
                Err(e) => {
                    log::warn!("weather failed: {e:#}");
                    false
                }
            };
            thread::sleep(retry.after(success));
        }
    }).expect("spawn weather thread");
}

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
    match r {
        Ok(u) => {
            log::info!("{name} usage: {u:?}");
            cache.store(name, &u); // file I/O before taking the lock the frame loop renders under
            let mut s = st.lock().unwrap();
            s.ai.insert(name, u);
            s.ai_err.remove(name);
        }
        Err(e) => {
            if e.downcast_ref::<ai::RateLimited>().is_none() {
                log::warn!("{name} usage failed: {e:#}");
            }
            st.lock().unwrap().ai_err.insert(name, AiErr::from(&e)); // keep the last value on screen, show why it is stale
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

fn usage_collector(
    st: &Shared, cache: &Cache, name: &'static str, source: Source,
    mut fetch: impl FnMut() -> anyhow::Result<Usage> + Send + 'static,
) {
    let (cached, fetched_at) = cache.last(name);
    if let Some(usage) = cached {
        st.lock().unwrap().ai.insert(name, usage);
    }
    let (st, cache) = (st.clone(), cache.clone());
    thread::Builder::new().name(name.into()).spawn(move || {
        let mut last_attempt = None::<Instant>;
        let mut handled_completion = None;
        let mut backoff_until = None::<Instant>;
        let mut reset_due = None;
        let mut retry_after = 0.0;
        loop {
            let now = Instant::now();
            let (active, completed) = st.lock().unwrap().agents.quota_activity(source, now);
            let completed_pending = completed.is_some() && completed != handled_completion;
            let interval = if active { AI_ACTIVE } else { AI_IDLE };
            let age = last_attempt.map_or_else(
                || Duration::from_secs_f64((ai::now() - fetched_at).clamp(0.0, AI_IDLE.as_secs_f64())),
                |last| now.saturating_duration_since(last),
            );
            let due = age >= interval || age >= AI_MIN &&
                (completed_pending || reset_due.is_some_and(|at| now >= at));
            if due && ai::now() >= retry_after && backoff_until.is_none_or(|at| now >= at) {
                last_attempt = Some(now);
                handled_completion = completed;
                let result = fetch();
                retry_after = result.as_ref().err()
                    .and_then(|e| e.downcast_ref::<ai::RateLimited>())
                    .map_or(0.0, |e| e.0);
                backoff_until = result.as_ref().err().map(|_| {
                    Instant::now() + if retry_after > 0.0 { AI_MIN } else { AI_IDLE }
                });
                set_ai(&st, &cache, name, result);
                reset_due = if source == Source::Claude {
                    until_reset(&st, name).map(|secs| Instant::now() + Duration::from_secs_f64(secs))
                } else { None };
            }
            thread::sleep(Duration::from_secs(1));
        }
    }).expect("spawn usage thread");
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

    weather_collector(st);

    if !ai_enabled {
        return; // --no-ai: test runs without touching credentials
    }
    let cache = Cache::open();
    let on = &crate::config::get().ai;
    if on.claude {
        usage_collector(st, &cache, "Claude", Source::Claude, || ClaudeCli.fetch(Duration::from_secs(45)));
    }

    // Gemini API spend, shown under Antigravity
    if on.antigravity && api_cost::subscription().is_some() {
        const COST_INTERVAL: f64 = 1800.0; // the budget reports a few times a day
        let (last, fetched_at) = api_cost::cached();
        if api_cost::key_file().is_file() {
            st.lock().unwrap().api_cost = last;
        }
        let wait = Duration::from_secs_f64((fetched_at + COST_INTERVAL - ai::now()).max(0.0));
        let s = st.clone();
        let mut logged = None;
        every("api-cost", Duration::from_secs_f64(COST_INTERVAL), wait, move || match api_cost::fetch() {
            Ok(c) => {
                let key = c.as_ref().map(|c| (c.interval_start, c.published));
                if let Some(c) = c.as_ref().filter(|_| key != logged) {
                    log::info!("Gemini API cost: {:.2} {} (period from {:.0})", c.amount, c.currency, c.interval_start);
                }
                logged = key;
                s.lock().unwrap().api_cost = c;
            }
            Err(e) => {
                if e.downcast_ref::<api_cost::NoKey>().is_some() {
                    s.lock().unwrap().api_cost = None; // key removed: the line goes too
                }
                log::warn!("Gemini API cost failed: {e:#}");
            }
        });
    }

    if on.codex {
        usage_collector(st, &cache, "Codex", Source::Codex, ai::codex);
    }
    if on.antigravity {
        let mut agy = Antigravity::new();
        usage_collector(st, &cache, "Antigravity", Source::Antigravity, move || agy.fetch());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weather_retry_schedule() {
        let mut retry = WeatherRetry::default();
        for minutes in [1, 2, 5, 10, 10] {
            assert_eq!(retry.after(false), Duration::from_secs(minutes * 60));
        }
        assert_eq!(retry.failures, 5);
    }

    #[test]
    fn weather_retry_resets_on_success() {
        let mut retry = WeatherRetry::default();
        for _ in 0..4 {
            retry.after(false);
        }
        assert_eq!(retry.after(true), WEATHER_INTERVAL);
        assert_eq!(retry.failures, 0);
        assert_eq!(retry.after(false), Duration::from_secs(60));
        assert_eq!(retry.after(true), WEATHER_INTERVAL);
        assert_eq!(retry.after(true), WEATHER_INTERVAL);
        assert_eq!(retry.failures, 0);
    }
}
