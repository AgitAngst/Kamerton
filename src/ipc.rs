// Канал для Antresol: именованный канал `\\.\pipe\kamerton-ipc`, один запрос на подключение.
// Протокол (версия 1) — docs/architecture/overview.md, раздел «Связь с Antresol». Разбор запросов и
// сбор ответов — чистые функции (тесты внизу); поток канала и вызовы Win32 — в модуле `server`.

use crate::audio::{AudioError, Control, Flow, Snapshot, Target, model};

pub const PIPE_NAME: &str = r"\\.\pipe\kamerton-ipc";

#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    State,
    Volume(u32),
    Mute(bool),
    AppVolume(String, u32),
    AppMute(String, bool),
    Open,
}

/// Что сделать со звуком после разбора запроса на снимке.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Volume(Target, f32),
    Mute(Target, bool),
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

/// Разобрать строку запроса (с `\n` или без). Ошибка — текст после `err\t`.
pub fn parse(line: &str) -> Result<Request, String> {
    let line = line.trim_end_matches(['\n', '\r']);
    let parts: Vec<&str> = line.split('\t').collect();
    match parts.as_slice() {
        ["state"] => Ok(Request::State),
        ["open"] => Ok(Request::Open),
        ["volume", n] => Ok(Request::Volume(percent_arg(n)?)),
        ["mute", f] => Ok(Request::Mute(flag_arg(f)?)),
        ["app-volume", k, n] => Ok(Request::AppVolume((*k).to_string(), percent_arg(n)?)),
        ["app-mute", k, f] => Ok(Request::AppMute((*k).to_string(), flag_arg(f)?)),
        ["state" | "open" | "volume" | "mute" | "app-volume" | "app-mute", ..] => Err("bad-args".into()),
        _ => Err("unknown".into()),
    }
}

fn default_output(s: &Snapshot) -> Option<&model::Device> {
    s.devices.iter().find(|d| d.flow == Flow::Render && d.is_default)
}

fn level(p: u32) -> f32 {
    p as f32 / 100.0
}

/// Перевести запрос на звук в действие над устройством по умолчанию. `State` и `Open` сюда не попадают.
pub fn resolve(req: &Request, s: &Snapshot) -> Result<Action, String> {
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
        Request::State | Request::Open => Err("unknown".into()),
    }
}

/// Ответ на `state`: устройство вывода по умолчанию и его программы в порядке окна.
pub fn format_state(s: &Snapshot, version: &str) -> String {
    let mut out = format!("kamerton\t1\t{}\n", sanitize(version));
    match default_output(s) {
        None => out.push_str("nodevice\n"),
        Some(d) => {
            out.push_str(&format!("device\t{}\t{}\t{}\n", sanitize(&d.name), model::percent(d.volume), d.muted as u8));
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
    }
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

/// Целиком запрос → ответ. `open` — просьба окну (само открытие — в потоке окна, не здесь).
pub fn answer(line: &str, control: &Control, open: &dyn Fn() -> bool) -> String {
    let req = match parse(line) {
        Ok(r) => r,
        Err(e) => return format_err(&e),
    };
    let snapshot = || control.snapshot().map_err(audio_err);
    let result: Result<String, String> = match &req {
        Request::State => snapshot().map(|s| format_state(&s, env!("CARGO_PKG_VERSION"))),
        Request::Open => {
            if open() {
                Ok("ok\n".into())
            } else {
                Err("no-tray".into())
            }
        }
        _ => snapshot().and_then(|s| resolve(&req, &s)).and_then(|a| {
            match a {
                Action::Volume(t, v) => control.set_volume(t, v),
                Action::Mute(t, m) => control.set_muted(t, m),
            }
            .map(|_| "ok\n".to_string())
            .map_err(audio_err)
        }),
    };
    result.unwrap_or_else(|e| format_err(&e))
}

mod server {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    use windows::Win32::Foundation::{CloseHandle, ERROR_PIPE_CONNECTED, GetLastError, HANDLE};
    use windows::Win32::Storage::FileSystem::{
        FILE_FLAG_FIRST_PIPE_INSTANCE, FlushFileBuffers, PIPE_ACCESS_DUPLEX, ReadFile, WriteFile,
    };
    use windows::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
        PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT, PeekNamedPipe,
    };
    use windows::core::HSTRING;

    use super::{Control, PIPE_NAME, answer, format_err};

    /// Столько подключений обслуживается разом; остальные закрываются сразу.
    const MAX_CLIENTS: usize = 8;
    /// Клиент должен прислать строку за это время.
    const READ_LIMIT: Duration = Duration::from_millis(1000);
    /// Дольше строка запроса быть не может.
    const MAX_LINE: usize = 4096;

    static ACTIVE: AtomicUsize = AtomicUsize::new(0);

    struct Pipe(HANDLE);
    unsafe impl Send for Pipe {}
    impl Drop for Pipe {
        fn drop(&mut self) {
            unsafe {
                let _ = DisconnectNamedPipe(self.0);
                let _ = CloseHandle(self.0);
            }
        }
    }

    fn create(first: bool) -> Option<Pipe> {
        let mut open_mode = PIPE_ACCESS_DUPLEX;
        if first {
            // Первый экземпляр: имя не должно быть уже занято чужим процессом.
            open_mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
        }
        let h = unsafe {
            CreateNamedPipeW(
                &HSTRING::from(PIPE_NAME),
                open_mode,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_UNLIMITED_INSTANCES,
                8192,
                8192,
                0,
                None,
            )
        };
        (!h.is_invalid()).then_some(Pipe(h))
    }

    /// Прочитать строку запроса до `\n`, не дольше `READ_LIMIT`. Опрос `PeekNamedPipe` — чтобы молчащий
    /// клиент не завесил поток навсегда.
    fn read_line(pipe: &Pipe) -> Option<String> {
        let start = Instant::now();
        let mut buf: Vec<u8> = Vec::new();
        loop {
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
                if start.elapsed() > READ_LIMIT {
                    return None;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        String::from_utf8(buf).ok()
    }

    fn write_all(pipe: &Pipe, data: &[u8]) {
        let mut rest = data;
        while !rest.is_empty() {
            let mut n = 0u32;
            if unsafe { WriteFile(pipe.0, Some(rest), Some(&mut n), None) }.is_err() || n == 0 {
                return;
            }
            rest = &rest[n as usize..];
        }
        unsafe {
            let _ = FlushFileBuffers(pipe.0);
        }
    }

    fn serve(pipe: Pipe, control: Control, open: fn() -> bool) {
        let reply = match read_line(&pipe) {
            Some(line) => answer(&line, &control, &open),
            None => format_err("bad-request"),
        };
        write_all(&pipe, reply.as_bytes());
    }

    pub fn run(control: Control, open: fn() -> bool) {
        let mut first = true;
        loop {
            let Some(pipe) = create(first) else {
                // Имя занято или Windows отказала: подождать и попробовать снова; программу это не роняет.
                std::thread::sleep(Duration::from_secs(2));
                first = false;
                continue;
            };
            first = false;
            let connected =
                unsafe { ConnectNamedPipe(pipe.0, None) }.is_ok() || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;
            if !connected {
                continue;
            }
            if ACTIVE.fetch_add(1, Ordering::SeqCst) >= MAX_CLIENTS {
                ACTIVE.fetch_sub(1, Ordering::SeqCst);
                continue;
            }
            let control = control.clone();
            let spawned = std::thread::Builder::new().name("ipc-client".into()).spawn(move || {
                serve(pipe, control, open);
                ACTIVE.fetch_sub(1, Ordering::SeqCst);
            });
            if spawned.is_err() {
                ACTIVE.fetch_sub(1, Ordering::SeqCst);
            }
        }
    }
}

/// Запустить поток канала. `open` — «открыть окно микшера»; `false`, если трей не готов.
pub fn start(control: Control, open: fn() -> bool) {
    let _ = std::thread::Builder::new().name("ipc".into()).spawn(move || server::run(control, open));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::{App, AppKey, Device};

    fn app(exe: &str, name: &str, vol: f32, muted: bool) -> App {
        App {
            key: AppKey { device: "d1".into(), exe: exe.into() },
            name: name.into(),
            path: None,
            volume: vol,
            muted,
            system: false,
            pids: vec![1],
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
    fn rejects_bad_requests() {
        assert_eq!(parse("nope\n"), Err("unknown".into()));
        assert_eq!(parse("\n"), Err("unknown".into()));
        assert_eq!(parse("volume\n"), Err("bad-args".into()));
        assert_eq!(parse("volume\tx\n"), Err("bad-args".into()));
        assert_eq!(parse("volume\t101\n"), Err("range".into()));
        assert_eq!(parse("volume\t-1\n"), Err("bad-args".into()));
        assert_eq!(parse("mute\t2\n"), Err("bad-args".into()));
        assert_eq!(parse("state\textra\n"), Err("bad-args".into()));
        assert_eq!(parse("app-mute\tk\n"), Err("bad-args".into()));
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
    fn error_line() {
        assert_eq!(format_err("unknown"), "err\tunknown\n");
    }
}
