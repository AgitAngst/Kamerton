// Мелкие вызовы Win32 для окна и трея: один экземпляр, консоль, мониторы, показ окна, тема панели задач.

use std::time::{SystemTime, UNIX_EPOCH};

use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateRoundRectRgn, GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint, SetWindowRgn,
};
use windows::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, EnumWindows, GetWindowThreadProcessId, HWND_TOPMOST, PostMessageW, SW_SHOWNORMAL,
    SWP_NOACTIVATE, SetForegroundWindow, SetWindowPos, WM_CLOSE,
};
use windows::core::{BOOL, PCWSTR, w};

use crate::placement::Rect;

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Захватить «Kamerton уже запущен». `None` — уже запущен другой экземпляр. Дескриптор держать до выхода.
pub fn single_instance() -> Option<HANDLE> {
    let h = unsafe { CreateMutexW(None, true, w!("Local\\Kamerton")) }.ok()?;
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS { None } else { Some(h) }
}

/// Release-сборка — без своей консоли: для консольных ключей подключаемся к консоли родителя.
pub fn attach_console() {
    unsafe {
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

fn rect(r: RECT) -> Rect {
    Rect { left: r.left, top: r.top, right: r.right, bottom: r.bottom }
}

/// Монитор у точки: весь монитор, рабочая область и масштаб (1.0 = 96 dpi).
pub fn monitor_at(x: i32, y: i32) -> (Rect, Rect, f32) {
    unsafe {
        let mon = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(mon, &mut info);
        let (mut dx, mut dy) = (96u32, 96u32);
        let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        (rect(info.rcMonitor), rect(info.rcWork), dx as f32 / 96.0)
    }
}

pub fn hwnd(raw: isize) -> HWND {
    HWND(raw as *mut _)
}

/// Поставить окно (поверх всех), не показывая и не активируя.
pub fn move_to(raw: isize, x: i32, y: i32, w: i32, h: i32) {
    unsafe {
        let _ = SetWindowPos(hwnd(raw), Some(HWND_TOPMOST), x, y, w, h, SWP_NOACTIVATE);
    }
}

/// Дать окну фокус — только когда его открыл щелчок пользователя.
pub fn activate(raw: isize) {
    unsafe {
        let _ = SetForegroundWindow(hwnd(raw));
    }
}

/// Попросить закрыться все окна процесса (`WM_CLOSE`).
pub fn close_windows_of(pid: u32) {
    unsafe extern "system" fn each(h: HWND, lparam: LPARAM) -> BOOL {
        let mut owner = 0u32;
        unsafe {
            GetWindowThreadProcessId(h, Some(&mut owner));
            if owner == lparam.0 as u32 {
                let _ = PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
        true.into()
    }
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(pid as isize));
    }
}

/// Разрешить процессу окна взять фокус: щелчок пришёл трею, а окно — другой процесс.
pub fn allow_foreground(pid: u32) {
    unsafe {
        let _ = AllowSetForegroundWindow(pid);
    }
}

/// Скруглить углы окна регионом: прозрачность окна OpenGL на Windows ненадёжна.
pub fn round_corners(raw: isize, w: i32, h: i32, radius: i32) {
    unsafe {
        let rgn = CreateRoundRectRgn(0, 0, w + 1, h + 1, radius * 2, radius * 2);
        let _ = SetWindowRgn(hwnd(raw), Some(rgn), true);
    }
}

/// Светлая ли панель задач (`SystemUsesLightTheme`): по ней цвет значка в трее.
pub fn taskbar_light() -> bool {
    let mut value = 0u32;
    let mut size = size_of::<u32>() as u32;
    let ok = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("SystemUsesLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut _),
            Some(&mut size),
        )
    };
    ok.is_ok() && value == 1
}

/// Открыть адрес или программу оболочкой Windows (`ms-settings:sound`, `sndvol.exe`).
pub fn open(target: &str) {
    let t = wide(target);
    unsafe {
        ShellExecuteW(None, w!("open"), PCWSTR(t.as_ptr()), None, None, SW_SHOWNORMAL);
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}
