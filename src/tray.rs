// Значок в трее: свой поток с невидимым окном сообщений. Щелчок — открыть или закрыть окно микшера
// (оно живёт в этом же процессе, `flyout::Shared`), правая кнопка — меню, колесо над значком — громкость
// устройства по умолчанию (хук мыши), вид значка — знак программы или колонка по громкости с движением
// (`mark` — ход, `mark_render` — кадр).

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::RemoteDesktop::{
    NOTIFY_FOR_THIS_SESSION, WTSRegisterSessionNotification, WTSUnRegisterSessionNotification,
};
use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetSystemMetricsForDpi};
use windows::Win32::UI::Input::KeyboardAndMouse::{HOT_KEY_MODIFIERS, MOD_NOREPEAT, RegisterHotKey, UnregisterHotKey};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NIN_SELECT, NINF_KEY,
    NOTIFYICON_VERSION_4, NOTIFYICONDATAW, NOTIFYICONIDENTIFIER, Shell_NotifyIconGetRect, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CallNextHookEx, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyMenu,
    DestroyWindow, DispatchMessageW, GetMessageW, HHOOK, HMENU, KillTimer, MENU_ITEM_FLAGS, MF_CHECKED, MF_GRAYED,
    MF_POPUP, MF_SEPARATOR, MF_STRING, MF_UNCHECKED, MSG, MSLLHOOKSTRUCT, PostMessageW, PostQuitMessage,
    RegisterClassW, RegisterWindowMessageW, SM_CXSMICON, SetForegroundWindow, SetTimer, SetWindowsHookExW,
    TPM_BOTTOMALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage, UnhookWindowsHookEx,
    WH_MOUSE_LL, WINDOW_EX_STYLE, WM_APP, WM_CLOSE, WM_CONTEXTMENU, WM_DESTROY, WM_DISPLAYCHANGE, WM_MBUTTONUP,
    WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NULL, WM_SETTINGCHANGE, WNDCLASSW, WS_POPUP,
};
use windows::core::{PCWSTR, w};

use anvil_ui::motion::machine::{Presenter, Scene, Show};

use crate::audio::{Control, Event, Flow, Roles, Snapshot, Target, model};
use crate::config::{IconColor, TrayIcon};
use crate::duck::{self, Ducker};
use crate::flyout::Shared;
use crate::hotkeys::{Action, AppAction, Combo, next_output};
use crate::mark::{self, MarkFrame, MarkScene, SpeakerFrame, SpeakerScene};
use crate::mark_render;
use crate::placement::Rect;
use crate::profiles::{self, Cmd};
use crate::rules;
use crate::sys;
use crate::texts::T;

const WM_TRAY: u32 = WM_APP + 1;
const WM_WHEEL: u32 = WM_APP + 2;
const WM_REFRESH: u32 = WM_APP + 3;
const WM_RELOAD_HOTKEYS: u32 = WM_APP + 4;
const WM_TIMER: u32 = 0x0113;
/// Таймер отложенной записи настроек (память громкости меняется часто — пишем не чаще раза в 2 с).
const SAVE_TIMER: usize = 1;
/// Раз в минуту — не пора ли проверить обновления.
const UPDATE_TIMER: usize = 2;
/// Кадры значка — только пока знак движется.
const MARK_TIMER: usize = 3;
/// Приглушение: шаг 250 мс, только пока оно включено.
const DUCK_TIMER: usize = 4;
const DUCK_STEP_MS: u32 = 250;
const WM_HOTKEY: u32 = 0x0312;
const WM_WTSSESSION_CHANGE: u32 = 0x02B1;
const WTS_CONSOLE_CONNECT: usize = 0x1;
const WTS_REMOTE_CONNECT: usize = 0x3;
const WTS_SESSION_UNLOCK: usize = 0x8;
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
const CMD_KMIXER: u32 = 7;
const CMD_UPDATE: u32 = 8;
const CMD_RESTART: u32 = 9;
/// Пункты устройств: `CMD_DEVICE + номер` в списке устройств вывода.
const CMD_DEVICE: u32 = 100;
/// Пункты профилей: `CMD_PROFILE + номер`.
const CMD_PROFILE: u32 = 500;

/// Последний снимок трея — от него считается разница по каждому `refresh`.
static LAST: Mutex<Option<Snapshot>> = Mutex::new(None);

/// Приглушённые программы и идёт ли таймер приглушения.
static DUCKER: Mutex<Option<Ducker>> = Mutex::new(None);
static DUCK_RUNNING: AtomicBool = AtomicBool::new(false);
/// Начало отсчёта времени для приглушения.
static CLOCK: OnceLock<Instant> = OnceLock::new();

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

/// Шаг громкости по повороту колеса (`delta` в единицах Windows, 120 на щелчок).
pub fn wheel_step(delta: i16) -> f32 {
    delta as f32 / 120.0 * WHEEL_STEP
}

/// Что висит на зарегистрированном сочетании.
#[derive(Debug, Clone)]
enum Bound {
    Global(Action),
    App(AppAction, String),
    /// Сделать устройство основным.
    Device(String),
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
        for (id, c) in &settings.device_hotkeys {
            items.push((format!("devhotkey.{id}"), Bound::Device(id.clone()), *c));
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
        Bound::Global(Action::Mixer) => open_mixer(),
        Bound::Global(Action::Louder) => adjust_default(hwnd, STEP),
        Bound::Global(Action::Quieter) => adjust_default(hwnd, -STEP),
        Bound::Global(Action::Mute) => toggle_default_mute(hwnd, c),
        Bound::Global(a @ (Action::NextOutput | Action::PrevOutput)) => {
            if let Ok(s) = c.snapshot() {
                let outs: Vec<&crate::audio::Device> = s.devices.iter().filter(|d| d.flow == Flow::Render).collect();
                let ids: Vec<&str> = outs.iter().map(|d| d.id.as_str()).collect();
                let current = outs.iter().find(|d| d.is_default).map(|d| d.id.as_str());
                if let Some(next) = next_output(&ids, current, a == Action::NextOutput) {
                    let _ = c.set_default(next, Roles::All);
                }
            }
        }
        Bound::Device(id) => {
            let _ = c.set_default(&id, Roles::All);
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
            Cmd::SetDeviceVolume(id, v) => c.set_volume(Target::Device(id), v),
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
    sync_duck(hwnd, &now);
    // Потолок — по снимку, а не по событиям: проверяется и при запуске, и после правки настроек.
    let (limit_device, limit_apps) = {
        let st = ctx.shared.settings.lock().unwrap();
        (rules::device_cap(st.limit_device, st.night(), sys::local_minutes()), st.limit_apps.clone())
    };
    run_cmds(&ctx.control, rules::limit_cmds(&now, limit_device, &limit_apps));
    let Some(prev) = prev else { return };
    let mut events = model::diff(&prev, &now);
    // Приглушённая громкость не память: события приглушённых программ профили не видят.
    if let Some(d) = DUCKER.lock().unwrap().as_ref() {
        events.retain(|e| match e {
            Event::AppChanged(a) => !d.is_ducked(&a.key),
            _ => true,
        });
    }
    if events.is_empty() {
        return;
    }
    let (cmds, dirty, profile, rule_cmds) = {
        let mut st = ctx.shared.settings.lock().unwrap();
        let rule_cmds = rules::rule_cmds(&events, &now, &st.rules);
        let list = st.profiles.clone();
        let routes = st.app_routes.clone();
        let rules = profiles::Rules {
            remember: st.remember,
            new_app_volume: st.new_app_volume.map(|p| p as f32 / 100.0),
            profiles: &list,
            routes: &routes,
        };
        let mut mem = std::mem::take(&mut st.memory);
        let (cmds, dirty, profile) = profiles::react(&events, &now, &rules, &mut mem);
        st.memory = mem;
        (cmds, dirty, profile, rule_cmds)
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
    // Правила идут после памяти и профилей: что задано правилом, побеждает.
    run_cmds(&ctx.control, rule_cmds);
}

/// Включить или выключить таймер приглушения по настройкам; выключили — вернуть приглушённое.
fn sync_duck(hwnd: HWND, now: &Snapshot) {
    let ctx = CTX.get().expect("tray context");
    let want = {
        let st = ctx.shared.settings.lock().unwrap();
        st.duck && !st.duck_apps.is_empty()
    };
    let running = DUCK_RUNNING.load(Ordering::SeqCst);
    if want && !running {
        if std::env::var_os("KAMERTON_TRACE").is_some() {
            eprintln!("trace: duck timer on");
        }
        DUCK_RUNNING.store(true, Ordering::SeqCst);
        unsafe {
            let _ = SetTimer(Some(hwnd), DUCK_TIMER, DUCK_STEP_MS, None);
        }
    } else if !want && running {
        DUCK_RUNNING.store(false, Ordering::SeqCst);
        unsafe {
            let _ = KillTimer(Some(hwnd), DUCK_TIMER);
        }
        release_duck(now);
    }
}

/// Вернуть приглушённым их громкость (выключили приглушение, выход из программы).
fn release_duck(now: &Snapshot) {
    let ctx = CTX.get().expect("tray context");
    let cmds = DUCKER.lock().unwrap().as_mut().map(|d| d.release_all(now)).unwrap_or_default();
    run_cmds(&ctx.control, cmds);
}

/// Шаг приглушения (таймер): пики главных программ → команды.
fn duck_tick() {
    let ctx = CTX.get().expect("tray context");
    let (level, priority) = {
        let st = ctx.shared.settings.lock().unwrap();
        (st.duck_level as f32 / 100.0, st.duck_apps.clone())
    };
    let Ok(now) = ctx.control.snapshot() else { return };
    let peaks: std::collections::HashMap<crate::audio::AppKey, f32> = ctx
        .control
        .peaks()
        .into_iter()
        .filter_map(|(t, p)| match t {
            Target::App(k) => Some((k, p)),
            Target::Device(_) => None,
        })
        .collect();
    let secs = CLOCK.get_or_init(Instant::now).elapsed().as_secs_f64();
    let cmds = DUCKER.lock().unwrap().get_or_insert_with(Ducker::default).tick(
        secs,
        &now,
        &peaks,
        &duck::Config { level, priority: &priority },
    );
    run_cmds(&ctx.control, cmds);
}

/// Окно микшера — свой процесс `kamerton --mixer` (обычное окно, нужно редко).
pub fn open_mixer() {
    let Ok(exe) = std::env::current_exe() else { return };
    if let Ok(child) = std::process::Command::new(exe).arg("--mixer").spawn() {
        sys::allow_foreground(child.id());
    }
}

/// Прогон на утечки (`KAMERTON_CYCLE=N`): N раз открыть окно под плащом (невидимо) и закрыть,
/// печатая свою память после 20-го и последнего раза.
fn start_cycle(hwnd: HWND) {
    let Some(n) = std::env::var("KAMERTON_CYCLE").ok().and_then(|v| v.parse::<u32>().ok()) else { return };
    let Some(anchor) = icon_rect(hwnd) else { return };
    let shared = CTX.get().expect("tray context").shared.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(2));
        eprintln!("cycle: start, private {:.1} MB", sys::private_mb());
        for i in 1..=n {
            shared.open_ghost(anchor);
            std::thread::sleep(std::time::Duration::from_millis(250));
            shared.request_close();
            std::thread::sleep(std::time::Duration::from_millis(150));
            if i == 20 || i == n {
                eprintln!("cycle: {i} opens, private {:.1} MB", sys::private_mb());
            }
        }
    });
}

/// Проверка плашки без колеса и клавиш (`KAMERTON_OSD_DEMO=1`): через 2 с и ещё раз через 0,8 с показать плашку
/// с нынешней громкостью устройства по умолчанию — ничего не меняя.
fn start_osd_demo() {
    if std::env::var_os("KAMERTON_OSD_DEMO").is_none() {
        return;
    }
    std::thread::spawn(|| {
        for pause in [2000, 800] {
            std::thread::sleep(std::time::Duration::from_millis(pause));
            let h = TRAY_HWND.load(Ordering::SeqCst);
            let ctx = CTX.get().expect("tray context");
            let state = ctx
                .control
                .snapshot()
                .ok()
                .and_then(|s| s.devices.into_iter().find(|d| d.flow == Flow::Render && d.is_default));
            if let Some(d) = state
                && h != 0
            {
                show_osd(sys::hwnd(h), d.volume, d.muted);
            }
        }
    });
}

/// Окно трея (0 — трей не запущен).
pub fn hwnd() -> isize {
    TRAY_HWND.load(Ordering::SeqCst)
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
        let _ = SetTimer(Some(hwnd), UPDATE_TIMER, 60_000, None);
        add_icon(hwnd);
        start_look(hwnd);
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
        // Вернулись в сеанс (разблокировка, смена пользователя, удалённый стол) — пересоздать ядро звука.
        let _ = WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION);
        start_cycle(hwnd);
        start_osd_demo();
        // Потолок и таймер приглушения — с первой минуты, а не с первого события звука.
        refresh();
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
    d.uFlags = NIF_MESSAGE | NIF_TIP | NIF_SHOWTIP;
    d.uCallbackMessage = WM_TRAY;
    let tip = {
        let mut guard = LOOK.lock().unwrap();
        let look = guard.get_or_insert_with(Look::new);
        // Значок ставится заново (запуск, перезапуск проводника) — кадр нужен в любом случае.
        look.show.invalidate();
        feed(look)
    };
    put_tip(&mut d, &tip);
    unsafe {
        let _ = Shell_NotifyIconW(NIM_ADD, &d);
        d.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        let _ = Shell_NotifyIconW(NIM_SETVERSION, &d);
        show_look(hwnd, None);
    }
    update_rect(hwnd);
}

/// Громкость сменилась (или тема панели задач, или вид значка в настройках): новая цель сцен и подсказка.
unsafe fn update_look(hwnd: HWND) {
    let tip = {
        let mut guard = LOOK.lock().unwrap();
        feed(guard.get_or_insert_with(Look::new))
    };
    unsafe { show_look(hwnd, Some(&tip)) };
}

fn put_tip(d: &mut NOTIFYICONDATAW, tip: &str) {
    let w: Vec<u16> = tip.encode_utf16().take(d.szTip.len() - 1).collect();
    d.szTip[..w.len()].copy_from_slice(&w);
    d.szTip[w.len()] = 0;
}

/// Обе сцены значка (смена вида в настройках — сразу верный кадр).
struct Scenes {
    mark: MarkScene,
    speaker: SpeakerScene,
}

/// Значок в трее: сцены, показ не чаще 10 кадров в секунду, устройство по умолчанию (смена — звон).
struct Look {
    clock: Instant,
    scenes: Scenes,
    show: Presenter<LookKey>,
    device: Option<String>,
}

type LookKey = (TrayIcon, IconColor, bool, i32, [i32; 4], [i32; 3]);

static LOOK: Mutex<Option<Look>> = Mutex::new(None);

impl Look {
    fn new() -> Look {
        Look {
            clock: Instant::now(),
            scenes: Scenes {
                mark: MarkScene::new([0.0, 0.0], mark::STRIKE_HZ_TRAY, false),
                speaker: SpeakerScene::new(0.0),
            },
            show: Presenter::new(mark::TRAY_FPS),
            device: None,
        }
    }

    fn now(&self) -> f64 {
        self.clock.elapsed().as_secs_f64()
    }
}

/// Кадр значка в трее.
enum LookFrame {
    Mark(MarkFrame),
    Speaker(SpeakerFrame),
}

/// Что показать: выбранный вид, цвет, тема панели задач и размер значка.
struct View<'a> {
    scenes: &'a Scenes,
    icon: TrayIcon,
    color: IconColor,
    dark: bool,
    size: i32,
}

impl Scene for View<'_> {
    type Frame = LookFrame;
    type Key = LookKey;

    fn frame(&self, now: f64, still: bool) -> LookFrame {
        match self.icon {
            TrayIcon::Mark => LookFrame::Mark(self.scenes.mark.frame(now, still)),
            TrayIcon::Speaker => LookFrame::Speaker(self.scenes.speaker.frame(now, still)),
        }
    }

    fn wants_frames(&self, now: f64, still: bool) -> bool {
        match self.icon {
            TrayIcon::Mark => self.scenes.mark.wants_frames(now, still),
            TrayIcon::Speaker => self.scenes.speaker.wants_frames(now, still),
        }
    }

    fn at_rest(&self, _now: f64, _still: bool) -> bool {
        // Покоя «как файл» в трее нет: значок всегда показывает громкость.
        false
    }

    fn key(&self) -> LookKey {
        (self.icon, self.color, self.dark, self.size, self.scenes.mark.key(), self.scenes.speaker.key())
    }
}

/// «Меньше движения» — флаг Windows «Показывать анимацию».
fn still() -> bool {
    !anvil_ui::motion::animations_enabled()
}

/// Передать сценам устройство вывода по умолчанию; вернуть подсказку значка.
fn feed(look: &mut Look) -> String {
    let ctx = CTX.get().expect("tray context");
    let log = ctx.shared.settings.lock().unwrap().log_scale;
    let default = ctx
        .control
        .snapshot()
        .ok()
        .and_then(|s| s.devices.into_iter().find(|d| d.flow == Flow::Render && d.is_default));
    let (now, still) = (look.now(), still());
    let s = &mut look.scenes;
    match &default {
        Some(d) => {
            // Ручки — громкость каналов по шкале ползунков; колонка — общая громкость.
            let (l, r) = model::channels_for(d.balance.unwrap_or(0.0), d.volume);
            s.mark.fail(now, false, still);
            s.mark.volume(now, model::to_pos(l, log), model::to_pos(r, log), still);
            s.mark.mute(now, d.muted, still);
            s.speaker.fail(now, false, still);
            s.speaker.volume(now, d.volume, still);
            s.speaker.mute(now, d.muted, still);
            if look.device.as_ref().is_some_and(|prev| prev != &d.id) {
                s.mark.strike(now);
                s.speaker.strike(now);
            }
            look.device = Some(d.id.clone());
            format!("Kamerton: {} — {} %", d.name, model::percent(d.volume))
        }
        None => {
            s.mark.fail(now, true, still);
            s.speaker.fail(now, true, still);
            look.device = None;
            format!("Kamerton: {}", T::NoDevices.get())
        }
    }
}

/// Запуск: ручки пробегают (знак) или волны загораются по одной (колонка).
fn start_look(hwnd: HWND) {
    if let Some(look) = LOOK.lock().unwrap().as_mut() {
        let now = look.now();
        look.scenes.mark.sweep(now);
        look.scenes.speaker.sweep(now);
    }
    unsafe { show_look(hwnd, None) };
}

/// Шаг показа: новый кадр значка, если пора, и подсказка, если дана; таймер кадров — пока знак движется.
unsafe fn show_look(hwnd: HWND, tip: Option<&str>) {
    let ctx = CTX.get().expect("tray context");
    let (icon, color) = {
        let st = ctx.shared.settings.lock().unwrap();
        (st.tray_icon, st.tray_color)
    };
    let size = unsafe { GetSystemMetricsForDpi(SM_CXSMICON, GetDpiForSystem()) }.max(16);
    let dark = !sys::taskbar_light();
    let (shown, next) = {
        let mut guard = LOOK.lock().unwrap();
        let look = guard.get_or_insert_with(Look::new);
        let (now, still) = (look.now(), still());
        let view = View { scenes: &look.scenes, icon, color, dark, size };
        (look.show.step(&view, now, still), look.show.next_in(&view, now, still))
    };
    let mut d = base_data(hwnd);
    d.uFlags = NIF_SHOWTIP;
    if let Show::Frame(f) = shown {
        let rgba = match f {
            LookFrame::Mark(f) => mark_render::mark(&f, size as u32, color),
            LookFrame::Speaker(f) => mark_render::speaker(&f, size as u32, dark, color),
        };
        d.hIcon = sys::icon_from_rgba(&rgba, size);
        d.uFlags |= NIF_ICON;
    }
    if let Some(tip) = tip {
        put_tip(&mut d, tip);
        d.uFlags |= NIF_TIP;
    }
    unsafe {
        if d.uFlags != NIF_SHOWTIP {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &d);
        }
        if !d.hIcon.is_invalid() {
            let _ = DestroyIcon(d.hIcon);
        }
        match next {
            Some(secs) => {
                let _ = SetTimer(Some(hwnd), MARK_TIMER, ((secs * 1000.0).ceil() as u32).max(10), None);
            }
            None => {
                let _ = KillTimer(Some(hwnd), MARK_TIMER);
            }
        }
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
        // Над значком — всегда; вне открытого окна — если включено «Колесо вне окна».
        let outside = CTX.get().is_some_and(|c| {
            c.shared.is_open()
                && c.shared.wheel_outside.load(Ordering::Relaxed)
                && sys::window_rect(c.shared.hwnd()).is_some_and(|w| !w.contains(info.pt.x, info.pt.y))
        });
        if tray != 0 && (r.contains(info.pt.x, info.pt.y) || outside) {
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

/// «Без звука» у устройства вывода по умолчанию — средний щелчок по значку и сочетание клавиш.
fn toggle_default_mute(hwnd: HWND, control: &Control) {
    if let Ok(s) = control.snapshot()
        && let Some(d) = s.devices.iter().find(|d| d.flow == Flow::Render && d.is_default)
    {
        let muted = !d.muted;
        if control.set_muted(Target::Device(d.id.clone()), muted).is_ok() {
            show_osd(hwnd, d.volume, muted);
        }
    }
}

/// Плашка громкости у значка: устройство вывода по умолчанию стало таким (если плашка включена).
fn show_osd(hwnd: HWND, volume: f32, muted: bool) {
    let ctx = CTX.get().expect("tray context");
    let anchor = icon_rect(hwnd).unwrap_or_else(|| {
        let (x, y) = sys::cursor();
        Rect { left: x, top: y, right: x + 1, bottom: y + 1 }
    });
    ctx.shared.show_osd(anchor, volume, muted);
}

/// Сдвинуть громкость устройства вывода по умолчанию (колесо, «Громче», «Тише») и показать плашку.
fn adjust_default(hwnd: HWND, delta: f32) {
    let ctx = CTX.get().expect("tray context");
    if let Ok(v) = ctx.control.adjust_default(delta) {
        let muted = ctx
            .control
            .snapshot()
            .ok()
            .and_then(|s| s.devices.into_iter().find(|d| d.flow == Flow::Render && d.is_default))
            .is_some_and(|d| d.muted);
        show_osd(hwnd, v, muted);
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
        if let Some(v) = crate::update::ready() {
            add(menu, MF_STRING, CMD_RESTART as usize, &format!("{} Kamerton {v}", T::MenuRestart.get()));
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        } else if let Some(v) = crate::update::available() {
            add(menu, MF_STRING, CMD_UPDATE as usize, &format!("{} Kamerton {v}", T::MenuUpdate.get()));
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        }
        let hidden = ctx.shared.settings.lock().unwrap().hidden_devices.clone();
        let devices: Vec<(String, String, bool)> =
            menu_devices(&ctx.control).into_iter().filter(|(id, _, _)| !hidden.contains(id)).collect();
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
        add(menu, MF_STRING, CMD_KMIXER as usize, T::MixerApp.get());
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
            CMD_KMIXER => open_mixer(),
            CMD_UPDATE => {
                if let Some(u) = crate::update::get() {
                    u.install();
                }
            }
            CMD_RESTART => {
                if let Err(e) = crate::update::restart(hwnd.0 as isize) {
                    eprintln!("update: restart failed: {e}");
                }
            }
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
                    WM_MBUTTONUP => toggle_default_mute(hwnd, &ctx.control),
                    WM_MOUSEMOVE => update_rect(hwnd),
                    _ => {}
                }
                LRESULT(0)
            }
            WM_WTSSESSION_CHANGE => {
                if matches!(wparam.0, WTS_CONSOLE_CONNECT | WTS_REMOTE_CONNECT | WTS_SESSION_UNLOCK) {
                    ctx.control.reset();
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
                adjust_default(hwnd, wheel_step(wparam.0 as u16 as i16));
                LRESULT(0)
            }
            WM_REFRESH => {
                update_look(hwnd);
                react_to_changes(hwnd);
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == MARK_TIMER => {
                let _ = KillTimer(Some(hwnd), MARK_TIMER);
                show_look(hwnd, None);
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == DUCK_TIMER => {
                duck_tick();
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == UPDATE_TIMER => {
                // Раз в минуту — ещё и граница часов ночного потолка.
                refresh();
                let settings = ctx.shared.settings.lock().unwrap().clone();
                crate::update::tick(&settings);
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
                // Сменилась тема панели задач — перекрасить колонку.
                update_look(hwnd);
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_CLOSE => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                let _ = WTSUnRegisterSessionNotification(hwnd);
                // Приглушённое вернуть до выхода; память громкости, ещё не записанная таймером, — записать.
                if let Ok(now) = ctx.control.snapshot() {
                    release_duck(&now);
                }
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
            balance: None,
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
