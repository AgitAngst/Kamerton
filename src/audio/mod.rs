// Ядро звука: свой поток с COM (MTA), команды туда и события обратно по каналам.
// Окно и консоль видят только `Audio` и модель из model.rs; COM наружу не выходит.

pub mod model;
mod policy;
mod win;

use std::collections::BTreeSet;
use std::fmt;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};

// Окно этапа 0.2 берёт типы модели отсюда.
#[allow(unused_imports)]
pub use model::{App, AppKey, Device, Event, Flow, Snapshot, Target};
pub use policy::Roles;
pub use win::live_subscriptions;

/// Склеивать метки, пока они идут чаще этого.
const QUIET: Duration = Duration::from_millis(30);
/// Но не дольше этого — иначе при перетаскивании ползунка снимок не обновится вовсе.
const MAX_HOLD: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, PartialEq)]
pub enum AudioError {
    /// Устройства или программы уже нет.
    NotFound,
    /// Поток звука остановлен.
    Stopped,
    Windows(String),
}

impl fmt::Display for AudioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AudioError::NotFound => f.write_str("device or app not found"),
            AudioError::Stopped => f.write_str("audio thread stopped"),
            AudioError::Windows(m) => write!(f, "windows error: {m}"),
        }
    }
}

type Reply<T> = Sender<T>;

pub(crate) enum Command {
    SetVolume(Target, f32, Reply<Result<(), AudioError>>),
    SetMuted(Target, bool, Reply<Result<(), AudioError>>),
    /// Сдвинуть громкость устройства вывода по умолчанию; ответ — новая громкость.
    AdjustDefault(f32, Reply<Result<f32, AudioError>>),
    Peaks(Reply<Vec<(Target, f32)>>),
    Snapshot(Reply<Snapshot>),
    SetDefault(String, Roles, Reply<Result<(), AudioError>>),
    MoveApp(AppKey, Option<String>, Reply<Result<(), AudioError>>),
    SetBalance(String, f32, Reply<Result<(), AudioError>>),
}

/// Сообщение потоку звука: метка «перечитай» от Windows, команда или выход.
pub(crate) enum Msg {
    /// Изменился список устройств или устройство по умолчанию.
    Devices,
    /// Изменилось устройство или его программы.
    Device(String),
    /// Закрылся процесс — перечитать программы всех устройств.
    Sessions,
    /// Пересоздать ядро: служба звука перезапустилась, вернулись в сеанс Windows.
    Reset,
    /// Windows сообщила о новой сессии на устройстве и отдала её саму.
    NewSession(String, win::NewControl),
    Do(Command),
    Quit,
}

/// Что перечитать после склейки меток.
#[derive(Debug, Default, PartialEq)]
struct Marks {
    reset: bool,
    devices: bool,
    sessions: bool,
    device_ids: BTreeSet<String>,
}

impl Marks {
    /// Учесть метку; `false` — это не метка.
    fn add(&mut self, m: &Msg) -> bool {
        match m {
            Msg::Reset => self.reset = true,
            Msg::Devices => self.devices = true,
            Msg::Sessions => self.sessions = true,
            Msg::Device(id) => {
                self.device_ids.insert(id.clone());
            }
            _ => return false,
        }
        true
    }

    fn any(&self) -> bool {
        self.reset || self.devices || self.sessions || !self.device_ids.is_empty()
    }

    /// Самое широкое перечитывание поглощает узкие.
    fn plan(self) -> Marks {
        if self.reset {
            Marks { reset: true, ..Default::default() }
        } else if self.devices {
            Marks { devices: true, ..Default::default() }
        } else if self.sessions {
            Marks { sessions: true, ..Default::default() }
        } else {
            self
        }
    }
}

/// Ядро звука. Остановка — при удалении.
pub struct Audio {
    control: Control,
    events: Receiver<Event>,
    thread: Option<JoinHandle<()>>,
}

/// Команды ядру из любого потока (трей, окно). Копируется дёшево; ядро живёт, пока жив `Audio`.
#[derive(Clone)]
pub struct Control {
    tx: Sender<Msg>,
}

impl Audio {
    /// Запустить поток звука. `wake` зовётся из него после каждой пачки событий (окну — перерисоваться).
    pub fn start(wake: impl Fn() + Send + 'static) -> Result<(Audio, Snapshot), AudioError> {
        Self::start_with(wake, true)
    }

    /// То же без очереди событий: для окна, которое само спрашивает снимок, пока видно. Иначе, пока
    /// окно спрятано и не читает очередь, события копились бы в ней.
    pub fn start_quiet(wake: impl Fn() + Send + 'static) -> Result<(Audio, Snapshot), AudioError> {
        Self::start_with(wake, false)
    }

    fn start_with(wake: impl Fn() + Send + 'static, with_events: bool) -> Result<(Audio, Snapshot), AudioError> {
        let (tx, rx) = channel();
        let (etx, erx) = channel();
        let etx = with_events.then_some(etx);
        let (ready_tx, ready_rx) = channel();
        let inner = tx.clone();
        let thread = std::thread::Builder::new()
            .name("audio".into())
            .spawn(move || run(inner, rx, etx, Box::new(wake), ready_tx))
            .map_err(|e| AudioError::Windows(e.to_string()))?;
        let snapshot = ready_rx.recv().map_err(|_| AudioError::Stopped)??;
        Ok((Audio { control: Control { tx }, events: erx, thread: Some(thread) }, snapshot))
    }

    /// События ядра по порядку (пусто у `start_quiet`).
    pub fn events(&self) -> &Receiver<Event> {
        &self.events
    }

    pub fn control(&self) -> Control {
        self.control.clone()
    }

    /// Громкость 0..1 устройству или программе (всем её сессиям).
    #[allow(dead_code)] // консоль только читает; окно зовёт через `Control`
    pub fn set_volume(&self, target: Target, level: f32) -> Result<(), AudioError> {
        self.control.set_volume(target, level)
    }

    #[allow(dead_code)] // консоль только читает; окно зовёт через `Control`
    pub fn set_muted(&self, target: Target, muted: bool) -> Result<(), AudioError> {
        self.control.set_muted(target, muted)
    }

    /// Текущие пиковые уровни 0..1.
    pub fn peaks(&self) -> Vec<(Target, f32)> {
        self.control.peaks()
    }
}

impl Control {
    fn ask<T>(&self, make: impl FnOnce(Reply<T>) -> Command) -> Result<T, AudioError> {
        let (rtx, rrx) = channel();
        self.tx.send(Msg::Do(make(rtx))).map_err(|_| AudioError::Stopped)?;
        rrx.recv().map_err(|_| AudioError::Stopped)
    }

    pub fn set_volume(&self, target: Target, level: f32) -> Result<(), AudioError> {
        self.ask(|r| Command::SetVolume(target, level, r))?
    }

    pub fn set_muted(&self, target: Target, muted: bool) -> Result<(), AudioError> {
        self.ask(|r| Command::SetMuted(target, muted, r))?
    }

    /// Сдвинуть громкость устройства вывода по умолчанию на `delta` (−1..1); ответ — новая 0..1.
    pub fn adjust_default(&self, delta: f32) -> Result<f32, AudioError> {
        self.ask(|r| Command::AdjustDefault(delta, r))?
    }

    pub fn peaks(&self) -> Vec<(Target, f32)> {
        self.ask(Command::Peaks).unwrap_or_default()
    }

    /// Снимок «как сейчас».
    pub fn snapshot(&self) -> Result<Snapshot, AudioError> {
        self.ask(Command::Snapshot)
    }

    /// Устройство по умолчанию (`Roles::All`) или устройство связи.
    pub fn set_default(&self, device: &str, roles: Roles) -> Result<(), AudioError> {
        self.ask(|r| Command::SetDefault(device.to_string(), roles, r))?
    }

    /// Пересоздать ядро (вернулись в сеанс Windows и т. п.).
    pub fn reset(&self) {
        let _ = self.tx.send(Msg::Reset);
    }

    /// Баланс стереоустройства: −1 — только левый, 1 — только правый.
    pub fn set_balance(&self, device: &str, balance: f32) -> Result<(), AudioError> {
        self.ask(|r| Command::SetBalance(device.to_string(), balance, r))?
    }

    /// Закрепить программу за устройством; `None` — «как в системе».
    pub fn move_app(&self, app: AppKey, to: Option<String>) -> Result<(), AudioError> {
        self.ask(|r| Command::MoveApp(app, to, r))?
    }
}

impl Drop for Audio {
    fn drop(&mut self) {
        let _ = self.control.tx.send(Msg::Quit);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn run(
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    events: Option<Sender<Event>>,
    wake: Box<dyn Fn() + Send>,
    ready: Sender<Result<Snapshot, AudioError>>,
) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let engine = win::Engine::new(tx.clone());
    let mut engine = match engine {
        Ok(e) => e,
        Err(e) => {
            let _ = ready.send(Err(AudioError::Windows(e.message())));
            unsafe { CoUninitialize() };
            return;
        }
    };
    let mut last = engine.snapshot();
    let _ = ready.send(Ok(last.clone()));

    // Ядро не пересоздалось — следующая попытка не раньше чем через 2 с.
    let mut retry: Option<Instant> = None;
    'outer: loop {
        let msg = match retry {
            Some(at) => match rx.recv_timeout(at.saturating_duration_since(Instant::now())) {
                Ok(m) => Some(m),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            },
            None => match rx.recv() {
                Ok(m) => Some(m),
                Err(_) => break,
            },
        };
        let mut marks = Marks::default();
        match msg {
            Some(m) => {
                if !handle(&mut engine, m, &mut marks) {
                    break;
                }
            }
            None => marks.reset = true,
        }
        if !marks.any() {
            continue;
        }
        let first = Instant::now();
        while first.elapsed() < MAX_HOLD {
            match rx.recv_timeout(QUIET) {
                Ok(m) => {
                    if !handle(&mut engine, m, &mut marks) {
                        break 'outer;
                    }
                }
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => break 'outer,
            }
        }
        let plan = marks.plan();
        if plan.devices {
            engine.rescan_devices();
        } else if plan.sessions {
            engine.rescan_sessions();
        } else if !plan.reset {
            for id in &plan.device_ids {
                engine.rescan_device(id);
            }
        }
        if plan.reset || !engine.healthy() {
            // Новое ядро заменяет старое; лишние события на миг пересменки снимает разница снимков.
            match win::Engine::new(tx.clone()) {
                Ok(fresh) => {
                    engine = fresh;
                    retry = None;
                    eprintln!("audio: core restarted");
                }
                Err(e) => {
                    retry = Some(Instant::now() + Duration::from_secs(2));
                    eprintln!("audio: restart failed, retry in 2 s: {}", e.message());
                }
            }
        }
        let now = engine.snapshot();
        let changes = model::diff(&last, &now);
        last = now;
        if !changes.is_empty() {
            if let Some(events) = &events {
                for e in changes {
                    let _ = events.send(e);
                }
            }
            wake();
        }
    }
    drop(engine);
    unsafe { CoUninitialize() };
}

/// Выполнить команду сразу, метку — запомнить. `false` — пора выходить.
fn handle(engine: &mut win::Engine, msg: Msg, marks: &mut Marks) -> bool {
    if marks.add(&msg) {
        return true;
    }
    match msg {
        Msg::NewSession(device, ctrl) => {
            engine.add_session(&device, ctrl);
            marks.device_ids.insert(device);
        }
        Msg::Do(Command::SetVolume(t, v, r)) => {
            let _ = r.send(engine.set_volume(&t, v));
        }
        Msg::Do(Command::SetMuted(t, m, r)) => {
            let _ = r.send(engine.set_muted(&t, m));
        }
        Msg::Do(Command::AdjustDefault(d, r)) => {
            let _ = r.send(engine.adjust_default(d));
        }
        Msg::Do(Command::Peaks(r)) => {
            let _ = r.send(engine.peaks());
        }
        Msg::Do(Command::Snapshot(r)) => {
            let _ = r.send(engine.snapshot());
        }
        Msg::Do(Command::SetDefault(id, roles, r)) => {
            let _ = r.send(engine.set_default(&id, roles));
        }
        Msg::Do(Command::SetBalance(id, b, r)) => {
            let _ = r.send(engine.set_balance(&id, b));
        }
        Msg::Do(Command::MoveApp(key, to, r)) => {
            let result = engine.move_app(&key, to.as_deref());
            if result.is_ok() {
                marks.sessions = true;
            }
            let _ = r.send(result);
        }
        Msg::Quit => return false,
        Msg::Reset | Msg::Devices | Msg::Device(_) | Msg::Sessions => {}
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_merge_device_ids() {
        let mut m = Marks::default();
        for msg in [Msg::Device("a".into()), Msg::Device("b".into()), Msg::Device("a".into())] {
            assert!(m.add(&msg));
        }
        let plan = m.plan();
        assert_eq!(plan.device_ids.into_iter().collect::<Vec<_>>(), vec!["a", "b"]);
    }

    #[test]
    fn marks_wider_rescan_wins() {
        let mut m = Marks::default();
        m.add(&Msg::Device("a".into()));
        m.add(&Msg::Sessions);
        assert_eq!(m.plan(), Marks { sessions: true, ..Default::default() });
        let mut m = Marks::default();
        m.add(&Msg::Sessions);
        m.add(&Msg::Devices);
        assert_eq!(m.plan(), Marks { devices: true, ..Default::default() });
    }

    #[test]
    fn reset_wins_over_everything() {
        let mut m = Marks::default();
        m.add(&Msg::Devices);
        m.add(&Msg::Reset);
        m.add(&Msg::Device("a".into()));
        assert_eq!(m.plan(), Marks { reset: true, ..Default::default() });
    }

    #[test]
    fn marks_ignore_commands() {
        let mut m = Marks::default();
        assert!(!m.add(&Msg::Quit));
        assert!(!m.any());
    }

    // Настоящий Core Audio: команда неизвестной цели ничего не трогает и отвечает NotFound.
    #[test]
    fn unknown_target_is_not_found() {
        let (audio, _) = Audio::start(|| {}).expect("audio core starts");
        let key = AppKey { device: "no-such-device".into(), exe: "nothing.exe".into() };
        assert_eq!(audio.set_volume(Target::App(key.clone()), 0.5), Err(AudioError::NotFound));
        assert_eq!(audio.set_muted(Target::App(key), true), Err(AudioError::NotFound));
        assert_eq!(audio.set_volume(Target::Device("no-such-device".into()), 0.5), Err(AudioError::NotFound));
        // Перенос и смена устройства неизвестной цели — «не найдено» до любого вызова Windows.
        let c = audio.control();
        let key = AppKey { device: "no-such-device".into(), exe: "nothing.exe".into() };
        assert_eq!(c.move_app(key, None), Err(AudioError::NotFound));
        assert_eq!(c.set_default("no-such-device", Roles::All), Err(AudioError::NotFound));
    }
}
