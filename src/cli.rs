// Командная строка: `--set`, `--mute`, `--get`. Разбор ключей, поиск цели в снимке и расчёт новых значений —
// чистые функции (тесты внизу); ядро звука зовёт `main.rs`.

use crate::audio::model::step;
use crate::audio::{Flow, Snapshot, Target};

/// Громкость из командной строки, доли 0..1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Level {
    /// `45`, `45%`
    Abs(f32),
    /// `+5`, `-5`
    Delta(f32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MuteMode {
    On,
    Off,
    Toggle,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Set(String, Level),
    Mute(String, MuteMode),
    Get(String),
}

/// Ошибка командной строки: код выхода и сообщение (по-английски, в stderr).
#[derive(Debug, Clone, PartialEq)]
pub struct CliError {
    pub code: i32,
    pub message: String,
}

pub const EXIT_AUDIO: i32 = 1;
pub const EXIT_USAGE: i32 = 2;
pub const EXIT_TARGET: i32 = 3;

fn usage(message: impl Into<String>) -> CliError {
    CliError { code: EXIT_USAGE, message: message.into() }
}

fn target_err(message: impl Into<String>) -> CliError {
    CliError { code: EXIT_TARGET, message: message.into() }
}

/// Громкость: `45`, `45%`, `+5`, `-5` (проценты).
pub fn parse_level(s: &str) -> Result<Level, CliError> {
    let t = s.trim().trim_end_matches('%');
    let bad = || usage(format!("bad volume '{s}': use 0-100, 45%, +5 or -5"));
    let v: f32 = t.parse().map_err(|_| bad())?;
    if !v.is_finite() {
        return Err(bad());
    }
    if t.starts_with('+') || t.starts_with('-') {
        return Ok(Level::Delta(v / 100.0));
    }
    if !(0.0..=100.0).contains(&v) {
        return Err(usage(format!("volume {v} is out of range 0-100")));
    }
    Ok(Level::Abs(v / 100.0))
}

fn parse_mode(s: &str) -> Result<MuteMode, CliError> {
    match s.to_ascii_lowercase().as_str() {
        "on" => Ok(MuteMode::On),
        "off" => Ok(MuteMode::Off),
        "toggle" => Ok(MuteMode::Toggle),
        _ => Err(usage(format!("bad mute mode '{s}': use on, off or toggle"))),
    }
}

/// Команда из ключей; `None` — это не команда управления (другие режимы разбирает `main`).
pub fn parse(args: &[String]) -> Option<Result<Command, CliError>> {
    let i = args.iter().position(|a| matches!(a.as_str(), "--set" | "--mute" | "--get"))?;
    let rest = &args[i + 1..];
    let target = |n: usize| -> Result<String, CliError> {
        rest.get(n)
            .filter(|t| !t.starts_with("--"))
            .cloned()
            .ok_or_else(|| usage(format!("{} needs a target (default, input, an app or a device name)", args[i])))
    };
    Some(match args[i].as_str() {
        "--set" => target(0).and_then(|t| {
            let v = rest.get(1).ok_or_else(|| usage("--set needs a volume: 0-100, 45%, +5 or -5"))?;
            Ok(Command::Set(t, parse_level(v)?))
        }),
        "--mute" => target(0).and_then(|t| {
            let mode = rest.get(1).filter(|m| !m.starts_with("--")).map_or(Ok(MuteMode::Toggle), |m| parse_mode(m))?;
            Ok(Command::Mute(t, mode))
        }),
        _ => target(0).map(Command::Get),
    })
}

/// Цель, её громкость и «без звука» сейчас.
pub type Item = (Target, f32, bool);

/// Найденная цель: что менять и как она выглядит сейчас.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    /// Имя для вывода.
    pub label: String,
    /// Цели и их громкость сейчас (у программы — по сессии на каждом устройстве).
    pub items: Vec<Item>,
}

impl Found {
    /// Громкость для показа: у самой громкой цели.
    pub fn volume(&self) -> f32 {
        self.items.iter().map(|i| i.1).fold(0.0, f32::max)
    }

    /// «Без звука» для показа: когда молчат все.
    pub fn muted(&self) -> bool {
        !self.items.is_empty() && self.items.iter().all(|i| i.2)
    }
}

fn exe_stem(exe: &str) -> &str {
    let file = exe.rsplit(['\\', '/']).next().unwrap_or(exe);
    file.strip_suffix(".exe").or_else(|| file.strip_suffix(".EXE")).unwrap_or(file)
}

/// Подходящие слова: точное совпадение важнее части имени; регистр не важен.
fn pick<T: Clone>(items: &[(T, Vec<String>)], query: &str) -> Vec<T> {
    let q = query.to_lowercase();
    let exact: Vec<T> =
        items.iter().filter(|(_, names)| names.iter().any(|n| n.to_lowercase() == q)).map(|(t, _)| t.clone()).collect();
    if !exact.is_empty() {
        return exact;
    }
    items
        .iter()
        .filter(|(_, names)| names.iter().any(|n| n.to_lowercase().contains(&q)))
        .map(|(t, _)| t.clone())
        .collect()
}

/// Найти цель в снимке.
pub fn find(s: &Snapshot, query: &str) -> Result<Found, CliError> {
    let q = query.trim();
    if q.is_empty() {
        return Err(usage("empty target"));
    }
    let device_found = |d: &crate::audio::Device| Found {
        label: d.name.clone(),
        items: vec![(Target::Device(d.id.clone()), d.volume, d.muted)],
    };
    for (word, flow) in [("default", Flow::Render), ("input", Flow::Capture)] {
        if q.eq_ignore_ascii_case(word) {
            return s.devices.iter().find(|d| d.flow == flow && d.is_default).map(device_found).ok_or_else(|| {
                target_err(format!("no default {} device", if flow == Flow::Render { "output" } else { "input" }))
            });
        }
    }
    // Программы на устройствах вывода: одна программа — все её сессии на всех устройствах.
    let mut by_exe: Vec<(String, String, Vec<Item>)> = Vec::new();
    for d in s.devices.iter().filter(|d| d.flow == Flow::Render) {
        for a in &d.apps {
            let item = (Target::App(a.key.clone()), a.volume, a.muted);
            match by_exe.iter_mut().find(|e| e.0 == a.key.exe) {
                Some(e) => e.2.push(item),
                None => by_exe.push((a.key.exe.clone(), a.name.clone(), vec![item])),
            }
        }
    }
    let apps: Vec<(usize, Vec<String>)> = by_exe
        .iter()
        .enumerate()
        .map(|(i, (exe, name, _))| (i, vec![name.clone(), exe_stem(exe).to_string()]))
        .collect();
    let hits = pick(&apps, q);
    match hits.as_slice() {
        [i] => return Ok(Found { label: by_exe[*i].1.clone(), items: by_exe[*i].2.clone() }),
        [] => {}
        many => {
            let names: Vec<&str> = many.iter().map(|i| by_exe[*i].1.as_str()).collect();
            return Err(target_err(format!("'{q}' is ambiguous: {}", names.join(", "))));
        }
    }
    let devices: Vec<(usize, Vec<String>)> =
        s.devices.iter().enumerate().map(|(i, d)| (i, vec![d.name.clone()])).collect();
    let hits = pick(&devices, q);
    match hits.as_slice() {
        [i] => Ok(device_found(&s.devices[*i])),
        [] => Err(target_err(format!("'{q}' not found: no app or device with this name"))),
        many => {
            let names: Vec<&str> = many.iter().map(|i| s.devices[*i].name.as_str()).collect();
            Err(target_err(format!("'{q}' is ambiguous: {}", names.join(", "))))
        }
    }
}

/// Новые громкости: каждой цели — своя (у `Delta` — от её нынешней).
pub fn volumes(f: &Found, level: Level) -> Vec<(Target, f32)> {
    f.items
        .iter()
        .map(|(t, now, _)| {
            let v = match level {
                Level::Abs(v) => step(0.0, v),
                Level::Delta(d) => step(*now, d),
            };
            (t.clone(), v)
        })
        .collect()
}

/// Новое «без звука» для режима: переключение — хоть одна звучит → заглушить все, иначе включить все.
pub fn muted_after(f: &Found, mode: MuteMode) -> bool {
    match mode {
        MuteMode::On => true,
        MuteMode::Off => false,
        MuteMode::Toggle => f.items.iter().any(|i| !i.2),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::{App, AppKey, Device};

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn app(dev: &str, exe: &str, name: &str, volume: f32, muted: bool) -> App {
        App {
            key: AppKey { device: dev.into(), exe: exe.into() },
            name: name.into(),
            path: None,
            volume,
            muted,
            system: false,
            pids: vec![1],
            pinned: None,
        }
    }

    fn device(id: &str, name: &str, flow: Flow, default: bool, apps: Vec<App>) -> Device {
        Device {
            id: id.into(),
            name: name.into(),
            flow,
            volume: 0.5,
            muted: false,
            is_default: default,
            is_comm: false,
            balance: None,
            apps,
        }
    }

    fn snapshot() -> Snapshot {
        Snapshot {
            can_route: false,
            devices: vec![
                device(
                    "out1",
                    "Speakers",
                    Flow::Render,
                    true,
                    vec![
                        app("out1", "c:\\apps\\chrome.exe", "Google Chrome", 0.4, false),
                        app("out1", "c:\\apps\\calc.exe", "Calculator", 0.9, false),
                    ],
                ),
                device(
                    "out2",
                    "Headphones",
                    Flow::Render,
                    false,
                    vec![app("out2", "c:\\apps\\chrome.exe", "Google Chrome", 0.8, true)],
                ),
                device("in1", "Microphone", Flow::Capture, true, vec![]),
            ],
        }
    }

    #[test]
    fn level_forms() {
        assert_eq!(parse_level("45"), Ok(Level::Abs(0.45)));
        assert_eq!(parse_level("45%"), Ok(Level::Abs(0.45)));
        assert_eq!(parse_level("+5"), Ok(Level::Delta(0.05)));
        assert_eq!(parse_level("-10%"), Ok(Level::Delta(-0.10)));
        assert_eq!(parse_level("100"), Ok(Level::Abs(1.0)));
    }

    #[test]
    fn level_errors_are_usage() {
        for bad in ["", "abc", "101", "NaN", "5.5.5"] {
            assert_eq!(parse_level(bad).unwrap_err().code, EXIT_USAGE, "{bad}");
        }
    }

    #[test]
    fn parses_commands() {
        assert_eq!(parse(&args(&["--set", "chrome", "30"])), Some(Ok(Command::Set("chrome".into(), Level::Abs(0.3)))));
        assert_eq!(parse(&args(&["--mute", "default"])), Some(Ok(Command::Mute("default".into(), MuteMode::Toggle))));
        assert_eq!(parse(&args(&["--mute", "chrome", "off"])), Some(Ok(Command::Mute("chrome".into(), MuteMode::Off))));
        assert_eq!(parse(&args(&["--get", "input"])), Some(Ok(Command::Get("input".into()))));
        assert_eq!(parse(&args(&["--list"])), None);
    }

    #[test]
    fn parse_errors_are_usage() {
        for a in [
            &["--set"][..],
            &["--set", "x"],
            &["--set", "x", "loud"],
            &["--mute", "x", "maybe"],
            &["--get"],
            &["--get", "--for"],
        ] {
            let e = parse(&args(a)).unwrap().unwrap_err();
            assert_eq!(e.code, EXIT_USAGE, "{a:?}");
        }
    }

    #[test]
    fn default_and_input_words() {
        let s = snapshot();
        assert_eq!(find(&s, "default").unwrap().label, "Speakers");
        assert_eq!(find(&s, "INPUT").unwrap().label, "Microphone");
    }

    #[test]
    fn app_is_all_its_sessions() {
        let s = snapshot();
        let f = find(&s, "chrome").unwrap();
        assert_eq!(f.label, "Google Chrome");
        assert_eq!(f.items.len(), 2);
        assert_eq!(f.volume(), 0.8);
        assert!(!f.muted());
    }

    #[test]
    fn exact_beats_part_and_part_may_be_ambiguous() {
        let s = snapshot();
        assert_eq!(find(&s, "calc").unwrap().label, "Calculator");
        assert_eq!(find(&s, "calculator").unwrap().label, "Calculator");
        let e = find(&s, "c").unwrap_err();
        assert_eq!(e.code, EXIT_TARGET);
        assert!(
            e.message.contains("ambiguous") && e.message.contains("Calculator") && e.message.contains("Google Chrome")
        );
    }

    #[test]
    fn device_by_name_and_missing() {
        let s = snapshot();
        assert_eq!(find(&s, "head").unwrap().label, "Headphones");
        let e = find(&s, "nosuch").unwrap_err();
        assert_eq!(e.code, EXIT_TARGET);
        assert!(e.message.contains("not found"));
    }

    #[test]
    fn volumes_abs_and_delta() {
        let s = snapshot();
        let f = find(&s, "chrome").unwrap();
        let abs: Vec<f32> = volumes(&f, Level::Abs(0.3)).into_iter().map(|v| v.1).collect();
        assert_eq!(abs, vec![0.3, 0.3]);
        let rel: Vec<f32> = volumes(&f, Level::Delta(0.1)).into_iter().map(|v| v.1).collect();
        assert_eq!(rel, vec![0.5, 0.9]);
        let low: Vec<f32> = volumes(&f, Level::Delta(-0.9)).into_iter().map(|v| v.1).collect();
        assert_eq!(low, vec![0.0, 0.0]);
    }

    #[test]
    fn toggle_mutes_all_when_any_sounds() {
        let s = snapshot();
        let f = find(&s, "chrome").unwrap();
        assert!(muted_after(&f, MuteMode::Toggle));
        assert!(muted_after(&f, MuteMode::On));
        assert!(!muted_after(&f, MuteMode::Off));
        let both = Found { label: "x".into(), items: f.items.iter().map(|i| (i.0.clone(), i.1, true)).collect() };
        assert!(both.muted());
        assert!(!muted_after(&both, MuteMode::Toggle));
    }
}
