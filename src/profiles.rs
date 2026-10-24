// Профили и память громкости программ. Здесь только решения — чистые функции: что сделать по событиям
// ядра и при применении профиля. Делает их трей (`tray.rs`) через `Control`.

use std::collections::BTreeMap;

use crate::audio::{AppKey, Event, Flow, Snapshot, model};

/// Громкость, «без звука» и закреплённое устройство программы.
#[derive(Debug, Clone, PartialEq)]
pub struct AppState {
    pub volume: f32,
    pub muted: bool,
    /// `None` — «как в системе».
    pub device: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Profile {
    pub name: String,
    pub output: Option<String>,
    pub input: Option<String>,
    /// По пути exe в нижнем регистре.
    pub apps: BTreeMap<String, AppState>,
    /// Применять, когда подключается это устройство.
    pub on_connect: Option<String>,
}

/// Что сделать.
#[derive(Debug, Clone, PartialEq)]
pub enum Cmd {
    SetDefault(String),
    /// Громкость устройства (потолок).
    SetDeviceVolume(String, f32),
    SetVolume(AppKey, f32),
    SetMuted(AppKey, bool),
    Move(AppKey, Option<String>),
}

/// Память громкости: exe → (громкость, без звука).
pub type Memory = BTreeMap<String, (f32, bool)>;

fn real_app(exe: &str) -> bool {
    !exe.starts_with("pid:") && exe != "system"
}

/// Снять профиль с того, что есть сейчас.
pub fn capture(name: &str, s: &Snapshot) -> Profile {
    let default = |flow: Flow| s.devices.iter().find(|d| d.flow == flow && d.is_default).map(|d| d.id.clone());
    let mut apps = BTreeMap::new();
    for d in s.devices.iter().filter(|d| d.flow == Flow::Render) {
        for a in d.apps.iter().filter(|a| real_app(&a.key.exe)) {
            apps.entry(a.key.exe.clone()).or_insert(AppState {
                volume: a.volume,
                muted: a.muted,
                device: a.pinned.clone(),
            });
        }
    }
    Profile {
        name: name.to_string(),
        output: default(Flow::Render),
        input: default(Flow::Capture),
        apps,
        on_connect: None,
    }
}

/// Применить профиль: команды для того, что запущено, и память — для остального.
pub fn apply(p: &Profile, s: &Snapshot, memory: &mut Memory) -> Vec<Cmd> {
    let mut out = Vec::new();
    let present = |id: &str, flow: Flow| s.devices.iter().any(|d| d.id == id && d.flow == flow);
    for (id, flow) in [(&p.output, Flow::Render), (&p.input, Flow::Capture)] {
        if let Some(id) = id
            && present(id, flow)
            && !s.devices.iter().any(|d| d.id == *id && d.is_default)
        {
            out.push(Cmd::SetDefault(id.clone()));
        }
    }
    for (exe, st) in &p.apps {
        memory.insert(exe.clone(), (st.volume, st.muted));
        let running: Vec<&crate::audio::App> = s
            .devices
            .iter()
            .filter(|d| d.flow == Flow::Render)
            .flat_map(|d| d.apps.iter())
            .filter(|a| &a.key.exe == exe)
            .collect();
        if let Some(first) = running.first() {
            let device_ok = st.device.as_ref().is_none_or(|id| present(id, Flow::Render));
            if device_ok && first.pinned != st.device {
                out.push(Cmd::Move(first.key.clone(), st.device.clone()));
            }
        }
        for a in running {
            if (a.volume - st.volume).abs() > 0.005 {
                out.push(Cmd::SetVolume(a.key.clone(), st.volume));
            }
            if a.muted != st.muted {
                out.push(Cmd::SetMuted(a.key.clone(), st.muted));
            }
        }
    }
    out
}

/// Настройки, от которых зависит реакция на события.
pub struct Rules<'a> {
    pub remember: bool,
    /// Громкость новых программ, 0..1.
    pub new_app_volume: Option<f32>,
    pub profiles: &'a [Profile],
}

/// Реакция на события ядра: команды, обновлённая память (`true` — её надо сохранить) и профиль, который
/// пора применить (подключилось устройство из правила).
pub fn react(events: &[Event], s: &Snapshot, rules: &Rules, memory: &mut Memory) -> (Vec<Cmd>, bool, Option<usize>) {
    // Помним и восстанавливаем только программы на устройствах вывода.
    let render = |key: &AppKey| s.devices.iter().any(|d| d.id == key.device && d.flow == Flow::Render);
    let mut cmds = Vec::new();
    let mut dirty = false;
    let mut profile = None;
    for e in events {
        match e {
            Event::AppAdded(a) if render(&a.key) && real_app(&a.key.exe) => match memory.get(&a.key.exe) {
                Some(&(vol, muted)) if rules.remember => {
                    if (a.volume - vol).abs() > 0.005 {
                        cmds.push(Cmd::SetVolume(a.key.clone(), vol));
                    }
                    if a.muted != muted {
                        cmds.push(Cmd::SetMuted(a.key.clone(), muted));
                    }
                }
                None => {
                    if let Some(v) = rules.new_app_volume {
                        cmds.push(Cmd::SetVolume(a.key.clone(), v));
                        if rules.remember {
                            memory.insert(a.key.exe.clone(), (v, a.muted));
                            dirty = true;
                        }
                    } else if rules.remember {
                        memory.insert(a.key.exe.clone(), (a.volume, a.muted));
                        dirty = true;
                    }
                }
                _ => {}
            },
            Event::AppChanged(a) if rules.remember && render(&a.key) && real_app(&a.key.exe) => {
                let now = (model::step(a.volume, 0.0), a.muted);
                if memory.get(&a.key.exe) != Some(&now) {
                    memory.insert(a.key.exe.clone(), now);
                    dirty = true;
                }
            }
            Event::DeviceAdded(d) => {
                if let Some(i) = rules.profiles.iter().position(|p| p.on_connect.as_deref() == Some(d.id.as_str())) {
                    profile = Some(i);
                }
            }
            _ => {}
        }
    }
    (cmds, dirty, profile)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::{App, Device};

    fn app(dev: &str, exe: &str, vol: f32) -> App {
        App {
            key: AppKey { device: dev.into(), exe: exe.into() },
            name: exe.into(),
            path: None,
            volume: vol,
            muted: false,
            system: false,
            pids: vec![1],
            pinned: None,
        }
    }

    fn dev(id: &str, flow: Flow, default: bool, apps: Vec<App>) -> Device {
        Device {
            id: id.into(),
            name: id.into(),
            flow,
            volume: 1.0,
            muted: false,
            is_default: default,
            is_comm: default,
            balance: None,
            apps,
        }
    }

    fn snap() -> Snapshot {
        Snapshot {
            can_route: true,
            devices: vec![
                dev("spk", Flow::Render, true, vec![app("spk", "browser.exe", 0.4)]),
                dev("phones", Flow::Render, false, vec![]),
                dev("mic", Flow::Capture, true, vec![]),
            ],
        }
    }

    #[test]
    fn capture_takes_defaults_and_apps() {
        let p = capture("Работа", &snap());
        assert_eq!(p.output.as_deref(), Some("spk"));
        assert_eq!(p.input.as_deref(), Some("mic"));
        assert_eq!(p.apps["browser.exe"], AppState { volume: 0.4, muted: false, device: None });
    }

    #[test]
    fn apply_sets_device_volume_route_and_memory() {
        let mut p = Profile { name: "Игра".into(), output: Some("phones".into()), ..Default::default() };
        p.apps.insert("browser.exe".into(), AppState { volume: 0.2, muted: false, device: Some("phones".into()) });
        p.apps.insert("player.exe".into(), AppState { volume: 0.7, muted: true, device: None });
        let mut mem = Memory::new();
        let cmds = apply(&p, &snap(), &mut mem);
        let key = AppKey { device: "spk".into(), exe: "browser.exe".into() };
        assert_eq!(
            cmds,
            vec![
                Cmd::SetDefault("phones".into()),
                Cmd::Move(key.clone(), Some("phones".into())),
                Cmd::SetVolume(key, 0.2),
            ]
        );
        // Плеер не запущен — только память.
        assert_eq!(mem["player.exe"], (0.7, true));
    }

    #[test]
    fn apply_skips_missing_devices() {
        let p = Profile { output: Some("gone".into()), ..Default::default() };
        assert!(apply(&p, &snap(), &mut Memory::new()).is_empty());
    }

    #[test]
    fn remembered_volume_returns() {
        let mut mem = Memory::from([("player.exe".to_string(), (0.3, false))]);
        let rules = Rules { remember: true, new_app_volume: None, profiles: &[] };
        let a = app("spk", "player.exe", 1.0);
        let (cmds, dirty, _) = react(&[Event::AppAdded(a.clone())], &snap(), &rules, &mut mem);
        assert_eq!(cmds, vec![Cmd::SetVolume(a.key, 0.3)]);
        assert!(!dirty);
    }

    #[test]
    fn new_app_gets_default_and_is_remembered() {
        let mut mem = Memory::new();
        let rules = Rules { remember: true, new_app_volume: Some(0.5), profiles: &[] };
        let a = app("spk", "new.exe", 1.0);
        let (cmds, dirty, _) = react(&[Event::AppAdded(a.clone())], &snap(), &rules, &mut mem);
        assert_eq!(cmds, vec![Cmd::SetVolume(a.key, 0.5)]);
        assert!(dirty);
        assert_eq!(mem["new.exe"], (0.5, false));
    }

    #[test]
    fn changes_are_remembered_only_when_on() {
        let mut mem = Memory::new();
        let a = app("spk", "player.exe", 0.25);
        let off = Rules { remember: false, new_app_volume: None, profiles: &[] };
        assert!(!react(&[Event::AppChanged(a.clone())], &snap(), &off, &mut mem).1);
        let on = Rules { remember: true, ..off };
        assert!(react(&[Event::AppChanged(a)], &snap(), &on, &mut mem).1);
        assert_eq!(mem["player.exe"], (0.25, false));
    }

    #[test]
    fn connect_rule_picks_profile() {
        let profiles = vec![
            Profile { name: "A".into(), ..Default::default() },
            Profile { name: "B".into(), on_connect: Some("phones".into()), ..Default::default() },
        ];
        let rules = Rules { remember: false, new_app_volume: None, profiles: &profiles };
        let d = dev("phones", Flow::Render, false, vec![]);
        let (_, _, p) = react(&[Event::DeviceAdded(d)], &snap(), &rules, &mut Memory::new());
        assert_eq!(p, Some(1));
    }

    #[test]
    fn recording_apps_are_not_remembered() {
        let mut mem = Memory::new();
        let a = app("mic", "rec.exe", 0.5);
        let rules = Rules { remember: true, new_app_volume: Some(0.9), profiles: &[] };
        let (cmds, dirty, _) = react(&[Event::AppAdded(a)], &snap(), &rules, &mut mem);
        assert!(cmds.is_empty() && !dirty);
    }
}
