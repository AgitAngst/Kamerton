// Горячие клавиши: сочетание ↔ текст (`Ctrl+Alt+Up`) ↔ (модификаторы, код клавиши Windows), действия.
// Регистрирует их поток трея (`tray.rs`), записывает страница настроек окна (`flyout.rs`).

use eframe::egui::{Key, Modifiers};

/// Общие действия.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Action {
    Open,
    Louder,
    Quieter,
    Mute,
    NextOutput,
    Mixer,
}

impl Action {
    pub const ALL: [Action; 6] =
        [Action::Open, Action::Louder, Action::Quieter, Action::Mute, Action::NextOutput, Action::Mixer];

    /// Имя в `settings.txt`.
    pub fn key(self) -> &'static str {
        match self {
            Action::Open => "open",
            Action::Louder => "louder",
            Action::Quieter => "quieter",
            Action::Mute => "mute",
            Action::NextOutput => "next_output",
            Action::Mixer => "mixer",
        }
    }

    pub fn from_key(s: &str) -> Option<Action> {
        Action::ALL.into_iter().find(|a| a.key() == s)
    }
}

/// Действия на программу.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AppAction {
    Louder,
    Quieter,
    Mute,
}

impl AppAction {
    pub const ALL: [AppAction; 3] = [AppAction::Louder, AppAction::Quieter, AppAction::Mute];

    pub fn key(self) -> &'static str {
        match self {
            AppAction::Louder => "louder",
            AppAction::Quieter => "quieter",
            AppAction::Mute => "mute",
        }
    }

    pub fn from_key(s: &str) -> Option<AppAction> {
        AppAction::ALL.into_iter().find(|a| a.key() == s)
    }
}

/// Сочетание клавиш.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Combo {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    pub key: Key,
}

const MOD_ALT: u32 = 0x1;
const MOD_CONTROL: u32 = 0x2;
const MOD_SHIFT: u32 = 0x4;
const MOD_WIN: u32 = 0x8;

impl Combo {
    /// `Ctrl+Alt+Up` → сочетание. Имена клавиш — как у egui (`Key::name`), регистр не важен.
    pub fn parse(text: &str) -> Option<Combo> {
        let mut c = Combo { ctrl: false, alt: false, shift: false, win: false, key: Key::Escape };
        let mut key = None;
        for part in text.split('+').map(str::trim).filter(|p| !p.is_empty()) {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => c.ctrl = true,
                "alt" => c.alt = true,
                "shift" => c.shift = true,
                "win" => c.win = true,
                _ => {
                    if key.is_some() {
                        return None;
                    }
                    key = Some(key_by_name(part)?);
                }
            }
        }
        c.key = key?;
        c.usable().then_some(c)
    }

    pub fn to_text(self) -> String {
        self.parts().join("+")
    }

    /// Части для показа клавишами: `["Ctrl", "Alt", "Up"]`.
    pub fn parts(self) -> Vec<String> {
        let mut v = Vec::new();
        for (on, name) in [(self.ctrl, "Ctrl"), (self.alt, "Alt"), (self.shift, "Shift"), (self.win, "Win")] {
            if on {
                v.push(name.to_string());
            }
        }
        v.push(self.key.name().to_string());
        v
    }

    /// Не перехватывать обычный набор текста: нужен модификатор, кроме клавиш F1–F24.
    pub fn usable(self) -> bool {
        (self.ctrl || self.alt || self.win || is_function(self.key)) && vk(self.key).is_some()
    }

    /// Модификаторы для `RegisterHotKey` (без `MOD_NOREPEAT`).
    pub fn mods(self) -> u32 {
        let mut m = 0;
        if self.alt {
            m |= MOD_ALT;
        }
        if self.ctrl {
            m |= MOD_CONTROL;
        }
        if self.shift {
            m |= MOD_SHIFT;
        }
        if self.win {
            m |= MOD_WIN;
        }
        m
    }

    pub fn vk(self) -> Option<u32> {
        vk(self.key)
    }

    /// Нажатие в окне → сочетание (если годится).
    pub fn from_press(key: Key, m: Modifiers) -> Option<Combo> {
        let c = Combo { ctrl: m.ctrl, alt: m.alt, shift: m.shift, win: false, key };
        c.usable().then_some(c)
    }
}

fn key_by_name(name: &str) -> Option<Key> {
    Key::from_name(name).or_else(|| Key::ALL.iter().copied().find(|k| k.name().eq_ignore_ascii_case(name)))
}

fn is_function(k: Key) -> bool {
    let n = k.name();
    n.len() >= 2 && n.starts_with('F') && n[1..].chars().all(|c| c.is_ascii_digit())
}

/// Код клавиши Windows (VK_*).
pub fn vk(k: Key) -> Option<u32> {
    let n = k.name();
    if n.len() == 1 {
        let c = n.chars().next()?;
        if c.is_ascii_uppercase() || c.is_ascii_digit() {
            return Some(c as u32);
        }
    }
    if let Some(num) = n.strip_prefix('F').and_then(|d| d.parse::<u32>().ok())
        && (1..=24).contains(&num)
    {
        return Some(0x70 + num - 1);
    }
    Some(match k {
        Key::ArrowLeft => 0x25,
        Key::ArrowUp => 0x26,
        Key::ArrowRight => 0x27,
        Key::ArrowDown => 0x28,
        Key::PageUp => 0x21,
        Key::PageDown => 0x22,
        Key::End => 0x23,
        Key::Home => 0x24,
        Key::Insert => 0x2D,
        Key::Delete => 0x2E,
        Key::Space => 0x20,
        Key::Enter => 0x0D,
        Key::Tab => 0x09,
        Key::Num0 => 0x30,
        Key::Num1 => 0x31,
        Key::Num2 => 0x32,
        Key::Num3 => 0x33,
        Key::Num4 => 0x34,
        Key::Num5 => 0x35,
        Key::Num6 => 0x36,
        Key::Num7 => 0x37,
        Key::Num8 => 0x38,
        Key::Num9 => 0x39,
        Key::Minus => 0xBD,
        Key::Plus | Key::Equals => 0xBB,
        Key::Comma => 0xBC,
        Key::Period => 0xBE,
        Key::Semicolon => 0xBA,
        Key::Slash => 0xBF,
        Key::Backtick => 0xC0,
        Key::OpenBracket => 0xDB,
        Key::Backslash => 0xDC,
        Key::CloseBracket => 0xDD,
        Key::Quote => 0xDE,
        _ => return None,
    })
}

/// Следующее устройство вывода по кругу (порядок — как в списке устройств).
pub fn next_output<'a>(ids: &[&'a str], current: Option<&str>) -> Option<&'a str> {
    if ids.is_empty() {
        return None;
    }
    let i = current.and_then(|c| ids.iter().position(|id| *id == c)).map_or(0, |i| (i + 1) % ids.len());
    Some(ids[i])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_print() {
        let c = Combo::parse("Ctrl+Alt+Up").unwrap();
        assert!(c.ctrl && c.alt && !c.shift && !c.win);
        assert_eq!(c.key, Key::ArrowUp);
        assert_eq!(c.to_text(), "Ctrl+Alt+Up");
        assert_eq!(Combo::parse(&c.to_text()), Some(c));
        assert_eq!(Combo::parse("ctrl + shift + f9").unwrap().to_text(), "Ctrl+Shift+F9");
        assert_eq!(Combo::parse("F13").unwrap().vk(), Some(0x7C));
    }

    #[test]
    fn bad_or_unsafe_combos_are_rejected() {
        assert_eq!(Combo::parse("Ctrl+Чепуха"), None);
        assert_eq!(Combo::parse("Ctrl+A+B"), None);
        // Без модификатора обычная клавиша перехватила бы набор текста.
        assert_eq!(Combo::parse("A"), None);
        assert_eq!(Combo::parse("Shift+A"), None);
        assert_eq!(Combo::parse(""), None);
    }

    #[test]
    fn windows_codes_and_mods() {
        let c = Combo::parse("Ctrl+Shift+Win+M").unwrap();
        assert_eq!(c.vk(), Some(0x4D));
        assert_eq!(c.mods(), MOD_CONTROL | MOD_SHIFT | MOD_WIN);
        assert_eq!(vk(Key::ArrowDown), Some(0x28));
        assert_eq!(vk(Key::Num5), Some(0x35));
        assert_eq!(vk(Key::Copy), None);
    }

    #[test]
    fn press_needs_modifier() {
        assert!(Combo::from_press(Key::A, Modifiers::NONE).is_none());
        assert!(Combo::from_press(Key::A, Modifiers::CTRL).is_some());
        assert!(Combo::from_press(Key::F5, Modifiers::NONE).is_some());
    }

    #[test]
    fn next_output_cycles() {
        let ids = ["a", "b", "c"];
        assert_eq!(next_output(&ids, Some("a")), Some("b"));
        assert_eq!(next_output(&ids, Some("c")), Some("a"));
        assert_eq!(next_output(&ids, None), Some("a"));
        assert_eq!(next_output(&ids, Some("zz")), Some("a"));
        assert_eq!(next_output(&[], Some("a")), None);
    }

    #[test]
    fn action_keys_round_trip() {
        for a in Action::ALL {
            assert_eq!(Action::from_key(a.key()), Some(a));
        }
        for a in AppAction::ALL {
            assert_eq!(AppAction::from_key(a.key()), Some(a));
        }
    }
}
