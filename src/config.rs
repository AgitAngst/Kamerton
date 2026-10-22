// Настройки: `%APPDATA%\Kamerton\settings.txt`, строки `ключ=значение`. Неизвестные строки сохраняются
// как есть — их могла записать более новая версия. Без зависимостей.

use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Settings {
    /// Окно развёрнуто: все устройства вывода и записи, а не только устройство по умолчанию.
    pub expanded: bool,
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
            match line.split_once('=').map(|(k, v)| (k.trim(), v.trim())) {
                Some(("expanded", v)) => s.expanded = v == "1" || v.eq_ignore_ascii_case("true"),
                _ if line.trim().is_empty() => {}
                _ => s.other.push(line.to_string()),
            }
        }
        s
    }

    pub fn format(&self) -> String {
        let mut out = format!("expanded={}\n", if self.expanded { 1 } else { 0 });
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
    fn missing_or_bad_values_are_defaults() {
        assert!(!Settings::parse("").expanded);
        assert!(!Settings::parse("expanded=maybe").expanded);
        assert!(Settings::parse("expanded = true").expanded);
    }
}
