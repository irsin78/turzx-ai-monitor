//! Passive worker measurements. No allocation or process queries on individual ticks.
use std::time::{Duration, Instant};
use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX};
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

const WINDOW: Duration = Duration::from_secs(10);
// The scheduler advances at least one period per rendered tick, with an occasional
// early full refresh. Twice the nominal window rate leaves room at both boundaries.
const SAMPLES: usize = 12 * 20 + 2;

#[derive(Default)]
struct Times {
    count: u64,
    sum_ms: f64,
    max_ms: f64,
}

impl Times {
    fn add(&mut self, elapsed: Duration) {
        let ms = elapsed.as_secs_f64() * 1000.0;
        self.count += 1;
        self.sum_ms += ms;
        self.max_ms = self.max_ms.max(ms);
    }

    fn mean(&self) -> f64 {
        self.sum_ms / self.count.max(1) as f64
    }
}

#[derive(Default)]
pub(super) struct Perf {
    window: Option<Window>,
}

struct Window {
    started: Instant,
    cpu: Option<u64>,
    late: Times,
    lateness: [f64; SAMPLES],
    skipped: u64,
    full_render: Times,
    full_send: Times,
    tick_render: Times,
    tick_send: Times,
    bytes: usize,
}

impl Window {
    fn p95(&mut self) -> f64 {
        let n = (self.late.count as usize).min(SAMPLES);
        if n == 0 { 0.0 } else if self.late.count as usize > SAMPLES {
            // Defensive only: never silently report a truncated percentile.
            f64::NAN
        } else {
            let index = (n * 95).div_ceil(100) - 1;
            *self.lateness[..n].select_nth_unstable_by(index, f64::total_cmp).1
        }
    }
}

impl Perf {
    pub fn active(&mut self, active: bool) {
        if !active {
            self.window = None;
        } else if self.window.is_none() {
            self.window = Some(Window {
                started: Instant::now(), cpu: cpu_time(), late: Times::default(),
                lateness: [0.0; SAMPLES], skipped: 0,
                full_render: Times::default(), full_send: Times::default(),
                tick_render: Times::default(), tick_send: Times::default(),
                bytes: 0,
            });
        }
    }

    // Full refreshes also advance the mascot, so they count as ticks when enabled.
    pub fn tick(&mut self, started: Instant, scheduled: Instant) {
        if let Some(w) = self.window.as_mut() {
            let late = started.saturating_duration_since(scheduled);
            let index = w.late.count as usize;
            if index < SAMPLES {
                w.lateness[index] = late.as_secs_f64() * 1000.0;
            }
            w.late.add(late);
        }
    }

    pub fn skipped(&mut self, count: u64) {
        if let Some(w) = self.window.as_mut() {
            w.skipped += count;
        }
    }

    pub fn render(&mut self, full: bool, elapsed: Duration) {
        if let Some(w) = self.window.as_mut() {
            if full { &mut w.full_render } else { &mut w.tick_render }.add(elapsed);
        }
    }

    pub fn send(&mut self, full: bool, elapsed: Duration) {
        if let Some(w) = self.window.as_mut() {
            if full { &mut w.full_send } else { &mut w.tick_send }.add(elapsed);
        }
    }

    pub fn bytes(&mut self, bytes: usize) {
        if let Some(w) = self.window.as_mut() { w.bytes += bytes; }
    }

    pub fn report(&mut self) {
        let Some(w) = self.window.as_mut() else { return };
        let elapsed = w.started.elapsed();
        if elapsed < WINDOW { return; }
        let cpu = w.cpu.zip(cpu_time()).map(|(start, end)| {
            end.saturating_sub(start) as f64 / 10_000_000.0 / elapsed.as_secs_f64() * 100.0
        });
        let mut memory = PROCESS_MEMORY_COUNTERS_EX::default();
        let size = std::mem::size_of_val(&memory) as u32;
        memory.cb = size;
        // EX shares the base structure prefix; cb requests the PrivateUsage field too.
        let memory_ok = unsafe {
            GetProcessMemoryInfo(GetCurrentProcess(), (&mut memory as *mut PROCESS_MEMORY_COUNTERS_EX).cast(), size)
        }.is_ok();
        let p95 = w.p95();
        let mb = |bytes: usize| if memory_ok { bytes as f64 / 1_048_576.0 } else { f64::NAN };
        log::info!(
            "perf: window={:.1}s ticks={} late_ms mean={:.1} p95={:.1} max={:.1} skipped={} | full={} render_ms {:.1}/{:.1} send_ms {:.1}/{:.1} | tick render_ms {:.1}/{:.1} send_ms {:.1}/{:.1} | mem private={:.1}MB peak_ws={:.1}MB | cpu={:.1}% | bytes_per_tick={:.0}",
            elapsed.as_secs_f64(), w.late.count, w.late.mean(), p95, w.late.max_ms, w.skipped,
            w.full_render.count, w.full_render.mean(), w.full_render.max_ms, w.full_send.mean(), w.full_send.max_ms,
            w.tick_render.mean(), w.tick_render.max_ms, w.tick_send.mean(), w.tick_send.max_ms,
            mb(memory.PrivateUsage), mb(memory.PeakWorkingSetSize), cpu.unwrap_or(f64::NAN),
            w.bytes as f64 / (w.tick_send.count + w.full_send.count).max(1) as f64,
        );
        self.window = None;
        self.active(true);
    }
}

fn cpu_time() -> Option<u64> {
    let mut created = FILETIME::default();
    let mut exited = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    unsafe { GetProcessTimes(GetCurrentProcess(), &mut created, &mut exited, &mut kernel, &mut user) }.ok()?;
    let ticks = |t: FILETIME| (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime);
    Some(ticks(kernel) + ticks(user))
}

/// Scheduled slots abandoned when the existing scheduler jumps forward.
pub(super) fn skipped_slots(next: Instant, resumed: Instant, period: Duration) -> u64 {
    let gap = resumed.saturating_duration_since(next).as_nanos();
    gap.div_ceil(period.as_nanos()) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_include_zero_lateness_and_reset_across_inactive_windows() {
        let mut perf = Perf::default();
        perf.active(true);
        let now = Instant::now();
        perf.tick(now, now + Duration::from_millis(5));
        perf.tick(now + Duration::from_millis(20), now);
        perf.render(true, Duration::from_millis(8));
        perf.send(true, Duration::from_millis(30));
        perf.skipped(2);
        let w = perf.window.as_ref().unwrap();
        assert_eq!(w.late.count, 2);
        assert_eq!(w.late.mean(), 10.0);
        assert_eq!(w.late.max_ms, 20.0);
        assert_eq!(w.skipped, 2);
        assert_eq!(w.full_render.count, 1);
        assert_eq!(w.full_send.mean(), 30.0);
        perf.active(false);
        perf.active(true);
        assert_eq!(perf.window.as_ref().unwrap().late.count, 0);
    }

    #[test]
    fn skips_count_only_slots_before_the_resume_deadline() {
        let now = Instant::now();
        let period = Duration::from_millis(80);
        assert_eq!(skipped_slots(now, now, period), 0);
        assert_eq!(skipped_slots(now, now + period, period), 1);
        assert_eq!(skipped_slots(now, now + period + Duration::from_millis(1), period), 2);
    }

    #[test]
    fn percentile_uses_nearest_rank_and_reports_reset_after_ten_seconds() {
        let mut perf = Perf::default();
        perf.active(true);
        assert_eq!(perf.window.as_mut().unwrap().p95(), 0.0);
        let now = Instant::now();
        for ms in (1..=100).rev() {
            perf.tick(now + Duration::from_millis(ms), now);
        }
        assert_eq!(perf.window.as_mut().unwrap().p95(), 95.0);
        perf.report();
        assert_eq!(perf.window.as_ref().unwrap().late.count, 100);
        perf.window.as_mut().unwrap().started = now - WINDOW;
        perf.report();
        assert_eq!(perf.window.as_ref().unwrap().late.count, 0);
        perf.active(false);
        perf.report();
        assert!(perf.window.is_none());
    }

    #[test]
    fn process_counters_are_available_without_a_panel() {
        assert!(cpu_time().is_some());
        let mut memory = PROCESS_MEMORY_COUNTERS_EX::default();
        let size = std::mem::size_of_val(&memory) as u32;
        memory.cb = size;
        assert!(unsafe {
            GetProcessMemoryInfo(GetCurrentProcess(), (&mut memory as *mut PROCESS_MEMORY_COUNTERS_EX).cast(), size)
        }.is_ok());
        assert!(memory.PrivateUsage > 0);
        assert!(memory.PeakWorkingSetSize > 0);
    }
}
