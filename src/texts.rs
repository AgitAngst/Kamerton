// Строки интерфейса на двух языках. Перевод — в одном `match`: строка без перевода не соберётся.

use std::sync::atomic::{AtomicBool, Ordering};

static ENGLISH: AtomicBool = AtomicBool::new(false);

/// Выбрать язык: `KAMERTON_LANG=ru|en`, иначе язык системы (русский — русский, остальное — английский).
pub fn init() {
    let en = match std::env::var("KAMERTON_LANG").ok().as_deref() {
        Some("en") => true,
        Some("ru") => false,
        _ => anvil_ui::lang::system_language() != anvil_ui::Lang::Ru,
    };
    ENGLISH.store(en, Ordering::Relaxed);
}

pub fn english() -> bool {
    ENGLISH.load(Ordering::Relaxed)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum T {
    NoApps,
    Mute,
    Unmute,
    WindowsMixer,
    SoundSettings,
    Exit,
    NoDevices,
    SystemSounds,
}

impl T {
    #[cfg(test)]
    pub const ALL: [T; 8] =
        [T::NoApps, T::Mute, T::Unmute, T::WindowsMixer, T::SoundSettings, T::Exit, T::NoDevices, T::SystemSounds];

    /// (русский, английский)
    fn pair(self) -> (&'static str, &'static str) {
        match self {
            T::NoApps => ("Нет программ со звуком", "No apps playing audio"),
            T::Mute => ("Без звука", "Mute"),
            T::Unmute => ("Включить звук", "Unmute"),
            T::WindowsMixer => ("Микшер громкости Windows", "Windows volume mixer"),
            T::SoundSettings => ("Параметры звука", "Sound settings"),
            T::Exit => ("Выход", "Exit"),
            T::NoDevices => ("Нет устройств вывода звука", "No audio output devices"),
            T::SystemSounds => ("Системные звуки", "System sounds"),
        }
    }

    pub fn get(self) -> &'static str {
        self.in_lang(english())
    }

    pub fn in_lang(self, en: bool) -> &'static str {
        let (ru, eng) = self.pair();
        if en { eng } else { ru }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_string_has_both_languages() {
        for t in T::ALL {
            let (ru, en) = t.pair();
            assert!(!ru.is_empty() && !en.is_empty(), "{t:?}");
            assert_ne!(ru, en, "{t:?}");
            // С заглавной только первое слово (правило набора).
            assert!(ru.chars().next().unwrap().is_uppercase(), "{t:?}");
        }
    }
}
