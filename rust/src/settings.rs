//! The settings window: a small native Win32 form for the common settings. Saving edits only
//! the changed keys of config.toml (comments and everything else are kept); the running
//! dashboard sees the file change and restarts. "Edit file…" opens the file for the rest.

mod document;
mod review;

use std::path::PathBuf;

use anyhow::{Context, Result};
use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::Controls::EM_SETLIMITTEXT;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::config::{self, Config};
use document::Document;

const CLASS: PCWSTR = w!("TurzxDashboardSettings");
const WIDTH: i32 = 520;
const HEIGHT: i32 = 440;
const LANGUAGE: usize = 100;
const CLOCK: usize = 101;
const TEMPERATURE: usize = 102;
const ROTATION: usize = 103;
const CITY: usize = 104;
const AIR: usize = 105;
const CLAUDE: usize = 106;
const CODEX: usize = 107;
const ANTIGRAVITY: usize = 108;
const SAVE: usize = 1;
const CANCEL: usize = 2;
const EDIT: usize = 3;
const CAPTURE_TIMER: usize = 123;

struct Control {
    hwnd: HWND,
    bounds: [i32; 4],
    heading: bool,
}

struct Window {
    document: Document,
    controls: Vec<Control>,
    fonts: [HFONT; 2],
    dpi: u32,
    ko: bool,
    languages: Vec<String>,
    capture: Option<PathBuf>,
    capture_result: Option<Result<()>>,
}

impl Drop for Window {
    fn drop(&mut self) {
        for font in self.fonts {
            if !font.0.is_null() {
                unsafe {
                    let _ = DeleteObject(font.into());
                }
            }
        }
    }
}

struct MutexHandle(HANDLE);

impl Drop for MutexHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// The window is in Korean or English, by the language the dashboard uses.
fn korean(c: &Config) -> bool {
    let code = std::env::var("TURZX_LANGUAGE").unwrap_or_else(|_| c.language.clone());
    if code.eq_ignore_ascii_case("auto") {
        config::windows_locale().to_ascii_lowercase().starts_with("ko")
    } else {
        code.eq_ignore_ascii_case("ko")
    }
}

fn language_name(code: &str) -> &str {
    match code {
        "en" => "English",
        "ko" => "한국어",
        "ja" => "日本語",
        "zh-CN" => "简体中文",
        "zh-TW" => "繁體中文",
        "es" => "Español",
        "fr" => "Français",
        "de" => "Deutsch",
        "it" => "Italiano",
        "pt" => "Português",
        "ru" => "Русский",
        "pl" => "Polski",
        "tr" => "Türkçe",
        "nl" => "Nederlands",
        "vi" => "Tiếng Việt",
        "id" => "Bahasa Indonesia",
        other => other,
    }
}

/// Open the settings window (or bring the open one to the front) and wait until it closes.
///
/// `TURZX_SETTINGS_CONFIG` edits another file instead of the real one; with it,
/// `TURZX_SETTINGS_SCREENSHOT=<png>` saves a picture of the window and closes it (previews).
pub fn open() -> Result<String> {
    unsafe {
        // per-monitor DPI for this thread only
        let previous = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let result = open_window();
        if !previous.0.is_null() {
            SetThreadDpiAwarenessContext(previous);
        }
        result.map(|()| String::new())
    }
}

unsafe fn open_window() -> Result<()> {
    unsafe {
        let mutex = CreateMutexW(None, false, w!("Local\\TurzxDashboardSettings"))?;
        let already_open = GetLastError() == ERROR_ALREADY_EXISTS;
        let _guard = MutexHandle(mutex);
        if already_open {
            if let Ok(hwnd) = FindWindowW(CLASS, None) {
                let _ = ShowWindow(hwnd, SW_RESTORE);
                let _ = SetForegroundWindow(hwnd);
            }
            return Ok(());
        }
        let redirected = std::env::var_os("TURZX_SETTINGS_CONFIG").map(PathBuf::from);
        let document = Document::load(redirected.clone().unwrap_or_else(config::path))?;
        let ko = korean(&document.config);
        let title = if ko { "TURZX AI Monitor 설정" } else { "TURZX AI Monitor settings" };
        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(procedure),
            hInstance: instance.into(),
            lpszClassName: CLASS,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hbrBackground: GetSysColorBrush(COLOR_BTNFACE),
            hIcon: LoadIconW(Some(instance.into()), PCWSTR(1usize as *const u16)).unwrap_or_default(), // resource 1 (build.rs)
            ..Default::default()
        };
        anyhow::ensure!(RegisterClassW(&class) != 0, "register the settings window class");
        let dpi = GetDpiForSystem();
        let scale = |v| v * dpi as i32 / 96;
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
        let ex = WS_EX_CONTROLPARENT;
        let mut rect = RECT { right: scale(WIDTH), bottom: scale(HEIGHT), ..Default::default() };
        AdjustWindowRectExForDpi(&mut rect, style, false, ex, dpi)?;
        let mut work = RECT::default();
        SystemParametersInfoW(SPI_GETWORKAREA, 0, Some((&mut work as *mut RECT).cast()), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0))?;
        let (width, height) = (rect.right - rect.left, rect.bottom - rect.top);
        let hwnd = CreateWindowExW(
            ex,
            CLASS,
            &HSTRING::from(title),
            style,
            work.left + ((work.right - work.left - width) / 2).max(0),
            work.top + ((work.bottom - work.top - height) / 2).max(0),
            width,
            height,
            None,
            None,
            Some(instance.into()),
            None,
        )?;
        let mut languages: Vec<String> = std::iter::once("auto".to_owned()).chain(crate::i18n::LANGUAGES.iter().map(|s| s.code.to_owned())).collect();
        if !languages.contains(&document.config.language) {
            languages.push(document.config.language.clone());
        }
        let mut state = Box::new(Window {
            document,
            controls: vec![],
            fonts: [HFONT::default(); 2],
            dpi,
            ko,
            languages,
            capture: None,
            capture_result: None,
        });
        state.populate(hwnd)?;
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, (&mut *state as *mut Window) as isize);
        state.layout();
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = UpdateWindow(hwnd);
        let _ = SetFocus(Some(state.control(LANGUAGE)));
        if let (Some(_), Some(png)) = (&redirected, std::env::var_os("TURZX_SETTINGS_SCREENSHOT")) {
            state.capture = Some(PathBuf::from(png));
            anyhow::ensure!(SetTimer(Some(hwnd), CAPTURE_TIMER, 400, None) != 0, "start the capture timer"); // let the controls paint
        }
        let mut msg = MSG::default();
        loop {
            let r = GetMessageW(&mut msg, None, 0, 0).0;
            if r == 0 {
                break;
            }
            anyhow::ensure!(r != -1, "settings message loop failed");
            if !IsDialogMessageW(hwnd, &msg).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        state.capture_result.take().unwrap_or(Ok(()))
    }
}

impl Window {
    fn text<'a>(&self, en: &'a str, ko: &'a str) -> &'a str {
        if self.ko {
            ko
        } else {
            en
        }
    }

    fn add(&mut self, parent: HWND, class: PCWSTR, text: &str, id: usize, bounds: [i32; 4], style: u32, heading: bool) -> Result<HWND> {
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                class,
                &HSTRING::from(text),
                WS_CHILD | WS_VISIBLE | WINDOW_STYLE(style),
                0,
                0,
                0,
                0,
                Some(parent),
                Some(HMENU(id as *mut _)),
                None,
                None,
            )
        }?;
        self.controls.push(Control { hwnd, bounds, heading });
        Ok(hwnd)
    }

    fn label(&mut self, parent: HWND, text: &str, y: i32) -> Result<()> {
        self.add(parent, w!("STATIC"), text, 0, [24, y + 5, 174, 24], 0, false)?;
        Ok(())
    }

    fn combo(&mut self, parent: HWND, id: usize, y: i32, options: &[String], selected: &str) -> Result<()> {
        let hwnd = self.add(parent, w!("COMBOBOX"), "", id, [204, y, 292, 290], WS_TABSTOP.0 | WS_VSCROLL.0 | CBS_DROPDOWNLIST as u32, false)?;
        for option in options {
            let display = match option.as_str() {
                "auto" => self.text("Automatic", "자동"),
                "kr" => self.text("Korean grades", "한국 환경부 등급"),
                "us" => "US AQI",
                "12h" => self.text("12-hour", "12시간"),
                "24h" => self.text("24-hour", "24시간"),
                "C" => "°C",
                "F" => "°F",
                "0" => self.text("Normal", "기본"),
                "180" => self.text("Upside down (180°)", "뒤집기 (180°)"),
                other => language_name(other),
            };
            let text = HSTRING::from(display);
            unsafe {
                SendMessageW(hwnd, CB_ADDSTRING, None, Some(LPARAM(text.as_ptr() as isize)));
            }
        }
        let index = options.iter().position(|s| s == selected).unwrap_or(0);
        unsafe {
            SendMessageW(hwnd, CB_SETCURSEL, Some(WPARAM(index)), None);
        }
        Ok(())
    }

    fn populate(&mut self, hwnd: HWND) -> Result<()> {
        let c = self.document.config.clone();
        let strings = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        self.add(hwnd, w!("STATIC"), self.text("Dashboard settings", "대시보드 설정"), 0, [24, 16, 472, 30], 0, true)?;
        let rows = [(84, "Language", "언어"), (120, "Clock", "시계"), (156, "Temperature", "온도"), (192, "Panel", "패널 방향"), (228, "City", "도시"), (264, "Air quality", "미세먼지 기준")];
        for (y, en, ko) in rows {
            self.label(hwnd, self.text(en, ko), y)?;
        }
        self.combo(hwnd, LANGUAGE, 84, &self.languages.clone(), &c.language)?;
        self.combo(hwnd, CLOCK, 120, &strings(&["12h", "24h"]), &c.clock)?;
        self.combo(hwnd, TEMPERATURE, 156, &strings(&["C", "F"]), &c.temperature.to_ascii_uppercase())?;
        self.combo(hwnd, ROTATION, 192, &strings(&["0", "180"]), if c.rotation == 180 { "180" } else { "0" })?;
        let city = self.add(hwnd, w!("EDIT"), &c.location.city, CITY, [204, 228, 292, 27], WS_BORDER.0 | WS_TABSTOP.0 | ES_AUTOHSCROLL as u32, false)?;
        unsafe {
            SendMessageW(city, EM_SETLIMITTEXT, Some(WPARAM(120)), None);
        }
        self.combo(hwnd, AIR, 264, &strings(&["auto", "kr", "us"]), &c.air_quality.scale)?;
        self.label(hwnd, self.text("AI usage", "AI 사용량"), 301)?;
        for (id, name, x, width, checked) in [(CLAUDE, "Claude", 204, 85, c.ai.claude), (CODEX, "Codex", 292, 85, c.ai.codex), (ANTIGRAVITY, "Antigravity", 380, 116, c.ai.antigravity)] {
            let b = self.add(hwnd, w!("BUTTON"), name, id, [x, 301, width, 28], WS_TABSTOP.0 | BS_AUTOCHECKBOX as u32, false)?;
            unsafe {
                SendMessageW(b, BM_SETCHECK, Some(WPARAM(usize::from(checked))), None);
            }
        }
        let notice = if c.location.latitude.is_some() && c.location.longitude.is_some() {
            self.text("The coordinates in the file are used instead of the city; change them with Edit file.", "파일에 적힌 좌표가 도시보다 우선합니다. 좌표는 파일 편집에서 바꾸세요.")
        } else {
            self.text("Saving restarts the dashboard. More options (fans, disks, plan names) are in the file.", "저장하면 대시보드가 다시 시작됩니다. 팬·디스크 이름, 요금제 표시 등은 파일에서 바꿀 수 있습니다.")
        };
        self.add(hwnd, w!("STATIC"), notice, 0, [24, 346, 472, 40], 0, false)?;
        self.add(hwnd, w!("BUTTON"), self.text("Edit file…", "파일 편집…"), EDIT, [24, 396, 126, 30], WS_TABSTOP.0 | BS_PUSHBUTTON as u32, false)?;
        self.add(hwnd, w!("BUTTON"), self.text("Save", "저장"), SAVE, [298, 396, 94, 30], WS_TABSTOP.0 | BS_DEFPUSHBUTTON as u32, false)?;
        self.add(hwnd, w!("BUTTON"), self.text("Cancel", "취소"), CANCEL, [402, 396, 94, 30], WS_TABSTOP.0 | BS_PUSHBUTTON as u32, false)?;
        Ok(())
    }

    /// Place the controls and set their fonts for the current DPI.
    fn layout(&mut self) {
        unsafe {
            let scale = |n: i32| n * self.dpi as i32 / 96;
            let face = if self.ko { w!("Malgun Gothic") } else { w!("Segoe UI") };
            let font = |px, weight| CreateFontW(-scale(px), 0, 0, 0, weight, 0, 0, 0, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, CLEARTYPE_QUALITY, 0, face);
            let fonts = [font(14, 400), font(23, 600)];
            for c in &self.controls {
                let [x, y, w, h] = c.bounds.map(scale);
                let _ = MoveWindow(c.hwnd, x, y, w, h, true);
                SendMessageW(c.hwnd, WM_SETFONT, Some(WPARAM(fonts[usize::from(c.heading)].0 as usize)), Some(LPARAM(1)));
            }
            for old in self.fonts {
                if !old.0.is_null() {
                    let _ = DeleteObject(old.into());
                }
            }
            self.fonts = fonts;
        }
    }

    fn control(&self, id: usize) -> HWND {
        self.controls.iter().find(|c| unsafe { GetDlgCtrlID(c.hwnd) } == id as i32).map(|c| c.hwnd).unwrap_or_default()
    }

    fn selection(&self, id: usize) -> Result<usize> {
        let i = unsafe { SendMessageW(self.control(id), CB_GETCURSEL, None, None).0 };
        anyhow::ensure!(i >= 0, "Select a value in every list.");
        Ok(i as usize)
    }

    /// The settings as the form shows them.
    fn read(&self) -> Result<Config> {
        let mut c = self.document.config.clone();
        c.language = self.languages[self.selection(LANGUAGE)?].clone();
        c.clock = ["12h", "24h"][self.selection(CLOCK)?].into();
        let unit = ["C", "F"][self.selection(TEMPERATURE)?];
        if !c.temperature.eq_ignore_ascii_case(unit) {
            c.temperature = unit.into();
        }
        c.rotation = [0, 180][self.selection(ROTATION)?];
        let mut city = [0u16; 256];
        let len = unsafe { GetWindowTextW(self.control(CITY), &mut city) };
        c.location.city = String::from_utf16_lossy(&city[..len as usize]).trim().into();
        c.air_quality.scale = ["auto", "kr", "us"][self.selection(AIR)?].into();
        let checked = |id| unsafe { SendMessageW(self.control(id), BM_GETCHECK, None, None).0 != 0 };
        c.ai.claude = checked(CLAUDE);
        c.ai.codex = checked(CODEX);
        c.ai.antigravity = checked(ANTIGRAVITY);
        document::validate(&c)?;
        Ok(c)
    }

    fn error(&self, hwnd: HWND, e: anyhow::Error) {
        let text = format!("{}\n\n{e:#}", self.text("The settings could not be saved.", "설정을 저장하지 못했습니다."));
        unsafe {
            MessageBoxW(Some(hwnd), &HSTRING::from(text), &HSTRING::from(self.text("Settings", "설정")), MB_OK | MB_ICONERROR);
        }
    }
}

unsafe extern "system" fn procedure(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Window;
        // controls send messages back while being created and painted: only borrow the state
        // for the messages that need it
        if !ptr.is_null() && matches!(msg, WM_COMMAND | WM_TIMER | WM_DPICHANGED) {
            let state = &mut *ptr;
            match msg {
                WM_TIMER if wp.0 == CAPTURE_TIMER => {
                    let _ = KillTimer(Some(hwnd), CAPTURE_TIMER);
                    if let Some(path) = state.capture.take() {
                        state.capture_result = Some(review::capture(hwnd, &path));
                        let _ = DestroyWindow(hwnd);
                    }
                    return LRESULT(0);
                }
                WM_COMMAND => {
                    match wp.0 & 0xffff {
                        SAVE => match state.read().and_then(|edited| state.document.save(&edited)) {
                            Ok(()) => {
                                let _ = DestroyWindow(hwnd);
                            }
                            Err(e) => state.error(hwnd, e),
                        },
                        CANCEL => {
                            let _ = DestroyWindow(hwnd);
                        }
                        EDIT => match state.document.edit_file().context("open the settings file") {
                            Ok(()) => {
                                let _ = DestroyWindow(hwnd);
                            }
                            Err(e) => state.error(hwnd, e),
                        },
                        _ => {}
                    }
                    return LRESULT(0);
                }
                WM_DPICHANGED => {
                    state.dpi = (wp.0 & 0xffff) as u32;
                    let r = &*(lp.0 as *const RECT);
                    let _ = SetWindowPos(hwnd, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
                    state.layout();
                    return LRESULT(0);
                }
                _ => {}
            }
        }
        if msg == WM_DESTROY {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            PostQuitMessage(0);
            return LRESULT(0);
        }
        DefWindowProcW(hwnd, msg, wp, lp)
    }
}
