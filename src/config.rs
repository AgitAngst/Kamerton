// Настройки: `%APPDATA%\Kamerton\settings.txt`, строки `ключ=значение`. Неизвестные строки сохраняются
// как есть — их могла записать более новая версия. Без зависимостей.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::hotkeys::{Action, AppAction, Combo};
use crate::profiles::{AppState, Memory, Profile};

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// Окно развёрнуто: все устройства вывода и записи, а не только устройство по умолчанию.
    pub expanded: bool,
    /// Общие сочетания: `hotkey.<действие>=<сочетание>`.
    pub hotkeys: BTreeMap<Action, Combo>,
    /// Сочетания на программу: `apphotkey.<действие>.<путь exe в нижнем регистре>=<сочетание>`.
    pub app_hotkeys: BTreeMap<(AppAction, String), Combo>,
    /// Запоминать громкость программ: `remember=1`.
    pub remember: bool,
    /// Громкость новых программ, проценты: `newapp_volume=50`; нет строки — выключено.
    pub new_app_volume: Option<u8>,
    /// Память громкости: строки `appvol=<exe>|<проценты>|<0|1>`.
    pub memory: Memory,
    /// Профили: `profile.<N>.name|output|input|on_connect=…`, `profile.<N>.app=<exe>|<проценты>|<0|1>|<устройство или ->`.
    pub profiles: Vec<Profile>,
    /// Строки, которых эта версия не знает.
    other: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            expanded: false,
            hotkeys: BTreeMap::new(),
            app_hotkeys: BTreeMap::new(),
            remember: true,
            new_app_volume: None,
            memory: Memory::new(),
            profiles: Vec::new(),
            other: Vec::new(),
        }
    }
}

fn path() -> Option<PathBuf> {
    let base = std::env::var_os("APPDATA")?;
    Some(PathBuf::from(base).join("Kamerton").join("settings.txt"))
}

fn percent(v: &str) -> Option<f32> {
    v.trim().parse::<u8>().ok().filter(|p| *p <= 100).map(|p| p as f32 / 100.0)
}

fn pct(v: f32) -> u32 {
    (v.clamp(0.0, 1.0) * 100.0).round() as u32
}

fn opt(v: &str) -> Option<String> {
    let v = v.trim();
    (!v.is_empty() && v != "-").then(|| v.to_string())
}

impl Settings {
    pub fn parse(text: &str) -> Settings {
        let mut s = Settings::default();
        let mut profiles: BTreeMap<usize, Profile> = BTreeMap::new();
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=').map(|(k, v)| (k.trim(), v.trim())) else {
                if !line.trim().is_empty() {
                    s.other.push(line.to_string());
                }
                continue;
            };
            if k == "expanded" {
                s.expanded = v == "1" || v.eq_ignore_ascii_case("true");
            } else if k == "remember" {
                s.remember = v == "1" || v.eq_ignore_ascii_case("true");
            } else if k == "newapp_volume" {
                s.new_app_volume = v.parse::<u8>().ok().filter(|p| *p <= 100);
            } else if k == "appvol" {
                let f: Vec<&str> = v.split('|').collect();
                if let [exe, vol, muted] = f[..]
                    && let Some(vol) = percent(vol)
                    && !exe.is_empty()
                {
                    s.memory.insert(exe.to_lowercase(), (vol, muted == "1"));
                }
            } else if let Some(rest) = k.strip_prefix("profile.") {
                // Неверная строка профиля — пропустить без падения.
                let Some((n, field)) = rest.split_once('.') else { continue };
                let Ok(n) = n.parse::<usize>() else { continue };
                let p = profiles.entry(n).or_default();
                match field {
                    "name" => p.name = v.to_string(),
                    "output" => p.output = opt(v),
                    "input" => p.input = opt(v),
                    "on_connect" => p.on_connect = opt(v),
                    "app" => {
                        let f: Vec<&str> = v.split('|').collect();
                        if let [exe, vol, muted, dev] = f[..]
                            && let Some(volume) = percent(vol)
                            && !exe.is_empty()
                        {
                            p.apps
                                .insert(exe.to_lowercase(), AppState { volume, muted: muted == "1", device: opt(dev) });
                        }
                    }
                    _ => {}
                }
            } else if let Some(a) = k.strip_prefix("hotkey.") {
                if let (Some(a), Some(c)) = (Action::from_key(a), Combo::parse(v)) {
                    s.hotkeys.insert(a, c);
                }
            } else if let Some(rest) = k.strip_prefix("apphotkey.") {
                if let Some((a, exe)) = rest.split_once('.')
                    && let (Some(a), Some(c)) = (AppAction::from_key(a), Combo::parse(v))
                    && !exe.is_empty()
                {
                    s.app_hotkeys.insert((a, exe.to_lowercase()), c);
                }
            } else {
                s.other.push(line.to_string());
            }
        }
        s.profiles = profiles.into_values().filter(|p| !p.name.is_empty()).collect();
        s
    }

    pub fn format(&self) -> String {
        let mut out = format!("expanded={}\n", if self.expanded { 1 } else { 0 });
        out.push_str(&format!("remember={}\n", if self.remember { 1 } else { 0 }));
        if let Some(v) = self.new_app_volume {
            out.push_str(&format!("newapp_volume={v}\n"));
        }
        for (a, c) in &self.hotkeys {
            out.push_str(&format!("hotkey.{}={}\n", a.key(), c.to_text()));
        }
        for ((a, exe), c) in &self.app_hotkeys {
            out.push_str(&format!("apphotkey.{}.{}={}\n", a.key(), exe, c.to_text()));
        }
        for (exe, (vol, muted)) in &self.memory {
            out.push_str(&format!("appvol={}|{}|{}\n", exe, pct(*vol), if *muted { 1 } else { 0 }));
        }
        for (n, p) in self.profiles.iter().enumerate() {
            out.push_str(&format!("profile.{n}.name={}\n", p.name));
            for (field, v) in [("output", &p.output), ("input", &p.input), ("on_connect", &p.on_connect)] {
                if let Some(v) = v {
                    out.push_str(&format!("profile.{n}.{field}={v}\n"));
                }
            }
            for (exe, st) in &p.apps {
                let dev = st.device.as_deref().unwrap_or("-");
                out.push_str(&format!(
                    "profile.{n}.app={}|{}|{}|{}\n",
                    exe,
                    pct(st.volume),
                    if st.muted { 1 } else { 0 },
                    dev
                ));
            }
        }
        for l in &self.other {
            out.push_str(l);
            out.push('\n');
        }
        out
    }

    /// Прочитать с диска. `KAMERTON_EXPANDED=0|1` — вид окна для проверки, файл при этом не меняется.
    pub fn load() -> Settings {
        let mut s =
            path().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| Settings::parse(&t)).unwrap_or_default();
        if let Ok(v) = std::env::var("KAMERTON_EXPANDED") {
            s.expanded = v == "1";
        }
        s
    }

    pub fn save(&self) {
        let Some(p) = path() else { return };
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(&p, self.format()) {
            eprintln!("settings: cannot write {}: {e}", p.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_keeps_unknown_lines() {
        let s = Settings::parse("expanded=1\nfuture=yes\n\n# note\n");
        assert!(s.expanded);
        assert_eq!(s.format(), "expanded=1\nremember=1\nfuture=yes\n# note\n");
        assert_eq!(Settings::parse(&s.format()), s);
    }

    #[test]
    fn hotkeys_round_trip_and_bad_lines_skipped() {
        let text = concat!(
            "hotkey.louder=Ctrl+Alt+Up\n",
            "hotkey.quieter=Ctrl+Чепуха\n",
            "hotkey.nope=Ctrl+A\n",
            r"apphotkey.mute.C:\Apps\player.exe=Ctrl+Shift+F9",
            "\n",
            "apphotkey.louder.=Ctrl+B\n",
        );
        let s = Settings::parse(text);
        assert_eq!(s.hotkeys.len(), 1);
        assert_eq!(s.hotkeys[&Action::Louder].to_text(), "Ctrl+Alt+Up");
        assert_eq!(s.app_hotkeys.len(), 1);
        assert_eq!(s.app_hotkeys[&(AppAction::Mute, r"c:\apps\player.exe".to_string())].to_text(), "Ctrl+Shift+F9");
        assert_eq!(Settings::parse(&s.format()), s);
    }

    #[test]
    fn profiles_and_memory_round_trip() {
        let text = concat!(
            "remember=0\n",
            "newapp_volume=40\n",
            r"appvol=C:\Apps\Player.exe|30|1",
            "\n",
            "appvol=bad|x|0\n",
            "profile.1.name=Игра\n",
            "profile.1.output={0.0.0.00000000}.{demo-phones}\n",
            "profile.1.on_connect={0.0.0.00000000}.{demo-phones}\n",
            r"profile.1.app=c:\apps\browser.exe|20|0|{0.0.0.00000000}.{demo-phones}",
            "\n",
            r"profile.1.app=c:\apps\player.exe|70|1|-",
            "\n",
            "profile.3.output=nameless-is-dropped\n",
        );
        let s = Settings::parse(text);
        assert!(!s.remember);
        assert_eq!(s.new_app_volume, Some(40));
        assert_eq!(s.memory.len(), 1);
        assert_eq!(s.memory[r"c:\apps\player.exe"], (0.3, true));
        assert_eq!(s.profiles.len(), 1);
        let p = &s.profiles[0];
        assert_eq!(p.name, "Игра");
        assert_eq!(p.apps[r"c:\apps\browser.exe"].device.as_deref(), Some("{0.0.0.00000000}.{demo-phones}"));
        assert_eq!(p.apps[r"c:\apps\player.exe"], AppState { volume: 0.7, muted: true, device: None });
        assert_eq!(Settings::parse(&s.format()), s);
    }

    #[test]
    fn missing_or_bad_values_are_defaults() {
        let s = Settings::parse("");
        assert!(!s.expanded && s.remember && s.new_app_volume.is_none());
        assert!(!Settings::parse("expanded=maybe").expanded);
        assert!(Settings::parse("expanded = true").expanded);
        assert_eq!(Settings::parse("newapp_volume=300").new_app_volume, None);
    }
}
