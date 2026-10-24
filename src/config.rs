// Настройки: `%APPDATA%\Kamerton\settings.txt` (переносная копия — `settings.txt` рядом с exe),
// строки `ключ=значение`. Неизвестные строки сохраняются
// как есть — их могла записать более новая версия. Без зависимостей.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use crate::hotkeys::{Action, AppAction, Combo};
pub use crate::mark_render::IconColor;
use crate::profiles::{AppState, Memory, Profile};

/// Тема окна.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemePref {
    #[default]
    System,
    Light,
    Dark,
}

/// Язык интерфейса.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LangPref {
    #[default]
    System,
    Ru,
    En,
}

/// Вид значка в трее.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrayIcon {
    /// Знак-камертон: ручки — громкость каналов.
    #[default]
    Mark,
    /// Классическая колонка с волнами.
    Speaker,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// `theme=system|light|dark`.
    pub theme: ThemePref,
    /// `lang=system|ru|en`.
    pub lang: LangPref,
    /// Анимация открытия окна: `animate=1`.
    pub animate: bool,
    /// Колесо вне открытого окна меняет громкость: `wheel_outside=1`.
    pub wheel_outside: bool,
    /// Логарифмическая (квадратичная) шкала ползунков: `log_scale=1`.
    pub log_scale: bool,
    /// Плашка громкости при изменении колесом над значком и сочетаниями: `osd=1`.
    pub osd: bool,
    /// Вид значка в трее: `tray_icon=mark|speaker`.
    pub tray_icon: TrayIcon,
    /// Цвет значка в трее: `tray_color=purple|azure`.
    pub tray_color: IconColor,
    /// Скрытые программы: строки `hide_app=<путь exe>`.
    pub hidden_apps: BTreeSet<String>,
    /// Скрытые устройства: строки `hide_device=<id>`.
    pub hidden_devices: BTreeSet<String>,
    /// Место и размер окна микшера, физические пиксели: `mixer_rect=x,y,w,h`.
    pub mixer_rect: Option<[i32; 4]>,
    /// Окно развёрнуто: все устройства вывода и записи, а не только устройство по умолчанию.
    pub expanded: bool,
    /// Общие сочетания: `hotkey.<действие>=<сочетание>`.
    pub hotkeys: BTreeMap<Action, Combo>,
    /// Сочетания на программу: `apphotkey.<действие>.<путь exe в нижнем регистре>=<сочетание>`.
    pub app_hotkeys: BTreeMap<(AppAction, String), Combo>,
    /// Сочетание на устройство — сделать его основным: `devhotkey.<id устройства>=<сочетание>`.
    pub device_hotkeys: BTreeMap<String, Combo>,
    /// Запоминать громкость программ: `remember=1`.
    pub remember: bool,
    /// Громкость новых программ, проценты: `newapp_volume=50`; нет строки — выключено.
    pub new_app_volume: Option<u8>,
    /// Память громкости: строки `appvol=<exe>|<проценты>|<0|1>`.
    pub memory: Memory,
    /// Профили: `profile.<N>.name|output|input|on_connect=…`, `profile.<N>.app=<exe>|<проценты>|<0|1>|<устройство или ->`.
    pub profiles: Vec<Profile>,
    /// Проверять обновления при запуске и раз в сутки: `updates=1`.
    pub check_updates: bool,
    /// Предлагать пред-выпуски: `prerelease=1`.
    pub prerelease: bool,
    /// Пропущенная версия: `skip_version=1.2.3`.
    pub skip_version: Option<String>,
    /// Строки, которых эта версия не знает.
    other: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: ThemePref::System,
            lang: LangPref::System,
            animate: true,
            wheel_outside: false,
            log_scale: false,
            osd: true,
            tray_icon: TrayIcon::Mark,
            tray_color: IconColor::Purple,
            hidden_apps: BTreeSet::new(),
            hidden_devices: BTreeSet::new(),
            mixer_rect: None,
            expanded: false,
            hotkeys: BTreeMap::new(),
            app_hotkeys: BTreeMap::new(),
            device_hotkeys: BTreeMap::new(),
            remember: true,
            new_app_volume: None,
            check_updates: true,
            prerelease: false,
            skip_version: None,
            memory: Memory::new(),
            profiles: Vec::new(),
            other: Vec::new(),
        }
    }
}

fn path() -> Option<PathBuf> {
    Some(data_dir()?.join("settings.txt"))
}

/// Папка exe, если программа переносная (рядом с exe лежит `settings.txt`).
pub fn portable_dir() -> Option<PathBuf> {
    let dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    dir.join("settings.txt").is_file().then_some(dir)
}

/// Где лежат настройки и `crash.log`: рядом с exe у переносной копии, иначе `%APPDATA%\Kamerton`.
pub fn data_dir() -> Option<PathBuf> {
    pick_dir(portable_dir(), std::env::var_os("APPDATA").map(PathBuf::from))
}

fn pick_dir(portable: Option<PathBuf>, appdata: Option<PathBuf>) -> Option<PathBuf> {
    portable.or_else(|| appdata.map(|a| a.join("Kamerton")))
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
            let on = v == "1" || v.eq_ignore_ascii_case("true");
            if k == "expanded" {
                s.expanded = on;
            } else if k == "theme" {
                s.theme = match v {
                    "light" => ThemePref::Light,
                    "dark" => ThemePref::Dark,
                    _ => ThemePref::System,
                };
            } else if k == "lang" {
                s.lang = match v {
                    "ru" => LangPref::Ru,
                    "en" => LangPref::En,
                    _ => LangPref::System,
                };
            } else if k == "animate" {
                s.animate = on;
            } else if k == "wheel_outside" {
                s.wheel_outside = on;
            } else if k == "log_scale" {
                s.log_scale = on;
            } else if k == "osd" {
                s.osd = on;
            } else if k == "tray_icon" {
                s.tray_icon = if v == "speaker" { TrayIcon::Speaker } else { TrayIcon::Mark };
            } else if k == "tray_color" {
                s.tray_color = if v == "azure" { IconColor::Azure } else { IconColor::Purple };
            } else if k == "hide_app" && !v.is_empty() {
                s.hidden_apps.insert(v.to_lowercase());
            } else if k == "hide_device" && !v.is_empty() {
                s.hidden_devices.insert(v.to_string());
            } else if k == "mixer_rect" {
                let n: Vec<i32> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                if let [x, y, w, h] = n[..]
                    && w > 0
                    && h > 0
                {
                    s.mixer_rect = Some([x, y, w, h]);
                }
            } else if k == "remember" {
                s.remember = v == "1" || v.eq_ignore_ascii_case("true");
            } else if k == "updates" {
                s.check_updates = v == "1" || v.eq_ignore_ascii_case("true");
            } else if k == "prerelease" {
                s.prerelease = v == "1" || v.eq_ignore_ascii_case("true");
            } else if k == "skip_version" {
                s.skip_version = opt(v);
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
            } else if let Some(id) = k.strip_prefix("devhotkey.") {
                if let Some(c) = Combo::parse(v)
                    && !id.is_empty()
                {
                    s.device_hotkeys.insert(id.to_string(), c);
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
        let b = |v: bool| if v { 1 } else { 0 };
        let mut out = format!("expanded={}\n", b(self.expanded));
        let theme = match self.theme {
            ThemePref::System => "system",
            ThemePref::Light => "light",
            ThemePref::Dark => "dark",
        };
        let lang = match self.lang {
            LangPref::System => "system",
            LangPref::Ru => "ru",
            LangPref::En => "en",
        };
        out.push_str(&format!("theme={theme}\nlang={lang}\n"));
        out.push_str(&format!(
            "animate={}\nwheel_outside={}\nlog_scale={}\nosd={}\n",
            b(self.animate),
            b(self.wheel_outside),
            b(self.log_scale),
            b(self.osd)
        ));
        let icon = match self.tray_icon {
            TrayIcon::Mark => "mark",
            TrayIcon::Speaker => "speaker",
        };
        let color = match self.tray_color {
            IconColor::Purple => "purple",
            IconColor::Azure => "azure",
        };
        out.push_str(&format!("tray_icon={icon}\ntray_color={color}\n"));
        for exe in &self.hidden_apps {
            out.push_str(&format!("hide_app={exe}\n"));
        }
        for id in &self.hidden_devices {
            out.push_str(&format!("hide_device={id}\n"));
        }
        if let Some([x, y, w, h]) = self.mixer_rect {
            out.push_str(&format!("mixer_rect={x},{y},{w},{h}\n"));
        }
        out.push_str(&format!("remember={}\n", if self.remember { 1 } else { 0 }));
        out.push_str(&format!("updates={}\n", if self.check_updates { 1 } else { 0 }));
        out.push_str(&format!("prerelease={}\n", if self.prerelease { 1 } else { 0 }));
        if let Some(v) = &self.skip_version {
            out.push_str(&format!("skip_version={v}\n"));
        }
        if let Some(v) = self.new_app_volume {
            out.push_str(&format!("newapp_volume={v}\n"));
        }
        for (a, c) in &self.hotkeys {
            out.push_str(&format!("hotkey.{}={}\n", a.key(), c.to_text()));
        }
        for ((a, exe), c) in &self.app_hotkeys {
            out.push_str(&format!("apphotkey.{}.{}={}\n", a.key(), exe, c.to_text()));
        }
        for (id, c) in &self.device_hotkeys {
            out.push_str(&format!("devhotkey.{id}={}\n", c.to_text()));
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
mod dir_tests {
    use super::pick_dir;
    use std::path::PathBuf;

    #[test]
    fn data_dir_choice() {
        let exe = PathBuf::from(r"C:\Tools\Kamerton");
        let app = PathBuf::from(r"C:\Users\demo\AppData\Roaming");
        assert_eq!(pick_dir(Some(exe.clone()), Some(app.clone())), Some(exe));
        assert_eq!(pick_dir(None, Some(app.clone())), Some(app.join("Kamerton")));
        assert_eq!(pick_dir(None, None), None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_keeps_unknown_lines() {
        let s = Settings::parse("expanded=1\nfuture=yes\n\n# note\n");
        assert!(s.expanded);
        assert_eq!(
            s.format(),
            "expanded=1\ntheme=system\nlang=system\nanimate=1\nwheel_outside=0\nlog_scale=0\nosd=1\ntray_icon=mark\ntray_color=purple\nremember=1\nupdates=1\nprerelease=0\nfuture=yes\n# note\n"
        );
        assert_eq!(Settings::parse(&s.format()), s);
    }

    #[test]
    fn device_hotkeys_round_trip() {
        let s = Settings::parse(
            "devhotkey.{0.0.0.00000000}.{demo-1}=Ctrl+Alt+1\ndevhotkey.=Ctrl+Alt+2\nhotkey.prev_output=Ctrl+Alt+Left\n",
        );
        assert_eq!(s.device_hotkeys.len(), 1);
        assert_eq!(s.device_hotkeys["{0.0.0.00000000}.{demo-1}"].to_text(), "Ctrl+Alt+1");
        assert!(s.hotkeys.contains_key(&Action::PrevOutput));
        assert_eq!(Settings::parse(&s.format()), s);
    }

    #[test]
    fn update_settings_round_trip() {
        let s = Settings::parse("updates=0\nprerelease=1\nskip_version=1.2.3\n");
        assert!(!s.check_updates && s.prerelease);
        assert_eq!(s.skip_version.as_deref(), Some("1.2.3"));
        assert_eq!(Settings::parse(&s.format()), s);
        assert!(Settings::parse("").check_updates);
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
    fn view_settings_round_trip() {
        let text = concat!(
            "theme=dark\nlang=en\nanimate=0\nwheel_outside=1\nlog_scale=1\n",
            r"hide_app=C:\Apps\Noisy.exe",
            "\nhide_device={0.0.0.00000000}.{demo}\nmixer_rect=10,20,900,500\nmixer_rect=bad\n",
        );
        let s = Settings::parse(text);
        assert_eq!((s.theme, s.lang), (ThemePref::Dark, LangPref::En));
        assert!(!s.animate && s.wheel_outside && s.log_scale);
        assert!(s.hidden_apps.contains(r"c:\apps\noisy.exe"));
        assert!(s.hidden_devices.contains("{0.0.0.00000000}.{demo}"));
        assert_eq!(s.mixer_rect, Some([10, 20, 900, 500]));
        assert_eq!(Settings::parse(&s.format()), s);
    }

    #[test]
    fn osd_setting_round_trip() {
        assert!(Settings::parse("").osd);
        let s = Settings::parse("osd=0\n");
        assert!(!s.osd);
        assert_eq!(Settings::parse(&s.format()), s);
    }

    #[test]
    fn tray_icon_round_trip() {
        let s = Settings::parse("tray_icon=speaker\ntray_color=azure\n");
        assert_eq!((s.tray_icon, s.tray_color), (TrayIcon::Speaker, IconColor::Azure));
        assert_eq!(Settings::parse(&s.format()), s);
        let s = Settings::parse("tray_icon=glyph\ntray_color=green\n");
        assert_eq!((s.tray_icon, s.tray_color), (TrayIcon::Mark, IconColor::Purple));
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
