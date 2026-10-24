#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // release: no console window

mod app;
mod config;
mod diag;
mod i18n;
mod install;
mod logging;
mod offline;
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
mod settings;
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
    // --quiet: no message boxes (the elevated half of a command, scripts); the log has it all
    let quiet = arg("--quiet");
    let done = |r: anyhow::Result<String>| -> Result<()> {
        match r {
            Ok(msg) if !msg.is_empty() => {
                log::info!("{msg}");
                if !quiet {
                    install::tell(&msg, false);
                }
            }
            Ok(_) => {}
            Err(e) => {
                log::error!("{e:#}");
                if !quiet {
                    install::tell(&format!("{e:#}"), true);
                }
                std::process::exit(1);
            }
        }
        Ok(())
    };
    match args.first().map(String::as_str) {
        Some("diag") => return diag::run(args.get(1).map_or("", String::as_str)),
        Some("install") => return done(install::install(!arg("--no-autostart"))),
        Some("uninstall") => return done(install::uninstall(arg("--purge"))),
        Some("autostart") => return done(install::autostart(args.get(1).map(String::as_str) != Some("off"))),
        Some("settings") => return done(settings::open()),
        Some("--help" | "-h" | "help") => {
            install::tell(USAGE, false);
            return Ok(());
        }
        _ => {}
    }
    if arg("--worker") {
        // the dashboard proper, run (and restarted) by the supervisor below
        std::panic::set_hook(Box::new(|info| log::error!("panic: {info}")));
        log::info!("dashboard starting (admin: {})", pawnio::is_admin());
        // no efficiency mode here: EcoQoS threads starve on busy E-cores, and the USB library's
        // completion thread is one of ours (frames stalled for seconds under load); the
        // supervisor, which does nothing, keeps it. Above-normal priority for the same reason:
        // under a full CPU load that thread otherwise waits for Windows' starvation boost
        // (seconds) before a finished frame is reported.
        unsafe {
            use windows::Win32::System::Threading::{GetCurrentProcess, SetPriorityClass, ABOVE_NORMAL_PRIORITY_CLASS};
            let _ = SetPriorityClass(GetCurrentProcess(), ABOVE_NORMAL_PRIORITY_CLASS);
        }
        config::watch();
        app::run(!arg("--no-ai"));
    }
    // double-clicked outside the install folder: offer to install
    if args.is_empty() && !install::running_installed() && install::ask("Install TURZX AI Monitor and start it with Windows?

No: just run it this time.") {
        return done(install::install(true));
    }
    if !app::supervisor::single_instance() {
        log::warn!("another dashboard is already running; exiting");
        return Ok(());
    }
    log::info!("supervisor starting");
    app::supervisor::eco_mode();
    app::supervisor::supervise()
}

const USAGE: &str = "turzx-dashboard [command]

  (none)                  run the dashboard (offers to install when run from elsewhere)
  run                     run the dashboard without asking
  install [--no-autostart]
  uninstall [--purge]     --purge also removes settings, logs and caches
  autostart on|off
  settings                open the settings window
  diag <command>          previews and diagnostics (diag help)
";
