// Поиск по странице «Настройки»: какие строки показать при введённом запросе. Чистые функции, тесты внизу.
// Слова строки — её подписи на обоих языках и несколько слов, которых в подписи нет («обновления», «версия»):
// искать можно и по-русски, и по-английски, независимо от языка окна.

use crate::texts::T;

/// Строки и блоки страницы «Настройки», которые находит поиск.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Item {
    Hotkeys,
    Profiles,
    Rules,
    Theme,
    Language,
    TrayIcon,
    IconColor,
    Animation,
    WheelOutside,
    LogScale,
    Osd,
    MixerOnTop,
    Autostart,
    Hidden,
    About,
}

/// Все строки страницы по порядку.
pub const ALL: [Item; 15] = [
    Item::Hotkeys,
    Item::Profiles,
    Item::Rules,
    Item::Theme,
    Item::Language,
    Item::TrayIcon,
    Item::IconColor,
    Item::Animation,
    Item::WheelOutside,
    Item::LogScale,
    Item::Osd,
    Item::MixerOnTop,
    Item::Autostart,
    Item::Hidden,
    Item::About,
];

/// Подписи строки: русская и английская, и слова, которых в подписи нет.
fn words(item: Item) -> (&'static [T], &'static [&'static str]) {
    match item {
        Item::Hotkeys => (&[T::Hotkeys], &["клавиши", "keyboard", "shortcut"]),
        Item::Profiles => (&[T::Profiles], &["наборы", "preset"]),
        Item::Rules => (
            &[T::Rules, T::LimitSection, T::DuckSection, T::RulesSection],
            &[
                "потолок",
                "лимит",
                "limit",
                "ceiling",
                "приглушение",
                "duck",
                "правило",
                "rule",
                "ночь",
                "ночью",
                "night",
            ],
        ),
        Item::Theme => (
            &[T::Theme, T::Appearance, T::ThemeSystem, T::ThemeLight, T::ThemeDark],
            &["светлая", "тёмная", "dark", "light"],
        ),
        Item::Language => (&[T::Language], &["русский", "english", "язык", "language"]),
        Item::TrayIcon => (
            &[T::IconSection, T::InTray, T::TrayMark, T::TraySpeaker, T::IconColorNote],
            &["значок", "icon", "колонка", "знак"],
        ),
        Item::IconColor => (&[T::IconColor, T::ColorPurple, T::ColorAzure], &["цвет", "color", "colour"]),
        Item::Animation => (&[T::Animation, T::WindowSection], &["анимация", "animation", "motion"]),
        Item::WheelOutside => (&[T::WheelOutside, T::WindowSection], &["колесо", "wheel", "scroll"]),
        Item::LogScale => (&[T::LogScale, T::WindowSection], &["шкала", "scale", "logarithmic"]),
        Item::Osd => (&[T::Osd, T::WindowSection], &["osd", "плашка", "popup", "overlay"]),
        Item::MixerOnTop => (&[T::MixerOnTop, T::WindowSection], &["микшер", "mixer", "поверх", "top"]),
        Item::Autostart => {
            (&[T::Autostart, T::ThemeSystem], &["автозапуск", "startup", "autostart", "boot", "windows"])
        }
        Item::Hidden => (&[T::Hidden, T::NothingHidden], &["скрыть", "hide", "hidden"]),
        Item::About => (
            &[T::About],
            &["версия", "version", "обновления", "updates", "update", "kamerton", "пред-выпуски", "pre-release"],
        ),
    }
}

/// Все слова строки в нижнем регистре.
fn haystack(item: Item) -> Vec<String> {
    let (texts, extra) = words(item);
    let mut v: Vec<String> = Vec::new();
    for t in texts {
        v.push(t.in_lang(false).to_lowercase());
        v.push(t.in_lang(true).to_lowercase());
    }
    v.extend(extra.iter().map(|w| w.to_lowercase()));
    v
}

/// Запрос из слов через пробел: подходит строка, у которой каждое слово запроса — часть какого-то её слова.
fn fits(query: &str, hay: &[String]) -> bool {
    let q = query.trim().to_lowercase();
    q.split_whitespace().all(|term| hay.iter().any(|w| w.contains(term)))
}

/// Показывать ли строку при запросе `query` (пустой запрос — всё).
pub fn shown(query: &str, item: Item) -> bool {
    fits(query, &haystack(item))
}

/// Показывать ли раздел: хоть одна его строка подходит (пустой запрос — всегда).
pub fn any(query: &str, items: &[Item]) -> bool {
    items.iter().any(|i| shown(query, *i))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(q: &str) -> Vec<Item> {
        ALL.iter().copied().filter(|i| shown(q, *i)).collect()
    }

    #[test]
    fn empty_query_shows_everything() {
        assert_eq!(found("").len(), ALL.len());
        assert_eq!(found("   ").len(), ALL.len());
    }

    #[test]
    fn finds_by_either_language() {
        assert_eq!(found("авто"), vec![Item::Autostart]);
        assert_eq!(found("startup"), vec![Item::Autostart]);
        assert_eq!(found("Start with"), vec![Item::Autostart]);
        assert_eq!(found("плашка"), vec![Item::Osd]);
        assert_eq!(found("popup"), vec![Item::Osd]);
    }

    #[test]
    fn case_does_not_matter() {
        assert_eq!(found("ТЕМА"), found("тема"));
        assert!(found("Theme").contains(&Item::Theme));
        assert!(found("ТЁМНАЯ").contains(&Item::Theme));
    }

    #[test]
    fn section_name_finds_all_its_rows() {
        let window = found("окно");
        for i in [Item::Animation, Item::WheelOutside, Item::LogScale, Item::Osd] {
            assert!(window.contains(&i), "{i:?}");
        }
        assert!(!window.contains(&Item::Theme));
    }

    #[test]
    fn several_words_must_all_fit() {
        assert_eq!(found("колесо окна"), vec![Item::WheelOutside]);
        assert!(found("колесо тема").is_empty());
    }

    #[test]
    fn nothing_found_and_sections() {
        assert!(found("zzz").is_empty());
        assert!(!any("zzz", &[Item::Theme, Item::Language]));
        assert!(any("язык", &[Item::Theme, Item::Language]));
        assert!(any("", &[Item::Theme]));
    }

    #[test]
    fn about_finds_updates() {
        assert!(found("обновления").contains(&Item::About));
        assert!(found("updates").contains(&Item::About));
    }
}
