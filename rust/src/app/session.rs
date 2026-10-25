//! Windows session events: shutdown / restart / logoff (standby screen), and when the panel
//! should go black: lock screen, monitors switched off, screen saver.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::OnceLock;
use std::thread;
use std::time::Duration;

use super::supervisor::{signal_ending, ENDING};
use crate::panel::lcd;
use crate::ui;

/// Windows is shutting down / restarting / logging off: the sender acknowledges once the
/// standby screen is on the panel.
pub(super) type EndSession = Sender<()>;

static END_SESSION: OnceLock<Sender<EndSession>> = OnceLock::new();

/// The monitors are off (GUID_CONSOLE_DISPLAY_STATE; dimmed counts as on).
static DISPLAY_OFF: AtomicBool = AtomicBool::new(false);

/// Windows asked whether the session may end (shutdown, restart, logoff are coming).
pub(super) static QUERY_END: AtomicBool = AtomicBool::new(false);

/// Hidden top-level window that receives WM_ENDSESSION (a windowless process gets no notice).
pub(super) fn watch_end_session() -> Receiver<EndSession> {
    use windows::core::w;
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Power::{RegisterPowerSettingNotification, POWERBROADCAST_SETTING};
    use windows::Win32::System::SystemServices::GUID_CONSOLE_DISPLAY_STATE;
    use windows::Win32::UI::WindowsAndMessaging::*;

    unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        match msg {
            WM_POWERBROADCAST if wp.0 == PBT_POWERSETTINGCHANGE as usize && lp.0 != 0 => {
                let setting = unsafe { &*(lp.0 as *const POWERBROADCAST_SETTING) };
                if setting.PowerSetting == GUID_CONSOLE_DISPLAY_STATE && setting.DataLength == 4 {
                    // 0 off, 1 on, 2 dimmed
                    let off = unsafe { std::ptr::read_unaligned(setting.Data.as_ptr().cast::<u32>()) } == 0;
                    DISPLAY_OFF.store(off, Ordering::Relaxed);
                }
                LRESULT(1)
            }
            WM_QUERYENDSESSION => {
                QUERY_END.store(true, Ordering::Relaxed);
                LRESULT(1)
            }
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
            let hwnd = match CreateWindowExW(WINDOW_EX_STYLE::default(), class, w!("TURZX dashboard"), WS_POPUP, 0, 0, 0, 0, None, None, Some(inst.into()), None) {
                Ok(h) => h,
                Err(e) => {
                    log::warn!("session window failed: {e}");
                    return;
                }
            };
            // the current state arrives right after registering
            if let Err(e) = RegisterPowerSettingNotification(hwnd.into(), &GUID_CONSOLE_DISPLAY_STATE, DEVICE_NOTIFY_WINDOW_HANDLE) {
                log::warn!("display power notification failed: {e}");
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
pub(super) fn wait(rx: &Receiver<EndSession>, d: Duration) -> Option<EndSession> {
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
pub(super) fn end_session(panel: Option<&mut lcd::Lcd>, ack: EndSession) -> ! {
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
            // a video on the panel's card plays by itself and keeps going after the PC is off
            let playing = crate::offline::video().is_some_and(|v| match p.keep_on_without_pc().and_then(|()| p.play_local(&v)) {
                Ok(()) => {
                    log::info!("offline video: {v}");
                    true
                }
                Err(e) => {
                    log::warn!("offline video {v} failed: {e:#}");
                    false
                }
            });
            if !playing {
                if let Err(e) = p.show_landscape(&ui::standby().rgb()) {
                    log::warn!("standby send failed: {e:#}");
                }
            }
        }
        None => log::warn!("standby skipped: LCD not connected"),
    }
    let _ = ack.send(());
    loop {
        thread::sleep(Duration::from_secs(60));
    }
}

/// Why the panel should be black right now, if it should: the PC is locked, the monitors are
/// off, or the screen saver runs.
pub fn blank_reason() -> Option<&'static str> {
    if session_locked() {
        Some("locked")
    } else if DISPLAY_OFF.load(Ordering::Relaxed) {
        Some("monitors off")
    } else if screen_saver_running() {
        Some("screen saver")
    } else {
        None
    }
}

fn screen_saver_running() -> bool {
    use windows::core::BOOL;
    use windows::Win32::UI::WindowsAndMessaging::{SystemParametersInfoW, SPI_GETSCREENSAVERRUNNING, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS};
    let mut running = BOOL::default();
    unsafe {
        SystemParametersInfoW(SPI_GETSCREENSAVERRUNNING, 0, Some((&mut running as *mut BOOL).cast()), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0)).is_ok()
            && running.as_bool()
    }
}

/// True while this Windows session shows the lock screen.
fn session_locked() -> bool {
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
