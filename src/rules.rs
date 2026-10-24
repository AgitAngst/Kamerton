// Потолок громкости и правила по программам. Здесь только решения — чистые функции: что сделать по снимку и
// событиям ядра. Делает их трей (`tray.rs`) через `Control`, как и профили (`profiles.rs`).

use std::collections::BTreeMap;

use crate::audio::{App, AppKey, Event, Flow, Snapshot};
use crate::profiles::Cmd;

/// Когда срабатывает правило.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum When {
    /// Программа запущена: срабатывает, когда она появляется.
    #[default]
    Running,
    /// Программа запущена, когда подключено устройство (его id): срабатывает и когда появляется программа,
    /// и когда подключается устройство.
    WithDevice(String),
}

impl When {
    /// Текст для `settings.txt`: `running` или `device:<id>`.
    pub fn to_text(&self) -> String {
        match self {
            When::Running => "running".into(),
            When::WithDevice(id) => format!("device:{id}"),
        }
    }

    pub fn from_text(s: &str) -> Option<When> {
        let s = s.trim();
        if s == "running" {
            return Some(When::Running);
        }
        s.strip_prefix("device:").filter(|id| !id.is_empty()).map(|id| When::WithDevice(id.to_string()))
    }
}

/// Что сделать с устройством программы.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum DeviceAct {
    /// Не менять.
    #[default]
    Keep,
    /// «Как в системе».
    System,
    /// Закрепить за устройством (id).
    Device(String),
}

impl DeviceAct {
    /// Текст для `settings.txt`: `keep`, `system` или id устройства.
    pub fn to_text(&self) -> String {
        match self {
            DeviceAct::Keep => "keep".into(),
            DeviceAct::System => "system".into(),
            DeviceAct::Device(id) => id.clone(),
        }
    }

    pub fn from_text(s: &str) -> DeviceAct {
        match s.trim() {
            "" | "keep" => DeviceAct::Keep,
            "system" => DeviceAct::System,
            id => DeviceAct::Device(id.to_string()),
        }
    }
}

/// Правило по программе: когда и что с ней сделать.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AppRule {
    /// Путь exe в нижнем регистре.
    pub app: String,
    pub when: When,
    pub device: DeviceAct,
    /// Громкость, проценты.
    pub volume: Option<u8>,
    pub muted: Option<bool>,
}

fn real_app(exe: &str) -> bool {
    !exe.starts_with("pid:") && exe != "system"
}

fn render(s: &Snapshot, device: &str) -> bool {
    s.devices.iter().any(|d| d.id == device && d.flow == Flow::Render)
}

fn present(s: &Snapshot, id: &str) -> bool {
    s.devices.iter().any(|d| d.id == id)
}

/// Устройство по умолчанию и программы выше потолка — вернуть на потолок. `device` — потолок устройства вывода
/// по умолчанию (проценты), `apps` — потолки программ по пути exe.
pub fn limit_cmds(s: &Snapshot, device: Option<u8>, apps: &BTreeMap<String, u8>) -> Vec<Cmd> {
    let mut out = Vec::new();
    let over = |now: f32, cap: u8| now > cap as f32 / 100.0 + 0.005;
    for d in s.devices.iter().filter(|d| d.flow == Flow::Render) {
        if d.is_default
            && let Some(cap) = device
            && over(d.volume, cap)
        {
            out.push(Cmd::SetDeviceVolume(d.id.clone(), cap as f32 / 100.0));
        }
        for a in &d.apps {
            if let Some(&cap) = apps.get(&a.key.exe)
                && over(a.volume, cap)
            {
                out.push(Cmd::SetVolume(a.key.clone(), cap as f32 / 100.0));
            }
        }
    }
    out
}

/// Действия правила для одной запущенной программы.
fn act(r: &AppRule, a: &App, s: &Snapshot, out: &mut Vec<Cmd>) {
    match &r.device {
        DeviceAct::Keep => {}
        DeviceAct::System => {
            if a.pinned.is_some() {
                out.push(Cmd::Move(a.key.clone(), None));
            }
        }
        DeviceAct::Device(id) => {
            if render(s, id) && a.pinned.as_deref() != Some(id.as_str()) {
                out.push(Cmd::Move(a.key.clone(), Some(id.clone())));
            }
        }
    }
    if let Some(v) = r.volume {
        let want = v as f32 / 100.0;
        if (a.volume - want).abs() > 0.005 {
            out.push(Cmd::SetVolume(a.key.clone(), want));
        }
    }
    if let Some(m) = r.muted
        && a.muted != m
    {
        out.push(Cmd::SetMuted(a.key.clone(), m));
    }
}

/// Правила по событиям ядра: программа появилась (условие верно сейчас) или подключилось устройство из условия
/// (программа уже играет). Не повторяется, пока ничего не появляется заново: пользователь волен менять сам.
pub fn rule_cmds(events: &[Event], s: &Snapshot, rules: &[AppRule]) -> Vec<Cmd> {
    let mut out = Vec::new();
    for e in events {
        match e {
            Event::AppAdded(a) if render(s, &a.key.device) && real_app(&a.key.exe) => {
                for r in rules.iter().filter(|r| r.app == a.key.exe) {
                    let ok = match &r.when {
                        When::Running => true,
                        When::WithDevice(id) => present(s, id),
                    };
                    if ok {
                        act(r, a, s, &mut out);
                    }
                }
            }
            Event::DeviceAdded(d) => {
                for r in rules.iter().filter(|r| matches!(&r.when, When::WithDevice(id) if *id == d.id)) {
                    for dev in s.devices.iter().filter(|d| d.flow == Flow::Render) {
                        for a in dev.apps.iter().filter(|a| a.key.exe == r.app) {
                            act(r, a, s, &mut out);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Ключ программы для списка выбора: путь exe и показываемое имя (по первой встрече), у запущенных на выводе.
pub fn running_apps(s: &Snapshot) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = Vec::new();
    for d in s.devices.iter().filter(|d| d.flow == Flow::Render) {
        for a in d.apps.iter().filter(|a| real_app(&a.key.exe)) {
            if !v.iter().any(|(exe, _)| *exe == a.key.exe) {
                v.push((a.key.exe.clone(), a.name.clone()));
            }
        }
    }
    v
}

#[allow(dead_code)] // нужен тестам и странице «Правила»
pub fn key_of(device: &str, exe: &str) -> AppKey {
    AppKey { device: device.into(), exe: exe.into() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::Device;

    fn app(dev: &str, exe: &str, vol: f32, pinned: Option<&str>) -> App {
        App {
            key: key_of(dev, exe),
            name: exe.into(),
            path: None,
            volume: vol,
            muted: false,
            system: false,
            pids: vec![1],
            pinned: pinned.map(|s| s.to_string()),
        }
    }

    fn dev(id: &str, flow: Flow, default: bool, vol: f32, apps: Vec<App>) -> Device {
        Device {
            id: id.into(),
            name: id.into(),
            flow,
            volume: vol,
            muted: false,
            is_default: default,
            is_comm: default,
            balance: None,
            apps,
        }
    }

    fn snap(apps: Vec<App>) -> Snapshot {
        Snapshot {
            can_route: true,
            devices: vec![
                dev("spk", Flow::Render, true, 0.9, apps),
                dev("phones", Flow::Render, false, 1.0, vec![]),
                dev("mic", Flow::Capture, true, 1.0, vec![]),
            ],
        }
    }

    #[test]
    fn limit_clamps_default_device_and_apps() {
        let s = snap(vec![app("spk", "a.exe", 0.9, None), app("spk", "b.exe", 0.3, None)]);
        let caps = BTreeMap::from([("a.exe".to_string(), 60u8), ("b.exe".to_string(), 50u8)]);
        let cmds = limit_cmds(&s, Some(80), &caps);
        assert_eq!(cmds, vec![Cmd::SetDeviceVolume("spk".into(), 0.8), Cmd::SetVolume(key_of("spk", "a.exe"), 0.6)]);
    }

    #[test]
    fn limit_ignores_values_at_or_below_cap_and_other_devices() {
        let s = snap(vec![app("spk", "a.exe", 0.6, None)]);
        let caps = BTreeMap::from([("a.exe".to_string(), 60u8)]);
        // Устройство «phones» не по умолчанию и на 100 %: общий потолок его не касается.
        assert!(limit_cmds(&s, Some(90), &caps).is_empty());
        assert!(limit_cmds(&s, None, &BTreeMap::new()).is_empty());
    }

    #[test]
    fn running_rule_moves_and_sets_volume() {
        let a = app("spk", "game.exe", 1.0, None);
        let s = snap(vec![a.clone()]);
        let rule = AppRule {
            app: "game.exe".into(),
            device: DeviceAct::Device("phones".into()),
            volume: Some(70),
            muted: Some(true),
            ..Default::default()
        };
        let cmds = rule_cmds(&[Event::AppAdded(a.clone())], &s, &[rule]);
        assert_eq!(
            cmds,
            vec![
                Cmd::Move(a.key.clone(), Some("phones".into())),
                Cmd::SetVolume(a.key.clone(), 0.7),
                Cmd::SetMuted(a.key, true)
            ]
        );
    }

    #[test]
    fn rule_does_nothing_when_already_so() {
        let mut a = app("spk", "game.exe", 0.7, Some("phones"));
        a.muted = true;
        let s = snap(vec![a.clone()]);
        let rule = AppRule {
            app: "game.exe".into(),
            device: DeviceAct::Device("phones".into()),
            volume: Some(70),
            muted: Some(true),
            ..Default::default()
        };
        assert!(rule_cmds(&[Event::AppAdded(a)], &s, &[rule]).is_empty());
    }

    #[test]
    fn system_device_unpins() {
        let a = app("spk", "game.exe", 0.5, Some("phones"));
        let s = snap(vec![a.clone()]);
        let rule = AppRule { app: "game.exe".into(), device: DeviceAct::System, ..Default::default() };
        assert_eq!(rule_cmds(&[Event::AppAdded(a.clone())], &s, &[rule]), vec![Cmd::Move(a.key, None)]);
    }

    #[test]
    fn missing_target_device_is_skipped() {
        let a = app("spk", "game.exe", 1.0, None);
        let s = snap(vec![a.clone()]);
        let rule = AppRule { app: "game.exe".into(), device: DeviceAct::Device("gone".into()), ..Default::default() };
        assert!(rule_cmds(&[Event::AppAdded(a)], &s, &[rule]).is_empty());
    }

    #[test]
    fn device_condition_needs_the_device() {
        let a = app("spk", "game.exe", 1.0, None);
        let s = snap(vec![a.clone()]);
        let rule = |id: &str| AppRule {
            app: "game.exe".into(),
            when: When::WithDevice(id.into()),
            volume: Some(40),
            ..Default::default()
        };
        assert_eq!(rule_cmds(&[Event::AppAdded(a.clone())], &s, &[rule("phones")]).len(), 1);
        assert!(rule_cmds(&[Event::AppAdded(a)], &s, &[rule("gone")]).is_empty());
    }

    #[test]
    fn connecting_the_device_triggers_running_app() {
        let a = app("spk", "game.exe", 1.0, None);
        let s = snap(vec![a.clone()]);
        let rule = AppRule {
            app: "game.exe".into(),
            when: When::WithDevice("phones".into()),
            volume: Some(40),
            ..Default::default()
        };
        let phones = dev("phones", Flow::Render, false, 1.0, vec![]);
        let cmds = rule_cmds(&[Event::DeviceAdded(phones)], &s, std::slice::from_ref(&rule));
        assert_eq!(cmds, vec![Cmd::SetVolume(a.key, 0.4)]);
        // Подключилось другое устройство — ничего.
        let other = dev("mic2", Flow::Capture, false, 1.0, vec![]);
        assert!(rule_cmds(&[Event::DeviceAdded(other)], &s, std::slice::from_ref(&rule)).is_empty());
    }

    #[test]
    fn rules_skip_system_sounds_and_recording() {
        let sys = app("spk", "system", 1.0, None);
        let rec = app("mic", "game.exe", 1.0, None);
        let s = snap(vec![sys.clone()]);
        let rule = AppRule { app: "system".into(), volume: Some(10), ..Default::default() };
        assert!(rule_cmds(&[Event::AppAdded(sys)], &s, &[rule]).is_empty());
        let rule = AppRule { app: "game.exe".into(), volume: Some(10), ..Default::default() };
        assert!(rule_cmds(&[Event::AppAdded(rec)], &s, &[rule]).is_empty());
    }

    #[test]
    fn text_forms_round_trip() {
        for w in [When::Running, When::WithDevice("{0.0.0.00000000}.{id}".into())] {
            assert_eq!(When::from_text(&w.to_text()), Some(w));
        }
        assert_eq!(When::from_text("device:"), None);
        assert_eq!(When::from_text("sometimes"), None);
        for d in [DeviceAct::Keep, DeviceAct::System, DeviceAct::Device("{0.0.0.00000000}.{id}".into())] {
            assert_eq!(DeviceAct::from_text(&d.to_text()), d);
        }
        assert_eq!(DeviceAct::from_text(""), DeviceAct::Keep);
    }

    #[test]
    fn running_apps_are_listed_once() {
        let s = Snapshot {
            can_route: false,
            devices: vec![
                dev("a", Flow::Render, true, 1.0, vec![app("a", "x.exe", 1.0, None), app("a", "system", 1.0, None)]),
                dev("b", Flow::Render, false, 1.0, vec![app("b", "x.exe", 1.0, None)]),
                dev("m", Flow::Capture, true, 1.0, vec![app("m", "rec.exe", 1.0, None)]),
            ],
        };
        assert_eq!(running_apps(&s), vec![("x.exe".to_string(), "x.exe".to_string())]);
    }
}
