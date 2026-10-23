// Обновления изнутри программы — `anvil-update` (GitHub Releases, SHA-256, перезапуск). Проверяет трей:
// раз в минуту спрашивает, не пора ли (сама проверка — при запуске и раз в сутки, если включена);
// управляют ею «Настройки → О программе» и меню значка.

use std::sync::OnceLock;

use anvil_update::{Config, State, Updater};

use crate::config::Settings;

/// Переменная окружения у нового процесса после «Перезапустить»: подождать, пока старый отпустит
/// «один экземпляр».
pub const RESTARTED: &str = "KAMERTON_RESTARTED";

static UPDATER: OnceLock<Updater> = OnceLock::new();

/// Завести проверяльщика (трей, один раз). `repaint` — перерисовать окно, когда состояние сменилось.
pub fn init(repaint: impl Fn() + Send + Sync + 'static) {
    anvil_update::cleanup();
    let config = Config::new("kamerton", env!("CARGO_PKG_VERSION"), "AgitAngst/Kamerton");
    let _ = UPDATER.set(Updater::with_repaint(config, repaint));
}

pub fn get() -> Option<&'static Updater> {
    UPDATER.get()
}

/// Настройки обновлений в виде, который понимает набор.
pub fn common(s: &Settings) -> anvil_ui::CommonSettings {
    anvil_ui::CommonSettings {
        check_updates: s.check_updates,
        prerelease: s.prerelease,
        skip_version: s.skip_version.clone(),
        ..Default::default()
    }
}

/// Проверка при запуске и раз в сутки, если включена. Дёшево — звать по таймеру.
pub fn tick(s: &Settings) {
    if let Some(u) = get() {
        u.auto(&common(s));
    }
}

/// Проверить сейчас (кнопка): пропущенную версию показать всё равно.
pub fn check_now(s: &Settings) {
    if let Some(u) = get() {
        u.check(s.prerelease, s.skip_version.as_deref(), true);
    }
}

/// Версия, которую можно поставить, — для пункта меню значка.
pub fn available() -> Option<String> {
    match get()?.state() {
        State::Available(u) if u.asset.is_some() => Some(u.version.to_string()),
        _ => None,
    }
}

/// Версия, которая уже скачана и ждёт перезапуска.
pub fn ready() -> Option<String> {
    match get()?.state() {
        State::Ready { update, .. } => Some(update.version.to_string()),
        _ => None,
    }
}

/// Запустить новую версию и закрыть трей этого процесса. `Err` — текст для пользователя.
pub fn restart(tray: isize) -> Result<(), String> {
    let u = get().ok_or("no updater")?;
    // Новый процесс наследует окружение: по нему он подождёт, пока этот выйдет.
    unsafe { std::env::set_var(RESTARTED, "1") };
    u.restart()?;
    crate::sys::post_close(tray);
    Ok(())
}
