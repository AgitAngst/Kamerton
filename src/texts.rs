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

/// Язык из настроек; `KAMERTON_LANG` (проверка вида) важнее.
pub fn apply(pref: crate::config::LangPref) {
    if std::env::var_os("KAMERTON_LANG").is_some() {
        return;
    }
    let en = match pref {
        crate::config::LangPref::Ru => false,
        crate::config::LangPref::En => true,
        crate::config::LangPref::System => anvil_ui::lang::system_language() != anvil_ui::Lang::Ru,
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
    Profiles,
    MemoryTitle,
    RememberVolumes,
    NewAppVolume,
    Apply,
    Delete,
    DeleteConfirm,
    SaveCurrent,
    OnConnect,
    Never,
    NotConnected,
    ProfileName,
    Output,
    Apps,
    NoProfiles,
    Settings,
    Appearance,
    Theme,
    ThemeSystem,
    ThemeLight,
    ThemeDark,
    Language,
    WindowSection,
    Animation,
    WheelOutside,
    LogScale,
    Autostart,
    About,
    Portable,
    Hidden,
    NothingHidden,
    Show,
    Hide,
    MixerApp,
    Balance,
    BalanceL,
    BalanceR,
    ActMixer,
}

impl T {
    #[cfg(test)]
    pub const ALL: [T; 72] = [
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
        T::Profiles,
        T::MemoryTitle,
        T::RememberVolumes,
        T::NewAppVolume,
        T::Apply,
        T::Delete,
        T::DeleteConfirm,
        T::SaveCurrent,
        T::OnConnect,
        T::Never,
        T::NotConnected,
        T::ProfileName,
        T::Output,
        T::Apps,
        T::NoProfiles,
        T::Settings,
        T::Appearance,
        T::Theme,
        T::ThemeSystem,
        T::ThemeLight,
        T::ThemeDark,
        T::Language,
        T::WindowSection,
        T::Animation,
        T::WheelOutside,
        T::LogScale,
        T::Autostart,
        T::About,
        T::Portable,
        T::Hidden,
        T::NothingHidden,
        T::Show,
        T::Hide,
        T::MixerApp,
        T::Balance,
        T::BalanceL,
        T::BalanceR,
        T::ActMixer,
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
            T::Profiles => ("Профили", "Profiles"),
            T::MemoryTitle => ("Память", "Memory"),
            T::RememberVolumes => ("Запоминать громкость программ", "Remember app volumes"),
            T::NewAppVolume => ("Громкость новых программ", "Volume for new apps"),
            T::Apply => ("Применить", "Apply"),
            T::Delete => ("Удалить", "Delete"),
            T::DeleteConfirm => ("Удалить профиль?", "Delete profile?"),
            T::SaveCurrent => ("Сохранить текущее", "Save current"),
            T::OnConnect => ("Включать при подключении", "Apply when connected"),
            T::Never => ("Никогда", "Never"),
            T::NotConnected => ("Не подключено", "Not connected"),
            T::ProfileName => ("Профиль", "Profile"),
            T::Output => ("Вывод", "Output"),
            T::Apps => ("Программы", "Apps"),
            T::NoProfiles => (
                "Профилей пока нет: настройте звук и нажмите «Сохранить текущее»",
                "No profiles yet: set up the sound and press «Save current»",
            ),
            T::Settings => ("Настройки", "Settings"),
            T::Appearance => ("Оформление", "Appearance"),
            T::Theme => ("Тема", "Theme"),
            T::ThemeSystem => ("Система", "System"),
            T::ThemeLight => ("Светлая", "Light"),
            T::ThemeDark => ("Тёмная", "Dark"),
            T::Language => ("Язык", "Language"),
            T::WindowSection => ("Окно", "Window"),
            T::Animation => ("Анимация открытия", "Opening animation"),
            T::WheelOutside => ("Колесо вне окна меняет громкость", "Wheel outside the window changes volume"),
            T::LogScale => ("Логарифмическая шкала", "Logarithmic scale"),
            T::Autostart => ("Запускать при входе в Windows", "Start with Windows"),
            T::About => ("О программе", "About"),
            T::Portable => ("Переносная копия: настройки рядом с exe", "Portable copy: settings next to the exe"),
            T::Hidden => ("Скрытое", "Hidden"),
            T::NothingHidden => ("Ничего не скрыто", "Nothing is hidden"),
            T::Show => ("Показать", "Show"),
            T::Hide => ("Скрыть", "Hide"),
            T::MixerApp => ("Микшер Kamerton", "Kamerton mixer"),
            T::Balance => ("Баланс", "Balance"),
            T::BalanceL => ("Л", "L"),
            T::BalanceR => ("П", "R"),
            T::ActMixer => ("Открыть микшер", "Open mixer"),
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
