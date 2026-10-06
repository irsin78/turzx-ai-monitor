//! The scheduled task starts the supervisor, which only runs the real dashboard (`--worker`) as
//! a child and starts it again whenever it exits. A crash inside a driver DLL (nvml.dll died
//! during an NVIDIA driver update) takes the whole process down, so recovery has to live in a
//! process that touches no drivers.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use super::collectors::every;

const ENDING_EVENT: windows::core::PCWSTR = windows::core::w!("Local\\TurzxDashboardEnding");

/// Worker: Windows is ending the session (standby screen sent).
pub(super) fn signal_ending() {
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

/// A USB call that never returns would freeze the panel for good: if the render loop has not
/// come round for this long, exit and let the supervisor start a fresh worker.
const STUCK_AFTER: Duration = Duration::from_secs(60);

/// Set while Windows ends the session (the standby screen is up): the watchdog stands down.
pub(super) static ENDING: AtomicBool = AtomicBool::new(false);

pub(super) fn start_watchdog() -> Arc<Mutex<Instant>> {
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
