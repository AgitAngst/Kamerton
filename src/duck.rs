// Автоприглушение: пока звучит хоть одна «главная» программа (звонок, игра), остальные тише; когда главные
// замолчали — громкость возвращается. Машина состояний без часов и без Windows: время — аргумент (секунды),
// пики приходят снаружи. Решения возвращают командами (`profiles::Cmd`), делает их трей.

use std::collections::{BTreeSet, HashMap, HashSet};

use crate::audio::{AppKey, Flow, Snapshot, model};
use crate::profiles::Cmd;

/// Пик громкее этого — программа звучит.
pub const LOUD: f32 = 0.02;
/// Главные молчат столько секунд подряд — громкость возвращается.
pub const HOLD: f64 = 1.5;
/// Громкость, которую поставили мы, и нынешняя отличаются меньше, чем на это, — пользователь её не менял.
const SAME: f32 = 0.02;

/// Настройка приглушения.
pub struct Config<'a> {
    /// Доля от прежней громкости, 0..1.
    pub level: f32,
    /// Главные программы: путь exe в нижнем регистре.
    pub priority: &'a BTreeSet<String>,
}

/// Приглушённые программы: прежняя громкость и та, что поставили мы.
#[derive(Debug, Default)]
pub struct Ducker {
    ducked: HashMap<AppKey, (f32, f32)>,
    /// Программы, чью громкость пользователь поменял сам во время приглушения: до конца этого приглушения не трогаем.
    left_alone: HashSet<AppKey>,
    /// С какого момента главные молчат (`None` — звучат или приглушения нет).
    quiet_since: Option<f64>,
}

fn find<'a>(s: &'a Snapshot, key: &AppKey) -> Option<&'a crate::audio::App> {
    s.devices.iter().filter(|d| d.flow == Flow::Render).flat_map(|d| d.apps.iter()).find(|a| &a.key == key)
}

impl Ducker {
    /// Приглушена ли программа сейчас (её события не должны попасть в «память громкости»).
    pub fn is_ducked(&self, key: &AppKey) -> bool {
        self.ducked.contains_key(key)
    }

    #[cfg(test)]
    pub fn active(&self) -> bool {
        !self.ducked.is_empty()
    }

    /// Один шаг (раз в 250 мс): что приглушить, что вернуть.
    pub fn tick(&mut self, now: f64, s: &Snapshot, peaks: &HashMap<AppKey, f32>, cfg: &Config) -> Vec<Cmd> {
        let mut out = Vec::new();
        let apps = || s.devices.iter().filter(|d| d.flow == Flow::Render).flat_map(|d| d.apps.iter());
        // Ушедшие программы — забыть; те, чью громкость пользователь поменял сам, — забыть, не возвращая, и до
        // конца приглушения не трогать.
        let mut touched = Vec::new();
        self.ducked.retain(|key, (_, set)| match find(s, key) {
            Some(a) if (a.volume - *set).abs() <= SAME => true,
            Some(_) => {
                touched.push(key.clone());
                false
            }
            None => false,
        });
        self.left_alone.extend(touched);
        self.left_alone.retain(|key| find(s, key).is_some());
        let loud = apps().any(|a| cfg.priority.contains(&a.key.exe) && peaks.get(&a.key).is_some_and(|p| *p > LOUD));
        if loud {
            self.quiet_since = None;
            for a in apps() {
                if cfg.priority.contains(&a.key.exe)
                    || a.system
                    || a.muted
                    || self.ducked.contains_key(&a.key)
                    || self.left_alone.contains(&a.key)
                {
                    continue;
                }
                let target = model::step(a.volume * cfg.level, 0.0);
                self.ducked.insert(a.key.clone(), (a.volume, target));
                if (a.volume - target).abs() > 0.005 {
                    out.push(Cmd::SetVolume(a.key.clone(), target));
                }
            }
        } else if !self.ducked.is_empty() {
            let since = *self.quiet_since.get_or_insert(now);
            if now - since >= HOLD {
                out.extend(self.release_all(s));
            }
        }
        out
    }

    /// Вернуть всё приглушённое: приглушение выключили, главные замолчали, программа выходит.
    pub fn release_all(&mut self, s: &Snapshot) -> Vec<Cmd> {
        self.quiet_since = None;
        self.left_alone.clear();
        let mut out = Vec::new();
        for (key, (base, set)) in self.ducked.drain() {
            if let Some(a) = find(s, &key)
                && (a.volume - set).abs() <= SAME
                && (a.volume - base).abs() > 0.005
            {
                out.push(Cmd::SetVolume(key, base));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::{App, Device};

    fn app(exe: &str, vol: f32) -> App {
        App {
            key: AppKey { device: "spk".into(), exe: exe.into() },
            name: exe.into(),
            path: None,
            volume: vol,
            muted: false,
            system: exe == "system",
            pids: vec![1],
            pinned: None,
        }
    }

    fn snap(apps: Vec<App>) -> Snapshot {
        Snapshot {
            can_route: false,
            devices: vec![Device {
                id: "spk".into(),
                name: "spk".into(),
                flow: Flow::Render,
                volume: 1.0,
                muted: false,
                is_default: true,
                is_comm: true,
                balance: None,
                apps,
            }],
        }
    }

    fn key(exe: &str) -> AppKey {
        AppKey { device: "spk".into(), exe: exe.into() }
    }

    fn prio() -> BTreeSet<String> {
        BTreeSet::from(["call.exe".to_string()])
    }

    fn peaks(call: f32) -> HashMap<AppKey, f32> {
        HashMap::from([(key("call.exe"), call)])
    }

    #[test]
    fn others_go_quiet_while_the_call_sounds() {
        let p = prio();
        let cfg = Config { level: 0.3, priority: &p };
        let mut d = Ducker::default();
        let s = snap(vec![app("call.exe", 1.0), app("music.exe", 0.8), app("system", 1.0)]);
        let cmds = d.tick(0.0, &s, &peaks(0.4), &cfg);
        // Музыка — 80 % × 30 % = 24 %; звонок и «Системные звуки» не трогаются.
        assert_eq!(cmds, vec![Cmd::SetVolume(key("music.exe"), 0.24)]);
        assert!(d.is_ducked(&key("music.exe")));
        assert!(!d.is_ducked(&key("call.exe")) && !d.is_ducked(&key("system")));
    }

    #[test]
    fn volume_returns_after_the_pause() {
        let p = prio();
        let cfg = Config { level: 0.3, priority: &p };
        let mut d = Ducker::default();
        d.tick(0.0, &snap(vec![app("call.exe", 1.0), app("music.exe", 0.8)]), &peaks(0.4), &cfg);
        let quiet = snap(vec![app("call.exe", 1.0), app("music.exe", 0.24)]);
        assert!(d.tick(1.0, &quiet, &peaks(0.0), &cfg).is_empty());
        assert!(d.tick(2.0, &quiet, &peaks(0.0), &cfg).is_empty(), "до паузы 1,5 с от первого тихого шага");
        assert_eq!(d.tick(2.6, &quiet, &peaks(0.0), &cfg), vec![Cmd::SetVolume(key("music.exe"), 0.8)]);
        assert!(!d.active());
    }

    #[test]
    fn call_sounding_again_resets_the_pause() {
        let p = prio();
        let cfg = Config { level: 0.5, priority: &p };
        let mut d = Ducker::default();
        let s = snap(vec![app("call.exe", 1.0), app("music.exe", 0.8)]);
        d.tick(0.0, &s, &peaks(0.4), &cfg);
        let quiet = snap(vec![app("call.exe", 1.0), app("music.exe", 0.4)]);
        d.tick(1.0, &quiet, &peaks(0.0), &cfg);
        d.tick(1.5, &quiet, &peaks(0.4), &cfg);
        assert!(d.tick(2.4, &quiet, &peaks(0.0), &cfg).is_empty());
        assert!(d.active());
    }

    #[test]
    fn user_change_is_left_alone() {
        let p = prio();
        let cfg = Config { level: 0.3, priority: &p };
        let mut d = Ducker::default();
        d.tick(0.0, &snap(vec![app("call.exe", 1.0), app("music.exe", 0.8)]), &peaks(0.4), &cfg);
        // Пользователь поднял музыку до 50 % во время звонка.
        let user = snap(vec![app("call.exe", 1.0), app("music.exe", 0.5)]);
        assert!(d.tick(0.5, &user, &peaks(0.4), &cfg).is_empty());
        assert!(!d.is_ducked(&key("music.exe")), "пользователь поменял сам — программа забыта");
        assert!(d.tick(5.0, &user, &peaks(0.0), &cfg).is_empty(), "возвращать нечего");
    }

    #[test]
    fn late_app_is_ducked_and_gone_app_forgotten() {
        let p = prio();
        let cfg = Config { level: 0.5, priority: &p };
        let mut d = Ducker::default();
        d.tick(0.0, &snap(vec![app("call.exe", 1.0), app("a.exe", 1.0)]), &peaks(0.4), &cfg);
        let later = snap(vec![app("call.exe", 1.0), app("a.exe", 0.5), app("b.exe", 0.6)]);
        assert_eq!(d.tick(0.25, &later, &peaks(0.4), &cfg), vec![Cmd::SetVolume(key("b.exe"), 0.3)]);
        let gone = snap(vec![app("call.exe", 1.0), app("b.exe", 0.3)]);
        d.tick(0.5, &gone, &peaks(0.4), &cfg);
        assert!(!d.is_ducked(&key("a.exe")) && d.is_ducked(&key("b.exe")));
    }

    #[test]
    fn muted_apps_and_quiet_call_do_nothing() {
        let p = prio();
        let cfg = Config { level: 0.3, priority: &p };
        let mut d = Ducker::default();
        let mut m = app("music.exe", 0.8);
        m.muted = true;
        let s = snap(vec![app("call.exe", 1.0), m]);
        assert!(d.tick(0.0, &s, &peaks(0.4), &cfg).is_empty());
        let s = snap(vec![app("call.exe", 1.0), app("music.exe", 0.8)]);
        assert!(d.tick(1.0, &s, &peaks(0.01), &cfg).is_empty(), "пик ниже порога — звонок молчит");
        assert!(!d.active());
    }

    #[test]
    fn release_all_restores_and_forgets() {
        let p = prio();
        let cfg = Config { level: 0.3, priority: &p };
        let mut d = Ducker::default();
        d.tick(0.0, &snap(vec![app("call.exe", 1.0), app("music.exe", 0.8)]), &peaks(0.4), &cfg);
        let quiet = snap(vec![app("call.exe", 1.0), app("music.exe", 0.24)]);
        assert_eq!(d.release_all(&quiet), vec![Cmd::SetVolume(key("music.exe"), 0.8)]);
        assert!(!d.active());
        assert!(d.release_all(&quiet).is_empty());
    }
}
