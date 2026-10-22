// Ядро звука: свой поток с COM (MTA), команды туда и события обратно по каналам.
// Окно и консоль видят только `Audio` и модель из model.rs; COM наружу не выходит.

pub mod model;
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
    Peaks(Reply<Vec<(Target, f32)>>),
}

/// Сообщение потоку звука: метка «перечитай» от Windows, команда или выход.
pub(crate) enum Msg {
    /// Изменился список устройств или устройство по умолчанию.
    Devices,
    /// Изменилось устройство или его программы.
    Device(String),
    /// Закрылся процесс — перечитать программы всех устройств.
    Sessions,
    Do(Command),
    Quit,
}

/// Что перечитать после склейки меток.
#[derive(Debug, Default, PartialEq)]
struct Marks {
    devices: bool,
    sessions: bool,
    device_ids: BTreeSet<String>,
}

impl Marks {
    /// Учесть метку; `false` — это не метка.
    fn add(&mut self, m: &Msg) -> bool {
        match m {
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
        self.devices || self.sessions || !self.device_ids.is_empty()
    }

    /// Самое широкое перечитывание поглощает узкие.
    fn plan(self) -> Marks {
        if self.devices {
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
    tx: Sender<Msg>,
    events: Receiver<Event>,
    thread: Option<JoinHandle<()>>,
}

impl Audio {
    /// Запустить поток звука. `wake` зовётся из него после каждой пачки событий (окну — перерисоваться).
    pub fn start(wake: impl Fn() + Send + 'static) -> Result<(Audio, Snapshot), AudioError> {
        let (tx, rx) = channel();
        let (etx, erx) = channel();
        let (ready_tx, ready_rx) = channel();
        let inner = tx.clone();
        let thread = std::thread::Builder::new()
            .name("audio".into())
            .spawn(move || run(inner, rx, etx, Box::new(wake), ready_tx))
            .map_err(|e| AudioError::Windows(e.to_string()))?;
        let snapshot = ready_rx.recv().map_err(|_| AudioError::Stopped)??;
        Ok((Audio { tx, events: erx, thread: Some(thread) }, snapshot))
    }

    /// События ядра по порядку.
    pub fn events(&self) -> &Receiver<Event> {
        &self.events
    }

    fn ask<T>(&self, make: impl FnOnce(Reply<T>) -> Command) -> Result<T, AudioError> {
        let (rtx, rrx) = channel();
        self.tx.send(Msg::Do(make(rtx))).map_err(|_| AudioError::Stopped)?;
        rrx.recv().map_err(|_| AudioError::Stopped)
    }

    /// Громкость 0..1 устройству или программе (всем её сессиям).
    #[allow(dead_code)] // зовёт окно этапа 0.2; консоль только читает
    pub fn set_volume(&self, target: Target, level: f32) -> Result<(), AudioError> {
        self.ask(|r| Command::SetVolume(target, level, r))?
    }

    #[allow(dead_code)] // зовёт окно этапа 0.2; консоль только читает
    pub fn set_muted(&self, target: Target, muted: bool) -> Result<(), AudioError> {
        self.ask(|r| Command::SetMuted(target, muted, r))?
    }

    /// Текущие пиковые уровни 0..1.
    pub fn peaks(&self) -> Vec<(Target, f32)> {
        self.ask(Command::Peaks).unwrap_or_default()
    }
}

impl Drop for Audio {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Quit);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn run(
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    events: Sender<Event>,
    wake: Box<dyn Fn() + Send>,
    ready: Sender<Result<Snapshot, AudioError>>,
) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let engine = win::Engine::new(tx);
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

    'outer: while let Ok(msg) = rx.recv() {
        let mut marks = Marks::default();
        if !handle(&engine, msg, &mut marks) {
            break;
        }
        if !marks.any() {
            continue;
        }
        let first = Instant::now();
        while first.elapsed() < MAX_HOLD {
            match rx.recv_timeout(QUIET) {
                Ok(m) => {
                    if !handle(&engine, m, &mut marks) {
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
        } else {
            for id in &plan.device_ids {
                engine.rescan_device(id);
            }
        }
        let now = engine.snapshot();
        let changes = model::diff(&last, &now);
        last = now;
        if !changes.is_empty() {
            for e in changes {
                let _ = events.send(e);
            }
            wake();
        }
    }
    drop(engine);
    unsafe { CoUninitialize() };
}

/// Выполнить команду сразу, метку — запомнить. `false` — пора выходить.
fn handle(engine: &win::Engine, msg: Msg, marks: &mut Marks) -> bool {
    if marks.add(&msg) {
        return true;
    }
    match msg {
        Msg::Do(Command::SetVolume(t, v, r)) => {
            let _ = r.send(engine.set_volume(&t, v));
        }
        Msg::Do(Command::SetMuted(t, m, r)) => {
            let _ = r.send(engine.set_muted(&t, m));
        }
        Msg::Do(Command::Peaks(r)) => {
            let _ = r.send(engine.peaks());
        }
        Msg::Quit => return false,
        Msg::Devices | Msg::Device(_) | Msg::Sessions => {}
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
    }
}
