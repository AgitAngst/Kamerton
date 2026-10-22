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
    WindowsPanels,
    PlaybackDevices,
    RecordingDevices,
    Sounds,
    MakeDefault,
    MakeComm,
    Device,
    AsSystem,
    RouteUnavailable,
    Recording,
    AllDevices,
    DefaultOnly,
    Hotkeys,
    HotkeysMenu,
    Back,
    General,
    ActOpen,
    ActLouder,
    ActQuieter,
    ActNext,
    Change,
    PressKeys,
    Free,
    Taken,
    NotSet,
    CaptureHint,
}

impl T {
    #[cfg(test)]
    pub const ALL: [T; 34] = [
        T::NoApps,
        T::Mute,
        T::Unmute,
        T::WindowsMixer,
        T::SoundSettings,
        T::Exit,
        T::NoDevices,
        T::SystemSounds,
        T::WindowsPanels,
        T::PlaybackDevices,
        T::RecordingDevices,
        T::Sounds,
        T::MakeDefault,
        T::MakeComm,
        T::Device,
        T::AsSystem,
        T::RouteUnavailable,
        T::Recording,
        T::AllDevices,
        T::DefaultOnly,
        T::Hotkeys,
        T::HotkeysMenu,
        T::Back,
        T::General,
        T::ActOpen,
        T::ActLouder,
        T::ActQuieter,
        T::ActNext,
        T::Change,
        T::PressKeys,
        T::Free,
        T::Taken,
        T::NotSet,
        T::CaptureHint,
    ];

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
            T::WindowsPanels => ("Окна Windows", "Windows panels"),
            T::PlaybackDevices => ("Устройства воспроизведения", "Playback devices"),
            T::RecordingDevices => ("Устройства записи", "Recording devices"),
            T::Sounds => ("Звуки", "Sounds"),
            T::MakeDefault => ("Сделать устройством по умолчанию", "Set as default device"),
            T::MakeComm => ("Сделать устройством связи", "Set as default communication device"),
            T::Device => ("Устройство", "Device"),
            T::AsSystem => ("Как в системе", "Same as system"),
            T::RouteUnavailable => ("Недоступно в этой версии Windows", "Not available in this Windows version"),
            T::Recording => ("Запись", "Recording"),
            T::AllDevices => ("Все устройства", "All devices"),
            T::DefaultOnly => ("Только основное", "Default only"),
            T::Hotkeys => ("Горячие клавиши", "Hotkeys"),
            T::HotkeysMenu => ("Горячие клавиши…", "Hotkeys…"),
            T::Back => ("Назад", "Back"),
            T::General => ("Общие", "General"),
            T::ActOpen => ("Открыть окно", "Open window"),
            T::ActLouder => ("Громче", "Volume up"),
            T::ActQuieter => ("Тише", "Volume down"),
            T::ActNext => ("Следующее устройство вывода", "Next output device"),
            T::Change => ("Изменить", "Change"),
            T::PressKeys => ("Нажмите сочетание…", "Press keys…"),
            T::Free => ("Свободно", "Free"),
            T::Taken => ("Занято другой программой", "Taken by another app"),
            T::NotSet => ("Не задано", "Not set"),
            T::CaptureHint => ("Esc — отмена, Backspace — убрать", "Esc — cancel, Backspace — remove"),
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
