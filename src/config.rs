// Настройки: `%APPDATA%\Kamerton\settings.txt`, строки `ключ=значение`. Неизвестные строки сохраняются
// как есть — их могла записать более новая версия. Без зависимостей.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::hotkeys::{Action, AppAction, Combo};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Settings {
    /// Окно развёрнуто: все устройства вывода и записи, а не только устройство по умолчанию.
    pub expanded: bool,
    /// Общие сочетания: `hotkey.<действие>=<сочетание>`.
    pub hotkeys: BTreeMap<Action, Combo>,
    /// Сочетания на программу: `apphotkey.<действие>.<путь exe в нижнем регистре>=<сочетание>`.
    pub app_hotkeys: BTreeMap<(AppAction, String), Combo>,
    /// Строки, которых эта версия не знает.
    other: Vec<String>,
}

fn path() -> Option<PathBuf> {
    let base = std::env::var_os("APPDATA")?;
    Some(PathBuf::from(base).join("Kamerton").join("settings.txt"))
}

impl Settings {
    pub fn parse(text: &str) -> Settings {
        let mut s = Settings::default();
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=').map(|(k, v)| (k.trim(), v.trim())) else {
                if !line.trim().is_empty() {
                    s.other.push(line.to_string());
                }
                continue;
            };
            if k == "expanded" {
                s.expanded = v == "1" || v.eq_ignore_ascii_case("true");
            } else if let Some(a) = k.strip_prefix("hotkey.") {
                // Неверное сочетание или действие — пропустить без падения.
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
        s
    }

    pub fn format(&self) -> String {
        let mut out = format!("expanded={}\n", if self.expanded { 1 } else { 0 });
        for (a, c) in &self.hotkeys {
            out.push_str(&format!("hotkey.{}={}\n", a.key(), c.to_text()));
        }
        for ((a, exe), c) in &self.app_hotkeys {
            out.push_str(&format!("apphotkey.{}.{}={}\n", a.key(), exe, c.to_text()));
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
        assert_eq!(s.format(), "expanded=1\nfuture=yes\n# note\n");
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
    fn missing_or_bad_values_are_defaults() {
        assert!(!Settings::parse("").expanded);
        assert!(!Settings::parse("expanded=maybe").expanded);
        assert!(Settings::parse("expanded = true").expanded);
    }
}
