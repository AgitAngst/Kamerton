// Мелкие вызовы Win32 для окна и трея: один экземпляр, консоль, мониторы, показ окна, тема панели задач,
// значки из кадров RGBA.

use std::time::{SystemTime, UNIX_EPOCH};

use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAK, DwmSetWindowAttribute};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, CreateRoundRectRgn, DIB_RGB_COLORS,
    DeleteObject, GetMonitorInfoW, HGDIOBJ, InvalidateRect, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
    SetWindowRgn,
};
use windows::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, GetDpiForWindow, GetSystemMetricsForDpi, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateIconIndirect, HICON, HWND_TOPMOST, ICON_BIG, ICON_SMALL, ICONINFO, PostMessageW, SM_CXICON, SM_CXSMICON,
    SW_HIDE, SW_SHOWNOACTIVATE, SW_SHOWNORMAL, SWP_NOACTIVATE, SWP_NOZORDER, SendMessageW, SetForegroundWindow,
    SetWindowPos, ShowWindow, WM_CLOSE, WM_SETICON,
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

pub fn cursor() -> (i32, i32) {
    let mut p = POINT::default();
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut p);
    }
    (p.x, p.y)
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

/// Плащ DWM: окно остаётся «показанным» (рисуется), но его не видно. Так окно проявляется мгновенно.
pub fn cloak(raw: isize, on: bool) {
    let v: BOOL = on.into();
    unsafe {
        let _ = DwmSetWindowAttribute(hwnd(raw), DWMWA_CLOAK, &v as *const BOOL as *const _, size_of::<BOOL>() as u32);
    }
}

/// Показать окно, не давая фокус (под плащом — невидимо, но ему снова идёт WM_PAINT).
pub fn show_noactivate(raw: isize) {
    unsafe {
        let _ = ShowWindow(hwnd(raw), SW_SHOWNOACTIVATE);
    }
}

/// Попросить у Windows перерисовку окна (WM_PAINT) напрямую, из любого потока.
pub fn invalidate(raw: isize) {
    unsafe {
        let _ = InvalidateRect(Some(hwnd(raw)), None, false);
    }
}

pub fn hide(raw: isize) {
    unsafe {
        let _ = ShowWindow(hwnd(raw), SW_HIDE);
    }
}

pub fn post_close(raw: isize) {
    unsafe {
        let _ = PostMessageW(Some(hwnd(raw)), WM_CLOSE, WPARAM(0), LPARAM(0));
    }
}

/// Дать окну фокус — только когда его открыл щелчок пользователя.
pub fn activate(raw: isize) {
    unsafe {
        let _ = SetForegroundWindow(hwnd(raw));
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

/// Запустить программу с параметрами (`control.exe`, `mmsys.cpl,,1`).
pub fn run(file: &str, params: &str) {
    let (f, p) = (wide(file), wide(params));
    unsafe {
        ShellExecuteW(None, w!("open"), PCWSTR(f.as_ptr()), PCWSTR(p.as_ptr()), None, SW_SHOWNORMAL);
    }
}

const RUN_KEY: windows::core::PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");

/// Запускается ли Kamerton при входе в Windows (`HKCU\…\Run`, значение `Kamerton`).
pub fn autostart() -> bool {
    let mut size = 0u32;
    unsafe {
        RegGetValueW(HKEY_CURRENT_USER, RUN_KEY, w!("Kamerton"), RRF_RT_REG_SZ, None, None, Some(&mut size)).is_ok()
    }
}

/// Включить или выключить запуск при входе: путь к этому exe в кавычках.
pub fn set_autostart(on: bool) {
    unsafe {
        if on {
            let Ok(exe) = std::env::current_exe() else { return };
            let value = wide(&format!("\"{}\"", exe.display()));
            let _ = RegSetKeyValueW(
                HKEY_CURRENT_USER,
                RUN_KEY,
                w!("Kamerton"),
                REG_SZ.0,
                Some(value.as_ptr() as *const _),
                (value.len() * 2) as u32,
            );
        } else {
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, w!("Kamerton"));
        }
    }
}

/// Поставить обычное (не «поверх всех») окно.
pub fn place_normal(raw: isize, r: [i32; 4]) {
    unsafe {
        let _ = SetWindowPos(hwnd(raw), None, r[0], r[1], r[2], r[3], SWP_NOACTIVATE | SWP_NOZORDER);
    }
}

/// Подогнать окно под размер содержимого `[ширина, высота]` (физические пиксели клиентской части):
/// не больше рабочей области монитора; `keep_on_screen` — сдвинуть, чтобы окно не вылезало за край.
pub fn fit_client(raw: isize, client: [i32; 2], keep_on_screen: bool) {
    use windows::Win32::UI::WindowsAndMessaging::GetClientRect;
    let Some(win) = window_rect(raw) else { return };
    let mut c = RECT::default();
    if unsafe { GetClientRect(hwnd(raw), &mut c) }.is_err() {
        return;
    }
    // Рамка и заголовок — разница окна и его клиентской части.
    let (dw, dh) = ((win.right - win.left) - (c.right - c.left), (win.bottom - win.top) - (c.bottom - c.top));
    let (_, work, _) = monitor_at((win.left + win.right) / 2, (win.top + win.bottom) / 2);
    let w = (client[0] + dw).min(work.right - work.left);
    let h = (client[1] + dh).min(work.bottom - work.top);
    let (mut x, mut y) = (win.left, win.top);
    if keep_on_screen {
        x = x.min(work.right - w).max(work.left);
        y = y.min(work.bottom - h).max(work.top);
    }
    if [x, y, w, h] == [win.left, win.top, win.right - win.left, win.bottom - win.top] {
        return;
    }
    unsafe {
        let _ = SetWindowPos(hwnd(raw), None, x, y, w, h, SWP_NOACTIVATE | SWP_NOZORDER);
    }
}

/// Найти окно по заголовку и вывести наверх (второй `--mixer`).
pub fn raise_window(title: &str) -> bool {
    let t = wide(title);
    unsafe {
        match windows::Win32::UI::WindowsAndMessaging::FindWindowW(None, PCWSTR(t.as_ptr())) {
            Ok(h) if !h.is_invalid() => {
                let _ = ShowWindow(h, windows::Win32::UI::WindowsAndMessaging::SW_RESTORE);
                let _ = SetForegroundWindow(h);
                true
            }
            _ => false,
        }
    }
}

/// Захватить «окно микшера уже открыто». `None` — уже открыто.
pub fn single_mixer() -> Option<HANDLE> {
    let h = unsafe { CreateMutexW(None, true, w!("Local\\KamertonMixer")) }.ok()?;
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS { None } else { Some(h) }
}

/// Разрешить процессу окна взять фокус: щелчок пришёл трею, а окно — другой процесс.
pub fn allow_foreground(pid: u32) {
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(pid);
    }
}

/// Прямоугольник окна на экране.
pub fn window_rect(raw: isize) -> Option<Rect> {
    let mut r = RECT::default();
    unsafe { windows::Win32::UI::WindowsAndMessaging::GetWindowRect(hwnd(raw), &mut r) }.ok()?;
    Some(rect(r))
}

/// Открыть адрес или программу оболочкой Windows (`ms-settings:sound`, `sndvol.exe`).
pub fn open(target: &str) {
    let t = wide(target);
    unsafe {
        ShellExecuteW(None, w!("open"), PCWSTR(t.as_ptr()), None, None, SW_SHOWNORMAL);
    }
}

/// Тихий системный звук «по умолчанию» — после смены громкости устройства.
pub fn beep() {
    unsafe {
        let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(windows::Win32::UI::WindowsAndMessaging::MB_OK);
    }
}

/// Своя память процесса (PrivateUsage), МБ.
pub fn private_mb() -> f64 {
    use windows::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
    };
    let mut c = PROCESS_MEMORY_COUNTERS_EX { cb: size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32, ..Default::default() };
    unsafe {
        let _ = GetProcessMemoryInfo(
            windows::Win32::System::Threading::GetCurrentProcess(),
            &mut c as *mut PROCESS_MEMORY_COUNTERS_EX as *mut PROCESS_MEMORY_COUNTERS,
            c.cb,
        );
    }
    c.PrivateUsage as f64 / 1_048_576.0
}

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Значок Windows из кадра RGBA (прямая альфа, как в файлах `.ico`). Отдать `DestroyIcon`, когда не нужен.
pub fn icon_from_rgba(rgba: &[u8], size: i32) -> HICON {
    unsafe {
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size,
                biHeight: -size,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let Ok(dib) = CreateDIBSection(None, &bmi, DIB_RGB_COLORS, &mut bits, None, 0) else {
            return HICON::default();
        };
        let px = std::slice::from_raw_parts_mut(bits as *mut u8, (size * size * 4) as usize);
        for (d, s) in px.as_chunks_mut::<4>().0.iter_mut().zip(rgba.as_chunks::<4>().0) {
            *d = [s[2], s[1], s[0], s[3]];
        }
        let mask = CreateBitmap(size, size, 1, 1, None);
        let info = ICONINFO { fIcon: true.into(), xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: dib };
        let icon = CreateIconIndirect(&info).unwrap_or_default();
        let _ = DeleteObject(HGDIOBJ(mask.0));
        let _ = DeleteObject(HGDIOBJ(dib.0));
        icon
    }
}

/// Значок окна (заголовок, панель задач, Alt+Tab) — знак программы в положениях файла, по DPI окна, цветом из
/// «Настроек». Прежний значок окна не освобождается: смена цвета редкая, утечка — пара значков.
pub fn set_window_icon(raw: isize, color: crate::mark_render::IconColor) {
    unsafe {
        let dpi = GetDpiForWindow(hwnd(raw)).max(96);
        for (which, metric) in [(ICON_SMALL, SM_CXSMICON), (ICON_BIG, SM_CXICON)] {
            let size = GetSystemMetricsForDpi(metric, dpi).max(16);
            let rgba = crate::mark_render::mark(&crate::mark_render::rest(size as u32), size as u32, color);
            let icon = icon_from_rgba(&rgba, size);
            SendMessageW(hwnd(raw), WM_SETICON, Some(WPARAM(which as usize)), Some(LPARAM(icon.0 as isize)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_from_a_frame() {
        let rgba = crate::mark_render::mark(&crate::mark_render::rest(32), 32, crate::mark_render::IconColor::Purple);
        let icon = icon_from_rgba(&rgba, 32);
        assert!(!icon.is_invalid());
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::DestroyIcon(icon);
        }
    }
}
