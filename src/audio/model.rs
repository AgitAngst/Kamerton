// Модель звука без COM: устройства, программы, события и разница двух снимков.
// Всё здесь — чистые функции, их проверяют тесты внизу файла.

use std::collections::HashMap;
use std::fmt;

/// Направление устройства.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Flow {
    Render,
    Capture,
}

/// Ключ программы: устройство + исполняемый файл (путь в нижнем регистре, `system` или `pid:<n>`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AppKey {
    pub device: String,
    pub exe: String,
}

/// Программа со звуком на одном устройстве — одна или несколько сессий Windows.
#[derive(Debug, Clone, PartialEq)]
pub struct App {
    pub key: AppKey,
    pub name: String,
    pub path: Option<String>,
    /// 0..1
    pub volume: f32,
    pub muted: bool,
    pub system: bool,
    pub pids: Vec<u32>,
    /// Устройство, за которым программа закреплена (`None` — «как в системе»).
    pub pinned: Option<String>,
}

/// Устройство вывода или ввода со своими программами.
#[derive(Debug, Clone, PartialEq)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub flow: Flow,
    /// 0..1
    pub volume: f32,
    pub muted: bool,
    pub is_default: bool,
    /// Устройство связи (роль communications).
    pub is_comm: bool,
    /// Баланс стереоустройства: −1 — только левый, 0 — оба, 1 — только правый; `None` — не стерео.
    pub balance: Option<f32>,
    pub apps: Vec<App>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Snapshot {
    pub devices: Vec<Device>,
    /// Перенос программ на другое устройство доступен в этой сборке Windows.
    pub can_route: bool,
}

/// Изменение, которое ядро отдаёт наружу. У событий устройства список программ пуст —
/// программы приходят своими событиями.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    DeviceAdded(Device),
    DeviceRemoved(String),
    DeviceChanged(Device),
    AppAdded(App),
    AppRemoved(AppKey),
    AppChanged(App),
}

/// Что именно задать громкостью или «без звука».
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Target {
    Device(String),
    App(AppKey),
}

/// Сессия Windows, как её прочитал слой COM.
#[derive(Debug, Clone, PartialEq)]
pub struct RawSession {
    pub pid: u32,
    pub path: Option<String>,
    /// Имя для показа: из сессии или из описания файла; пусто — не нашлось.
    pub name: String,
    pub volume: f32,
    pub muted: bool,
    pub system: bool,
    /// Закрепление процесса за устройством.
    pub pinned: Option<String>,
}

/// Служебные процессы, которые не показываются как программы (EarTrumpet #1095).
pub fn hidden_exe(path: &str) -> bool {
    let file = path.rsplit(['\\', '/']).next().unwrap_or(path);
    file.eq_ignore_ascii_case("audiodg.exe")
}

fn bare(d: &Device) -> Device {
    Device { apps: Vec::new(), ..d.clone() }
}

/// Ключ файла для `AppKey::exe`.
pub fn exe_key(s: &RawSession) -> String {
    if s.system {
        "system".into()
    } else if let Some(p) = &s.path {
        p.to_lowercase()
    } else {
        format!("pid:{}", s.pid)
    }
}

fn file_stem(path: &str) -> &str {
    let file = path.rsplit(['\\', '/']).next().unwrap_or(path);
    file.strip_suffix(".exe").or_else(|| file.strip_suffix(".EXE")).unwrap_or(file)
}

/// Слить сессии одного устройства в программы. Порядок — по первой встрече.
pub fn group(device: &str, sessions: &[RawSession]) -> Vec<App> {
    let mut apps: Vec<App> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for s in sessions {
        let exe = exe_key(s);
        if let Some(&i) = index.get(&exe) {
            let a = &mut apps[i];
            if !a.pids.contains(&s.pid) {
                a.pids.push(s.pid);
            }
            // Громкость программы — у самой громкой сессии; «без звука» — только если молчат все.
            a.volume = a.volume.max(s.volume);
            a.muted &= s.muted;
            if a.name.is_empty() {
                a.name = s.name.clone();
            }
            if a.pinned.is_none() {
                a.pinned = s.pinned.clone();
            }
            continue;
        }
        let name = if s.system {
            "System sounds".to_string()
        } else if !s.name.is_empty() {
            s.name.clone()
        } else if let Some(p) = &s.path {
            file_stem(p).to_string()
        } else {
            format!("pid {}", s.pid)
        };
        index.insert(exe.clone(), apps.len());
        apps.push(App {
            key: AppKey { device: device.to_string(), exe },
            name,
            path: s.path.clone(),
            volume: s.volume,
            muted: s.muted,
            system: s.system,
            pids: vec![s.pid],
            pinned: s.pinned.clone(),
        });
    }
    // Windows отдаёт сессии в разном порядке — без сортировки каждый раз была бы «смена».
    for a in &mut apps {
        a.pids.sort_unstable();
    }
    apps
}

/// События, которые переводят `old` в `new`. Пустой список — ничего не поменялось.
pub fn diff(old: &Snapshot, new: &Snapshot) -> Vec<Event> {
    let mut out = Vec::new();
    let olds: HashMap<&str, &Device> = old.devices.iter().map(|d| (d.id.as_str(), d)).collect();
    let news: HashMap<&str, &Device> = new.devices.iter().map(|d| (d.id.as_str(), d)).collect();

    // Сначала уходы: программы ушедшего устройства, потом само устройство.
    for d in &old.devices {
        if !news.contains_key(d.id.as_str()) {
            for a in &d.apps {
                out.push(Event::AppRemoved(a.key.clone()));
            }
            out.push(Event::DeviceRemoved(d.id.clone()));
        }
    }
    for d in &new.devices {
        match olds.get(d.id.as_str()) {
            None => {
                out.push(Event::DeviceAdded(bare(d)));
                for a in &d.apps {
                    out.push(Event::AppAdded(a.clone()));
                }
            }
            Some(o) => {
                if bare(o) != bare(d) {
                    out.push(Event::DeviceChanged(bare(d)));
                }
                let oa: HashMap<&AppKey, &App> = o.apps.iter().map(|a| (&a.key, a)).collect();
                let na: HashMap<&AppKey, &App> = d.apps.iter().map(|a| (&a.key, a)).collect();
                for a in &o.apps {
                    if !na.contains_key(&a.key) {
                        out.push(Event::AppRemoved(a.key.clone()));
                    }
                }
                for a in &d.apps {
                    match oa.get(&a.key) {
                        None => out.push(Event::AppAdded(a.clone())),
                        Some(prev) if *prev != a => out.push(Event::AppChanged(a.clone())),
                        Some(_) => {}
                    }
                }
            }
        }
    }
    out
}

impl Snapshot {
    /// Применить событие — так окно держит свою копию снимка.
    pub fn apply(&mut self, e: &Event) {
        match e {
            Event::DeviceAdded(d) => self.devices.push(d.clone()),
            Event::DeviceRemoved(id) => self.devices.retain(|d| &d.id != id),
            Event::DeviceChanged(d) => {
                if let Some(x) = self.devices.iter_mut().find(|x| x.id == d.id) {
                    let apps = std::mem::take(&mut x.apps);
                    *x = Device { apps, ..d.clone() };
                }
            }
            Event::AppAdded(a) => {
                if let Some(x) = self.devices.iter_mut().find(|x| x.id == a.key.device) {
                    x.apps.push(a.clone());
                }
            }
            Event::AppRemoved(k) => {
                if let Some(x) = self.devices.iter_mut().find(|x| x.id == k.device) {
                    x.apps.retain(|a| &a.key != k);
                }
            }
            Event::AppChanged(a) => {
                if let Some(x) = self.devices.iter_mut().find(|x| x.id == a.key.device)
                    && let Some(y) = x.apps.iter_mut().find(|y| y.key == a.key)
                {
                    *y = a.clone();
                }
            }
        }
    }

    pub fn device(&self, id: &str) -> Option<&Device> {
        self.devices.iter().find(|d| d.id == id)
    }
}

/// Громкость после шага колеса или клавиши: в пределах 0..1 и по сетке шага, чтобы 2 % × 3 давали
/// ровно 6 %, а не 5.9999.
pub fn step(now: f32, delta: f32) -> f32 {
    let v = ((now + delta).clamp(0.0, 1.0) * 100.0).round() / 100.0;
    v.clamp(0.0, 1.0)
}

/// Баланс по громкости левого и правого канала (0..1 каждый).
pub fn balance_of(left: f32, right: f32) -> f32 {
    let top = left.max(right);
    if top <= 0.0 { 0.0 } else { ((right - left) / top).clamp(-1.0, 1.0) }
}

/// Громкость левого и правого канала для баланса: в сторону — противоположный канал тише.
pub fn channels_for(balance: f32) -> (f32, f32) {
    let b = balance.clamp(-1.0, 1.0);
    (if b > 0.0 { 1.0 - b } else { 1.0 }, if b < 0.0 { 1.0 + b } else { 1.0 })
}

/// Логарифмическая (квадратичная) шкала: положение ползунка ↔ громкость. Тихое — точнее.
pub fn to_pos(volume: f32, log: bool) -> f32 {
    if log { volume.clamp(0.0, 1.0).sqrt() } else { volume.clamp(0.0, 1.0) }
}

pub fn from_pos(pos: f32, log: bool) -> f32 {
    let p = pos.clamp(0.0, 1.0);
    if log { p * p } else { p }
}

/// Снимок без скрытого пользователем.
pub fn visible(
    s: &Snapshot,
    hidden_apps: &std::collections::BTreeSet<String>,
    hidden_devices: &std::collections::BTreeSet<String>,
) -> Snapshot {
    let devices = s
        .devices
        .iter()
        .filter(|d| !hidden_devices.contains(&d.id))
        .map(|d| Device {
            apps: d.apps.iter().filter(|a| !hidden_apps.contains(&a.key.exe)).cloned().collect(),
            ..d.clone()
        })
        .collect();
    Snapshot { devices, ..s.clone() }
}

pub fn percent(v: f32) -> u32 {
    (v.clamp(0.0, 1.0) * 100.0).round() as u32
}

impl fmt::Display for Flow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Flow::Render => "out",
            Flow::Capture => "in",
        })
    }
}

impl fmt::Display for Device {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {} {}%", self.flow, self.name, percent(self.volume))?;
        if self.muted {
            f.write_str(" muted")?;
        }
        if self.is_default {
            f.write_str(" default")?;
        }
        Ok(())
    }
}

impl fmt::Display for App {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}%", self.name, percent(self.volume))?;
        if self.muted {
            f.write_str(" muted")?;
        }
        let pids: Vec<String> = self.pids.iter().map(|p| p.to_string()).collect();
        write!(f, " (pid {})", pids.join(","))
    }
}

impl fmt::Display for Event {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Event::DeviceAdded(d) => write!(f, "device added   {d}"),
            Event::DeviceRemoved(id) => write!(f, "device removed {id}"),
            Event::DeviceChanged(d) => write!(f, "device changed {d}"),
            Event::AppAdded(a) => write!(f, "app added      {a}"),
            Event::AppRemoved(k) => write!(f, "app removed    {}", k.exe),
            Event::AppChanged(a) => write!(f, "app changed    {a}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(pid: u32, path: Option<&str>, vol: f32) -> RawSession {
        RawSession {
            pid,
            path: path.map(Into::into),
            name: String::new(),
            volume: vol,
            muted: false,
            system: false,
            pinned: None,
        }
    }

    fn device(id: &str, apps: Vec<App>) -> Device {
        Device {
            id: id.into(),
            name: id.into(),
            flow: Flow::Render,
            volume: 0.5,
            muted: false,
            is_default: false,
            is_comm: false,
            balance: None,
            apps,
        }
    }

    #[test]
    fn group_merges_processes_of_one_exe() {
        let s = [
            session(1, Some(r"C:\Apps\Chrome.exe"), 0.3),
            session(2, Some(r"c:\apps\chrome.exe"), 0.7),
            session(3, Some(r"C:\Games\game.exe"), 1.0),
        ];
        let apps = group("dev", &s);
        assert_eq!(apps.len(), 2);
        assert_eq!(apps[0].name, "Chrome");
        assert_eq!(apps[0].pids, vec![1, 2]);
        assert_eq!(apps[0].volume, 0.7);
        assert_eq!(apps[1].key.exe, r"c:\games\game.exe");
        let reversed = [s[1].clone(), s[0].clone(), s[2].clone()];
        assert_eq!(group("dev", &reversed)[0].pids, vec![1, 2]);
    }

    #[test]
    fn group_names_system_and_unknown() {
        let mut sys = session(0, None, 1.0);
        sys.system = true;
        let mut named = session(5, None, 1.0);
        named.name = "Player".into();
        let apps = group("dev", &[sys, session(9, None, 1.0), named]);
        assert_eq!(apps[0].name, "System sounds");
        assert!(apps[0].system);
        assert_eq!(apps[1].name, "pid 9");
        assert_eq!(apps[1].key.exe, "pid:9");
        assert_eq!(apps[2].name, "Player");
    }

    #[test]
    fn group_mute_only_when_all_muted() {
        let mut a = session(1, Some("x.exe"), 1.0);
        a.muted = true;
        let b = session(2, Some("x.exe"), 1.0);
        assert!(!group("d", &[a.clone(), b])[0].muted);
        let mut c = a.clone();
        c.pid = 3;
        assert!(group("d", &[a, c])[0].muted);
    }

    #[test]
    fn diff_same_is_empty() {
        let s = Snapshot { can_route: true, devices: vec![device("a", group("a", &[session(1, Some("x.exe"), 1.0)]))] };
        assert!(diff(&s, &s.clone()).is_empty());
    }

    #[test]
    fn diff_reports_each_change_once() {
        let old = Snapshot {
            can_route: true,
            devices: vec![
                device("a", group("a", &[session(1, Some("x.exe"), 1.0), session(2, Some("y.exe"), 1.0)])),
                device("gone", group("gone", &[session(3, Some("z.exe"), 1.0)])),
            ],
        };
        let mut a = device("a", group("a", &[session(1, Some("x.exe"), 0.4), session(4, Some("w.exe"), 1.0)]));
        a.is_default = true;
        let new = Snapshot { can_route: true, devices: vec![a, device("b", Vec::new())] };
        let ev = diff(&old, &new);
        let text: Vec<String> = ev.iter().map(|e| e.to_string()).collect();
        assert_eq!(
            text,
            vec![
                "app removed    z.exe",
                "device removed gone",
                "device changed [out] a 50% default",
                "app removed    y.exe",
                "app changed    x 40% (pid 1)",
                "app added      w 100% (pid 4)",
                "device added   [out] b 50%",
            ]
        );
    }

    #[test]
    fn apply_diff_reaches_new_snapshot() {
        let old = Snapshot {
            can_route: true,
            devices: vec![
                device("a", group("a", &[session(1, Some("x.exe"), 1.0)])),
                device("c", group("c", &[session(7, Some("q.exe"), 1.0)])),
            ],
        };
        let mut a = device("a", group("a", &[session(1, Some("x.exe"), 0.2), session(2, Some("y.exe"), 1.0)]));
        a.muted = true;
        let new = Snapshot { can_route: true, devices: vec![a, device("b", group("b", &[session(3, None, 1.0)]))] };
        let mut s = old.clone();
        for e in diff(&old, &new) {
            s.apply(&e);
        }
        assert_eq!(s, new);
    }

    #[test]
    fn step_keeps_grid_and_limits() {
        let mut v = 0.5;
        for _ in 0..3 {
            v = step(v, -0.02);
        }
        assert_eq!(v, 0.44);
        assert_eq!(step(0.99, 0.02), 1.0);
        assert_eq!(step(0.01, -0.02), 0.0);
        assert_eq!(step(0.333, 0.02), 0.35);
    }

    #[test]
    fn audiodg_is_hidden() {
        assert!(hidden_exe(r"C:\Windows\System32\AUDIODG.EXE"));
        assert!(!hidden_exe(r"C:\Apps\player.exe"));
    }

    #[test]
    fn pinned_device_kept_in_group() {
        let mut a = session(1, Some("x.exe"), 1.0);
        let mut b = session(2, Some("x.exe"), 1.0);
        b.pinned = Some("phones".into());
        assert_eq!(group("d", &[a.clone(), b.clone()])[0].pinned.as_deref(), Some("phones"));
        a.pinned = None;
        b.pinned = None;
        assert_eq!(group("d", &[a, b])[0].pinned, None);
    }

    #[test]
    fn balance_math() {
        assert_eq!(channels_for(-0.5), (1.0, 0.5));
        assert_eq!(channels_for(0.25), (0.75, 1.0));
        assert_eq!(channels_for(0.0), (1.0, 1.0));
        assert_eq!(balance_of(1.0, 0.5), -0.5);
        assert_eq!(balance_of(0.4, 0.4), 0.0);
        assert_eq!(balance_of(0.0, 0.0), 0.0);
        for b in [-1.0, -0.3, 0.0, 0.6, 1.0] {
            let (l, r) = channels_for(b);
            assert!((balance_of(l, r) - b).abs() < 1e-6);
        }
    }

    #[test]
    fn log_scale_maps_middle_to_quarter() {
        assert_eq!(from_pos(0.5, true), 0.25);
        assert_eq!(to_pos(0.25, true), 0.5);
        assert_eq!(from_pos(0.5, false), 0.5);
        assert_eq!(to_pos(from_pos(0.37, true), true), 0.37);
    }

    #[test]
    fn hidden_things_are_filtered() {
        let s = Snapshot {
            can_route: true,
            devices: vec![
                device("a", group("a", &[session(1, Some("x.exe"), 1.0), session(2, Some("y.exe"), 1.0)])),
                device("b", Vec::new()),
            ],
        };
        let apps = std::collections::BTreeSet::from(["y.exe".to_string()]);
        let devs = std::collections::BTreeSet::from(["b".to_string()]);
        let v = visible(&s, &apps, &devs);
        assert_eq!(v.devices.len(), 1);
        assert_eq!(v.devices[0].apps.len(), 1);
        assert_eq!(v.devices[0].apps[0].key.exe, "x.exe");
    }

    #[test]
    fn percent_rounds_and_clamps() {
        assert_eq!(percent(0.444), 44);
        assert_eq!(percent(1.7), 100);
        assert_eq!(percent(-1.0), 0);
    }
}
