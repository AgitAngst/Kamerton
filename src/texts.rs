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
    Hotkeys,
    HotkeysMenu,
    Back,
    ActOpen,
    ActLouder,
    ActQuieter,
    ActNext,
    Change,
    PressKeys,
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
    ActPrev,
    GroupWindow,
    GroupVolume,
    HotkeysHint,

    OutputDevice,
    InputDevice,
    VolumeMixer,
    MoreSettings,
    ShowInputs,
    HideInputs,

    Portable,
    CheckUpdates,
    Prerelease,
    CheckNow,
    UpdateNow,
    SkipVersion,
    RestartNow,
    Retry,
    MenuUpdate,
    MenuRestart,

    Hidden,
    NothingHidden,
    Show,
    Hide,
    MixerApp,
    Balance,
    BalanceL,
    BalanceR,
    ActMixer,

    UnknownApp,
    DefaultMuted,
    AppsSilent,
    MicMuted,
    MicSilent,
    TurnOn,
    RecordingApps,
    NoRecordingApps,
    MakeDefaultCaption,
    AppHotkeysHint,
    DefaultBadge,
    Input,
    BalanceCenter,

    IconSection,
    InTray,
    TrayMark,
    TraySpeaker,
    IconColor,
    ColorPurple,
    ColorAzure,
    IconColorNote,
}

impl T {
    #[cfg(test)]
    pub const ALL: [T; 107] = [
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
        T::Hotkeys,
        T::HotkeysMenu,
        T::Back,
        T::ActOpen,
        T::ActLouder,
        T::ActQuieter,
        T::ActNext,
        T::Change,
        T::PressKeys,
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
        T::ActPrev,
        T::GroupWindow,
        T::GroupVolume,
        T::HotkeysHint,
        T::OutputDevice,
        T::InputDevice,
        T::VolumeMixer,
        T::MoreSettings,
        T::ShowInputs,
        T::HideInputs,
        T::Portable,
        T::CheckUpdates,
        T::Prerelease,
        T::CheckNow,
        T::UpdateNow,
        T::SkipVersion,
        T::RestartNow,
        T::Retry,
        T::MenuUpdate,
        T::MenuRestart,
        T::Hidden,
        T::NothingHidden,
        T::Show,
        T::Hide,
        T::MixerApp,
        T::Balance,
        T::BalanceL,
        T::BalanceR,
        T::ActMixer,
        T::UnknownApp,
        T::DefaultMuted,
        T::AppsSilent,
        T::MicMuted,
        T::MicSilent,
        T::TurnOn,
        T::RecordingApps,
        T::NoRecordingApps,
        T::MakeDefaultCaption,
        T::AppHotkeysHint,
        T::DefaultBadge,
        T::Input,
        T::BalanceCenter,
        T::IconSection,
        T::InTray,
        T::TrayMark,
        T::TraySpeaker,
        T::IconColor,
        T::ColorPurple,
        T::ColorAzure,
        T::IconColorNote,
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
            T::Hotkeys => ("Горячие клавиши", "Hotkeys"),
            T::HotkeysMenu => ("Горячие клавиши…", "Hotkeys…"),
            T::Back => ("Назад", "Back"),
            T::ActOpen => ("Открыть окно", "Open window"),
            T::ActLouder => ("Громче", "Volume up"),
            T::ActQuieter => ("Тише", "Volume down"),
            T::ActNext => ("Следующее", "Next"),
            T::Change => ("Изменить", "Change"),
            T::PressKeys => ("Нажмите сочетание…", "Press keys…"),
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
            T::ActPrev => ("Предыдущее", "Previous"),
            T::GroupWindow => ("Окно", "Window"),
            T::GroupVolume => ("Громкость", "Volume"),
            T::HotkeysHint => (
                "Щёлкните поле, чтобы задать сочетание. Esc — отмена, Backspace — убрать.",
                "Click a field to set a hotkey. Esc — cancel, Backspace — remove.",
            ),

            T::OutputDevice => ("Устройство вывода", "Output device"),
            T::InputDevice => ("Устройство ввода", "Input device"),
            T::VolumeMixer => ("Микшер громкости", "Volume mixer"),
            T::MoreSettings => ("Другие параметры звука", "More sound settings"),
            T::ShowInputs => ("Показать устройства ввода", "Show input devices"),
            T::HideInputs => ("Скрыть устройства ввода", "Hide input devices"),

            T::CheckUpdates => ("Проверять обновления", "Check for updates"),
            T::Prerelease => ("Предлагать пред-выпуски", "Offer pre-releases"),
            T::CheckNow => ("Проверить сейчас", "Check now"),
            T::UpdateNow => ("Обновить", "Update"),
            T::SkipVersion => ("Пропустить", "Skip"),
            T::RestartNow => ("Перезапустить", "Restart"),
            T::Retry => ("Повторить", "Retry"),
            T::MenuUpdate => ("Обновить до", "Update to"),
            T::MenuRestart => ("Перезапустить на", "Restart into"),

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

            T::UnknownApp => ("Неизвестная программа", "Unknown app"),
            T::DefaultMuted => ("Устройство без звука", "Device is muted"),
            T::AppsSilent => ("Программы на нём не слышно", "Apps on it can’t be heard"),
            T::MicMuted => ("Микрофон без звука", "Microphone is muted"),
            T::MicSilent => ("Программы его не слышат", "Apps can’t hear it"),
            T::TurnOn => ("Включить", "Unmute"),
            T::RecordingApps => ("Программы записи", "Recording apps"),
            T::NoRecordingApps => ("Микрофон никто не слушает", "No app is using the microphone"),
            T::MakeDefaultCaption => ("Сделать основным:", "Make default:"),
            T::AppHotkeysHint => (
                "Сочетания программы — правый щелчок по её строке → «Горячие клавиши…»",
                "App hotkeys: right-click its row → “Hotkeys…”",
            ),
            T::DefaultBadge => ("Основное", "Default"),
            T::Input => ("Ввод", "Input"),
            T::BalanceCenter => ("Центр", "Center"),
            T::IconSection => ("Значок", "Icon"),
            T::InTray => ("В трее", "In the tray"),
            T::TrayMark => ("Знак", "Mark"),
            T::TraySpeaker => ("Колонка", "Speaker"),
            T::IconColor => ("Цвет", "Color"),
            T::ColorPurple => ("Пурпур", "Purple"),
            T::ColorAzure => ("Лазурь", "Azure"),
            T::IconColorNote => {
                ("Для трея, окна микшера и «О программе»", "Used by the tray, the mixer window and «About»")
            }
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
