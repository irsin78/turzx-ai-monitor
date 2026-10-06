//! Windows session events: shutdown / restart / logoff (standby screen) and the lock screen.

use std::sync::atomic::Ordering;
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

/// Hidden top-level window that receives WM_ENDSESSION (a windowless process gets no notice).
pub(super) fn watch_end_session() -> Receiver<EndSession> {
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
            if let Err(e) = p.show_landscape(&ui::standby().rgb()) {
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
