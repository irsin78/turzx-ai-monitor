//! `install`, `uninstall`, `autostart on|off`, `settings`.
//!
//! Per-user install, no installer package:
//! - the exe goes to `%LOCALAPPDATA%\Programs\TurzxDashboard`
//! - Start menu shortcuts (dashboard, settings) and an "Apps & features" entry (HKCU)
//! - autostart: a scheduled task at logon with highest privileges when PawnIO is installed
//!   (its sensors need administrator rights; one UAC prompt now, none at logon), otherwise the
//!   HKCU Run key (no prompt; CPU/RAM temperatures and board fans are not shown)
//!
//! Settings (`%APPDATA%\TurzxDashboard`) and logs/caches (`%LOCALAPPDATA%\TurzxDashboard`) are
//! kept on uninstall unless `--purge` is given.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use windows::core::{Interface, HSTRING, PCWSTR};
use windows::Win32::System::Registry::{RegDeleteKeyValueW, RegDeleteTreeW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_DWORD, REG_SZ};

use crate::config;
use crate::sensors::pawnio;

const APP: &str = "TURZX AI Monitor";
const TASK: &str = "TurzxDashboard";
const EXE: &str = "turzx-dashboard.exe";
const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\TurzxAiMonitor";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const DETACHED_PROCESS: u32 = 0x0000_0008;

pub fn install_dir() -> PathBuf {
    PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_default()).join(r"Programs\TurzxDashboard")
}

fn installed_exe() -> PathBuf {
    install_dir().join(EXE)
}

fn start_menu() -> PathBuf {
    PathBuf::from(std::env::var("APPDATA").unwrap_or_default()).join(r"Microsoft\Windows\Start Menu\Programs")
}

/// Is this exe the installed copy?
pub fn running_installed() -> bool {
    let same = |a: &Path, b: &Path| a.canonicalize().ok().zip(b.canonicalize().ok()).is_some_and(|(a, b)| a == b);
    std::env::current_exe().is_ok_and(|e| same(&e, &installed_exe()))
}

fn pawnio_installed() -> bool {
    Path::new(r"C:\Program Files\PawnIO\PawnIOLib.dll").is_file()
}

/// Run a console tool without a window; Ok(true) when it exits with 0.
fn quiet(exe: &str, args: &[&str]) -> Result<bool> {
    Ok(Command::new(exe).args(args).creation_flags(CREATE_NO_WINDOW).output()?.status.success())
}

fn task_exists() -> bool {
    quiet("schtasks", &["/Query", "/TN", TASK]).unwrap_or(false)
}

/// Run this exe again elevated with `args` (UAC prompt) and wait for it.
fn elevated(args: &str) -> Result<()> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
    use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
    let exe = HSTRING::from(std::env::current_exe()?.as_os_str());
    let params = HSTRING::from(args);
    let verb = HSTRING::from("runas");
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(exe.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        nShow: 0,
        ..Default::default()
    };
    unsafe {
        ShellExecuteExW(&mut info).context("administrator rights were not granted")?;
        WaitForSingleObject(info.hProcess, INFINITE);
        let mut code = 1u32;
        let _ = GetExitCodeProcess(info.hProcess, &mut code);
        let _ = CloseHandle(info.hProcess);
        if code != 0 {
            bail!("the elevated step failed (exit code {code}); see the log");
        }
    }
    Ok(())
}

/// Stop the running dashboard (task, supervisor and worker), waiting until the exe is free.
fn stop_running() {
    if task_exists() {
        let _ = quiet("schtasks", &["/End", "/TN", TASK]);
    }
    let me = std::process::id();
    // the dashboard runs with no arguments, `run` or `--worker`; leave other commands alone
    // (an elevated install must not kill the install that started it)
    let is_dashboard = |p: &sysinfo::Process| {
        let args: Vec<String> = p.cmd().iter().skip(1).map(|a| a.to_string_lossy().to_lowercase()).collect();
        args.is_empty() || args.iter().any(|a| a == "run" || a == "--worker")
    };
    for _ in 0..20 {
        let mut sys = sysinfo::System::new();
        sys.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::All,
            true,
            sysinfo::ProcessRefreshKind::nothing().with_cmd(sysinfo::UpdateKind::Always),
        );
        let others: Vec<_> = sys
            .processes()
            .values()
            .filter(|p| p.name().eq_ignore_ascii_case(EXE) && p.pid().as_u32() != me && is_dashboard(p))
            .collect();
        if others.is_empty() {
            return;
        }
        for p in others {
            p.kill();
        }
        std::thread::sleep(Duration::from_millis(300));
    }
}

fn reg_set_str(key: &str, name: &str, value: &str) {
    let data: Vec<u16> = value.encode_utf16().chain([0]).collect();
    unsafe {
        let _ = RegSetKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(key), &HSTRING::from(name), REG_SZ.0, Some(data.as_ptr().cast()), (data.len() * 2) as u32);
    }
}

fn reg_set_dword(key: &str, name: &str, value: u32) {
    unsafe {
        let _ = RegSetKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(key), &HSTRING::from(name), REG_DWORD.0, Some((&value as *const u32).cast()), 4);
    }
}

/// A Start menu shortcut to `target args`.
fn shortcut(name: &str, target: &Path, args: &str, description: &str) -> Result<()> {
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, IPersistFile, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        link.SetPath(&HSTRING::from(target.as_os_str()))?;
        link.SetArguments(&HSTRING::from(args))?;
        link.SetDescription(&HSTRING::from(description))?;
        link.SetWorkingDirectory(&HSTRING::from(install_dir().as_os_str()))?;
        let file: IPersistFile = link.cast()?;
        file.Save(&HSTRING::from(start_menu().join(format!("{name}.lnk")).as_os_str()), true)?;
    }
    Ok(())
}

/// Scheduled task: at logon, highest privileges, no time limit, restart on failure.
fn create_task(exe: &Path) -> Result<()> {
    let user = format!("{}\\{}", std::env::var("USERDOMAIN").unwrap_or_default(), std::env::var("USERNAME").unwrap_or_default());
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo><Description>{APP}: dashboard for the TURZX USB LCD</Description></RegistrationInfo>
  <Triggers><LogonTrigger><Enabled>true</Enabled><UserId>{user}</UserId><Delay>PT10S</Delay></LogonTrigger></Triggers>
  <Principals><Principal id="Author"><UserId>{user}</UserId><LogonType>InteractiveToken</LogonType><RunLevel>HighestAvailable</RunLevel></Principal></Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <RestartOnFailure><Interval>PT1M</Interval><Count>3</Count></RestartOnFailure>
    <Priority>7</Priority>
  </Settings>
  <Actions Context="Author"><Exec><Command>{exe}</Command><WorkingDirectory>{dir}</WorkingDirectory></Exec></Actions>
</Task>"#,
        user = esc(&user),
        exe = esc(&exe.to_string_lossy()),
        dir = esc(&install_dir().to_string_lossy()),
    );
    let file = std::env::temp_dir().join("turzx-dashboard-task.xml");
    let mut bytes = vec![0xFF, 0xFE]; // UTF-16LE BOM
    bytes.extend(xml.encode_utf16().flat_map(u16::to_le_bytes));
    std::fs::write(&file, bytes)?;
    let ok = quiet("schtasks", &["/Create", "/TN", TASK, "/XML", &file.to_string_lossy(), "/F"])?;
    let _ = std::fs::remove_file(&file);
    if !ok {
        bail!("schtasks could not create the task");
    }
    Ok(())
}

/// Turn autostart on (task with PawnIO, else Run key) or off. Needs admin for the task.
fn set_autostart(on: bool, exe: &Path) -> Result<&'static str> {
    if task_exists() {
        quiet("schtasks", &["/Delete", "/TN", TASK, "/F"])?;
    }
    unsafe {
        let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(RUN_KEY), &HSTRING::from(TASK));
    }
    if !on {
        return Ok("off");
    }
    if pawnio_installed() && pawnio::is_admin() {
        create_task(exe)?;
        Ok("scheduled task (all sensors)")
    } else {
        reg_set_str(RUN_KEY, TASK, &format!("\"{}\"", exe.display()));
        Ok("Run key (PawnIO sensors off)")
    }
}

/// Start the installed dashboard in the background.
fn start(exe: &Path) {
    if task_exists() {
        let _ = quiet("schtasks", &["/Run", "/TN", TASK]);
    } else {
        let _ = Command::new(exe).current_dir(install_dir()).creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW).spawn();
    }
}

/// Tell the user (console when there is one, otherwise a message box).
pub fn tell(text: &str, error: bool) {
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_ICONINFORMATION, MB_OK};
    if unsafe { windows::Win32::System::Console::GetConsoleWindow() }.0.is_null() {
        let flags = MB_OK | if error { MB_ICONERROR } else { MB_ICONINFORMATION };
        unsafe { MessageBoxW(None, &HSTRING::from(text), &HSTRING::from(APP), flags) };
    } else if error {
        eprintln!("{text}");
    } else {
        println!("{text}");
    }
}

/// Ask a yes/no question in a message box.
pub fn ask(text: &str) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, IDYES, MB_ICONQUESTION, MB_YESNO};
    unsafe { MessageBoxW(None, &HSTRING::from(text), &HSTRING::from(APP), MB_YESNO | MB_ICONQUESTION) == IDYES }
}

pub fn install(autostart: bool) -> Result<String> {
    // the task (PawnIO) and replacing an elevated install need administrator rights
    let need_admin = (autostart && pawnio_installed()) || task_exists();
    if need_admin && !pawnio::is_admin() {
        let args = if autostart { "install --quiet" } else { "install --no-autostart --quiet" };
        if elevated(args).is_ok() {
            return Ok(String::new()); // the elevated copy did it and told the user
        }
        log::warn!("no administrator rights: installing with the Run key instead of a task");
    }
    stop_running();
    let dir = install_dir();
    std::fs::create_dir_all(&dir)?;
    let exe = installed_exe();
    if !running_installed() {
        let me = std::env::current_exe()?;
        let mut copied = std::fs::copy(&me, &exe).map(|_| ());
        for _ in 0..10 {
            if copied.is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(300));
            copied = std::fs::copy(&me, &exe).map(|_| ());
        }
        copied.with_context(|| format!("could not copy to {}", exe.display()))?;
    }
    // earlier versions ran from the data folder
    let _ = std::fs::remove_file(PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_default()).join(r"TurzxDashboard").join(EXE));

    shortcut(APP, &exe, "", "Dashboard for the TURZX USB LCD")?;
    shortcut(&format!("{APP} settings"), &exe, "settings", "Edit the dashboard settings")?;
    let size_kb = std::fs::metadata(&exe).map_or(0, |m| m.len() / 1024) as u32;
    reg_set_str(UNINSTALL_KEY, "DisplayName", APP);
    reg_set_str(UNINSTALL_KEY, "DisplayVersion", env!("CARGO_PKG_VERSION"));
    reg_set_str(UNINSTALL_KEY, "Publisher", "turzx-ai-monitor");
    reg_set_str(UNINSTALL_KEY, "InstallLocation", &dir.to_string_lossy());
    reg_set_str(UNINSTALL_KEY, "DisplayIcon", &exe.to_string_lossy());
    reg_set_str(UNINSTALL_KEY, "UninstallString", &format!("\"{}\" uninstall", exe.display()));
    reg_set_dword(UNINSTALL_KEY, "NoModify", 1);
    reg_set_dword(UNINSTALL_KEY, "NoRepair", 1);
    reg_set_dword(UNINSTALL_KEY, "EstimatedSize", size_kb);
    let how = set_autostart(autostart, &exe)?;
    let _ = config::get(); // writes the settings file with comments on the first run
    start(&exe);
    log::info!("installed to {} (autostart: {how})", dir.display());
    Ok(format!(
        "{APP} is installed and running.\n\nProgram: {}\nAutostart: {how}\nSettings: {}\n\nUninstall from Settings > Apps, or run `{EXE} uninstall`.",
        dir.display(),
        config::path().display()
    ))
}

pub fn uninstall(purge: bool) -> Result<String> {
    if task_exists() && !pawnio::is_admin() {
        elevated(if purge { "uninstall --purge --quiet" } else { "uninstall --quiet" })?;
        return Ok(String::new());
    }
    stop_running();
    set_autostart(false, &installed_exe())?;
    let _ = std::fs::remove_file(start_menu().join(format!("{APP}.lnk")));
    let _ = std::fs::remove_file(start_menu().join(format!("{APP} settings.lnk")));
    unsafe {
        let _ = RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(UNINSTALL_KEY));
    }
    if purge {
        let _ = std::fs::remove_dir_all(config::dir());
        let _ = std::fs::remove_dir_all(PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_default()).join("TurzxDashboard"));
    }
    // a running exe cannot be deleted, but it can be moved: park it in %TEMP% (deleted at the
    // next reboot when we may) and remove the folder now
    let dir = install_dir();
    if running_installed() {
        let parked = std::env::temp_dir().join(format!("turzx-dashboard-removed-{}.exe", std::process::id()));
        if std::fs::rename(installed_exe(), &parked).is_ok() {
            use windows::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_DELAY_UNTIL_REBOOT};
            unsafe {
                let _ = MoveFileExW(&HSTRING::from(parked.as_os_str()), PCWSTR::null(), MOVEFILE_DELAY_UNTIL_REBOOT);
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    log::info!("uninstalled (purge: {purge})");
    Ok(format!(
        "{APP} is uninstalled.{}",
        if purge { "" } else { "\n\nYour settings were kept in %APPDATA%\\TurzxDashboard; uninstall with --purge to remove them too." }
    ))
}

pub fn autostart(on: bool) -> Result<String> {
    if (task_exists() || (on && pawnio_installed())) && !pawnio::is_admin() {
        elevated(if on { "autostart on --quiet" } else { "autostart off --quiet" })?;
        return Ok(String::new());
    }
    let exe = if installed_exe().is_file() { installed_exe() } else { std::env::current_exe()? };
    Ok(format!("Autostart: {}", set_autostart(on, &exe)?))
}

/// Open the settings file in Notepad (creating it with the defaults first).
pub fn settings() -> Result<String> {
    let _ = config::get();
    Command::new("notepad.exe").arg(config::path()).spawn()?;
    Ok(String::new())
}
