// Значок в трее: свой поток с невидимым окном сообщений. Щелчок — открыть или закрыть окно микшера
// (оно живёт в этом же процессе, `flyout::Shared`), правая кнопка — меню, колесо над значком — громкость
// устройства по умолчанию (хук мыши), вид значка — уровень громкости глифом системного шрифта.

use std::sync::atomic::{AtomicI32, AtomicIsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    ANTIALIASED_QUALITY, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CLIP_DEFAULT_PRECIS, CreateBitmap, CreateCompatibleDC,
    CreateDIBSection, CreateFontW, DEFAULT_CHARSET, DEFAULT_PITCH, DIB_RGB_COLORS, DT_CENTER, DT_NOCLIP, DT_SINGLELINE,
    DT_VCENTER, DeleteDC, DeleteObject, DrawTextW, FW_NORMAL, GetTextFaceW, HGDIOBJ, OUT_DEFAULT_PRECIS, SelectObject,
    SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetSystemMetricsForDpi};
use windows::Win32::UI::Input::KeyboardAndMouse::{HOT_KEY_MODIFIERS, MOD_NOREPEAT, RegisterHotKey, UnregisterHotKey};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NIN_SELECT, NINF_KEY,
    NOTIFYICON_VERSION_4, NOTIFYICONDATAW, NOTIFYICONIDENTIFIER, Shell_NotifyIconGetRect, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CallNextHookEx, CreateIconIndirect, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon,
    DestroyMenu, DestroyWindow, DispatchMessageW, GetMessageW, HHOOK, HICON, HMENU, ICONINFO, KillTimer,
    MENU_ITEM_FLAGS, MF_CHECKED, MF_GRAYED, MF_POPUP, MF_SEPARATOR, MF_STRING, MF_UNCHECKED, MSG, MSLLHOOKSTRUCT,
    PostMessageW, PostQuitMessage, RegisterClassW, RegisterWindowMessageW, SM_CXSMICON, SetForegroundWindow, SetTimer,
    SetWindowsHookExW, TPM_BOTTOMALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage,
    UnhookWindowsHookEx, WH_MOUSE_LL, WINDOW_EX_STYLE, WM_APP, WM_CLOSE, WM_CONTEXTMENU, WM_DESTROY, WM_DISPLAYCHANGE,
    WM_MBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NULL, WM_SETTINGCHANGE, WNDCLASSW, WS_POPUP,
};
use windows::core::{PCWSTR, w};

use crate::audio::{Control, Flow, Roles, Snapshot, Target, model};
use crate::flyout::Shared;
use crate::hotkeys::{Action, AppAction, Combo, next_output};
use crate::placement::Rect;
use crate::profiles::{self, Cmd};
use crate::sys;
use crate::texts::T;

const WM_TRAY: u32 = WM_APP + 1;
const WM_WHEEL: u32 = WM_APP + 2;
const WM_REFRESH: u32 = WM_APP + 3;
const WM_RELOAD_HOTKEYS: u32 = WM_APP + 4;
const WM_TIMER: u32 = 0x0113;
/// Таймер отложенной записи настроек (память громкости меняется часто — пишем не чаще раза в 2 с).
const SAVE_TIMER: usize = 1;
const WM_HOTKEY: u32 = 0x0312;
const ICON_ID: u32 = 1;
/// Выбор значка с клавиатуры (Win+B, стрелки, Enter).
const NIN_KEYSELECT: u32 = NIN_SELECT | NINF_KEY;
/// Шаг колеса над значком — 2 % на щелчок (`WHEEL_DELTA` = 120).
const WHEEL_STEP: f32 = 0.02;

const CMD_MIXER: u32 = 1;
const CMD_SETTINGS: u32 = 2;
const CMD_EXIT: u32 = 3;
const CMD_PLAYBACK: u32 = 4;
const CMD_RECORDING: u32 = 5;
const CMD_SOUNDS: u32 = 6;
/// Пункты устройств: `CMD_DEVICE + номер` в списке устройств вывода.
const CMD_DEVICE: u32 = 100;
/// Пункты профилей: `CMD_PROFILE + номер`.
const CMD_PROFILE: u32 = 500;

/// Последний снимок трея — от него считается разница по каждому `refresh`.
static LAST: Mutex<Option<Snapshot>> = Mutex::new(None);

/// Окно трея — для `refresh` из потока звука и для хука.
static TRAY_HWND: AtomicIsize = AtomicIsize::new(0);
/// Прямоугольник значка для хука колеса (физические пиксели); обновляется, когда курсор над значком.
static ICON_RECT: [AtomicI32; 4] = [AtomicI32::new(0), AtomicI32::new(0), AtomicI32::new(0), AtomicI32::new(0)];

struct Ctx {
    control: Control,
    shared: Arc<Shared>,
    taskbar_created: u32,
}

static CTX: OnceLock<Ctx> = OnceLock::new();

/// Вид значка по громкости устройства по умолчанию.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Mute,
    Zero,
    Low,
    Mid,
    High,
}

impl Level {
    pub fn of(volume: f32, muted: bool) -> Level {
        let p = model::percent(volume);
        match () {
            _ if muted => Level::Mute,
            _ if p == 0 => Level::Zero,
            _ if p <= 33 => Level::Low,
            _ if p <= 66 => Level::Mid,
            _ => Level::High,
        }
    }

    /// Глиф в Segoe Fluent Icons и Segoe MDL2 Assets (коды одинаковые).
    fn glyph(self) -> u16 {
        match self {
            Level::Mute => 0xE74F,
            Level::Zero => 0xE992,
            Level::Low => 0xE993,
            Level::Mid => 0xE994,
            Level::High => 0xE995,
        }
    }
}

/// Шаг громкости по повороту колеса (`delta` в единицах Windows, 120 на щелчок).
pub fn wheel_step(delta: i16) -> f32 {
    delta as f32 / 120.0 * WHEEL_STEP
}

/// Что висит на зарегистрированном сочетании.
#[derive(Debug, Clone)]
enum Bound {
    Global(Action),
    App(AppAction, String),
}

/// Зарегистрированные сочетания трея: id → действие. Живут в потоке трея.
static BOUND: Mutex<Vec<(i32, Bound)>> = Mutex::new(Vec::new());

/// Попросить трей перерегистрировать сочетания (поменялись настройки или идёт запись сочетания).
pub fn reload_hotkeys() {
    let h = TRAY_HWND.load(Ordering::SeqCst);
    if h != 0 {
        unsafe {
            let _ = PostMessageW(Some(sys::hwnd(h)), WM_RELOAD_HOTKEYS, WPARAM(0), LPARAM(0));
        }
    }
}

/// Снять все сочетания и зарегистрировать заново по настройкам; пока окно пишет сочетание — никаких.
fn register_hotkeys(hwnd: HWND) {
    let ctx = CTX.get().expect("tray context");
    let mut bound = BOUND.lock().unwrap();
    for (id, _) in bound.drain(..) {
        unsafe {
            let _ = UnregisterHotKey(Some(hwnd), id);
        }
    }
    let mut free = std::collections::HashMap::new();
    if !ctx.shared.capturing.load(Ordering::SeqCst) {
        let settings = ctx.shared.settings.lock().unwrap().clone();
        let mut items: Vec<(String, Bound, Combo)> = Vec::new();
        for (a, c) in &settings.hotkeys {
            items.push((format!("hotkey.{}", a.key()), Bound::Global(*a), *c));
        }
        for ((a, exe), c) in &settings.app_hotkeys {
            items.push((format!("apphotkey.{}.{}", a.key(), exe), Bound::App(*a, exe.clone()), *c));
        }
        for (n, (key, what, combo)) in items.into_iter().enumerate() {
            let id = n as i32 + 1;
            let Some(vk) = combo.vk() else { continue };
            let mods = HOT_KEY_MODIFIERS(combo.mods()) | MOD_NOREPEAT;
            let ok = unsafe { RegisterHotKey(Some(hwnd), id, mods, vk) }.is_ok();
            free.insert(key, ok);
            if ok {
                bound.push((id, what));
            }
        }
    }
    if std::env::var_os("KAMERTON_TRACE").is_some() {
        eprintln!("trace: hotkeys {free:?}");
    }
    *ctx.shared.hotkey_free.lock().unwrap() = free;
    ctx.shared.request_repaint();
}

/// Программы с этим exe на всех устройствах вывода.
pub fn apps_with_exe<'a>(s: &'a crate::audio::Snapshot, exe: &str) -> Vec<&'a crate::audio::App> {
    s.devices
        .iter()
        .filter(|d| d.flow == Flow::Render)
        .flat_map(|d| d.apps.iter())
        .filter(|a| a.key.exe == exe)
        .collect()
}

fn run_hotkey(hwnd: HWND, id: i32) {
    let ctx = CTX.get().expect("tray context");
    let what = BOUND.lock().unwrap().iter().find(|(i, _)| *i == id).map(|(_, w)| w.clone());
    let Some(what) = what else { return };
    let c = &ctx.control;
    match what {
        Bound::Global(Action::Open) => {
            let (x, y) = sys::cursor();
            let anchor = icon_rect(hwnd).unwrap_or(Rect { left: x, top: y, right: x + 1, bottom: y + 1 });
            ctx.shared.toggle(anchor);
        }
        Bound::Global(Action::Louder) => {
            let _ = c.adjust_default(STEP);
        }
        Bound::Global(Action::Quieter) => {
            let _ = c.adjust_default(-STEP);
        }
        Bound::Global(Action::Mute) => toggle_default_mute(c),
        Bound::Global(Action::NextOutput) => {
            if let Ok(s) = c.snapshot() {
                let outs: Vec<&crate::audio::Device> = s.devices.iter().filter(|d| d.flow == Flow::Render).collect();
                let ids: Vec<&str> = outs.iter().map(|d| d.id.as_str()).collect();
                let current = outs.iter().find(|d| d.is_default).map(|d| d.id.as_str());
                if let Some(next) = next_output(&ids, current) {
                    let _ = c.set_default(next, Roles::All);
                }
            }
        }
        Bound::App(action, exe) => {
            let Ok(s) = c.snapshot() else { return };
            let apps = apps_with_exe(&s, &exe);
            // «Без звука» по всем сессиям: хоть одна звучит — заглушить все, иначе включить все.
            let mute_all = apps.iter().any(|a| !a.muted);
            for a in apps {
                let t = Target::App(a.key.clone());
                let _ = match action {
                    AppAction::Louder => c.set_volume(t, model::step(a.volume, STEP)),
                    AppAction::Quieter => c.set_volume(t, model::step(a.volume, -STEP)),
                    AppAction::Mute => c.set_muted(t, mute_all),
                };
            }
        }
    }
}

/// Выполнить команды профилей и памяти.
pub fn run_cmds(c: &Control, cmds: Vec<Cmd>) {
    for cmd in cmds {
        let _ = match cmd {
            Cmd::SetDefault(id) => c.set_default(&id, Roles::All),
            Cmd::SetVolume(k, v) => c.set_volume(Target::App(k), v),
            Cmd::SetMuted(k, m) => c.set_muted(Target::App(k), m),
            Cmd::Move(k, to) => c.move_app(k, to),
        };
    }
}

/// Применить профиль номер `i` (из меню значка, со страницы «Профили» или по правилу).
pub fn run_profile(shared: &Shared, c: &Control, i: usize) {
    let Ok(now) = c.snapshot() else { return };
    let cmds = {
        let mut st = shared.settings.lock().unwrap();
        let Some(p) = st.profiles.get(i).cloned() else { return };
        let mut mem = std::mem::take(&mut st.memory);
        let cmds = profiles::apply(&p, &now, &mut mem);
        st.memory = mem;
        st.save();
        cmds
    };
    run_cmds(c, cmds);
}

/// Изменения ядра: память громкости, громкость новых программ, правило подключения.
fn react_to_changes(hwnd: HWND) {
    let ctx = CTX.get().expect("tray context");
    let Ok(now) = ctx.control.snapshot() else { return };
    let prev = LAST.lock().unwrap().replace(now.clone());
    let Some(prev) = prev else { return };
    let events = model::diff(&prev, &now);
    if events.is_empty() {
        return;
    }
    let (cmds, dirty, profile) = {
        let mut st = ctx.shared.settings.lock().unwrap();
        let list = st.profiles.clone();
        let rules = profiles::Rules {
            remember: st.remember,
            new_app_volume: st.new_app_volume.map(|p| p as f32 / 100.0),
            profiles: &list,
        };
        let mut mem = std::mem::take(&mut st.memory);
        let out = profiles::react(&events, &now, &rules, &mut mem);
        st.memory = mem;
        out
    };
    if dirty {
        unsafe {
            let _ = SetTimer(Some(hwnd), SAVE_TIMER, 2000, None);
        }
    }
    run_cmds(&ctx.control, cmds);
    if let Some(i) = profile {
        run_profile(&ctx.shared, &ctx.control, i);
    }
}

/// Шаг громкости сочетаний — как колесо: 2 %.
const STEP: f32 = 0.02;

/// Разбудить трей: перечитать громкость для значка. Зовётся из потока звука после изменений.
pub fn refresh() {
    let h = TRAY_HWND.load(Ordering::SeqCst);
    if h != 0 {
        unsafe {
            let _ = PostMessageW(Some(sys::hwnd(h)), WM_REFRESH, WPARAM(0), LPARAM(0));
        }
    }
}

/// Запустить поток трея.
pub fn start(control: Control, shared: Arc<Shared>) {
    let _ = std::thread::Builder::new().name("tray".into()).spawn(move || run(control, shared));
}

/// Значок и цикл сообщений трея; возвращается после «Выхода».
fn run(control: Control, shared: Arc<Shared>) {
    unsafe {
        let taskbar_created = RegisterWindowMessageW(w!("TaskbarCreated"));
        let _ = CTX.set(Ctx { control, shared, taskbar_created });
        let instance = GetModuleHandleW(None).unwrap_or_default();
        let class = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: w!("KamertonTray"),
            ..Default::default()
        };
        RegisterClassW(&class);
        // Обычное (не message-only) невидимое окно: только оно получает TaskbarCreated после перезапуска
        // проводника.
        let Ok(hwnd) = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("KamertonTray"),
            w!("Kamerton tray"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        ) else {
            eprintln!("tray: cannot create window");
            return;
        };
        TRAY_HWND.store(hwnd.0 as isize, Ordering::SeqCst);
        add_icon(hwnd);
        if std::env::var_os("KAMERTON_TRACE").is_some() {
            eprintln!("trace: tray icon rect {:?}", icon_rect(hwnd));
        }
        // Проверка вида: открыть окно у значка сразу, без фокуса.
        if std::env::var_os("KAMERTON_OPEN").is_some()
            && let Some(r) = icon_rect(hwnd)
        {
            let shared = CTX.get().expect("tray context").shared.clone();
            // `KAMERTON_OPEN=2` — открыть через 2 с и ещё раз через 4 с (замер открытия после прогрева).
            if std::env::var("KAMERTON_OPEN").as_deref() == Ok("2") {
                std::thread::spawn(move || {
                    for _ in 0..2 {
                        std::thread::sleep(std::time::Duration::from_secs(2));
                        shared.open(r, false);
                    }
                });
            } else {
                shared.open(r, false);
            }
        }
        register_hotkeys(hwnd);
        let hook = SetWindowsHookExW(WH_MOUSE_LL, Some(wheel_hook), Some(instance.into()), 0).ok();
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        if let Some(h) = hook {
            let _ = UnhookWindowsHookEx(h);
        }
        TRAY_HWND.store(0, Ordering::SeqCst);
    }
}

fn base_data(hwnd: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW { cbSize: size_of::<NOTIFYICONDATAW>() as u32, hWnd: hwnd, uID: ICON_ID, ..Default::default() }
}

unsafe fn add_icon(hwnd: HWND) {
    let mut d = base_data(hwnd);
    d.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
    d.uCallbackMessage = WM_TRAY;
    let (icon, tip) = current_look();
    d.hIcon = icon;
    put_tip(&mut d, &tip);
    unsafe {
        let _ = Shell_NotifyIconW(NIM_ADD, &d);
        d.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        let _ = Shell_NotifyIconW(NIM_SETVERSION, &d);
        let _ = DestroyIcon(icon);
    }
    update_rect(hwnd);
}

unsafe fn update_look(hwnd: HWND) {
    let mut d = base_data(hwnd);
    d.uFlags = NIF_ICON | NIF_TIP | NIF_SHOWTIP;
    let (icon, tip) = current_look();
    d.hIcon = icon;
    put_tip(&mut d, &tip);
    unsafe {
        let _ = Shell_NotifyIconW(NIM_MODIFY, &d);
        let _ = DestroyIcon(icon);
    }
}

fn put_tip(d: &mut NOTIFYICONDATAW, tip: &str) {
    let w: Vec<u16> = tip.encode_utf16().take(d.szTip.len() - 1).collect();
    d.szTip[..w.len()].copy_from_slice(&w);
    d.szTip[w.len()] = 0;
}

/// Значок и подсказка по устройству вывода по умолчанию.
fn current_look() -> (HICON, String) {
    let ctx = CTX.get().expect("tray context");
    let default = ctx
        .control
        .snapshot()
        .ok()
        .and_then(|s| s.devices.into_iter().find(|d| d.flow == Flow::Render && d.is_default));
    let (level, tip) = match &default {
        Some(d) => (Level::of(d.volume, d.muted), format!("Kamerton: {} — {} %", d.name, model::percent(d.volume))),
        None => (Level::Mute, format!("Kamerton: {}", T::NoDevices.get())),
    };
    (make_icon(level, sys::taskbar_light()), tip)
}

/// Нарисовать глиф GDI белым на чёрном, яркость — в прозрачность, цвет — по теме панели задач.
fn make_icon(level: Level, light_taskbar: bool) -> HICON {
    unsafe {
        let size = GetSystemMetricsForDpi(SM_CXSMICON, GetDpiForSystem()).max(16);
        let dc = CreateCompatibleDC(None);
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
        let Ok(dib) = CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) else {
            let _ = DeleteDC(dc);
            return HICON::default();
        };
        let old_bmp = SelectObject(dc, HGDIOBJ(dib.0));
        let font = icon_font(dc, size);
        let old_font = SelectObject(dc, HGDIOBJ(font.0));
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, COLORREF(0x00FF_FFFF));
        let mut glyph = [level.glyph()];
        let mut r = RECT { left: 0, top: 0, right: size, bottom: size };
        DrawTextW(dc, &mut glyph, &mut r, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOCLIP);
        let px = std::slice::from_raw_parts_mut(bits as *mut u8, (size * size * 4) as usize);
        let c: u32 = if light_taskbar { 0 } else { 255 };
        for p in px.as_chunks_mut::<4>().0 {
            let a = p[0].max(p[1]).max(p[2]) as u32;
            let v = (c * a / 255) as u8;
            p[0] = v;
            p[1] = v;
            p[2] = v;
            p[3] = a as u8;
        }
        SelectObject(dc, old_font);
        SelectObject(dc, old_bmp);
        let _ = DeleteObject(HGDIOBJ(font.0));
        let mask = CreateBitmap(size, size, 1, 1, None);
        let info = ICONINFO { fIcon: true.into(), xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: dib };
        let icon = CreateIconIndirect(&info).unwrap_or_default();
        let _ = DeleteObject(HGDIOBJ(mask.0));
        let _ = DeleteObject(HGDIOBJ(dib.0));
        let _ = DeleteDC(dc);
        icon
    }
}

/// Шрифт значков: Segoe Fluent Icons (Windows 11), иначе Segoe MDL2 Assets (Windows 10).
unsafe fn icon_font(dc: windows::Win32::Graphics::Gdi::HDC, size: i32) -> windows::Win32::Graphics::Gdi::HFONT {
    for face in [w!("Segoe Fluent Icons"), w!("Segoe MDL2 Assets")] {
        unsafe {
            let font = CreateFontW(
                -size,
                0,
                0,
                0,
                FW_NORMAL.0 as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                ANTIALIASED_QUALITY,
                DEFAULT_PITCH.0 as u32,
                face,
            );
            let old = SelectObject(dc, HGDIOBJ(font.0));
            let mut name = [0u16; 64];
            let n = GetTextFaceW(dc, Some(&mut name)) as usize;
            SelectObject(dc, old);
            let got = String::from_utf16_lossy(&name[..n.saturating_sub(1).min(name.len())]);
            if got.trim_end_matches('\0') == face.to_string().unwrap_or_default() {
                return font;
            }
            let _ = DeleteObject(HGDIOBJ(font.0));
        }
    }
    unsafe {
        CreateFontW(
            -size,
            0,
            0,
            0,
            FW_NORMAL.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            ANTIALIASED_QUALITY,
            DEFAULT_PITCH.0 as u32,
            w!("Segoe MDL2 Assets"),
        )
    }
}

fn icon_rect(hwnd: HWND) -> Option<Rect> {
    let id = NOTIFYICONIDENTIFIER {
        cbSize: size_of::<NOTIFYICONIDENTIFIER>() as u32,
        hWnd: hwnd,
        uID: ICON_ID,
        ..Default::default()
    };
    let r = unsafe { Shell_NotifyIconGetRect(&id) }.ok()?;
    Some(Rect { left: r.left, top: r.top, right: r.right, bottom: r.bottom })
}

fn update_rect(hwnd: HWND) {
    if let Some(r) = icon_rect(hwnd) {
        for (a, v) in ICON_RECT.iter().zip([r.left, r.top, r.right, r.bottom]) {
            a.store(v, Ordering::Relaxed);
        }
    }
}

/// Хук колеса: только сравнить курсор с прямоугольником значка — хук держит мышь всей системы.
unsafe extern "system" fn wheel_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 && wparam.0 as u32 == WM_MOUSEWHEEL {
        let info = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
        let r = Rect {
            left: ICON_RECT[0].load(Ordering::Relaxed),
            top: ICON_RECT[1].load(Ordering::Relaxed),
            right: ICON_RECT[2].load(Ordering::Relaxed),
            bottom: ICON_RECT[3].load(Ordering::Relaxed),
        };
        let tray = TRAY_HWND.load(Ordering::Relaxed);
        if tray != 0 && r.contains(info.pt.x, info.pt.y) {
            let delta = (info.mouseData >> 16) as u16 as i16;
            unsafe {
                let _ = PostMessageW(Some(sys::hwnd(tray)), WM_WHEEL, WPARAM(delta as u16 as usize), LPARAM(0));
            }
            return LRESULT(1);
        }
    }
    unsafe { CallNextHookEx(Some(HHOOK::default()), code, wparam, lparam) }
}

/// Устройства вывода для меню: (id, имя, по умолчанию).
fn menu_devices(control: &Control) -> Vec<(String, String, bool)> {
    control
        .snapshot()
        .map(|s| {
            s.devices.into_iter().filter(|d| d.flow == Flow::Render).map(|d| (d.id, d.name, d.is_default)).collect()
        })
        .unwrap_or_default()
}

/// «Без звука» у устройства вывода по умолчанию — средний щелчок по значку.
fn toggle_default_mute(control: &Control) {
    if let Ok(s) = control.snapshot()
        && let Some(d) = s.devices.iter().find(|d| d.flow == Flow::Render && d.is_default)
    {
        let _ = control.set_muted(Target::Device(d.id.clone()), !d.muted);
    }
}

unsafe fn show_menu(hwnd: HWND, x: i32, y: i32) {
    let ctx = CTX.get().expect("tray context");
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let Ok(panels) = CreatePopupMenu() else { return };
        let add = |m: HMENU, flags: MENU_ITEM_FLAGS, id: usize, text: &str| {
            let w = sys::wide(text);
            let _ = AppendMenuW(m, flags, id, PCWSTR(w.as_ptr()));
        };
        let devices = menu_devices(&ctx.control);
        if devices.is_empty() {
            add(menu, MF_STRING | MF_GRAYED, 0, T::NoDevices.get());
        }
        for (i, (_, name, default)) in devices.iter().enumerate() {
            let check = if *default { MF_CHECKED } else { MF_UNCHECKED };
            add(menu, MF_STRING | check, (CMD_DEVICE + i as u32) as usize, name);
        }
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        add(panels, MF_STRING, CMD_MIXER as usize, T::WindowsMixer.get());
        add(panels, MF_STRING, CMD_PLAYBACK as usize, T::PlaybackDevices.get());
        add(panels, MF_STRING, CMD_RECORDING as usize, T::RecordingDevices.get());
        add(panels, MF_STRING, CMD_SOUNDS as usize, T::Sounds.get());
        add(panels, MF_STRING, CMD_SETTINGS as usize, T::SoundSettings.get());
        let profile_names: Vec<String> =
            ctx.shared.settings.lock().unwrap().profiles.iter().map(|p| p.name.clone()).collect();
        if !profile_names.is_empty()
            && let Ok(sub) = CreatePopupMenu()
        {
            for (i, name) in profile_names.iter().enumerate() {
                add(sub, MF_STRING, (CMD_PROFILE + i as u32) as usize, name);
            }
            add(menu, MF_POPUP, sub.0 as usize, T::Profiles.get());
        }
        add(menu, MF_POPUP, panels.0 as usize, T::WindowsPanels.get());
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        add(menu, MF_STRING, CMD_EXIT as usize, T::Exit.get());
        // Без этого меню не закрывается щелчком мимо (известное правило TrackPopupMenu для трея).
        let _ = SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, x, y, None, hwnd, None);
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        match cmd.0 as u32 {
            CMD_MIXER => sys::open("sndvol.exe"),
            CMD_SETTINGS => sys::open("ms-settings:sound"),
            CMD_PLAYBACK => sys::run("control.exe", "mmsys.cpl,,0"),
            CMD_RECORDING => sys::run("control.exe", "mmsys.cpl,,1"),
            CMD_SOUNDS => sys::run("control.exe", "mmsys.cpl,,2"),
            c if c >= CMD_PROFILE => run_profile(&ctx.shared, &ctx.control, (c - CMD_PROFILE) as usize),
            c if c >= CMD_DEVICE => {
                if let Some((id, _, _)) = devices.get((c - CMD_DEVICE) as usize) {
                    let _ = ctx.control.set_default(id, Roles::All);
                }
            }
            CMD_EXIT => {
                let _ = DestroyWindow(hwnd);
            }
            _ => {}
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let Some(ctx) = CTX.get() else {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    };
    unsafe {
        match msg {
            WM_TRAY => {
                let event = (lparam.0 as u32) & 0xFFFF;
                let x = (wparam.0 & 0xFFFF) as u16 as i16 as i32;
                let y = ((wparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
                match event {
                    NIN_SELECT | NIN_KEYSELECT => {
                        let anchor = icon_rect(hwnd).unwrap_or(Rect { left: x, top: y, right: x + 1, bottom: y + 1 });
                        ctx.shared.toggle(anchor);
                    }
                    WM_CONTEXTMENU => show_menu(hwnd, x, y),
                    WM_MBUTTONUP => toggle_default_mute(&ctx.control),
                    WM_MOUSEMOVE => update_rect(hwnd),
                    _ => {}
                }
                LRESULT(0)
            }
            WM_HOTKEY => {
                run_hotkey(hwnd, wparam.0 as i32);
                LRESULT(0)
            }
            WM_RELOAD_HOTKEYS => {
                register_hotkeys(hwnd);
                LRESULT(0)
            }
            WM_WHEEL => {
                let _ = ctx.control.adjust_default(wheel_step(wparam.0 as u16 as i16));
                LRESULT(0)
            }
            WM_REFRESH => {
                update_look(hwnd);
                react_to_changes(hwnd);
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == SAVE_TIMER => {
                let _ = KillTimer(Some(hwnd), SAVE_TIMER);
                ctx.shared.settings.lock().unwrap().save();
                LRESULT(0)
            }
            WM_DISPLAYCHANGE => {
                update_rect(hwnd);
                LRESULT(0)
            }
            WM_SETTINGCHANGE => {
                // Сменилась тема панели задач — перекрасить значок.
                update_look(hwnd);
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_CLOSE => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                // Память громкости, ещё не записанная таймером.
                ctx.shared.settings.lock().unwrap().save();
                // Значок убрать до выхода, иначе в трее остаётся «призрак» до наведения мыши.
                let d = base_data(hwnd);
                let _ = Shell_NotifyIconW(NIM_DELETE, &d);
                ctx.shared.exit();
                PostQuitMessage(0);
                LRESULT(0)
            }
            m if m == ctx.taskbar_created => {
                // Проводник перезапустился — значок надо поставить заново.
                add_icon(hwnd);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_follows_volume() {
        assert_eq!(Level::of(0.8, true), Level::Mute);
        assert_eq!(Level::of(0.0, false), Level::Zero);
        assert_eq!(Level::of(0.004, false), Level::Zero);
        assert_eq!(Level::of(0.33, false), Level::Low);
        assert_eq!(Level::of(0.34, false), Level::Mid);
        assert_eq!(Level::of(0.66, false), Level::Mid);
        assert_eq!(Level::of(0.67, false), Level::High);
    }

    #[test]
    fn app_hotkeys_find_every_output_session() {
        use crate::audio::{App, AppKey, Device, Snapshot};
        let app = |dev: &str, exe: &str| App {
            key: AppKey { device: dev.into(), exe: exe.into() },
            name: exe.into(),
            path: None,
            volume: 1.0,
            muted: false,
            system: false,
            pids: vec![1],
            pinned: None,
        };
        let dev = |id: &str, flow: Flow, apps: Vec<App>| Device {
            id: id.into(),
            name: id.into(),
            flow,
            volume: 1.0,
            muted: false,
            is_default: false,
            is_comm: false,
            apps,
        };
        let s = Snapshot {
            can_route: true,
            devices: vec![
                dev("spk", Flow::Render, vec![app("spk", "player.exe"), app("spk", "game.exe")]),
                dev("phones", Flow::Render, vec![app("phones", "player.exe")]),
                dev("mic", Flow::Capture, vec![app("mic", "player.exe")]),
            ],
        };
        let found: Vec<&str> = apps_with_exe(&s, "player.exe").iter().map(|a| a.key.device.as_str()).collect();
        assert_eq!(found, vec!["spk", "phones"]);
    }

    #[test]
    fn wheel_two_percent_per_notch() {
        assert_eq!(wheel_step(120), 0.02);
        assert_eq!(wheel_step(-360), -0.06);
    }
}
