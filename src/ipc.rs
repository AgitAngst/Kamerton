// Канал для Antresol: именованный канал `\\.\pipe\kamerton-ipc`, один запрос на подключение (кроме подписки
// `watch` — она держит канал открытым). Протокол (версии 1 и 2) — docs/guide/usage.md, раздел «Связь с
// Antresol». Разбор запросов и сбор ответов — чистые функции (тесты внизу); поток канала и вызовы Win32 — в
// модуле `server`.

use std::sync::{Arc, Condvar, Mutex};

use crate::audio::{AudioError, Control, Flow, Snapshot, Target, model};
use crate::flyout::Shared;

pub const PIPE_NAME: &str = r"\\.\pipe\kamerton-ipc";

/// Что умеет канал версии 2 — в ответе на `hello`.
const CAPS: &str = "state2,watch,devices,input,default,profiles,night";

#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    State,
    Volume(u32),
    Mute(bool),
    AppVolume(String, u32),
    AppMute(String, bool),
    Open,
    Hello,
    State2,
    DevVolume(String, u32),
    DevMute(String, bool),
    Default(String),
    Profile(String),
    Night(bool),
    Watch,
}

/// Что сделать со звуком после разбора запроса на снимке.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Volume(Target, f32),
    Mute(Target, bool),
    Default(String),
}

/// Ночной режим для ответа `state\t2`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct NightInfo {
    pub on: bool,
    /// Сейчас ночные часы (и режим включён).
    pub now: bool,
    pub from: u16,
    pub to: u16,
    pub limit: u8,
}

/// Чего нет в снимке звука: профили и ночной режим из настроек.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Extra {
    pub profiles: Vec<String>,
    pub night: NightInfo,
}

/// Табуляции и переводы строк в имени — пробелом.
pub fn sanitize(s: &str) -> String {
    s.chars().map(|c| if matches!(c, '\t' | '\n' | '\r') { ' ' } else { c }).collect()
}

fn percent_arg(s: &str) -> Result<u32, String> {
    let n: u32 = s.parse().map_err(|_| "bad-args".to_string())?;
    if n > 100 { Err("range".into()) } else { Ok(n) }
}

fn flag_arg(s: &str) -> Result<bool, String> {
    match s {
        "0" => Ok(false),
        "1" => Ok(true),
        _ => Err("bad-args".into()),
    }
}

fn text_arg(s: &str) -> Result<String, String> {
    if s.is_empty() { Err("bad-args".into()) } else { Ok(s.to_string()) }
}

/// Разобрать строку запроса (с `\n` или без). Ошибка — текст после `err\t`.
pub fn parse(line: &str) -> Result<Request, String> {
    let line = line.trim_end_matches(['\n', '\r']);
    let parts: Vec<&str> = line.split('\t').collect();
    match parts.as_slice() {
        ["state"] => Ok(Request::State),
        ["state", "2"] => Ok(Request::State2),
        ["open"] => Ok(Request::Open),
        ["hello"] => Ok(Request::Hello),
        ["watch"] => Ok(Request::Watch),
        ["volume", n] => Ok(Request::Volume(percent_arg(n)?)),
        ["mute", f] => Ok(Request::Mute(flag_arg(f)?)),
        ["app-volume", k, n] => Ok(Request::AppVolume((*k).to_string(), percent_arg(n)?)),
        ["app-mute", k, f] => Ok(Request::AppMute((*k).to_string(), flag_arg(f)?)),
        ["dev-volume", id, n] => Ok(Request::DevVolume(text_arg(id)?, percent_arg(n)?)),
        ["dev-mute", id, f] => Ok(Request::DevMute(text_arg(id)?, flag_arg(f)?)),
        ["default", id] => Ok(Request::Default(text_arg(id)?)),
        ["profile", name] => Ok(Request::Profile(text_arg(name)?)),
        ["night", f] => Ok(Request::Night(flag_arg(f)?)),
        [
            "state" | "open" | "hello" | "watch" | "volume" | "mute" | "app-volume" | "app-mute" | "dev-volume"
            | "dev-mute" | "default" | "profile" | "night",
            ..,
        ] => Err("bad-args".into()),
        _ => Err("unknown".into()),
    }
}

fn default_output(s: &Snapshot) -> Option<&model::Device> {
    s.devices.iter().find(|d| d.flow == Flow::Render && d.is_default)
}

fn level(p: u32) -> f32 {
    p as f32 / 100.0
}

/// Перевести запрос на звук в действие: версия 1 — над устройством вывода по умолчанию и его программами,
/// версия 2 — над устройством по ID. Остальные запросы сюда не попадают.
pub fn resolve(req: &Request, s: &Snapshot) -> Result<Action, String> {
    let device =
        |id: &str| s.devices.iter().find(|d| d.id == id).map(|d| d.id.clone()).ok_or_else(|| "no-device".to_string());
    match req {
        Request::DevVolume(id, n) => return Ok(Action::Volume(Target::Device(device(id)?), level(*n))),
        Request::DevMute(id, m) => return Ok(Action::Mute(Target::Device(device(id)?), *m)),
        Request::Default(id) => return Ok(Action::Default(device(id)?)),
        _ => {}
    }
    let dev = default_output(s).ok_or_else(|| "no-device".to_string())?;
    let app = |key: &str| {
        dev.apps
            .iter()
            .find(|a| sanitize(&a.key.exe) == key)
            .map(|a| Target::App(a.key.clone()))
            .ok_or_else(|| "no-app".to_string())
    };
    match req {
        Request::Volume(n) => Ok(Action::Volume(Target::Device(dev.id.clone()), level(*n))),
        Request::Mute(m) => Ok(Action::Mute(Target::Device(dev.id.clone()), *m)),
        Request::AppVolume(k, n) => Ok(Action::Volume(app(k)?, level(*n))),
        Request::AppMute(k, m) => Ok(Action::Mute(app(k)?, *m)),
        _ => Err("unknown".into()),
    }
}

fn app_lines(out: &mut String, d: &model::Device) {
    for a in &d.apps {
        out.push_str(&format!(
            "app\t{}\t{}\t{}\t{}\n",
            sanitize(&a.key.exe),
            sanitize(&a.name),
            model::percent(a.volume),
            a.muted as u8
        ));
    }
}

/// Ответ на `state` (версия 1): устройство вывода по умолчанию и его программы в порядке окна.
pub fn format_state(s: &Snapshot, version: &str) -> String {
    let mut out = format!("kamerton\t1\t{}\n", sanitize(version));
    match default_output(s) {
        None => out.push_str("nodevice\n"),
        Some(d) => {
            out.push_str(&format!("device\t{}\t{}\t{}\n", sanitize(&d.name), model::percent(d.volume), d.muted as u8));
            app_lines(&mut out, d);
        }
    }
    out.push_str("end\n");
    out
}

/// Ответ на `hello`.
pub fn format_hello(version: &str) -> String {
    format!("kamerton\t2\t{}\t{CAPS}\n", sanitize(version))
}

/// Ответ на `state\t2`: все устройства вывода и ввода, программы устройства вывода по умолчанию, профили,
/// ночной режим.
pub fn format_state2(s: &Snapshot, version: &str, extra: &Extra) -> String {
    let mut out = format!("kamerton\t2\t{}\n", sanitize(version));
    for (flow, tag) in [(Flow::Render, "output"), (Flow::Capture, "input")] {
        for d in s.devices.iter().filter(|d| d.flow == flow) {
            out.push_str(&format!(
                "{tag}\t{}\t{}\t{}\t{}\t{}\n",
                sanitize(&d.id),
                sanitize(&d.name),
                model::percent(d.volume),
                d.muted as u8,
                d.is_default as u8
            ));
        }
    }
    if let Some(d) = default_output(s) {
        app_lines(&mut out, d);
    }
    for p in &extra.profiles {
        out.push_str(&format!("profile\t{}\n", sanitize(p)));
    }
    let n = &extra.night;
    out.push_str(&format!(
        "night\t{}\t{}\t{}\t{}\t{}\n",
        n.on as u8,
        n.now as u8,
        crate::rules::time_text(n.from),
        crate::rules::time_text(n.to),
        n.limit
    ));
    out.push_str("end\n");
    out
}

pub fn format_err(text: &str) -> String {
    format!("err\t{}\n", sanitize(text))
}

fn audio_err(e: AudioError) -> String {
    match e {
        AudioError::NotFound => "not-found".into(),
        AudioError::Stopped => "audio-stopped".into(),
        AudioError::Windows(m) => format!("windows: {m}"),
    }
}

/// Что каналу нужно от трея и настроек. В программе — `Tray`; в тестах — заглушка.
pub trait Host {
    fn snapshot(&self) -> Result<Snapshot, AudioError>;
    fn control(&self) -> Option<&Control>;
    fn extra(&self) -> Extra;
    /// Открыть окно микшера у значка; `false` — трей не готов.
    fn open(&self) -> bool;
    /// Применить профиль номер `i`; `false` — трей не готов.
    fn profile(&self, i: usize) -> bool;
    /// Включить или выключить «Ночью тише»; `false` — трей не готов.
    fn night(&self, on: bool) -> bool;
}

fn done(ok: bool) -> Result<String, String> {
    if ok { Ok("ok\n".into()) } else { Err("no-tray".into()) }
}

/// Целиком запрос → ответ (кроме `watch`: его ведёт поток канала). Окно, профиль и ночной режим — просьбы
/// потоку трея, а не работа здесь.
pub fn answer(line: &str, host: &dyn Host) -> String {
    let req = match parse(line) {
        Ok(r) => r,
        Err(e) => return format_err(&e),
    };
    let version = env!("CARGO_PKG_VERSION");
    let snapshot = || host.snapshot().map_err(audio_err);
    let result: Result<String, String> = match &req {
        Request::State => snapshot().map(|s| format_state(&s, version)),
        Request::Hello => Ok(format_hello(version)),
        Request::State2 | Request::Watch => snapshot().map(|s| format_state2(&s, version, &host.extra())),
        Request::Open => done(host.open()),
        Request::Night(on) => done(host.night(*on)),
        Request::Profile(name) => match host.extra().profiles.iter().position(|p| sanitize(p) == *name) {
            Some(i) => done(host.profile(i)),
            None => Err("no-profile".into()),
        },
        _ => snapshot().and_then(|s| resolve(&req, &s)).and_then(|a| {
            let c = host.control().ok_or_else(|| "no-tray".to_string())?;
            match a {
                Action::Volume(t, v) => c.set_volume(t, v),
                Action::Mute(t, m) => c.set_muted(t, m),
                Action::Default(id) => c.set_default(&id, crate::audio::Roles::All),
            }
            .map(|_| "ok\n".to_string())
            .map_err(audio_err)
        }),
    };
    result.unwrap_or_else(|e| format_err(&e))
}

/// Счётчик изменений для подписчиков: растёт по каждому `notify`.
static CHANGES: Mutex<u64> = Mutex::new(0);
static CHANGED: Condvar = Condvar::new();

/// Что-то изменилось (звук, профили, ночной режим): разбудить подписчиков. Дёшево; зовётся из трея.
pub fn notify() {
    *CHANGES.lock().unwrap_or_else(|e| e.into_inner()) += 1;
    CHANGED.notify_all();
}

/// Дождаться изменения после `seen` не дольше `limit`; вернуть новый счётчик.
fn wait_change(seen: u64, limit: std::time::Duration) -> u64 {
    let g = CHANGES.lock().unwrap_or_else(|e| e.into_inner());
    let (g, _) = CHANGED.wait_timeout_while(g, limit, |n| *n == seen).unwrap_or_else(|e| e.into_inner());
    *g
}

/// Хозяин канала в программе: звук через `Control`, профили и ночной режим — из настроек окна и трея.
struct Tray {
    control: Control,
    shared: Arc<Shared>,
}

impl Host for Tray {
    fn snapshot(&self) -> Result<Snapshot, AudioError> {
        self.control.snapshot()
    }

    fn control(&self) -> Option<&Control> {
        Some(&self.control)
    }

    fn extra(&self) -> Extra {
        let st = self.shared.settings.lock().unwrap_or_else(|e| e.into_inner());
        Extra {
            profiles: st.profiles.iter().map(|p| p.name.clone()).collect(),
            night: NightInfo {
                on: st.night,
                now: st.night && crate::rules::in_window(st.night_from, st.night_to, crate::sys::local_minutes()),
                from: st.night_from,
                to: st.night_to,
                limit: st.night_limit,
            },
        }
    }

    fn open(&self) -> bool {
        crate::tray::post_ipc(crate::tray::IpcAsk::Open)
    }

    fn profile(&self, i: usize) -> bool {
        crate::tray::post_ipc(crate::tray::IpcAsk::Profile(i))
    }

    fn night(&self, on: bool) -> bool {
        crate::tray::post_ipc(crate::tray::IpcAsk::Night(on))
    }
}

mod server {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    use windows::Win32::Foundation::{CloseHandle, ERROR_PIPE_CONNECTED, GetLastError, HANDLE, HLOCAL, LocalFree};
    use windows::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
    use windows::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX, ReadFile, WriteFile};
    use windows::Win32::System::IO::CancelIoEx;
    use windows::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
        PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT, PeekNamedPipe,
    };
    use windows::core::{HSTRING, w};

    use super::{Host, answer, format_err, wait_change};

    pub type SharedHost = Arc<dyn Host + Send + Sync>;

    /// Столько подключений обслуживается разом; остальные закрываются сразу.
    const MAX_CLIENTS: usize = 8;
    /// Подписчиков `watch` — не больше.
    const MAX_WATCHERS: usize = 2;
    /// Клиент должен прислать всю строку за это время (общий срок, а не пауза между байтами).
    const READ_LIMIT: Duration = Duration::from_millis(1000);
    /// Дольше строка запроса быть не может.
    const MAX_LINE: usize = 4096;
    /// Буфер ответа: обычный ответ целиком входит в него, запись не ждёт клиента.
    const OUT_BUF: u32 = 64 * 1024;
    /// Доступ: владелец канала (пользователь, запустивший Kamerton) и система; остальным — ничего.
    const SDDL: windows::core::PCWSTR = w!("D:P(A;;GA;;;OW)(A;;GA;;;SY)");
    /// Подписка: частые изменения сводятся к одному ответу за столько.
    const SETTLE: Duration = Duration::from_millis(100);
    /// Подписка: состояние перечитывается не реже (настройки окна меняются без `notify`).
    const RECHECK: Duration = Duration::from_secs(1);
    /// Подписка: `ping`, если столько ничего не писали.
    const PING: Duration = Duration::from_secs(10);
    /// Подписка: запись дольше — клиент не читает, отключить.
    const WRITE_LIMIT: Duration = Duration::from_millis(1000);

    static ACTIVE: AtomicUsize = AtomicUsize::new(0);
    static WATCHERS: AtomicUsize = AtomicUsize::new(0);

    /// Сколько подписчиков сейчас (для тестов).
    #[cfg(test)]
    pub fn watchers() -> usize {
        WATCHERS.load(Ordering::SeqCst)
    }

    struct Pipe(HANDLE);
    unsafe impl Send for Pipe {}
    unsafe impl Sync for Pipe {}
    impl Drop for Pipe {
        fn drop(&mut self) {
            // Без `DisconnectNamedPipe`: закрытие сервером оставляет клиенту уже записанные данные.
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    /// Вернуть счётчик, даже если поток упал.
    struct Slot(&'static AtomicUsize);
    impl Drop for Slot {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }

    /// Занять место в счётчике, если там меньше `max`.
    fn take(counter: &'static AtomicUsize, max: usize) -> Option<Slot> {
        if counter.fetch_add(1, Ordering::SeqCst) >= max {
            counter.fetch_sub(1, Ordering::SeqCst);
            return None;
        }
        Some(Slot(counter))
    }

    fn create(name: &str, first: bool) -> Option<Pipe> {
        let mut open_mode = PIPE_ACCESS_DUPLEX;
        if first {
            // Первый экземпляр: имя не должно быть уже занято чужим процессом.
            open_mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
        }
        let mut sd = PSECURITY_DESCRIPTOR::default();
        unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(SDDL, SDDL_REVISION_1, &mut sd, None) }.ok()?;
        let sa = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: sd.0,
            bInheritHandle: false.into(),
        };
        let h = unsafe {
            CreateNamedPipeW(
                &HSTRING::from(name),
                open_mode,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_UNLIMITED_INSTANCES,
                OUT_BUF,
                8192,
                0,
                Some(&sa),
            )
        };
        unsafe {
            let _ = LocalFree(Some(HLOCAL(sd.0)));
        }
        (!h.is_invalid()).then_some(Pipe(h))
    }

    /// Прочитать строку запроса до `\n`, не дольше `READ_LIMIT` всего. Опрос `PeekNamedPipe` — чтобы
    /// молчащий клиент не завесил поток навсегда.
    fn read_line(pipe: &Pipe) -> Option<String> {
        let start = Instant::now();
        let mut buf: Vec<u8> = Vec::new();
        loop {
            if start.elapsed() > READ_LIMIT {
                return None;
            }
            let mut avail = 0u32;
            unsafe { PeekNamedPipe(pipe.0, None, 0, None, Some(&mut avail), None) }.ok()?;
            if avail > 0 {
                let mut chunk = vec![0u8; (avail as usize).min(MAX_LINE)];
                let mut got = 0u32;
                unsafe { ReadFile(pipe.0, Some(&mut chunk), Some(&mut got), None) }.ok()?;
                buf.extend_from_slice(&chunk[..got as usize]);
                if let Some(i) = buf.iter().position(|&b| b == b'\n') {
                    buf.truncate(i);
                    break;
                }
                if buf.len() >= MAX_LINE {
                    return None;
                }
            } else {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        String::from_utf8(buf).ok()
    }

    /// Записать ответ в буфер канала; `false` — клиент ушёл. Обычный ответ меньше буфера, запись не ждёт
    /// клиента.
    fn write_all(pipe: &Pipe, data: &[u8]) -> bool {
        let mut rest = &data[..data.len().min(OUT_BUF as usize)];
        while !rest.is_empty() {
            let mut n = 0u32;
            if unsafe { WriteFile(pipe.0, Some(rest), Some(&mut n), None) }.is_err() || n == 0 {
                return false;
            }
            rest = &rest[n as usize..];
        }
        true
    }

    /// Сторож записи подписки: запись висит дольше `WRITE_LIMIT` (буфер полон, клиент не читает) —
    /// прервать её и отключить клиента.
    struct Guard {
        /// Срок текущей записи в мс от `start`; 0 — запись не идёт.
        deadline: AtomicU64,
        stop: AtomicBool,
        handle: AtomicIsize,
        start: Instant,
    }

    fn guard_loop(g: Arc<Guard>) {
        while !g.stop.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(100));
            let d = g.deadline.load(Ordering::SeqCst);
            if d != 0 && g.start.elapsed().as_millis() as u64 > d {
                let h = HANDLE(g.handle.load(Ordering::SeqCst) as *mut _);
                unsafe {
                    let _ = CancelIoEx(h, None);
                    let _ = DisconnectNamedPipe(h);
                }
                return;
            }
        }
    }

    /// Подписка: состояние сразу, потом — при изменении (сведённом за `SETTLE`), `ping` в тишине.
    fn watch(pipe: &Pipe, host: &dyn Host) {
        let guard = Arc::new(Guard {
            deadline: AtomicU64::new(0),
            stop: AtomicBool::new(false),
            handle: AtomicIsize::new(pipe.0.0 as isize),
            start: Instant::now(),
        });
        let g = guard.clone();
        let Ok(sentry) = std::thread::Builder::new().name("ipc-guard".into()).spawn(move || guard_loop(g)) else {
            return;
        };
        let send = |data: &[u8]| {
            let ms = guard.start.elapsed().as_millis() as u64 + WRITE_LIMIT.as_millis() as u64;
            guard.deadline.store(ms, Ordering::SeqCst);
            let ok = write_all(pipe, data);
            guard.deadline.store(0, Ordering::SeqCst);
            ok
        };
        let mut seen = wait_change(u64::MAX, Duration::ZERO);
        let mut last = answer("watch", host);
        let mut wrote = Instant::now();
        let mut alive = send(last.as_bytes());
        while alive {
            let now = wait_change(seen, RECHECK);
            // Ушедший клиент: `PeekNamedPipe` отказывает (канал закрыт) — место освободить сразу, а не на
            // следующей записи.
            if !client_here(pipe) {
                break;
            }
            if now != seen {
                seen = now;
                std::thread::sleep(SETTLE);
            }
            let state = answer("watch", host);
            if state != last {
                alive = send(state.as_bytes());
                last = state;
                wrote = Instant::now();
            } else if wrote.elapsed() >= PING {
                alive = send(b"ping\n");
                wrote = Instant::now();
            }
        }
        // Дождаться сторожа: дескриптор закроется (`Drop` у `Pipe`) только после того, как сторож его больше
        // не трогает — иначе освобождённое значение могло бы достаться новому экземпляру канала.
        guard.stop.store(true, Ordering::SeqCst);
        let _ = sentry.join();
    }

    /// Клиент ещё держит свою сторону канала.
    fn client_here(pipe: &Pipe) -> bool {
        let mut avail = 0u32;
        unsafe { PeekNamedPipe(pipe.0, None, 0, None, Some(&mut avail), None) }.is_ok()
    }

    fn serve(pipe: Pipe, host: SharedHost) {
        match read_line(&pipe) {
            Some(line) if line.trim_end_matches('\r') == "watch" => match take(&WATCHERS, MAX_WATCHERS) {
                Some(_w) => watch(&pipe, host.as_ref()),
                None => {
                    write_all(&pipe, format_err("busy").as_bytes());
                }
            },
            Some(line) => {
                write_all(&pipe, answer(&line, host.as_ref()).as_bytes());
            }
            None => {
                write_all(&pipe, format_err("bad-request").as_bytes());
            }
        }
    }

    pub fn run(name: &str, host: SharedHost) {
        // Пока не создан первый экземпляр, имя берётся только с FIRST_PIPE_INSTANCE: занятое чужим
        // процессом имя не присоединяет нас к чужому каналу. Дальше всегда есть живой экземпляр:
        // следующий создаётся, пока предыдущий ещё держит имя.
        let mut have_first = false;
        let mut pending: Option<Pipe> = None;
        loop {
            let pipe = match pending.take() {
                Some(p) => p,
                None => match create(name, !have_first) {
                    Some(p) => {
                        have_first = true;
                        p
                    }
                    None => {
                        // Имя занято или Windows отказала: подождать и попробовать снова.
                        std::thread::sleep(Duration::from_secs(2));
                        continue;
                    }
                },
            };
            let connected =
                unsafe { ConnectNamedPipe(pipe.0, None) }.is_ok() || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;
            if !connected {
                unsafe {
                    let _ = DisconnectNamedPipe(pipe.0);
                }
                pending = Some(pipe);
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
            // Следующий экземпляр — до обслуживания этого: имя не пустеет.
            pending = create(name, false);
            let Some(slot) = take(&ACTIVE, MAX_CLIENTS) else { continue };
            // Место уходит в поток и возвращается с его концом; поток не создался — с замыканием.
            let host = host.clone();
            let _ = std::thread::Builder::new().name("ipc-client".into()).spawn(move || {
                let _slot = slot;
                serve(pipe, host)
            });
        }
    }
}

/// Запустить поток канала.
pub fn start(control: Control, shared: Arc<Shared>) {
    let host: server::SharedHost = Arc::new(Tray { control, shared });
    let _ = std::thread::Builder::new().name("ipc".into()).spawn(move || server::run(PIPE_NAME, host));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::{App, AppKey, Device};
    use std::cell::RefCell;

    fn app(exe: &str, name: &str, vol: f32, muted: bool) -> App {
        App {
            key: AppKey { device: "d1".into(), exe: exe.into() },
            name: name.into(),
            path: None,
            volume: vol,
            muted,
            system: false,
            pids: vec![1],
            pinned: None,
        }
    }

    fn snap() -> Snapshot {
        let dev = |id: &str, name: &str, def: bool, flow, apps| Device {
            id: id.into(),
            name: name.into(),
            flow,
            volume: 0.42,
            muted: false,
            is_default: def,
            is_comm: def,
            balance: None,
            apps,
        };
        Snapshot {
            devices: vec![
                dev("d0", "Mic", true, Flow::Capture, vec![]),
                dev(
                    "d1",
                    "Speakers\tTest",
                    true,
                    Flow::Render,
                    vec![app("c:\\a\\player.exe", "Player", 0.5, false), app("system", "System sounds", 1.0, true)],
                ),
                dev("d2", "Other", false, Flow::Render, vec![]),
            ],
            can_route: true,
        }
    }

    fn extra() -> Extra {
        Extra {
            profiles: vec!["Game".into(), "Work\tday".into()],
            night: NightInfo { on: true, now: false, from: 23 * 60, to: 7 * 60, limit: 40 },
        }
    }

    /// Заглушка трея: записывает просьбы.
    #[derive(Default)]
    struct Fake {
        asked: RefCell<Vec<String>>,
        no_tray: bool,
    }

    impl Host for Fake {
        fn snapshot(&self) -> Result<Snapshot, AudioError> {
            Ok(snap())
        }
        fn control(&self) -> Option<&Control> {
            None
        }
        fn extra(&self) -> Extra {
            extra()
        }
        fn open(&self) -> bool {
            self.asked.borrow_mut().push("open".into());
            !self.no_tray
        }
        fn profile(&self, i: usize) -> bool {
            self.asked.borrow_mut().push(format!("profile {i}"));
            !self.no_tray
        }
        fn night(&self, on: bool) -> bool {
            self.asked.borrow_mut().push(format!("night {on}"));
            !self.no_tray
        }
    }

    #[test]
    fn parses_requests() {
        assert_eq!(parse("state\n"), Ok(Request::State));
        assert_eq!(parse("open\r\n"), Ok(Request::Open));
        assert_eq!(parse("volume\t35\n"), Ok(Request::Volume(35)));
        assert_eq!(parse("mute\t1"), Ok(Request::Mute(true)));
        assert_eq!(parse("app-volume\tk\t0\n"), Ok(Request::AppVolume("k".into(), 0)));
        assert_eq!(parse("app-mute\tk\t0\n"), Ok(Request::AppMute("k".into(), false)));
    }

    #[test]
    fn parses_v2_requests() {
        assert_eq!(parse("hello\n"), Ok(Request::Hello));
        assert_eq!(parse("state\t2\n"), Ok(Request::State2));
        assert_eq!(parse("watch\n"), Ok(Request::Watch));
        assert_eq!(parse("dev-volume\t{0.0.1}.{x}\t70"), Ok(Request::DevVolume("{0.0.1}.{x}".into(), 70)));
        assert_eq!(parse("dev-mute\tid\t1"), Ok(Request::DevMute("id".into(), true)));
        assert_eq!(parse("default\tid"), Ok(Request::Default("id".into())));
        assert_eq!(parse("profile\tИгра"), Ok(Request::Profile("Игра".into())));
        assert_eq!(parse("night\t0"), Ok(Request::Night(false)));
    }

    #[test]
    fn rejects_bad_requests() {
        assert_eq!(parse("nope\n"), Err("unknown".into()));
        assert_eq!(parse("\n"), Err("unknown".into()));
        assert_eq!(parse("volume\n"), Err("bad-args".into()));
        assert_eq!(parse("volume\tx\n"), Err("bad-args".into()));
        assert_eq!(parse("volume\t101\n"), Err("range".into()));
        assert_eq!(parse("volume\t-1\n"), Err("bad-args".into()));
        assert_eq!(parse("mute\t2\n"), Err("bad-args".into()));
        assert_eq!(parse("state\textra\n"), Err("bad-args".into()));
        assert_eq!(parse("state\t3\n"), Err("bad-args".into()));
        assert_eq!(parse("app-mute\tk\n"), Err("bad-args".into()));
        assert_eq!(parse("dev-volume\t\t5\n"), Err("bad-args".into()));
        assert_eq!(parse("dev-volume\tid\t300\n"), Err("range".into()));
        assert_eq!(parse("default\n"), Err("bad-args".into()));
        assert_eq!(parse("profile\t\n"), Err("bad-args".into()));
        assert_eq!(parse("night\tyes\n"), Err("bad-args".into()));
        assert_eq!(parse("watch\tnow\n"), Err("bad-args".into()));
    }

    #[test]
    fn sanitizes_names() {
        assert_eq!(sanitize("a\tb\nc\r"), "a b c ");
    }

    #[test]
    fn state_reply_follows_contract() {
        let r = format_state(&snap(), "0.2.1");
        assert_eq!(
            r,
            "kamerton\t1\t0.2.1\ndevice\tSpeakers Test\t42\t0\napp\tc:\\a\\player.exe\tPlayer\t50\t0\napp\tsystem\tSystem sounds\t100\t1\nend\n"
        );
    }

    #[test]
    fn state_without_device() {
        let r = format_state(&Snapshot::default(), "0.2.1");
        assert_eq!(r, "kamerton\t1\t0.2.1\nnodevice\nend\n");
    }

    #[test]
    fn hello_names_caps() {
        let r = format_hello("1.4.0");
        assert!(r.starts_with("kamerton\t2\t1.4.0\t"));
        assert!(r.contains("state2") && r.contains("watch"));
        assert!(r.ends_with('\n'));
    }

    #[test]
    fn state2_reply_follows_contract() {
        let r = format_state2(&snap(), "1.4.0", &extra());
        assert_eq!(
            r,
            "kamerton\t2\t1.4.0\n\
             output\td1\tSpeakers Test\t42\t0\t1\n\
             output\td2\tOther\t42\t0\t0\n\
             input\td0\tMic\t42\t0\t1\n\
             app\tc:\\a\\player.exe\tPlayer\t50\t0\n\
             app\tsystem\tSystem sounds\t100\t1\n\
             profile\tGame\n\
             profile\tWork day\n\
             night\t1\t0\t23:00\t07:00\t40\n\
             end\n"
        );
    }

    #[test]
    fn state2_without_devices() {
        let r = format_state2(&Snapshot::default(), "1.4.0", &Extra::default());
        assert_eq!(r, "kamerton\t2\t1.4.0\nnight\t0\t0\t00:00\t00:00\t0\nend\n");
    }

    #[test]
    fn resolves_on_default_output() {
        let s = snap();
        assert_eq!(resolve(&Request::Volume(30), &s), Ok(Action::Volume(Target::Device("d1".into()), 0.3)));
        assert_eq!(resolve(&Request::Mute(true), &s), Ok(Action::Mute(Target::Device("d1".into()), true)));
        let key = AppKey { device: "d1".into(), exe: "system".into() };
        assert_eq!(resolve(&Request::AppMute("system".into(), false), &s), Ok(Action::Mute(Target::App(key), false)));
        assert_eq!(resolve(&Request::AppVolume("zzz".into(), 1), &s), Err("no-app".into()));
        assert_eq!(resolve(&Request::Volume(1), &Snapshot::default()), Err("no-device".into()));
    }

    #[test]
    fn resolves_devices_by_id() {
        let s = snap();
        assert_eq!(
            resolve(&Request::DevVolume("d0".into(), 70), &s),
            Ok(Action::Volume(Target::Device("d0".into()), 0.7))
        );
        assert_eq!(
            resolve(&Request::DevMute("d2".into(), true), &s),
            Ok(Action::Mute(Target::Device("d2".into()), true))
        );
        assert_eq!(resolve(&Request::Default("d2".into()), &s), Ok(Action::Default("d2".into())));
        assert_eq!(resolve(&Request::Default("zz".into()), &s), Err("no-device".into()));
        // Устройство по ID не требует устройства по умолчанию.
        let mut none = snap();
        none.devices.iter_mut().for_each(|d| d.is_default = false);
        assert_eq!(
            resolve(&Request::DevMute("d1".into(), false), &none),
            Ok(Action::Mute(Target::Device("d1".into()), false))
        );
    }

    #[test]
    fn answers_through_host() {
        let host = Fake::default();
        assert_eq!(answer("profile\tGame", &host), "ok\n");
        // Имя с табуляцией клиент видит с пробелом — по нему и находится.
        assert_eq!(answer("profile\tWork day", &host), "ok\n");
        assert_eq!(answer("profile\tWork\tday", &host), "err\tbad-args\n");
        assert_eq!(answer("profile\tNope", &host), "err\tno-profile\n");
        assert_eq!(answer("night\t1", &host), "ok\n");
        assert_eq!(answer("open", &host), "ok\n");
        assert_eq!(*host.asked.borrow(), vec!["profile 0", "profile 1", "night true", "open"]);
        assert!(answer("state\t2", &host).starts_with("kamerton\t2\t"));
        assert!(answer("state", &host).starts_with("kamerton\t1\t"));
        assert_eq!(answer("frobnicate", &host), "err\tunknown\n");
        // Без управления звуком (трей не готов) — команды громкости отказывают, а не падают.
        assert_eq!(answer("volume\t10", &host), "err\tno-tray\n");
        let off = Fake { no_tray: true, ..Default::default() };
        assert_eq!(answer("night\t0", &off), "err\tno-tray\n");
    }

    #[test]
    fn change_counter_wakes_waiters() {
        let seen = wait_change(u64::MAX, std::time::Duration::ZERO);
        let t = std::thread::spawn(move || wait_change(seen, std::time::Duration::from_secs(5)));
        std::thread::sleep(std::time::Duration::from_millis(50));
        notify();
        assert!(t.join().unwrap() > seen);
    }

    /// Заглушка для живого канала: громкость устройства меняется из теста.
    struct Live {
        volume: Mutex<f32>,
        /// Столько выдуманных профилей сверх обычных — раздуть ответ больше буфера канала.
        padding: usize,
    }

    impl Host for Live {
        fn snapshot(&self) -> Result<Snapshot, AudioError> {
            let mut s = snap();
            s.devices[1].volume = *self.volume.lock().unwrap();
            Ok(s)
        }
        fn control(&self) -> Option<&Control> {
            None
        }
        fn extra(&self) -> Extra {
            let mut e = extra();
            e.profiles.extend((0..self.padding).map(|i| format!("padding profile number {i}")));
            e
        }
        fn open(&self) -> bool {
            true
        }
        fn profile(&self, _: usize) -> bool {
            true
        }
        fn night(&self, _: bool) -> bool {
            true
        }
    }

    fn connect(name: &str) -> std::fs::File {
        for _ in 0..100 {
            if let Ok(f) = std::fs::OpenOptions::new().read(true).write(true).open(name) {
                return f;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!("pipe {name} did not appear");
    }

    fn ask(name: &str, line: &str) -> String {
        use std::io::{Read, Write};
        let mut f = connect(name);
        f.write_all(line.as_bytes()).unwrap();
        let mut out = Vec::new();
        let _ = f.read_to_end(&mut out);
        String::from_utf8_lossy(&out).into_owned()
    }

    /// Блок ответа подписки до `end` (или `ping`).
    fn block(r: &mut impl std::io::BufRead) -> String {
        let mut out = String::new();
        loop {
            let mut line = String::new();
            assert!(r.read_line(&mut line).unwrap() > 0, "pipe closed");
            out.push_str(&line);
            if line == "end\n" || line == "ping\n" || line.starts_with("err\t") {
                return out;
            }
        }
    }

    /// Счётчики подписчиков общие на процесс: тесты живого канала — по одному.
    static PIPE_TESTS: Mutex<()> = Mutex::new(());

    /// Живой канал на своём имени: запросы, молчащий клиент, подписка и предел подписчиков. Звук не трогает.
    #[test]
    fn live_pipe_serves_and_watches() {
        let _one = PIPE_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        use std::io::{BufReader, Write};
        let name = format!(r"\\.\pipe\kamerton-ipc-test-{}", std::process::id());
        let live = Arc::new(Live { volume: Mutex::new(0.42), padding: 0 });
        let host: server::SharedHost = live.clone();
        let n = name.clone();
        std::thread::spawn(move || server::run(&n, host));

        assert!(ask(&name, "hello\n").starts_with("kamerton\t2\t"));
        assert!(ask(&name, "state\n").starts_with("kamerton\t1\t"));
        assert_eq!(ask(&name, "frobnicate\n"), "err\tunknown\n");
        // Молчащий клиент получает отказ через ~1 с, остальные обслуживаются.
        let silent = std::thread::spawn({
            let name = name.clone();
            move || ask(&name, "")
        });
        assert_eq!(ask(&name, "open\n"), "ok\n");
        assert_eq!(silent.join().unwrap(), "err\tbad-request\n");

        let mut w1 = connect(&name);
        w1.write_all(b"watch\n").unwrap();
        let mut r1 = BufReader::new(w1.try_clone().unwrap());
        assert!(block(&mut r1).contains("output\td1\tSpeakers Test\t42\t0\t1\n"));
        *live.volume.lock().unwrap() = 0.7;
        let t = std::time::Instant::now();
        notify();
        assert!(block(&mut r1).contains("output\td1\tSpeakers Test\t70\t0\t1\n"));
        assert!(t.elapsed() < std::time::Duration::from_millis(500), "watch update took {:?}", t.elapsed());

        let mut w2 = connect(&name);
        w2.write_all(b"watch\n").unwrap();
        let mut r2 = BufReader::new(w2.try_clone().unwrap());
        assert!(block(&mut r2).starts_with("kamerton\t2\t"));
        assert_eq!(ask(&name, "watch\n"), "err\tbusy\n");
        // Подписчик ушёл — место освобождается.
        drop(r2);
        drop(w2);
        *live.volume.lock().unwrap() = 0.1;
        notify();
        assert!(block(&mut r1).contains("\t10\t0\t1\n"));
        let mut again = None;
        for _ in 0..30 {
            let mut w3 = connect(&name);
            w3.write_all(b"watch\n").unwrap();
            let mut r3 = BufReader::new(w3.try_clone().unwrap());
            let first = block(&mut r3);
            if first.starts_with("kamerton\t2\t") {
                again = Some(first);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        assert!(again.is_some(), "a new watcher was not accepted after one left");
        drop(r1);
        drop(w1);
        wait_no_watchers();
    }

    /// Подписчик, который не читает: запись упирается в полный буфер, сторож через ~1 с отключает его, и место
    /// освобождается. Ответ раздут профилями больше буфера канала (64 КиБ).
    #[test]
    fn live_pipe_drops_non_reading_watcher() {
        let _one = PIPE_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        use std::io::{BufReader, Write};
        let name = format!(r"\\.\pipe\kamerton-ipc-stuck-{}", std::process::id());
        let live = Arc::new(Live { volume: Mutex::new(0.42), padding: 3000 });
        let host: server::SharedHost = live.clone();
        let n = name.clone();
        std::thread::spawn(move || server::run(&n, host));

        // Два не читающих подписчика занимают оба места.
        let mut stuck: Vec<std::fs::File> = Vec::new();
        for _ in 0..2 {
            let mut f = connect(&name);
            f.write_all(b"watch\n").unwrap();
            stuck.push(f);
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert_eq!(ask(&name, "watch\n"), "err\tbusy\n");
        // Изменения: следующее уже не влезает в их буфер.
        for v in [0.5, 0.6, 0.7] {
            *live.volume.lock().unwrap() = v;
            notify();
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        let t = std::time::Instant::now();
        let accepted = loop {
            let mut w = connect(&name);
            w.write_all(b"watch\n").unwrap();
            let mut first = String::new();
            let _ = std::io::BufRead::read_line(&mut BufReader::new(w), &mut first);
            if first.starts_with("kamerton\t2\t") {
                break true;
            }
            if t.elapsed() > std::time::Duration::from_secs(6) {
                break false;
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        };
        assert!(accepted, "the non-reading watcher was not dropped");
        drop(stuck);
        wait_no_watchers();
    }

    /// Дождаться, пока сервер отпустит всех подписчиков теста (ушедших замечает раз в секунду).
    fn wait_no_watchers() {
        let t = std::time::Instant::now();
        while server::watchers() > 0 && t.elapsed() < std::time::Duration::from_secs(5) {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert_eq!(server::watchers(), 0, "watchers left behind");
    }

    #[test]
    fn error_line() {
        assert_eq!(format_err("unknown"), "err\tunknown\n");
    }
}
