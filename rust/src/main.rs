#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // release: no console window

mod app;
mod config;
mod diag;
mod logging;
#[cfg(mascots)]
mod mascot {
    // personal mascot engine and art, outside the public repository (see build.rs)
    include!(concat!(env!("TURZX_MASCOTS_DIR"), "/mascot.rs"));
}
#[cfg(not(mascots))]
#[path = "mascot_off.rs"]
mod mascot;
mod panel;
mod sensors;
mod sources;
mod state;
mod ui;

use sensors::pawnio;

use anyhow::Result;

fn main() -> Result<()> {
    // Started from a terminal: print there (the release build has no console of its own)
    unsafe {
        let _ = windows::Win32::System::Console::AttachConsole(windows::Win32::System::Console::ATTACH_PARENT_PROCESS);
    }
    logging::init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |a: &str| args.iter().any(|x| x == a);
    if args.first().map(String::as_str) == Some("diag") {
        return diag::run(args.get(1).map_or("", String::as_str));
    }
    if arg("--worker") {
        // the dashboard proper, run (and restarted) by the supervisor below
        std::panic::set_hook(Box::new(|info| log::error!("panic: {info}")));
        log::info!("dashboard starting (admin: {})", pawnio::is_admin());
        app::supervisor::eco_mode();
        app::run(!arg("--no-ai"));
    }
    if !app::supervisor::single_instance() {
        log::warn!("another dashboard is already running; exiting");
        return Ok(());
    }
    log::info!("supervisor starting");
    app::supervisor::eco_mode();
    app::supervisor::supervise()
}
