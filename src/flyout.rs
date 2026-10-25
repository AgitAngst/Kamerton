// Выпадающее окно микшера. Живёт в процессе трея с самого старта (как у EarTrumpet): спрятано и закрыто
// DWM-плащом (cloak), по щелчку встаёт на место, рисует кадр и проявляется сразу после него — без создания
// окна и устройства на щелчок. Пока окно закрыто, кадров нет; пока открыто — снимок и уровни каждый кадр.
// `kamerton --flyout` — то же окно отдельным процессом, для проверки вида.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anvil_ui::motion::machine::Scene;
use anvil_ui::theme::radius;
use eframe::egui::{
    self, Align2, Color32, CornerRadius, CursorIcon, Event, FontId, Key, MouseWheelUnit, Rect, Sense, Stroke,
    StrokeKind, TextureHandle, TextureOptions, Ui, Vec2, pos2, vec2,
};

use crate::audio::{App, AppKey, Audio, Control, Device, Flow, Roles, Snapshot, Target, model};
use crate::config::{IconColor, LangPref, Settings, ThemePref, TrayIcon};
use crate::hotkeys::{Action, AppAction, Combo};
use crate::icons;
use crate::mark::{self, MarkScene};
use crate::mark_render;
use crate::placement::{self, Edge, Rect as PxRect};
use crate::rules::{self, AppRule, DeviceAct, When};
use crate::settings_search::{self, Item};
use crate::sys;
use crate::texts::T;

/// Размеры окна и строк, логические пиксели (`docs/ui-guideline.md`).
pub const WIDTH: f32 = 360.0;
pub const PAD_X: f32 = 16.0;
pub const PAD_Y: f32 = 12.0;
/// Строка устройства в колонке окна микшера (`--mixer`).
pub const DEVICE_ROW: f32 = 52.0;
pub const APP_ROW: f32 = 50.0;
/// На сколько строка программы раскрывается вниз, когда наведён курсор на подпись «→ устройство»: ниже имени
/// появляется подпись целиком. Окно растёт на столько же (`Flyout::row_extra`).
const APP_EXTRA: f32 = 20.0;
/// Окно у значка (вид «Вывод звука» Windows 11): отступ сверху, заголовок раздела, строка устройства
/// в списке и зазор между ними, черта между разделами, заголовок микшера, отступ снизу, полоса внизу.
pub const TOP: f32 = 10.0;
pub const HEADER_ROW: f32 = 36.0;
pub const ITEM_ROW: f32 = 56.0;
pub const ITEM_GAP: f32 = 2.0;
pub const DIVIDER: f32 = 9.0;
pub const MIXER_HEADER: f32 = 40.0;
/// Полоса «основное устройство без звука» над его программами — с полями 2 сверху и 6 снизу.
pub const MUTED_ROW: f32 = 52.0;
pub const BOTTOM: f32 = 8.0;
/// Строки списка устройств отступают от края окна на 4, всё остальное — на `PAD_X`.
const INSET: f32 = 4.0;
/// Поле внутри строки устройства слева и справа: значок и «без звука» встают в колонки программ.
const ITEM_PAD: f32 = PAD_X - INSET;
/// Страница сочетаний: строка, ширина поля сочетания (по содержимому, от и до), строка подписи раздела,
/// подпись «Сделать основным:» над устройствами.
const HOTKEY_ROW: f32 = 32.0;
const HOTKEY_FIELD_MIN: f32 = 96.0;
const HOTKEY_FIELD: f32 = 160.0;
const HOTKEY_GROUP: f32 = 30.0;
const HOTKEY_CAPTION: f32 = 22.0;
/// Страница настроек: ширина переключателей «Тема» и «Язык», строка перехода на страницу.
const SEGMENTED_W: f32 = 240.0;
const NAV_ROW: f32 = 40.0;
/// Кнопка «устройства ввода» внизу: микрофон и стрелка.
const TOGGLE_W: f32 = 44.0;
pub const EMPTY_ROW: f32 = 30.0;
pub const FOOTER: f32 = 48.0;
const NUMBER_W: f32 = 34.0;
const BUTTON: f32 = 30.0;
const ICON: f32 = 24.0;
/// Шаг клавиш и колеса над ползунком.
const STEP: f32 = 0.02;
const PAGE: f32 = 0.10;
/// Сколько держать своё значение ползунка после движения, пока Windows не сообщила новое.
const HOLD: Duration = Duration::from_millis(400);
/// Въезд окна при открытии: расстояние и время (`docs/ui-guideline.md`, «Движение»).
const SLIDE_PX: f32 = 12.0;
/// Окно микшера: ширина колонки устройства, промежуток между группами «Вывод» и «Ввод», строка баланса.
const MIXER_COLUMN: f32 = 340.0;
const MIXER_GROUP_GAP: f32 = 24.0;
const BALANCE_ROW: f32 = 40.0;
/// Поля окна микшера и его наименьший размер (точки).
const MIXER_MARGIN: f32 = 16.0;
const MIXER_MIN: [f32; 2] = [MIXER_COLUMN + 2.0 * MIXER_MARGIN, 200.0];
const SLIDE_SECS: f32 = 0.18;
/// Окно закрылось — щелчок по значку в эти мс его не открывает: это тот же щелчок, что снял фокус.
const REOPEN_GUARD_MS: u64 = 300;

const HIDDEN: u8 = 0;
const OPENING: u8 = 1;
const OPEN: u8 = 2;
/// Окно показано плашкой громкости (не окном микшера).
const OSD: u8 = 3;

/// Плашка громкости: размер, отступ от панели задач (логические пиксели), сколько держать, сдвиг при появлении.
const OSD_W: f32 = 260.0;
const OSD_H: f32 = 56.0;
/// Высота плашки с именем устройства (сочетания громкости устройства): имя над ползунком.
const OSD_H_NAMED: f32 = 74.0;
const OSD_GAP: f32 = 24.0;
const OSD_SHOW: Duration = Duration::from_millis(1500);
const OSD_SLIDE_PX: f32 = 8.0;

/// Что показывает плашка: громкость и «без звука» устройства по умолчанию, место значка (по нему — монитор)
/// и до какого времени держать. `name` — устройство, если меняли не основное (сочетание громкости устройства).
#[derive(Debug, Clone)]
struct OsdInfo {
    anchor: PxRect,
    volume: f32,
    muted: bool,
    name: Option<String>,
    until: Instant,
}

/// Устройства одного направления: по умолчанию — первым.
fn of_flow(s: &Snapshot, flow: Flow) -> Vec<&Device> {
    let mut v: Vec<&Device> = s.devices.iter().filter(|d| d.flow == flow).collect();
    v.sort_by_key(|d| !d.is_default);
    v
}

/// Устройства вывода: по умолчанию — первым.
pub fn outputs(s: &Snapshot) -> Vec<&Device> {
    of_flow(s, Flow::Render)
}

/// Программы устройства в окне: у записи — без «Системных звуков».
fn shown_apps(d: &Device) -> Vec<&App> {
    d.apps.iter().filter(|a| !(d.flow == Flow::Capture && a.system)).collect()
}

/// Программы направления для микшера: сначала у устройства по умолчанию, затем у остальных.
/// «Системные звуки» есть у каждого устройства — в микшере только у устройства по умолчанию.
pub fn mixer_apps<'a>(devs: &[&'a Device]) -> Vec<(&'a Device, &'a App)> {
    devs.iter()
        .flat_map(|d| shown_apps(d).into_iter().filter(|a| !a.system || d.is_default).map(move |a| (*d, a)))
        .collect()
}

/// Высота строк программ: нет программ — строка «Нет программ со звуком».
fn apps_height(n: usize) -> f32 {
    if n == 0 { EMPTY_ROW } else { n as f32 * APP_ROW }
}

/// Основное устройство направления «без звука» — его программ не слышно, над ними полоса.
fn default_muted<'a>(devs: &[&'a Device]) -> Option<&'a Device> {
    devs.iter().find(|d| d.is_default && d.muted).copied()
}

/// Раздел направления: заголовок и список устройств, черта, заголовок программ («Микшер громкости» или
/// «Программы записи»), полоса «без звука», программы.
fn section_height(devs: &[&Device]) -> f32 {
    let muted = if default_muted(devs).is_some() { MUTED_ROW } else { 0.0 };
    HEADER_ROW
        + devs.len() as f32 * (ITEM_ROW + ITEM_GAP)
        + DIVIDER
        + MIXER_HEADER
        + muted
        + apps_height(mixer_apps(devs).len())
}

/// Высота содержимого окна: от неё место окна считается ещё до кадра. Сверху вниз: раздел вывода;
/// развёрнуто — черта и раздел ввода; внизу — полоса.
pub fn content_height(s: &Snapshot, expanded: bool) -> f32 {
    let outs = outputs(s);
    let ins = if expanded { of_flow(s, Flow::Capture) } else { Vec::new() };
    if outs.is_empty() && ins.is_empty() {
        return TOP + EMPTY_ROW + BOTTOM + FOOTER;
    }
    let mut h = TOP;
    if !outs.is_empty() {
        h += section_height(&outs);
    }
    if !ins.is_empty() {
        if !outs.is_empty() {
            h += DIVIDER;
        }
        h += section_height(&ins);
    }
    h + BOTTOM + FOOTER
}

/// Программу тянут на другое устройство.
#[derive(Debug, Clone)]
struct DragApp {
    key: AppKey,
    name: String,
    flow: Flow,
}

struct Spot {
    anchor: PxRect,
    work: PxRect,
    edge: Edge,
    scale: f32,
}

impl Spot {
    fn at(anchor: PxRect) -> Spot {
        let (mon, work, scale) = sys::monitor_at((anchor.left + anchor.right) / 2, (anchor.top + anchor.bottom) / 2);
        Spot { anchor, work, edge: placement::taskbar_edge(mon, work), scale }
    }
}

/// Общее у окна и трея: состояние окна и просьбы трея (открыть у значка, закрыть, выйти).
pub struct Shared {
    hwnd: AtomicIsize,
    state: AtomicU8,
    hidden_at: AtomicU64,
    /// Открыть у этого места; `true` — по щелчку: взять фокус и закрываться, когда он потерян.
    pending: Mutex<Option<(PxRect, bool)>>,
    close: AtomicBool,
    exiting: AtomicBool,
    ctx: egui::Context,
    /// Настройки — общие у окна (пишет) и трея (горячие клавиши).
    pub settings: Mutex<Settings>,
    /// Зарегистрировалось ли сочетание: ключ — строка из `settings.txt` (`hotkey.louder`, …).
    pub hotkey_free: Mutex<HashMap<String, bool>>,
    /// Окно записывает сочетание: трей снимает свои, чтобы нажатие дошло до окна.
    pub capturing: AtomicBool,
    /// Колесо вне открытого окна меняет громкость — копия настройки для хука мыши (без блокировки).
    pub wheel_outside: AtomicBool,
    /// Прогон на утечки: открыть, но не снимать плащ.
    ghost: AtomicBool,
    /// Плашка громкости, пока показана (состояние `OSD`).
    osd: Mutex<Option<OsdInfo>>,
}

impl Shared {
    pub fn new(ctx: egui::Context, hwnd: isize) -> Shared {
        let settings = Settings::load();
        let wheel = settings.wheel_outside;
        let s = Shared {
            hwnd: AtomicIsize::new(hwnd),
            state: AtomicU8::new(HIDDEN),
            hidden_at: AtomicU64::new(0),
            pending: Mutex::new(None),
            close: AtomicBool::new(false),
            exiting: AtomicBool::new(false),
            ctx,
            settings: Mutex::new(settings),
            hotkey_free: Mutex::new(HashMap::new()),
            capturing: AtomicBool::new(false),
            wheel_outside: AtomicBool::new(false),
            ghost: AtomicBool::new(false),
            osd: Mutex::new(None),
        };
        s.wheel_outside.store(wheel, Ordering::SeqCst);
        s
    }

    /// Показать плашку громкости (из потока трея) или продлить уже показанную. Окно микшера открыто или
    /// открывается — плашки нет, оно само показывает громкость; выключено в настройках — тоже.
    pub fn show_osd(&self, anchor: PxRect, volume: f32, muted: bool, name: Option<String>) {
        if std::env::var_os("KAMERTON_TRACE").is_some() {
            eprintln!(
                "trace: osd requested, state {}, volume {volume}, muted {muted}",
                self.state.load(Ordering::SeqCst)
            );
        }
        if !self.settings.lock().unwrap().osd {
            return;
        }
        if matches!(self.state.load(Ordering::SeqCst), OPEN | OPENING) {
            return;
        }
        *self.osd.lock().unwrap() = Some(OsdInfo { anchor, volume, muted, name, until: Instant::now() + OSD_SHOW });
        if self.state.compare_exchange(HIDDEN, OSD, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
            // Как при открытии: показать под плащом, без фокуса, и попросить кадр напрямую.
            let hwnd = self.hwnd.load(Ordering::SeqCst);
            sys::show_noactivate(hwnd);
            sys::invalidate(hwnd);
        }
        self.ctx.request_repaint();
    }

    /// Прогон на утечки: открыть как обычно, но оставить под плащом.
    pub fn open_ghost(&self, anchor: PxRect) {
        self.ghost.store(true, Ordering::SeqCst);
        self.open(anchor, false);
    }

    /// Попросить окно закрыться (из другого потока).
    pub fn request_close(&self) {
        self.close.store(true, Ordering::SeqCst);
        self.ctx.request_repaint();
        sys::invalidate(self.hwnd());
    }

    pub fn is_open(&self) -> bool {
        self.state.load(Ordering::SeqCst) == OPEN
    }

    pub fn hwnd(&self) -> isize {
        self.hwnd.load(Ordering::SeqCst)
    }

    /// Снимок без скрытого пользователем.
    pub fn visible(&self, s: &Snapshot) -> Snapshot {
        let st = self.settings.lock().unwrap();
        model::visible(s, &st.hidden_apps, &st.hidden_devices)
    }

    pub fn request_repaint(&self) {
        self.ctx.request_repaint();
    }

    pub fn expanded(&self) -> bool {
        self.settings.lock().unwrap().expanded
    }

    /// Щелчок по значку: открыто — закрыть; закрыто — открыть у значка; открывается — ничего.
    pub fn toggle(&self, anchor: PxRect) {
        match self.state.load(Ordering::SeqCst) {
            OPEN => {
                self.close.store(true, Ordering::SeqCst);
                self.ctx.request_repaint();
            }
            HIDDEN if sys::now_ms().saturating_sub(self.hidden_at.load(Ordering::SeqCst)) > REOPEN_GUARD_MS => {
                self.open(anchor, true)
            }
            // Плашка не окно микшера: щелчок по значку открывает его сразу.
            OSD => self.open(anchor, true),
            _ => {}
        }
    }

    /// Открыть окно у `anchor`. Спрятанному окну Windows не шлёт WM_PAINT, поэтому сначала показать его
    /// под плащом (невидимым, без фокуса), потом попросить кадр.
    pub fn open(&self, anchor: PxRect, by_click: bool) {
        if std::env::var_os("KAMERTON_TRACE").is_some() {
            eprintln!("trace: open requested at {} ms", sys::now_ms() % 100_000);
        }
        *self.osd.lock().unwrap() = None;
        *self.pending.lock().unwrap() = Some((anchor, by_click));
        self.state.store(OPENING, Ordering::SeqCst);
        let hwnd = self.hwnd.load(Ordering::SeqCst);
        sys::show_noactivate(hwnd);
        // Побудка egui может не дойти: после кадра спрятанного окна egui считает перерисовку уже
        // запрошенной. Поэтому — прямо WM_PAINT.
        sys::invalidate(hwnd);
        self.ctx.request_repaint();
    }

    /// «Выход» из меню трея: закрыть окно по-настоящему (обычно закрытие его только прячет).
    pub fn exit(&self) {
        self.exiting.store(true, Ordering::SeqCst);
        sys::post_close(self.hwnd.load(Ordering::SeqCst));
    }
}

pub struct Flyout {
    /// Ядро звука; в трее оно общее с значком (тот же `Control`).
    _audio: Audio,
    control: Control,
    hwnd: isize,
    shared: Arc<Shared>,
    /// Отдельный процесс: закрытие окна — выход.
    standalone: bool,
    spot: Option<Spot>,
    want_h: f32,
    /// Насколько раскрыты строки программ в этом кадре (прибавляется к высоте окна) и сколько насчитали за кадр.
    row_extra: f32,
    row_extra_next: f32,
    by_click: bool,
    was_focused: bool,
    /// Снять плащ после ближайшего `Present` — кадр уже на месте.
    reveal: bool,
    /// Первый кадр в трее: окно показал цикл окна — спрятать.
    started: bool,
    frames: u32,
    held: HashMap<Target, (f32, Instant)>,
    icons: HashMap<String, Option<TextureHandle>>,
    page: Page,
    /// Профиль, у которого нажали «Удалить» — ждёт подтверждения.
    deleting: Option<usize>,
    /// Раздел программы, открытый из её меню (путь exe в нижнем регистре).
    focus_app: Option<String>,
    /// Какое сочетание сейчас записывается.
    capture: Option<Slot>,
    /// Высота страницы настроек по прошлому кадру.
    page_h: f32,
    /// Начало въезда окна; `None` — окно на месте.
    slide: Option<Instant>,
    /// Полный снимок (со скрытым) — для раздела «Скрытое».
    all: Snapshot,
    /// Это окно микшера (`--mixer`): обычное окно, устройства колонками.
    mixer: bool,
    /// Куда поставили окно в последний раз (x, y, ширина, высота) — сверка после смены масштаба.
    placed: std::cell::Cell<Option<[i32; 4]>>,
    /// Окно микшера: под какой размер содержимого (физические пиксели) его подогнали в последний раз.
    fitted: std::cell::Cell<Option<[i32; 2]>>,
    /// Блоки устройств на этом кадре — цели перетаскивания.
    targets: Vec<(String, Flow, Rect)>,
    /// Знак в «О программе».
    about: AboutMark,
    /// Окно микшера: каким цветом стоит его значок и был ли у окна фокус на прошлом кадре.
    icon_color: Option<IconColor>,
    had_focus: bool,
    /// Запрос в поиске по «Настройкам».
    search: String,
    /// Плашка громкости: где поставлена; `None` — окно не в режиме плашки.
    osd_at: Option<OsdPlace>,
    /// Снять плащ после ближайшего `Present` — кадр плашки уже на месте.
    osd_reveal: bool,
}

/// Где стоит плашка громкости: левый верхний угол и размер (физические пиксели), масштаб, сторона панели
/// задач и начало сдвига при появлении.
#[derive(Debug, Clone, Copy)]
struct OsdPlace {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    scale: f32,
    edge: Edge,
    slide: Option<Instant>,
}

/// Знак 56 в «О программе»: фейдеры при первом показе раздела после открытия «Настроек», в покое — как файл.
struct AboutMark {
    clock: Instant,
    scene: MarkScene,
    played: bool,
    texture: Option<TextureHandle>,
    /// Что сейчас в текстуре: размер в пикселях, цвет и положения ручек.
    shown: Option<(u32, IconColor, [i32; 2])>,
}

impl AboutMark {
    fn new() -> AboutMark {
        AboutMark {
            clock: Instant::now(),
            // Знак 56 и крупнее — полная геометрия.
            scene: MarkScene::new(mark_render::home(64), mark::STRIKE_HZ, true),
            played: false,
            texture: None,
            shown: None,
        }
    }
}

/// Тема из настроек; `KAMERTON_THEME` (проверка вида) важнее.
fn theme_choice(pref: ThemePref) -> anvil_ui::ThemeChoice {
    match std::env::var("KAMERTON_THEME").ok().as_deref() {
        Some("light") => return anvil_ui::ThemeChoice::Light,
        Some("dark") => return anvil_ui::ThemeChoice::Dark,
        _ => {}
    }
    match pref {
        ThemePref::System => anvil_ui::ThemeChoice::System,
        ThemePref::Light => anvil_ui::ThemeChoice::Light,
        ThemePref::Dark => anvil_ui::ThemeChoice::Dark,
    }
}

fn kit_lang() -> anvil_ui::Lang {
    if crate::texts::english() { anvil_ui::Lang::En } else { anvil_ui::Lang::Ru }
}

fn install_look(ctx: &egui::Context, settings: &Settings) {
    crate::texts::apply(settings.lang);
    anvil_ui::install(ctx, anvil_ui::Accent::BLUE, theme_choice(settings.theme));
    anvil_ui::lang::set_language(ctx, kit_lang());
    anvil_ui::motion::install(ctx);
}

impl Flyout {
    /// Окно в процессе трея: создаётся при старте спрятанным под плащом и ждёт щелчка.
    pub fn in_tray(ctx: &egui::Context, hwnd: isize, audio: Audio, snapshot: &Snapshot, shared: Arc<Shared>) -> Flyout {
        sys::cloak(hwnd, true);
        install_look(ctx, &shared.settings.lock().unwrap());
        let mut f = Flyout::make(audio, hwnd, shared, false, 0.0);
        f.want_h = content_height(snapshot, f.shared.expanded());
        f
    }

    /// Отдельный процесс `--flyout`: открыто сразу. `anchor` — прямоугольник значка; `None` — угол рабочей
    /// области основного монитора. `passive` — для проверки вида: не брать фокус и не закрываться без него.
    pub fn standalone(
        ctx: &egui::Context,
        hwnd: isize,
        anchor: Option<PxRect>,
        passive: bool,
    ) -> Result<Flyout, String> {
        let wake_ctx = ctx.clone();
        let (audio, snapshot) = Audio::start_quiet(move || wake_ctx.request_repaint()).map_err(|e| e.to_string())?;
        let by_click = anchor.is_some() && !passive;
        let anchor = anchor.unwrap_or_else(|| {
            let (_, work, _) = sys::monitor_at(0, 0);
            PxRect { left: work.right - 1, top: work.bottom - 1, right: work.right, bottom: work.bottom }
        });
        let shared = Arc::new(Shared::new(ctx.clone(), hwnd));
        install_look(ctx, &shared.settings.lock().unwrap());
        *shared.pending.lock().unwrap() = Some((anchor, by_click));
        shared.state.store(OPENING, Ordering::SeqCst);
        let mut f = Flyout::make(audio, hwnd, shared, true, 0.0);
        f.want_h = content_height(&snapshot, f.shared.expanded());
        // Проверка вида: `KAMERTON_PAGE=hotkeys|profiles` — сразу страница настроек.
        match std::env::var("KAMERTON_PAGE").as_deref() {
            Ok("hotkeys") => f.page = Page::Hotkeys,
            Ok("profiles") => f.page = Page::Profiles,
            Ok("rules") => f.page = Page::Rules,
            // `KAMERTON_SEARCH=<запрос>` — страница с запросом в поиске.
            Ok("settings") => {
                f.page = Page::Settings;
                f.search = std::env::var("KAMERTON_SEARCH").unwrap_or_default();
            }
            // Вид плашки громкости: `KAMERTON_OSD=<проценты>[m]` (m — «без звука»), держится час.
            Ok("osd") => {
                let spec = std::env::var("KAMERTON_OSD").unwrap_or_default();
                let volume: f32 = spec.trim_end_matches('m').parse().unwrap_or(45.0);
                *f.shared.pending.lock().unwrap() = None;
                *f.shared.osd.lock().unwrap() = Some(OsdInfo {
                    anchor,
                    volume: volume / 100.0,
                    muted: spec.ends_with('m'),
                    name: std::env::var("KAMERTON_OSD_NAME").ok(),
                    until: Instant::now() + Duration::from_secs(3600),
                });
                f.shared.state.store(OSD, Ordering::SeqCst);
            }
            _ => {}
        }
        if f.page != Page::Mixer {
            f.want_h = f.page_h;
        }
        Ok(f)
    }

    fn make(audio: Audio, hwnd: isize, shared: Arc<Shared>, standalone: bool, want_h: f32) -> Flyout {
        Flyout {
            control: audio.control(),
            _audio: audio,
            hwnd,
            shared,
            standalone,
            spot: None,
            want_h,
            row_extra: 0.0,
            row_extra_next: 0.0,
            by_click: false,
            was_focused: false,
            reveal: false,
            started: false,
            frames: 0,
            held: HashMap::new(),
            icons: HashMap::new(),
            page: Page::Mixer,
            focus_app: None,
            deleting: None,
            capture: None,
            // Пока страница не нарисована ни разу — примерная высота, чтобы не мигнуть крошечным окном.
            page_h: 460.0,
            slide: None,
            all: Snapshot::default(),
            mixer: false,
            placed: std::cell::Cell::new(None),
            fitted: std::cell::Cell::new(None),
            targets: Vec::new(),
            about: AboutMark::new(),
            icon_color: None,
            had_focus: false,
            search: String::new(),
            osd_at: None,
            osd_reveal: false,
        }
    }

    /// Окно микшера (`kamerton --mixer`): обычное окно, все устройства колонками.
    pub fn mixer_window(ctx: &egui::Context, hwnd: isize) -> Result<Flyout, String> {
        let wake_ctx = ctx.clone();
        let (audio, _) = Audio::start_quiet(move || wake_ctx.request_repaint()).map_err(|e| e.to_string())?;
        let shared = Arc::new(Shared::new(ctx.clone(), hwnd));
        install_look(ctx, &shared.settings.lock().unwrap());
        if let Some(r) = shared.settings.lock().unwrap().mixer_rect
            && std::env::var_os("KAMERTON_SNAP").is_none()
        {
            sys::place_normal(hwnd, r);
        }
        shared.state.store(OPEN, Ordering::SeqCst);
        let color = shared.settings.lock().unwrap().tray_color;
        sys::set_window_icon(hwnd, color);
        let mut f = Flyout::make(audio, hwnd, shared, true, 0.0);
        f.mixer = true;
        f.icon_color = Some(color);
        Ok(f)
    }

    /// Окно микшера: колонка на устройство — строка устройства, баланс, программы.
    fn mixer_ui(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let Ok(all) = self.control.snapshot() else { return };
        // Выдуманные устройства и программы для снимков (`KAMERTON_DEMO_MIXER=1`): без настоящих имён.
        let all = demo_snapshot().unwrap_or(all);
        let snapshot = self.shared.visible(&all);
        self.all = all;
        let peaks: HashMap<Target, f32> = self.control.peaks().into_iter().collect();
        let p = crate::contrast::palette(&ctx);
        self.targets.clear();
        let frame = egui::Frame::new().fill(p.bg).inner_margin(egui::Margin::same(MIXER_MARGIN as i8));
        let mut content = Vec2::ZERO;
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
                content = ui
                    .horizontal_top(|ui| {
                        // Группы «Вывод» и «Ввод» — каждая со своей подписью, между ними 24; колонки — через 12.
                        ui.spacing_mut().item_spacing = vec2(MIXER_GROUP_GAP, 0.0);
                        let groups = [(T::Output, outputs(&snapshot)), (T::Input, of_flow(&snapshot, Flow::Capture))];
                        for (label, devs) in groups {
                            if devs.is_empty() {
                                continue;
                            }
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing = vec2(0.0, 6.0);
                                anvil_ui::widgets::section_label(ui, label.get());
                                ui.horizontal_top(|ui| {
                                    ui.spacing_mut().item_spacing = vec2(12.0, 0.0);
                                    for d in devs {
                                        self.mixer_column(ui, d, &snapshot, &peaks);
                                    }
                                });
                            });
                        }
                    })
                    .response
                    .rect
                    .size();
            });
        });
        self.fit_mixer(&ctx, content);
        self.finish_drag(&ctx);
        if self.snap_check(&ctx) {
            return;
        }
        ctx.request_repaint_after(Duration::from_millis(33));
    }

    /// Колонка устройства в окне микшера: строка устройства, баланс, программы. Вся колонка — цель
    /// перетаскивания.
    fn mixer_column(&mut self, ui: &mut Ui, d: &Device, snapshot: &Snapshot, peaks: &HashMap<Target, f32>) {
        ui.allocate_ui_with_layout(
            vec2(MIXER_COLUMN, ui.available_height()),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                anvil_ui::widgets::card(ui, |ui| {
                    ui.set_width(MIXER_COLUMN - 32.0);
                    ui.spacing_mut().item_spacing = Vec2::ZERO;
                    let top = ui.cursor().top();
                    let peak = peaks.get(&Target::Device(d.id.clone())).copied().unwrap_or(0.0);
                    self.device_row(ui, d, peak);
                    if let Some(b) = d.balance {
                        self.balance_row(ui, d, b);
                    }
                    let apps = shown_apps(d);
                    if apps.is_empty() {
                        note_row(ui, T::NoApps.get());
                    }
                    for a in apps {
                        let peak = peaks.get(&Target::App(a.key.clone())).copied().unwrap_or(0.0);
                        self.app_row(ui, a, d.flow, snapshot, peak, None, d.muted);
                    }
                    let block = Rect::from_min_max(
                        pos2(ui.min_rect().left(), top),
                        pos2(ui.min_rect().right(), ui.cursor().top()),
                    );
                    self.targets.push((d.id.clone(), d.flow, block));
                });
            },
        );
    }

    /// Окно микшера по размеру колонок: сколько устройств — столько колонок, высота — по самой длинной.
    /// Больше экрана не растёт: дальше — прокрутка.
    fn fit_mixer(&self, ctx: &egui::Context, content: Vec2) {
        let ppp = ctx.pixels_per_point();
        let w = (content.x + 2.0 * MIXER_MARGIN).max(MIXER_MIN[0]);
        let h = (content.y + 2.0 * MIXER_MARGIN).max(MIXER_MIN[1]);
        let want = [(w * ppp).ceil() as i32, (h * ppp).ceil() as i32];
        if self.fitted.get() == Some(want) {
            return;
        }
        self.fitted.set(Some(want));
        // Снимок для проверки рисуется за экранами — туда окно и остаётся.
        sys::fit_client(self.hwnd, want, std::env::var_os("KAMERTON_SNAP").is_none());
    }

    /// Баланс устройства: «Баланс» и значение («Центр», «Л 20», «П 35»), под ними «Л» — ползунок — «П».
    /// Полоска идёт от середины к ручке, а не от левого края: иначе баланс похож на громкость 50 %.
    fn balance_row(&mut self, ui: &mut Ui, d: &Device, b: f32) {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), BALANCE_ROW), Sense::hover());
        let p = crate::contrast::palette(ui.ctx());
        ui.painter().text(
            pos2(rect.left(), rect.top() + 9.0),
            Align2::LEFT_CENTER,
            T::Balance.get(),
            FontId::proportional(12.0),
            p.weak,
        );
        let target = Target::Device(format!("balance:{}", d.id));
        let shown = self.shown_value(&target, (b + 1.0) / 2.0);
        ui.painter().text(
            pos2(rect.right() - 36.0, rect.top() + 9.0),
            Align2::RIGHT_CENTER,
            balance_text(shown * 2.0 - 1.0),
            FontId::proportional(12.0),
            p.weak,
        );
        let line = rect.top() + 26.0;
        ui.painter().text(
            pos2(rect.left(), line),
            Align2::LEFT_CENTER,
            T::BalanceL.get(),
            FontId::proportional(12.0),
            p.weak,
        );
        ui.painter().text(
            pos2(rect.right() - 44.0, line),
            Align2::LEFT_CENTER,
            T::BalanceR.get(),
            FontId::proportional(12.0),
            p.weak,
        );
        let track = Rect::from_min_max(pos2(rect.left() + 14.0, line - 8.0), pos2(rect.right() - 52.0, line + 8.0));
        // Ползунок окна работает с 0..1: 0 — Л, 0.5 — середина, 1 — П.
        let out = slider_scaled(ui, ui.id().with(("balance", &d.id)), track, shown, false, 0.0, false, 0.5);
        if let Some(v) = out.value {
            // Возле середины — ровно середина: так проще вернуть оба канала.
            let v = if (v - 0.5).abs() < 0.03 { 0.5 } else { v };
            self.held.insert(target, (v, Instant::now()));
            let _ = self.control.set_balance(&d.id, v * 2.0 - 1.0);
        }
    }

    /// Поставить окно по месту и желаемой высоте; во время въезда — со сдвигом от панели задач.
    fn layout(&self) {
        let Some(s) = &self.spot else { return };
        let w = (WIDTH * s.scale).round() as i32;
        let gap = (placement::GAP * s.scale).round() as i32;
        // Высота — вверх, а не до ближайшего: окно на долю пикселя ниже содержимого показывает полосу прокрутки.
        let (mut x, mut y, h) =
            placement::place(s.anchor, s.work, s.edge, (w, (self.want_h * s.scale).ceil() as i32), gap);
        if let Some(start) = self.slide {
            let t = (start.elapsed().as_secs_f32() / SLIDE_SECS).min(1.0);
            let off = ((1.0 - anvil_ui::motion::curve::ease_out(t)) * SLIDE_PX * s.scale).round() as i32;
            match s.edge {
                Edge::Bottom => y += off,
                Edge::Top => y -= off,
                Edge::Left => x -= off,
                Edge::Right => x += off,
            }
        }
        if std::env::var_os("KAMERTON_TRACE").is_some() {
            eprintln!(
                "trace: flyout at {x},{y} size {w}x{h}, anchor {:?}, edge {:?}, scale {}",
                s.anchor, s.edge, s.scale
            );
        }
        sys::move_to(self.hwnd, x, y, w, h);
        self.placed.set(Some([x, y, w, h]));
        sys::round_corners(self.hwnd, w, h, (radius::WINDOW as f32 * s.scale).round() as i32);
    }

    /// Плашка громкости: маленькое окно у панели задач по центру, без фокуса, на `OSD_SHOW` после последнего
    /// изменения. Громкость, «без звука» и время берёт из `Shared::osd`.
    fn osd_ui(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let snapping = std::env::var_os("KAMERTON_SNAP").is_some();
        let Some(info) = self.shared.osd.lock().unwrap().clone() else {
            self.hide_osd();
            return;
        };
        let now = Instant::now();
        if now >= info.until {
            self.hide_osd();
            return;
        }
        let h_logical = if info.name.is_some() { OSD_H_NAMED } else { OSD_H };
        // Плашка с именем выше обычной: сменилась — ставим окно заново.
        let stale = self.osd_at.is_some_and(|p| p.h != (h_logical * p.scale).round() as i32);
        let fresh = self.osd_at.is_none() || stale;
        if std::env::var_os("KAMERTON_TRACE").is_some() {
            eprintln!("trace: osd frame, fresh {fresh}");
        }
        let place = match self.osd_at.filter(|_| !stale) {
            Some(p) => p,
            None => {
                let spot = Spot::at(info.anchor);
                let s = spot.scale;
                let (w, h) = ((OSD_W * s).round() as i32, (h_logical * s).round() as i32);
                let (x, y) = placement::place_osd(spot.work, spot.edge, (w, h), (OSD_GAP * s).round() as i32);
                let animate =
                    self.shared.settings.lock().unwrap().animate && anvil_ui::motion::pref::animations_enabled();
                sys::set_no_activate(self.hwnd, true);
                sys::round_corners(self.hwnd, w, h, (radius::WINDOW as f32 * s).round() as i32);
                let p = OsdPlace { x, y, w, h, scale: s, edge: spot.edge, slide: animate.then_some(now) };
                self.osd_at = Some(p);
                self.osd_reveal = true;
                p
            }
        };
        // Появление: сдвиг от панели задач на `OSD_SLIDE_PX` и на место, по кривой набора.
        let mut off = 0;
        if let Some(start) = place.slide {
            let t = (start.elapsed().as_secs_f32() / SLIDE_SECS).min(1.0);
            off = ((1.0 - anvil_ui::motion::curve::ease_out(t)) * OSD_SLIDE_PX * place.scale).round() as i32;
            if t >= 1.0 {
                self.osd_at = Some(OsdPlace { slide: None, ..place });
            } else {
                ctx.request_repaint();
            }
        }
        if fresh || place.slide.is_some() {
            if std::env::var_os("KAMERTON_TRACE").is_some() {
                eprintln!("trace: osd at {},{} size {}x{}, offset {off}", place.x, place.y, place.w, place.h);
            }
            let dy = if place.edge == Edge::Top { -off } else { off };
            sys::move_to(self.hwnd, place.x, place.y + dy, place.w, place.h);
        }

        let log = self.shared.settings.lock().unwrap().log_scale;
        let p = crate::contrast::palette(&ctx);
        let frame =
            egui::Frame::new().fill(p.card).stroke(Stroke::new(1.0, p.border_strong)).corner_radius(radius::WINDOW);
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            let r = ui.max_rect();
            // С именем устройства: имя сверху, ползунок ниже.
            let cy = if info.name.is_some() { r.bottom() - OSD_H / 2.0 } else { r.center().y };
            if let Some(name) = &info.name {
                let line =
                    Rect::from_min_max(pos2(r.left() + PAD_X, r.top() + 6.0), pos2(r.right() - PAD_X, r.top() + 24.0));
                text_clipped(ui, line, name, FontId::proportional(11.5), p.weak);
            }
            let (fill, icon_color, number) =
                if info.muted { (p.faint, p.warning, p.faint) } else { (p.accent, p.text, p.weak) };
            let icon = Rect::from_center_size(pos2(r.left() + PAD_X + ICON / 2.0, cy), Vec2::splat(ICON));
            icons::speaker(ui.painter(), icon, icon_color, info.volume, info.muted);
            let right = r.right() - PAD_X;
            ui.painter().text(
                pos2(right, cy),
                Align2::RIGHT_CENTER,
                format!("{}", (info.volume * 100.0).round() as i32),
                FontId::monospace(12.5),
                number,
            );
            let track =
                Rect::from_min_max(pos2(icon.right() + 12.0, cy - 2.0), pos2(right - NUMBER_W - 12.0, cy + 2.0));
            ui.painter().rect_filled(track, CornerRadius::same(2), p.border_strong);
            let done = Rect::from_min_max(
                track.min,
                pos2(track.left() + track.width() * model::to_pos(info.volume, log), track.bottom()),
            );
            if done.width() > 0.5 {
                ui.painter().rect_filled(done, CornerRadius::same(2), fill);
            }
        });
        if snapping && self.snap_check(&ctx) {
            return;
        }
        ctx.request_repaint_after(info.until.saturating_duration_since(now));
    }

    /// Плашка закончилась: спрятать окно под плащ.
    fn hide_osd(&mut self) {
        self.end_osd();
        *self.shared.osd.lock().unwrap() = None;
        if self.shared.state.compare_exchange(OSD, HIDDEN, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
            sys::cloak(self.hwnd, true);
            sys::hide(self.hwnd);
        }
    }

    /// Окно больше не плашка (она закончилась или открывают окно микшера): снова берёт фокус от щелчка.
    fn end_osd(&mut self) {
        if self.osd_at.take().is_some() {
            sys::set_no_activate(self.hwnd, false);
        }
        self.osd_reveal = false;
    }

    /// Закрыть: в трее — под плащ и спрятать; отдельный процесс — выйти.
    fn close(&mut self, ctx: &egui::Context) {
        if self.standalone {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        sys::cloak(self.hwnd, true);
        sys::hide(self.hwnd);
        self.shared.ghost.store(false, Ordering::SeqCst);
        self.placed.set(None);
        self.held.clear();
        self.page = Page::Mixer;
        self.search.clear();
        self.stop_capture();
        self.reveal = false;
        self.shared.close.store(false, Ordering::SeqCst);
        self.shared.hidden_at.store(sys::now_ms(), Ordering::SeqCst);
        self.shared.state.store(HIDDEN, Ordering::SeqCst);
    }

    fn shown_value(&mut self, target: &Target, actual: f32) -> f32 {
        match self.held.get(target) {
            Some((v, at)) if at.elapsed() < HOLD => *v,
            _ => {
                self.held.remove(target);
                actual
            }
        }
    }

    fn set_volume(&mut self, target: Target, v: f32) {
        self.held.insert(target.clone(), (v, Instant::now()));
        let _ = self.control.set_volume(target, v);
    }

    fn icon_for(&mut self, ctx: &egui::Context, app: &App) -> Option<TextureHandle> {
        let path = app.path.clone()?;
        self.icons
            .entry(path.clone())
            .or_insert_with(|| {
                let (w, h, rgba) = icons::exe_icon(&path)?;
                let img = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
                Some(ctx.load_texture(path, img, TextureOptions::LINEAR))
            })
            .clone()
    }
}

impl crate::shell::Content for Flyout {
    fn clear(&self, ctx: &egui::Context) -> Color32 {
        let p = crate::contrast::palette(ctx);
        if self.mixer { p.bg } else { p.card }
    }

    fn after_present(&mut self) {
        if !self.started {
            self.started = true;
            // Первый кадр в трее: цикл окна только что показал окно (под плащом) — спрятать до щелчка.
            if !self.standalone && self.shared.state.load(Ordering::SeqCst) == HIDDEN {
                sys::hide(self.hwnd);
            }
        }
        if self.osd_reveal {
            self.osd_reveal = false;
            sys::cloak(self.hwnd, false);
        }
        if self.reveal {
            self.reveal = false;
            if !self.shared.ghost.load(Ordering::SeqCst) {
                sys::cloak(self.hwnd, false);
            }
            if std::env::var_os("KAMERTON_TRACE").is_some() {
                eprintln!("trace: revealed at {} ms", sys::now_ms() % 100_000);
            }
            if self.by_click {
                sys::activate(self.hwnd);
            }
            self.shared.state.store(OPEN, Ordering::SeqCst);
        }
    }

    fn close_requested(&mut self) -> bool {
        // Окно микшера: запомнить место (размер подгоняется под содержимое).
        if self.mixer {
            if let Some(r) = sys::window_rect(self.hwnd) {
                let mut st = self.shared.settings.lock().unwrap();
                st.mixer_rect = Some([r.left, r.top, r.right - r.left, r.bottom - r.top]);
                st.save();
            }
            return true;
        }
        // Alt+F4 только прячет окно; закрывает программу «Выход» в трее.
        if self.standalone || self.shared.exiting.load(Ordering::SeqCst) {
            return true;
        }
        self.close(&self.shared.ctx.clone());
        false
    }

    fn ui(&mut self, ui: &mut Ui) {
        let ctx = &ui.ctx().clone();
        anvil_ui::motion::tick(ctx);
        let theme = self.shared.settings.lock().unwrap().theme;
        crate::contrast::follow(ctx, theme_choice(theme));
        let log = self.shared.settings.lock().unwrap().log_scale;
        ctx.data_mut(|d| d.insert_temp(egui::Id::new("kamerton-log-scale"), log));
        if self.mixer {
            self.follow_icon_color(ctx);
            self.mixer_ui(ui);
            return;
        }
        let opening = self.shared.pending.lock().unwrap().take();
        if let Some((anchor, by_click)) = opening {
            self.end_osd();
            self.spot = Some(Spot::at(anchor));
            self.by_click = by_click;
            self.was_focused = false;
            self.frames = 0;
            self.reveal = true;
            let animate = self.shared.settings.lock().unwrap().animate;
            self.slide = (animate && anvil_ui::motion::pref::animations_enabled()).then(Instant::now);
            if let Ok(s) = self.control.snapshot() {
                self.want_h = content_height(&self.shared.visible(&s), self.shared.expanded());
            }
            self.layout();
        }
        if self.shared.state.load(Ordering::SeqCst) == OSD {
            self.osd_ui(ui);
            return;
        }
        // Первый кадр в трее рисуем целиком (окно за экранами и под плащом): прогрев — значки программ,
        // глифы шрифтов, текстуры. Иначе первый щелчок ждал бы их ~0,2 с.
        let warming = !self.started && !self.standalone;
        if self.shared.state.load(Ordering::SeqCst) == HIDDEN && !warming {
            self.row_extra_next = 0.0;
            return;
        }
        self.frames += 1;
        let focused = ctx.input(|i| i.viewport().focused).unwrap_or(false);
        let lost = self.by_click && self.was_focused && !focused;
        let asked = self.shared.close.load(Ordering::SeqCst);
        // Esc на «Настройках» с запросом в поиске очищает запрос, а не закрывает окно.
        let searching = self.page == Page::Settings && !self.search.is_empty();
        let esc = self.capture.is_none() && !searching && ctx.input(|i| i.key_pressed(Key::Escape));
        if !warming && (lost || asked || esc) {
            self.close(ctx);
            return;
        }
        self.was_focused |= focused;

        let Ok(all) = self.control.snapshot() else { return };
        // Выдуманные устройства и программы для снимков (`KAMERTON_DEMO_MIXER=1`): без настоящих имён.
        let all = demo_snapshot().unwrap_or(all);
        let snapshot = self.shared.visible(&all);
        let peaks: HashMap<Target, f32> = self.control.peaks().into_iter().collect();
        // Windows сама поменяла размер или место (смена масштаба монитора, EarTrumpet #1304) — на место.
        if self.slide.is_none()
            && let (Some([x, y, w, h]), Some(r)) = (self.placed.get(), sys::window_rect(self.hwnd))
            && (r.left != x || r.top != y || r.right - r.left != w || r.bottom - r.top != h)
        {
            self.layout();
        }
        if let Some(start) = self.slide {
            self.layout();
            if start.elapsed().as_secs_f32() >= SLIDE_SECS {
                self.slide = None;
            }
            ctx.request_repaint();
        }
        self.all = all;
        // Раскрытые строки программ (подпись «→ устройство» целиком) растят окно: их насчитал прошлый кадр.
        self.row_extra = std::mem::take(&mut self.row_extra_next);
        let want = if self.page != Page::Mixer {
            self.page_h
        } else {
            content_height(&snapshot, self.shared.expanded()) + self.row_extra
        };
        if (self.want_h - want).abs() > 0.5 {
            self.want_h = want;
            self.layout();
        }

        let p = crate::contrast::palette(ctx);
        let frame = egui::Frame::new()
            .fill(p.card)
            .stroke(Stroke::new(1.0, p.border_strong))
            .corner_radius(radius::WINDOW)
            .inner_margin(egui::Margin::symmetric(PAD_X as i8, PAD_Y as i8));
        // Страница микшера: поля у каждой части свои (список устройств — 4, остальное — 16), полоса внизу
        // во всю ширину.
        let bare = frame.inner_margin(egui::Margin::ZERO);
        self.targets.clear();
        if self.page != Page::Settings {
            // Ушли со страницы «Настройки» — в следующий раз знак в «О программе» сыграет снова.
            self.about.played = false;
        }
        if self.page != Page::Mixer {
            let shown = egui::CentralPanel::default()
                .frame(frame)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| match self.page {
                            Page::Profiles => self.profiles_page(ui, &snapshot),
                            Page::Rules => self.rules_page(ui, &snapshot),
                            Page::Settings => self.settings_page(ui),
                            _ => self.hotkeys_page(ui, &snapshot),
                        })
                        .content_size
                        .y
                })
                .inner;
            let h = shown + 2.0 * PAD_Y + 2.0;
            if (self.page_h - h).abs() > 0.5 {
                self.page_h = h;
                ctx.request_repaint();
            }
            if !self.snap_check(ctx) {
                ctx.request_repaint_after(Duration::from_millis(250));
            }
            return;
        }
        egui::CentralPanel::default().frame(bare).show(ui, |ui| {
            let full = ui.max_rect();
            let foot = Rect::from_min_max(pos2(full.left(), full.bottom() - FOOTER), full.max);
            let body = Rect::from_min_max(full.min, pos2(full.right(), foot.top()));
            ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.spacing_mut().item_spacing = Vec2::ZERO;
                    ui.add_space(TOP);
                    let outs = outputs(&snapshot);
                    let ins = if self.shared.expanded() { of_flow(&snapshot, Flow::Capture) } else { Vec::new() };
                    if outs.is_empty() && ins.is_empty() {
                        padded(ui, PAD_X, |ui| note_row(ui, T::NoDevices.get()));
                    }
                    if !outs.is_empty() {
                        self.section(ui, &outs, T::OutputDevice.get(), &peaks, &snapshot, true);
                    }
                    if !ins.is_empty() {
                        if !outs.is_empty() {
                            divider_row(ui);
                        }
                        self.section(ui, &ins, T::InputDevice.get(), &peaks, &snapshot, false);
                    }
                });
            });
            self.footer(ui, foot);
        });
        self.finish_drag(ctx);
        if self.snap_check(ctx) {
            return;
        }
        // Уровни живые, пока окно открыто.
        ctx.request_repaint_after(Duration::from_millis(33));
    }
}

fn note_row(ui: &mut Ui, text: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), EMPTY_ROW), Sense::hover());
    let p = crate::contrast::palette(ui.ctx());
    ui.painter().text(
        pos2(rect.left(), rect.center().y),
        Align2::LEFT_CENTER,
        text,
        FontId::proportional(13.0),
        p.weak,
    );
}

/// Значение баланса словами (−1 … 1): «Центр», «Л 20», «П 35».
fn balance_text(b: f32) -> String {
    let n = (b.abs() * 100.0).round() as u32;
    if n == 0 {
        T::BalanceCenter.get().to_string()
    } else {
        format!("{} {n}", if b < 0.0 { T::BalanceL.get() } else { T::BalanceR.get() })
    }
}

/// Имя программы в окне: «Системные звуки» — на языке окна; без имени — «Неизвестная программа» (номер
/// процесса — мелко рядом, в строке программы).
fn app_name(a: &App) -> String {
    if a.system {
        T::SystemSounds.get().to_string()
    } else if a.name.is_empty() {
        T::UnknownApp.get().to_string()
    } else {
        a.name.clone()
    }
}

/// Что показывает окно.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Mixer,
    Hotkeys,
    Profiles,
    Rules,
    Settings,
}

/// Поле сочетания: общее действие или действие программы.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Slot {
    Global(Action),
    App(AppAction, String),
    /// Сочетание устройства (id) — сделать его основным.
    Device(String),
    /// Громкость устройства вывода (id): «Громче», «Тише», «Без звука».
    DeviceVol(AppAction, String),
}

impl Slot {
    /// Ключ в `settings.txt` — он же ключ состояния регистрации.
    fn key(&self) -> String {
        match self {
            Slot::Global(a) => format!("hotkey.{}", a.key()),
            Slot::App(a, exe) => format!("apphotkey.{}.{}", a.key(), exe),
            Slot::Device(id) => format!("devhotkey.{id}"),
            Slot::DeviceVol(a, id) => format!("devvolkey.{}.{id}", a.key()),
        }
    }
}

fn action_text(a: Action) -> &'static str {
    match a {
        Action::Open => T::ActOpen.get(),
        Action::Louder => T::ActLouder.get(),
        Action::Quieter => T::ActQuieter.get(),
        Action::Mute => T::Mute.get(),
        Action::NextOutput => T::ActNext.get(),
        Action::PrevOutput => T::ActPrev.get(),
        Action::Mixer => T::ActMixer.get(),
    }
}

fn app_action_text(a: AppAction) -> &'static str {
    match a {
        AppAction::Louder => T::ActLouder.get(),
        AppAction::Quieter => T::ActQuieter.get(),
        AppAction::Mute => T::Mute.get(),
    }
}

impl Flyout {
    /// Снимок для проверки вида (`KAMERTON_SNAP`): через несколько кадров после показа; `true` — окно закрыто.
    fn snap_check(&mut self, ctx: &egui::Context) -> bool {
        let Ok(path) = std::env::var("KAMERTON_SNAP") else { return false };
        self.frames += 1;
        if self.frames == 10 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        let shot = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = shot {
            crate::snap::save(&path, &image);
            self.close(ctx);
            return true;
        }
        ctx.request_repaint();
        false
    }

    fn stop_capture(&mut self) {
        if self.capture.take().is_some() {
            self.shared.capturing.store(false, Ordering::SeqCst);
            crate::tray::reload_hotkeys();
        }
    }

    fn start_capture(&mut self, slot: Slot) {
        self.capture = Some(slot);
        self.shared.capturing.store(true, Ordering::SeqCst);
        crate::tray::reload_hotkeys();
    }

    /// Нажатие во время записи: Esc — отмена, Backspace — убрать, годное сочетание — записать.
    fn capture_keys(&mut self, ui: &Ui) {
        let Some(slot) = self.capture.clone() else { return };
        let press = ui.input(|i| {
            i.events.iter().find_map(|e| match e {
                Event::Key { key, pressed: true, modifiers, .. } => Some((*key, *modifiers)),
                _ => None,
            })
        });
        // Медиа-клавиши громкости egui не знает: их запомнил цикл окна.
        if let Some(media) = crate::hotkeys::take_media() {
            let mods = ui.input(|i| i.modifiers);
            if let Some(c) = Combo::from_media(media, mods) {
                self.store_combo(&slot, Some(c));
                self.stop_capture();
            }
            return;
        }
        let Some((key, mods)) = press else { return };
        let new = match key {
            Key::Escape => {
                self.stop_capture();
                return;
            }
            Key::Backspace => None,
            _ => match Combo::from_press(key, mods) {
                Some(c) => Some(c),
                None => return,
            },
        };
        self.store_combo(&slot, new);
        self.stop_capture();
    }

    /// Записать (или убрать) сочетание ячейки в настройки.
    fn store_combo(&self, slot: &Slot, new: Option<Combo>) {
        let mut s = self.shared.settings.lock().unwrap();
        match (slot, new) {
            (Slot::Global(a), Some(c)) => {
                s.hotkeys.insert(*a, c);
            }
            (Slot::Global(a), None) => {
                s.hotkeys.remove(a);
            }
            (Slot::App(a, exe), Some(c)) => {
                s.app_hotkeys.insert((*a, exe.clone()), c);
            }
            (Slot::App(a, exe), None) => {
                s.app_hotkeys.remove(&(*a, exe.clone()));
            }
            (Slot::Device(id), Some(c)) => {
                s.device_hotkeys.insert(id.clone(), c);
            }
            (Slot::Device(id), None) => {
                s.device_hotkeys.remove(id);
            }
            (Slot::DeviceVol(a, id), Some(c)) => {
                s.device_vol_hotkeys.insert((*a, id.clone()), c);
            }
            (Slot::DeviceVol(a, id), None) => {
                s.device_vol_hotkeys.remove(&(*a, id.clone()));
            }
        }
        s.save();
    }

    /// Строка в два столбца: слева действие (у устройства — его значок), справа поле сочетания. Поле видно
    /// и без курсора (рамка `border`), ширина — по содержимому. Щелчок — запись; занятое другой
    /// программой — красная точка; не задано — «Не задано».
    fn hotkey_row(
        &mut self,
        ui: &mut Ui,
        name: &str,
        slot: Slot,
        combo: Option<Combo>,
        kind: Option<icons::DeviceKind>,
    ) {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), HOTKEY_ROW), Sense::hover());
        let p = crate::contrast::palette(ui.ctx());
        let capturing = self.capture.as_ref() == Some(&slot);
        let taken = combo.is_some() && self.shared.hotkey_free.lock().unwrap().get(&slot.key()).copied() == Some(false);
        let text_w = |text: &str, font: FontId| ui.painter().layout_no_wrap(text.to_string(), font, p.text).size().x;
        let content = if capturing {
            text_w(T::PressKeys.get(), FontId::proportional(12.5))
        } else if let Some(combo) = combo {
            let parts = combo.parts();
            let keys: f32 = parts.iter().map(|k| text_w(k, FontId::monospace(11.5)) + 10.0).sum();
            keys + 3.0 * (parts.len() as f32 - 1.0) + if taken { 12.0 } else { 0.0 }
        } else {
            text_w(T::NotSet.get(), FontId::proportional(12.5))
        };
        let width = (content + 14.0).clamp(HOTKEY_FIELD_MIN, HOTKEY_FIELD);
        let field =
            Rect::from_min_max(pos2(rect.right() - width, rect.top() + 3.0), pos2(rect.right(), rect.bottom() - 3.0));
        let mut left = rect.left();
        if let Some(kind) = kind {
            let glyph = Rect::from_center_size(pos2(left + 7.5, rect.center().y), Vec2::splat(15.0));
            icons::device(ui.painter(), glyph, p.text, kind);
            left = glyph.right() + 8.0;
        }
        let label = Rect::from_min_max(pos2(left, rect.top()), pos2(field.left() - 10.0, rect.bottom()));
        text_clipped(ui, label, name, FontId::proportional(13.5), p.text);

        let response = ui.interact(field, ui.id().with(("hotkey", slot.key())), Sense::click());
        if capturing {
            ui.painter().rect_filled(field, radius::CONTROL, p.soft(p.accent));
            ui.painter().rect_stroke(field, radius::CONTROL, Stroke::new(1.5, p.accent), StrokeKind::Inside);
        } else {
            if response.hovered() {
                ui.painter().rect_filled(field, radius::CONTROL, p.hover);
            }
            ui.painter().rect_stroke(field, radius::CONTROL, Stroke::new(1.0, p.border), StrokeKind::Inside);
        }
        let mut x = field.right() - 6.0;
        let y = field.center().y;
        if capturing {
            ui.painter().text(
                pos2(x, y),
                Align2::RIGHT_CENTER,
                T::PressKeys.get(),
                FontId::proportional(12.5),
                p.accent_text,
            );
        } else if let Some(combo) = combo {
            let color = if taken { p.text } else { p.weak };
            for part in combo.parts().iter().rev() {
                let galley = ui.painter().layout_no_wrap(part.clone(), FontId::monospace(11.5), color);
                let chip = Rect::from_min_max(pos2(x - galley.size().x - 10.0, y - 9.5), pos2(x, y + 9.5));
                ui.painter().rect(chip, radius::SMALL, p.raised, Stroke::new(1.0, p.border), StrokeKind::Inside);
                ui.painter().galley(chip.center() - galley.size() / 2.0, galley, color);
                x = chip.left() - 3.0;
            }
            if taken {
                ui.painter().circle_filled(pos2(x - 5.0, y), 3.5, p.danger);
            }
        } else {
            ui.painter().text(pos2(x, y), Align2::RIGHT_CENTER, T::NotSet.get(), FontId::proportional(12.5), p.faint);
        }
        anvil_ui::widgets::focus_ring(ui, field, &response, radius::CONTROL);
        let hint = if capturing { T::CaptureHint.get() } else { T::Change.get() };
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name));
        if response.on_hover_cursor(CursorIcon::PointingHand).on_hover_text(hint).clicked() && !capturing {
            self.stop_capture();
            self.start_capture(slot);
        }
    }

    fn settings_page(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        ui.horizontal(|ui| {
            if anvil_ui::widgets::icon_button(ui, anvil_ui::Icon::ArrowLeft, T::Back.get()).clicked() {
                self.page = Page::Mixer;
                self.search.clear();
            }
            ui.label(egui::RichText::new(T::Settings.get()).font(anvil_ui::semibold(14.5)));
        });
        ui.add_space(6.0);
        let mut st = self.shared.settings.lock().unwrap().clone();
        let before = st.clone();
        // Шапка: знак, версия и проверка обновления. При поиске остаётся, если подходит («версия», «обновления»).
        if settings_search::shown(&self.search, Item::About) {
            self.about_card(ui, &mut st);
            ui.add_space(6.0);
        }
        // Поиск по строкам страницы; Esc очищает запрос, а не закрывает окно (см. `ui`).
        if !self.search.is_empty() && ui.input(|i| i.key_pressed(Key::Escape)) {
            self.search.clear();
        }
        ui.add(
            egui::TextEdit::singleline(&mut self.search)
                .hint_text(T::SearchSettings.get())
                .desired_width(f32::INFINITY),
        );
        ui.add_space(6.0);
        let query = self.search.clone();
        let show = |i: Item| settings_search::shown(&query, i);

        // Переходы на другие страницы — первыми, строками со стрелкой, а не кнопками внутри раздела.
        let [hotkeys, profiles, rules] = nav_rows(ui, [show(Item::Hotkeys), show(Item::Profiles), show(Item::Rules)]);
        if hotkeys {
            self.page = Page::Hotkeys;
        }
        if profiles {
            self.page = Page::Profiles;
        }
        if rules {
            self.page = Page::Rules;
        }
        if !settings_search::any(&query, &settings_search::ALL) {
            anvil_ui::widgets::note(ui, T::NothingFound.get());
        }

        if settings_search::any(&query, &[Item::Theme, Item::Language]) {
            ui.add_space(10.0);
            anvil_ui::widgets::section_label(ui, T::Appearance.get());
        }
        if show(Item::Theme) {
            anvil_ui::chrome::setting_row(ui, T::Theme.get(), |ui| {
                segmented_even(
                    ui,
                    &mut st.theme,
                    &[
                        (ThemePref::System, Some(anvil_ui::Icon::Monitor), T::ThemeSystem.get()),
                        (ThemePref::Light, Some(anvil_ui::Icon::Sun), T::ThemeLight.get()),
                        (ThemePref::Dark, Some(anvil_ui::Icon::Moon), T::ThemeDark.get()),
                    ],
                );
            });
        }
        if show(Item::Language) {
            anvil_ui::chrome::setting_row(ui, T::Language.get(), |ui| {
                segmented_even(
                    ui,
                    &mut st.lang,
                    &[
                        (LangPref::System, Some(anvil_ui::Icon::Monitor), T::ThemeSystem.get()),
                        (LangPref::Ru, None, "Русский"),
                        (LangPref::En, None, "English"),
                    ],
                );
            });
        }

        // Значок — свой раздел: вид в трее и цвет, который берут трей, окно микшера и «О программе».
        if settings_search::any(&query, &[Item::TrayIcon, Item::IconColor]) {
            ui.add_space(10.0);
            anvil_ui::widgets::section_label(ui, T::IconSection.get());
        }
        if show(Item::TrayIcon) {
            anvil_ui::chrome::setting_row(ui, T::InTray.get(), |ui| {
                let tint = st.tray_color;
                segmented_glyphs(
                    ui,
                    &mut st.tray_icon,
                    &[
                        (TrayIcon::Mark, Glyph::Tray(TrayIcon::Mark, tint), T::TrayMark.get()),
                        (TrayIcon::Speaker, Glyph::Tray(TrayIcon::Speaker, tint), T::TraySpeaker.get()),
                    ],
                );
            });
        }
        if show(Item::IconColor) {
            anvil_ui::chrome::setting_row(ui, T::IconColor.get(), |ui| {
                segmented_glyphs(
                    ui,
                    &mut st.tray_color,
                    &[
                        (IconColor::Purple, Glyph::Dot(IconColor::Purple), T::ColorPurple.get()),
                        (IconColor::Azure, Glyph::Dot(IconColor::Azure), T::ColorAzure.get()),
                    ],
                );
            });
        }
        if settings_search::any(&query, &[Item::TrayIcon, Item::IconColor]) {
            anvil_ui::widgets::note(ui, T::IconColorNote.get());
        }

        if settings_search::any(
            &query,
            &[Item::Animation, Item::WheelOutside, Item::LogScale, Item::Osd, Item::MixerOnTop],
        ) {
            ui.add_space(10.0);
            anvil_ui::widgets::section_label(ui, T::WindowSection.get());
        }
        if show(Item::Animation) {
            anvil_ui::widgets::switch(ui, &mut st.animate, T::Animation.get());
        }
        if show(Item::WheelOutside) {
            anvil_ui::widgets::switch(ui, &mut st.wheel_outside, T::WheelOutside.get());
        }
        if show(Item::LogScale) {
            anvil_ui::widgets::switch(ui, &mut st.log_scale, T::LogScale.get());
        }
        if show(Item::Osd) {
            anvil_ui::widgets::switch(ui, &mut st.osd, T::Osd.get());
        }
        if show(Item::MixerOnTop) {
            anvil_ui::widgets::switch(ui, &mut st.mixer_on_top, T::MixerOnTop.get());
        }

        if settings_search::any(&query, &[Item::Autostart, Item::CheckUpdates, Item::Prerelease]) {
            ui.add_space(10.0);
            anvil_ui::widgets::section_label(ui, T::ThemeSystem.get());
        }
        if show(Item::Autostart) {
            let mut auto = sys::autostart();
            if anvil_ui::widgets::switch(ui, &mut auto, T::Autostart.get()).changed() {
                sys::set_autostart(auto);
            }
        }
        if show(Item::CheckUpdates) {
            anvil_ui::widgets::switch(ui, &mut st.check_updates, T::CheckUpdates.get());
        }
        if show(Item::Prerelease) {
            anvil_ui::widgets::switch(ui, &mut st.prerelease, T::Prerelease.get());
        }

        if show(Item::Hidden) {
            ui.add_space(10.0);
            anvil_ui::widgets::section_label(ui, T::Hidden.get());
            if st.hidden_apps.is_empty() && st.hidden_devices.is_empty() {
                anvil_ui::widgets::note(ui, T::NothingHidden.get());
            }
            let all = self.all.clone();
            let mut show_app = None;
            for exe in &st.hidden_apps {
                let name = all
                    .devices
                    .iter()
                    .flat_map(|d| d.apps.iter())
                    .find(|a| &a.key.exe == exe)
                    .map(app_name)
                    .unwrap_or_else(|| exe.rsplit(['\\', '/']).next().unwrap_or(exe).to_string());
                anvil_ui::chrome::setting_row(ui, &name, |ui| {
                    if anvil_ui::widgets::button(ui, anvil_ui::Kind::Ghost, None, T::Show.get()).clicked() {
                        show_app = Some(exe.clone());
                    }
                });
            }
            let mut show_dev = None;
            for id in &st.hidden_devices {
                let name = all
                    .devices
                    .iter()
                    .find(|d| &d.id == id)
                    .map(|d| d.name.clone())
                    .unwrap_or_else(|| T::NotConnected.get().to_string());
                anvil_ui::chrome::setting_row(ui, &name, |ui| {
                    if anvil_ui::widgets::button(ui, anvil_ui::Kind::Ghost, None, T::Show.get()).clicked() {
                        show_dev = Some(id.clone());
                    }
                });
            }
            if let Some(e) = show_app {
                st.hidden_apps.remove(&e);
            }
            if let Some(d) = show_dev {
                st.hidden_devices.remove(&d);
            }
        }

        if st != before {
            // При высоком контрасте тему задаёт система; выбор из настроек вступит в силу, когда его выключат.
            if st.theme != before.theme && !crate::contrast::active(&ctx) {
                anvil_ui::choose(&ctx, theme_choice(st.theme));
            }
            if st.lang != before.lang {
                crate::texts::apply(st.lang);
                anvil_ui::lang::set_language(&ctx, kit_lang());
            }
            self.shared.wheel_outside.store(st.wheel_outside, Ordering::SeqCst);
            let tray =
                (st.tray_icon, st.tray_color, st.log_scale) != (before.tray_icon, before.tray_color, before.log_scale);
            st.save();
            *self.shared.settings.lock().unwrap() = st;
            if tray {
                // Значок в трее перечитает вид, цвет и шкалу.
                crate::tray::refresh();
            }
        }
    }

    /// Знак 56 в «О программе» цветом из «Настроек»: фейдеры при первом показе после открытия «Настроек», потом —
    /// как файл.
    fn about_mark(&mut self, ui: &mut Ui, color: IconColor) {
        let ctx = ui.ctx().clone();
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(56.0), Sense::hover());
        let still = !anvil_ui::motion::Motion::of(&ctx).enabled;
        let a = &mut self.about;
        let now = a.clock.elapsed().as_secs_f64();
        if !a.played && ui.is_rect_visible(rect) {
            a.played = true;
            a.scene.sweep(now);
        }
        let px = (56.0 * ctx.pixels_per_point()).round() as u32;
        let f = a.scene.frame(now, still);
        let key = (px, color, [(f.l * 1000.0).round() as i32, (f.r * 1000.0).round() as i32]);
        if a.shown != Some(key) {
            let rgba = mark_render::mark(&f, px, color);
            let image = egui::ColorImage::from_rgba_unmultiplied([px as usize, px as usize], &rgba);
            match &mut a.texture {
                Some(t) => t.set(image, TextureOptions::LINEAR),
                None => a.texture = Some(ctx.load_texture("kamerton-about-mark", image, TextureOptions::LINEAR)),
            }
            a.shown = Some(key);
        }
        if let Some(t) = &a.texture {
            ui.painter().image(t.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        if a.scene.wants_frames(now, still) {
            ctx.request_repaint();
        }
    }

    /// Окно микшера — отдельный процесс и читает настройки при запуске: цвет значка перечитывается из файла, когда
    /// окно получает фокус (его могли сменить в «Настройках» окна у значка).
    fn follow_icon_color(&mut self, ctx: &egui::Context) {
        let focused = ctx.input(|i| i.viewport().focused).unwrap_or(false);
        if focused && !self.had_focus {
            let color = Settings::load().tray_color;
            if self.icon_color != Some(color) {
                sys::set_window_icon(self.hwnd, color);
                self.icon_color = Some(color);
            }
        }
        self.had_focus = focused;
    }

    /// Шапка «Настроек» (вариант A макета): сверху знак, «Kamerton» и версия, ниже — полоса обновления: слева
    /// состояние, справа кнопки по состоянию. Под версией — «Переносная копия», если это она.
    fn about_card(&mut self, ui: &mut Ui, st: &mut Settings) {
        let p = crate::contrast::palette(ui.ctx());
        let color = st.tray_color;
        egui::Frame::new().stroke(Stroke::new(1.0, p.border)).corner_radius(radius::CARD).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            egui::Frame::new().inner_margin(egui::Margin { left: 14, right: 14, top: 14, bottom: 12 }).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    self.about_mark(ui, color);
                    ui.add_space(8.0);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.label(egui::RichText::new("Kamerton").font(anvil_ui::semibold(16.0)));
                        let version = format!("{} {}", T::Version.get(), env!("CARGO_PKG_VERSION"));
                        ui.label(egui::RichText::new(version).size(12.5).color(p.weak));
                        if crate::config::portable_dir().is_some() {
                            ui.label(egui::RichText::new(T::Portable.get()).size(11.5).color(p.faint));
                        }
                    });
                });
            });
            if let Some(u) = crate::update::get() {
                let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
                ui.painter().rect_filled(line, 0, p.border);
                let bottom = CornerRadius { nw: 0, ne: 0, sw: radius::CARD - 1, se: radius::CARD - 1 };
                egui::Frame::new()
                    .fill(p.surface)
                    .corner_radius(bottom)
                    .inner_margin(egui::Margin::symmetric(12, 10))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        self.update_strip(ui, u, st);
                    });
            }
        });
    }

    /// Полоса обновления: состояние слева, кнопки справа.
    fn update_strip(&mut self, ui: &mut Ui, u: &anvil_update::Updater, st: &mut Settings) {
        let state = crate::update::view_state(u);
        // Справа кнопки: две («Пропустить», «Обновить») занимают больше всего, одна — меньше.
        let buttons_w = match &state {
            anvil_update::State::Available(_) => 225.0,
            anvil_update::State::Ready { .. } => 190.0,
            anvil_update::State::Failed { .. } => 130.0,
            _ => 215.0,
        };
        let left_w = (ui.available_width() - buttons_w).max(100.0);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.set_max_width(left_w);
                ui.spacing_mut().item_spacing.y = 4.0;
                Self::update_status(ui, &state);
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.update_buttons(ui, u, st, &state);
            });
        });
        // Ошибка — под строкой, на всю ширину полосы: рядом с кнопкой («Проверить сейчас» по-русски) ей не хватает места.
        if let anvil_update::State::Failed { error, .. } = &state {
            let p = crate::contrast::palette(ui.ctx());
            // Ошибка — понятным текстом на языке окна (лимит GitHub и т. п.); длинную чужую — обрезать.
            let ru = anvil_ui::lang::language(ui.ctx()) == anvil_ui::Lang::Ru;
            let mut text = anvil_update::errors::describe(error, ru);
            if text.chars().count() > 110 {
                text = text.chars().take(109).collect::<String>() + "…";
            }
            ui.horizontal_top(|ui| {
                ui.add_space(22.0);
                ui.add(egui::Label::new(egui::RichText::new(text).size(11.5).color(p.faint)).wrap());
            });
        }
    }

    /// Строка состояния обновления: значок и текст; под ней ошибка или полоса загрузки.
    fn update_status(ui: &mut Ui, state: &anvil_update::State) {
        use anvil_update::State;
        let p = crate::contrast::palette(ui.ctx());
        let line = |ui: &mut Ui, icon: Option<anvil_ui::Icon>, tint: Color32, text: String, strong: bool| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if let Some(icon) = icon {
                    let (r, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                    anvil_ui::icons::paint(ui.painter(), r, icon, tint);
                }
                ui.label(egui::RichText::new(text).size(12.5).color(if strong { p.text } else { p.weak }));
            });
        };
        match state {
            State::Idle => {}
            State::UpToDate => line(ui, Some(anvil_ui::Icon::Check), p.success, T::UpdUpToDate.get().into(), false),
            State::Checking => line(ui, None, p.weak, T::UpdChecking.get().into(), false),
            // Без значка: у «Обновить» он уже есть, а в узкой полосе по-русски («Пропустить») места не хватает.
            State::Available(up) => {
                line(ui, None, p.accent_text, format!("{} {}", T::UpdAvailable.get(), up.version), true)
            }
            State::Downloading { update, done, total } => {
                let pct = if *total > 0 { (*done as f64 / *total as f64 * 100.0).round() as u32 } else { 0 };
                line(ui, None, p.weak, format!("{} {} · {pct} %", T::UpdDownloading.get(), update.version), true);
                anvil_ui::widgets::progress(ui, (*total > 0).then(|| *done as f32 / *total as f32), 150.0);
            }
            State::Ready { update, .. } => line(
                ui,
                Some(anvil_ui::Icon::Check),
                p.success,
                format!("{} {}", T::UpdReady.get(), update.version),
                true,
            ),
            State::Failed { .. } => {
                line(ui, Some(anvil_ui::Icon::Close), p.danger, T::UpdFailed.get().into(), false);
            }
        }
    }

    /// Кнопки по состоянию. Колонка идёт справа налево: первой кладётся крайняя справа («Обновить», потом «Пропустить»).
    fn update_buttons(
        &mut self,
        ui: &mut Ui,
        u: &anvil_update::Updater,
        st: &mut Settings,
        state: &anvil_update::State,
    ) {
        use anvil_update::State;
        match state {
            State::Available(up) if up.asset.is_some() => {
                if anvil_ui::widgets::button(
                    ui,
                    anvil_ui::Kind::Primary,
                    Some(anvil_ui::Icon::Download),
                    T::UpdateNow.get(),
                )
                .clicked()
                {
                    u.install();
                }
                if anvil_ui::widgets::button(ui, anvil_ui::Kind::Ghost, None, T::SkipVersion.get()).clicked() {
                    st.skip_version = Some(up.version.to_string());
                    u.dismiss();
                }
            }
            State::Ready { .. } => {
                if anvil_ui::widgets::button(
                    ui,
                    anvil_ui::Kind::Primary,
                    Some(anvil_ui::Icon::Refresh),
                    T::RestartNow.get(),
                )
                .clicked()
                    && let Err(e) = crate::update::restart(crate::tray::hwnd())
                {
                    eprintln!("update: restart failed: {e}");
                }
            }
            State::Failed { update, .. } => {
                // «Повторить»: скачать ещё раз, если было что скачивать, иначе — проверить ещё раз.
                if anvil_ui::widgets::button(
                    ui,
                    anvil_ui::Kind::Secondary,
                    Some(anvil_ui::Icon::Refresh),
                    T::Retry.get(),
                )
                .clicked()
                {
                    if update.is_some() {
                        u.install();
                    } else {
                        crate::update::check_now(st);
                    }
                }
            }
            State::Downloading { .. } => {}
            State::Checking => {
                // Кнопка на месте, но недоступна: раскладка не прыгает, пока идёт проверка.
                ui.add_enabled_ui(false, |ui| {
                    anvil_ui::widgets::button(
                        ui,
                        anvil_ui::Kind::Secondary,
                        Some(anvil_ui::Icon::Refresh),
                        T::CheckNow.get(),
                    );
                });
            }
            _ => {
                if anvil_ui::widgets::button(
                    ui,
                    anvil_ui::Kind::Secondary,
                    Some(anvil_ui::Icon::Refresh),
                    T::CheckNow.get(),
                )
                .clicked()
                {
                    crate::update::check_now(st);
                }
            }
        }
    }

    fn profiles_page(&mut self, ui: &mut Ui, snapshot: &Snapshot) {
        ui.horizontal(|ui| {
            if anvil_ui::widgets::icon_button(ui, anvil_ui::Icon::ArrowLeft, T::Back.get()).clicked() {
                self.page = Page::Mixer;
                self.deleting = None;
            }
            ui.label(egui::RichText::new(T::Profiles.get()).font(anvil_ui::semibold(14.5)));
        });
        ui.add_space(6.0);
        let mut st = self.shared.settings.lock().unwrap().clone();
        let before = st.clone();

        anvil_ui::widgets::section_label(ui, T::MemoryTitle.get());
        anvil_ui::widgets::switch(ui, &mut st.remember, T::RememberVolumes.get());
        ui.add_space(4.0);
        let mut new_on = st.new_app_volume.is_some();
        anvil_ui::widgets::switch(ui, &mut new_on, T::NewAppVolume.get());
        if new_on {
            let value = st.new_app_volume.unwrap_or(50) as f32 / 100.0;
            let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
            let number = Rect::from_min_max(pos2(row.right() - NUMBER_W, row.top()), row.max);
            let track = Rect::from_min_max(row.min - vec2(7.0, 0.0), pos2(number.left() - 3.0, row.bottom()));
            // Всегда ровная шкала: 50 — посередине (владелец, 23.10.2026).
            let out = slider_scaled(ui, ui.id().with("newapp"), track, value, false, 0.0, false, 0.0);
            let v = out.value.unwrap_or(value);
            number_text(ui, number, v, false);
            st.new_app_volume = Some(model::percent(v) as u8);
        } else {
            st.new_app_volume = None;
        }

        ui.add_space(10.0);
        anvil_ui::widgets::section_label(ui, T::Profiles.get());
        if st.profiles.is_empty() {
            anvil_ui::widgets::note(ui, T::NoProfiles.get());
        }
        let name_of = |id: &Option<String>| -> String {
            match id {
                None => T::Never.get().to_string(),
                Some(id) => snapshot
                    .devices
                    .iter()
                    .find(|d| &d.id == id)
                    .map(|d| d.name.clone())
                    .unwrap_or_else(|| T::NotConnected.get().to_string()),
            }
        };
        let mut apply = None;
        let mut remove = None;
        for (i, p) in st.profiles.iter_mut().enumerate() {
            anvil_ui::widgets::card(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut p.name).desired_width(150.0));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if self.deleting == Some(i) {
                            if anvil_ui::widgets::button(ui, anvil_ui::Kind::Danger, None, T::Delete.get()).clicked() {
                                remove = Some(i);
                            }
                            ui.label(egui::RichText::new(T::DeleteConfirm.get()).size(13.0));
                        } else {
                            if anvil_ui::widgets::icon_button(ui, anvil_ui::Icon::Trash, T::Delete.get()).clicked() {
                                self.deleting = Some(i);
                            }
                            if anvil_ui::widgets::button(ui, anvil_ui::Kind::Secondary, None, T::Apply.get()).clicked()
                            {
                                apply = Some(i);
                            }
                        }
                    });
                });
                let output = if p.output.is_some() { name_of(&p.output) } else { "—".to_string() };
                let summary = format!("{}: {} · {}: {}", T::Output.get(), output, T::Apps.get(), p.apps.len());
                anvil_ui::widgets::note(ui, summary);
                ui.add_space(4.0);
                // Подпись и выбор — друг под другом: в строку по-русски не помещаются.
                ui.label(egui::RichText::new(T::OnConnect.get()).size(13.0));
                egui::ComboBox::from_id_salt(("onconnect", i))
                    .selected_text(name_of(&p.on_connect))
                    .width(ui.available_width())
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut p.on_connect, None, T::Never.get());
                        for d in &snapshot.devices {
                            ui.selectable_value(&mut p.on_connect, Some(d.id.clone()), &d.name);
                        }
                    });
            });
            ui.add_space(8.0);
        }
        if let Some(i) = remove {
            st.profiles.remove(i);
            self.deleting = None;
        }
        ui.add_space(4.0);
        if anvil_ui::widgets::button(ui, anvil_ui::Kind::Primary, Some(anvil_ui::Icon::Save), T::SaveCurrent.get())
            .clicked()
        {
            let name = format!("{} {}", T::ProfileName.get(), st.profiles.len() + 1);
            st.profiles.push(crate::profiles::capture(&name, snapshot));
        }
        if st != before {
            st.save();
            *self.shared.settings.lock().unwrap() = st;
        }
        if let Some(i) = apply {
            crate::tray::run_profile(&self.shared, &self.control, i);
        }
    }

    /// Страница «Правила»: потолок громкости, приглушение, правила по программам.
    fn rules_page(&mut self, ui: &mut Ui, snapshot: &Snapshot) {
        ui.horizontal(|ui| {
            if anvil_ui::widgets::icon_button(ui, anvil_ui::Icon::ArrowLeft, T::Back.get()).clicked() {
                self.page = Page::Mixer;
                self.deleting = None;
            }
            ui.label(egui::RichText::new(T::Rules.get()).font(anvil_ui::semibold(14.5)));
        });
        ui.add_space(6.0);
        let mut st = self.shared.settings.lock().unwrap().clone();
        let before = st.clone();
        let running = rules::running_apps(snapshot);
        let app_name = |exe: &str| -> String {
            running
                .iter()
                .find(|(e, _)| e == exe)
                .map(|(_, n)| n.clone())
                .unwrap_or_else(|| exe.rsplit(['\\', '/']).next().unwrap_or(exe).to_string())
        };
        let outputs: Vec<&Device> = outputs(snapshot);

        // Потолок громкости: общий и по программам.
        anvil_ui::widgets::section_label(ui, T::LimitSection.get());
        let mut device_on = st.limit_device.is_some();
        anvil_ui::widgets::switch(ui, &mut device_on, T::LimitDevice.get());
        st.limit_device = device_on.then(|| percent_slider(ui, "limit-device", st.limit_device.unwrap_or(80), 1, 99));
        // Ночью тише: часы и потолок на это время.
        anvil_ui::widgets::switch(ui, &mut st.night, T::NightSwitch.get());
        if st.night {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(T::NightFrom.get()).size(13.0));
                time_combo(ui, "night-from", &mut st.night_from);
                ui.add_space(8.0);
                ui.label(egui::RichText::new(T::NightTo.get()).size(13.0));
                time_combo(ui, "night-to", &mut st.night_to);
            });
            ui.label(egui::RichText::new(T::NightLimit.get()).size(13.0));
            st.night_limit = percent_slider(ui, "night-limit", st.night_limit, 1, 99);
        }
        let mut drop_cap = None;
        for (exe, cap) in st.limit_apps.iter_mut() {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(app_name(exe)).size(13.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if anvil_ui::widgets::icon_button(ui, anvil_ui::Icon::Trash, T::Delete.get()).clicked() {
                        drop_cap = Some(exe.clone());
                    }
                });
            });
            *cap = percent_slider(ui, ("limit-app", exe.as_str()), *cap, 1, 99);
        }
        if let Some(e) = drop_cap {
            st.limit_apps.remove(&e);
        }
        let free: Vec<&(String, String)> = running.iter().filter(|(e, _)| !st.limit_apps.contains_key(e)).collect();
        if !free.is_empty() {
            let mut pick = None;
            egui::ComboBox::from_id_salt("limit-add")
                .selected_text(T::AddApp.get())
                .width(ui.available_width())
                .show_ui(ui, |ui| {
                    for (exe, name) in &free {
                        if ui.selectable_label(false, name).clicked() {
                            pick = Some(exe.clone());
                        }
                    }
                });
            if let Some(e) = pick {
                st.limit_apps.insert(e, 80);
            }
        }
        anvil_ui::widgets::note(ui, T::LimitNote.get());

        // Приглушение.
        ui.add_space(10.0);
        anvil_ui::widgets::section_label(ui, T::DuckSection.get());
        anvil_ui::widgets::switch(ui, &mut st.duck, T::DuckSwitch.get());
        if st.duck {
            ui.label(egui::RichText::new(T::DuckLevel.get()).size(13.0));
            st.duck_level = percent_slider(ui, "duck-level", st.duck_level, 5, 95);
            ui.label(egui::RichText::new(T::DuckApps.get()).size(13.0));
            let mut drop_app = None;
            for exe in &st.duck_apps {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(app_name(exe)).size(13.0));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if anvil_ui::widgets::icon_button(ui, anvil_ui::Icon::Trash, T::Delete.get()).clicked() {
                            drop_app = Some(exe.clone());
                        }
                    });
                });
            }
            if let Some(e) = drop_app {
                st.duck_apps.remove(&e);
            }
            let free: Vec<&(String, String)> = running.iter().filter(|(e, _)| !st.duck_apps.contains(e)).collect();
            if !free.is_empty() {
                let mut pick = None;
                egui::ComboBox::from_id_salt("duck-add")
                    .selected_text(T::AddApp.get())
                    .width(ui.available_width())
                    .show_ui(ui, |ui| {
                        for (exe, name) in &free {
                            if ui.selectable_label(false, name).clicked() {
                                pick = Some(exe.clone());
                            }
                        }
                    });
                if let Some(e) = pick {
                    st.duck_apps.insert(e);
                }
            }
            anvil_ui::widgets::note(ui, T::DuckNote.get());
        }

        // Правила по программам.
        ui.add_space(10.0);
        anvil_ui::widgets::section_label(ui, T::RulesSection.get());
        if st.rules.is_empty() {
            anvil_ui::widgets::note(ui, T::NoRules.get());
        }
        let device_name = |id: &str| -> String {
            snapshot
                .devices
                .iter()
                .find(|d| d.id == id)
                .map(|d| d.name.clone())
                .unwrap_or_else(|| T::NotConnected.get().to_string())
        };
        let mut remove = None;
        for (i, r) in st.rules.iter_mut().enumerate() {
            anvil_ui::widgets::card(ui, |ui| {
                ui.horizontal(|ui| {
                    let shown = if r.app.is_empty() { T::PickApp.get().to_string() } else { app_name(&r.app) };
                    egui::ComboBox::from_id_salt(("rule-app", i)).selected_text(shown).width(190.0).show_ui(ui, |ui| {
                        for (exe, name) in &running {
                            ui.selectable_value(&mut r.app, exe.clone(), name);
                        }
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if self.deleting == Some(i) {
                            if anvil_ui::widgets::button(ui, anvil_ui::Kind::Danger, None, T::Delete.get()).clicked() {
                                remove = Some(i);
                            }
                            ui.label(egui::RichText::new(T::DeleteRuleConfirm.get()).size(13.0));
                        } else if anvil_ui::widgets::icon_button(ui, anvil_ui::Icon::Trash, T::Delete.get()).clicked() {
                            self.deleting = Some(i);
                        }
                    });
                });
                ui.add_space(4.0);
                ui.label(egui::RichText::new(T::RuleWhen.get()).size(13.0));
                let when_text = match &r.when {
                    When::Running => T::WhenRunning.get().to_string(),
                    When::WithDevice(id) => format!("{} {}", T::WhenWith.get(), device_name(id)),
                };
                egui::ComboBox::from_id_salt(("rule-when", i))
                    .selected_text(when_text)
                    .width(ui.available_width())
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut r.when, When::Running, T::WhenRunning.get());
                        for d in &snapshot.devices {
                            let text = format!("{} {}", T::WhenWith.get(), d.name);
                            ui.selectable_value(&mut r.when, When::WithDevice(d.id.clone()), text);
                        }
                    });
                ui.add_space(4.0);
                ui.label(egui::RichText::new(T::RuleDevice.get()).size(13.0));
                let dev_text = match &r.device {
                    DeviceAct::Keep => T::KeepDevice.get().to_string(),
                    DeviceAct::System => T::AsSystem.get().to_string(),
                    DeviceAct::Device(id) => device_name(id),
                };
                egui::ComboBox::from_id_salt(("rule-device", i))
                    .selected_text(dev_text)
                    .width(ui.available_width())
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut r.device, DeviceAct::Keep, T::KeepDevice.get());
                        ui.selectable_value(&mut r.device, DeviceAct::System, T::AsSystem.get());
                        for d in &outputs {
                            ui.selectable_value(&mut r.device, DeviceAct::Device(d.id.clone()), &d.name);
                        }
                    });
                ui.add_space(4.0);
                let mut set_volume = r.volume.is_some();
                anvil_ui::widgets::switch(ui, &mut set_volume, T::RuleVolume.get());
                r.volume = set_volume.then(|| percent_slider(ui, ("rule-volume", i), r.volume.unwrap_or(100), 0, 100));
                let mut mute = r.muted == Some(true);
                anvil_ui::widgets::switch(ui, &mut mute, T::RuleMute.get());
                r.muted = mute.then_some(true);
            });
            ui.add_space(8.0);
        }
        if let Some(i) = remove {
            st.rules.remove(i);
            self.deleting = None;
        }
        ui.add_space(4.0);
        if anvil_ui::widgets::button(ui, anvil_ui::Kind::Secondary, Some(anvil_ui::Icon::Plus), T::AddRule.get())
            .clicked()
        {
            st.rules.push(AppRule {
                app: running.first().map(|(e, _)| e.clone()).unwrap_or_default(),
                ..Default::default()
            });
        }
        if st != before {
            st.save();
            *self.shared.settings.lock().unwrap() = st;
            // Потолок и приглушение вступают в силу сразу: трей перечитает настройки.
            crate::tray::refresh();
        }
    }

    /// Одна страница в два столбца: «Окно», «Громкость», «Устройство вывода» (следующее, предыдущее,
    /// каждое устройство), «Устройство ввода», затем программы со своими сочетаниями; внизу подсказка.
    fn hotkeys_page(&mut self, ui: &mut Ui, snapshot: &Snapshot) {
        self.capture_keys(ui);
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.horizontal(|ui| {
            if anvil_ui::widgets::icon_button(ui, anvil_ui::Icon::ArrowLeft, T::Back.get()).clicked() {
                self.stop_capture();
                self.page = Page::Mixer;
            }
            ui.label(egui::RichText::new(T::Hotkeys.get()).font(anvil_ui::semibold(14.5)));
        });
        let settings = self.shared.settings.lock().unwrap().clone();
        let global = |a: Action| (action_text(a), Slot::Global(a), settings.hotkeys.get(&a).copied());
        let group = |ui: &mut Ui,
                     me: &mut Self,
                     title: &str,
                     rows: Vec<(&str, Slot, Option<Combo>, Option<icons::DeviceKind>)>| {
            if rows.is_empty() {
                return;
            }
            hotkey_group(ui, title);
            for (name, slot, combo, kind) in rows {
                me.hotkey_row(ui, name, slot, combo, kind);
            }
        };
        let plain = |(n, s, c): (&'static str, Slot, Option<Combo>)| (n, s, c, None);
        group(ui, self, T::GroupWindow.get(), vec![plain(global(Action::Open)), plain(global(Action::Mixer))]);
        group(
            ui,
            self,
            T::GroupVolume.get(),
            vec![plain(global(Action::Louder)), plain(global(Action::Quieter)), plain(global(Action::Mute))],
        );
        let device_rows = |flow: Flow| -> Vec<(&str, Slot, Option<Combo>, Option<icons::DeviceKind>)> {
            of_flow(snapshot, flow)
                .into_iter()
                .map(|d| {
                    let kind = icons::device_kind(&d.name, flow == Flow::Capture);
                    (
                        d.name.as_str(),
                        Slot::Device(d.id.clone()),
                        settings.device_hotkeys.get(&d.id).copied(),
                        Some(kind),
                    )
                })
                .collect()
        };
        // Сочетания устройств — под подписью «Сделать основным:», иначе непонятно, что они делают.
        let captioned = |ui: &mut Ui,
                         me: &mut Self,
                         rows: Vec<(&str, Slot, Option<Combo>, Option<icons::DeviceKind>)>,
                         text: &str| {
            if rows.is_empty() {
                return;
            }
            hotkey_caption(ui, text);
            for (name, slot, combo, kind) in rows {
                me.hotkey_row(ui, name, slot, combo, kind);
            }
        };
        group(
            ui,
            self,
            T::OutputDevice.get(),
            vec![plain(global(Action::NextOutput)), plain(global(Action::PrevOutput))],
        );
        captioned(ui, self, device_rows(Flow::Render), T::MakeDefaultCaption.get());
        // Громкость каждого устройства вывода — в том числе неосновного.
        for d in of_flow(snapshot, Flow::Render) {
            let rows: Vec<_> = AppAction::ALL
                .into_iter()
                .map(|a| {
                    let combo = settings.device_vol_hotkeys.get(&(a, d.id.clone())).copied();
                    (app_action_text(a), Slot::DeviceVol(a, d.id.clone()), combo, None)
                })
                .collect();
            hotkey_caption(ui, &format!("{} · {}", T::DeviceVolumeCaption.get().trim_end_matches(':'), d.name));
            for (name, slot, combo, kind) in rows {
                self.hotkey_row(ui, name, slot, combo, kind);
            }
        }
        let ins = device_rows(Flow::Capture);
        if !ins.is_empty() {
            hotkey_group(ui, T::InputDevice.get());
            captioned(ui, self, ins, T::MakeDefaultCaption.get());
        }
        // Программы: у которых уже есть сочетания, и та, чьё меню открыло страницу.
        let mut exes: Vec<String> = settings.app_hotkeys.keys().map(|(_, e)| e.clone()).collect();
        if let Some(e) = &self.focus_app {
            exes.push(e.clone());
        }
        exes.sort();
        exes.dedup();
        for exe in exes {
            let name = snapshot
                .devices
                .iter()
                .flat_map(|d| d.apps.iter())
                .find(|a| a.key.exe == exe)
                .map(app_name)
                .unwrap_or_else(|| exe.rsplit(['\\', '/']).next().unwrap_or(&exe).to_string());
            let rows = AppAction::ALL
                .into_iter()
                .map(|a| {
                    (
                        app_action_text(a),
                        Slot::App(a, exe.clone()),
                        settings.app_hotkeys.get(&(a, exe.clone())).copied(),
                        None,
                    )
                })
                .collect();
            group(ui, self, &name, rows);
        }
        ui.add_space(8.0);
        anvil_ui::widgets::divider(ui);
        ui.add_space(6.0);
        let p = crate::contrast::palette(ui.ctx());
        ui.label(egui::RichText::new(T::HotkeysHint.get()).size(12.0).color(p.weak));
        ui.add_space(3.0);
        ui.label(egui::RichText::new(T::AppHotkeysHint.get()).size(12.0).color(p.weak));
        // Пояснение к красной точке — только когда она где-то есть.
        if self.shared.hotkey_free.lock().unwrap().values().any(|free| !free) {
            ui.add_space(3.0);
            ui.horizontal(|ui| {
                let (dot, _) = ui.allocate_exact_size(vec2(7.0, 14.0), Sense::hover());
                ui.painter().circle_filled(dot.center(), 3.5, p.danger);
                ui.add_space(6.0);
                ui.label(egui::RichText::new(T::Taken.get()).size(12.0).color(p.weak));
            });
        }
    }
}

/// Подпись раздела страницы сочетаний: как `section_label` набора, в строке точной высоты.
fn hotkey_group(ui: &mut Ui, text: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), HOTKEY_GROUP), Sense::hover());
    let p = crate::contrast::palette(ui.ctx());
    ui.painter().text(
        pos2(rect.left(), rect.bottom() - 9.0),
        Align2::LEFT_CENTER,
        text.to_uppercase(),
        anvil_ui::semibold(11.0),
        p.weak,
    );
}

/// Подпись над сочетаниями устройств: «Сделать основным:» — что делает сочетание.
fn hotkey_caption(ui: &mut Ui, text: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), HOTKEY_CAPTION), Sense::hover());
    let p = crate::contrast::palette(ui.ctx());
    ui.painter().text(
        pos2(rect.left(), rect.center().y),
        Align2::LEFT_CENTER,
        text,
        FontId::proportional(11.5),
        p.weak,
    );
}

/// Время суток списком с шагом 30 минут; другое число минут из файла показывается как есть.
fn time_combo(ui: &mut Ui, id: &str, minutes: &mut u16) {
    egui::ComboBox::from_id_salt(id).selected_text(rules::time_text(*minutes)).width(86.0).show_ui(ui, |ui| {
        for m in (0..24 * 60).step_by(30) {
            ui.selectable_value(minutes, m as u16, rules::time_text(m as u16));
        }
    });
}

/// Ползунок процентов с числом в строке 30 (всегда ровная шкала): значение `lo..=hi`.
fn percent_slider(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, value: u8, lo: u8, hi: u8) -> u8 {
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
    let number = Rect::from_min_max(pos2(row.right() - NUMBER_W, row.top()), row.max);
    let track = Rect::from_min_max(row.min - vec2(7.0, 0.0), pos2(number.left() - 3.0, row.bottom()));
    let v = value as f32 / 100.0;
    let out = slider_scaled(ui, ui.id().with(id), track, v, false, 0.0, false, 0.0);
    let v = out.value.unwrap_or(v);
    number_text(ui, number, v, false);
    (model::percent(v) as u8).clamp(lo, hi)
}

/// Переходы со страницы настроек: «Горячие клавиши» и «Профили» — строки со значком и стрелкой в рамке
/// `border`, под курсором — подложка `hover`. Возвращает, по какой щёлкнули.
fn nav_rows(ui: &mut Ui, show: [bool; 3]) -> [bool; 3] {
    let rows = show.iter().filter(|s| **s).count();
    if rows == 0 {
        return [false; 3];
    }
    let height = rows as f32 * NAV_ROW + (rows - 1) as f32;
    let (frame, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let p = crate::contrast::palette(ui.ctx());
    ui.painter().rect_stroke(frame, radius::CARD, Stroke::new(1.0, p.border), StrokeKind::Inside);
    let mut clicked = [false; 3];
    let mut k = 0;
    for (i, text) in [T::Hotkeys.get(), T::Profiles.get(), T::Rules.get()].into_iter().enumerate() {
        if !show[i] {
            continue;
        }
        let top = frame.top() + k as f32 * (NAV_ROW + 1.0);
        k += 1;
        let row = Rect::from_min_size(pos2(frame.left(), top), vec2(frame.width(), NAV_ROW)).shrink(1.0);
        let r = ui.interact(row, ui.id().with(("nav", i)), Sense::click());
        if r.hovered() {
            ui.painter().rect_filled(row, radius::CONTROL, p.hover);
        }
        let color = if r.hovered() { p.text } else { p.weak };
        let glyph = Rect::from_center_size(pos2(row.left() + 12.0 + 8.0, row.center().y), Vec2::splat(16.0));
        match i {
            0 => icons::keyboard(ui.painter(), glyph, color),
            1 => anvil_ui::icons::paint(ui.painter(), glyph, anvil_ui::Icon::Layers, color),
            _ => icons::rules(ui.painter(), glyph, color),
        }
        ui.painter().text(
            pos2(glyph.right() + 10.0, row.center().y),
            Align2::LEFT_CENTER,
            text,
            FontId::proportional(14.0),
            p.text,
        );
        let arrow = Rect::from_center_size(pos2(row.right() - 8.0 - 8.0, row.center().y), Vec2::splat(16.0));
        anvil_ui::icons::paint(ui.painter(), arrow, anvil_ui::Icon::ArrowRight, p.weak);
        if k < rows {
            // Черта под строкой, если за ней есть ещё одна.
            let y = frame.top() + k as f32 * (NAV_ROW + 1.0) - 0.5;
            ui.painter().line_segment(
                [pos2(frame.left() + 12.0, y), pos2(frame.right() - 12.0, y)],
                Stroke::new(1.0, p.border),
            );
        }
        anvil_ui::widgets::focus_ring(ui, row, &r, radius::CONTROL);
        r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, text));
        clicked[i] = r.on_hover_cursor(CursorIcon::PointingHand).clicked();
    }
    clicked
}

/// Значок в сегменте: системный значок набора, образец значка в трее (в цвете, как будет) или точка цвета.
#[derive(Clone, Copy)]
enum Glyph {
    None,
    Icon(anvil_ui::Icon),
    Tray(TrayIcon, IconColor),
    Dot(IconColor),
}

/// Текстура значка в трее для сегмента, `px` пикселей: рисуется тем же растеризатором, что и трей; хранится в памяти egui.
fn tray_glyph(ctx: &egui::Context, kind: TrayIcon, color: IconColor, dark: bool, px: u32) -> TextureHandle {
    let name = format!("kamerton-seg-{kind:?}-{color:?}-{dark}-{px}");
    let id = egui::Id::new(&name);
    if let Some(t) = ctx.data(|d| d.get_temp::<TextureHandle>(id)) {
        return t;
    }
    let rgba = match kind {
        TrayIcon::Mark => mark_render::mark(&mark_render::rest(px), px, color),
        TrayIcon::Speaker => {
            let frame = mark::SpeakerFrame { waves: [1.0; 3], cross: 0.0, thump: 1.0, ripple: None, error: false };
            mark_render::speaker(&frame, px, dark, color)
        }
    };
    let image = egui::ColorImage::from_rgba_unmultiplied([px as usize, px as usize], &rgba);
    let texture = ctx.load_texture(name, image, TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, texture.clone()));
    texture
}

/// Переключатель из равных сегментов ширины `SEGMENTED_W`: «Тема» и «Язык» стоят одной колонкой. Как
/// `segmented` набора, только ширина сегмента не зависит от подписи.
fn segmented_even<V: PartialEq + Copy>(ui: &mut Ui, value: &mut V, options: &[(V, Option<anvil_ui::Icon>, &str)]) {
    let options: Vec<(V, Glyph, &str)> =
        options.iter().map(|(v, icon, text)| (*v, icon.map_or(Glyph::None, Glyph::Icon), *text)).collect();
    segmented_glyphs(ui, value, &options);
}

/// То же, но со значком любого вида (`Glyph`): «В трее» и «Цвет» показывают, что выбираешь.
fn segmented_glyphs<V: PartialEq + Copy>(ui: &mut Ui, value: &mut V, options: &[(V, Glyph, &str)]) {
    let p = crate::contrast::palette(ui.ctx());
    let font = FontId::proportional(13.0);
    let (rect, response) = ui.allocate_exact_size(vec2(SEGMENTED_W, 30.0), Sense::hover());
    ui.painter().rect(rect, radius::CONTROL + 1, p.raised, Stroke::new(1.0, p.border), StrokeKind::Inside);
    let seg_w = (rect.width() - 4.0) / options.len() as f32;
    let seg =
        |i: usize| Rect::from_min_size(pos2(rect.left() + 2.0 + seg_w * i as f32, rect.top() + 2.0), vec2(seg_w, 26.0));
    if let Some(sel) = options.iter().position(|(o, ..)| *value == *o) {
        let ctx = ui.ctx().clone();
        let motion = anvil_ui::motion::Motion::of(&ctx);
        let left =
            motion.value(&ctx, response.id.with("thumb"), seg(sel).left() - rect.left(), anvil_ui::motion::LAYOUT);
        let thumb = Rect::from_min_size(pos2(rect.left() + left, rect.top() + 2.0), vec2(seg_w, 26.0));
        ui.painter().rect(thumb, radius::CONTROL - 1, p.card, Stroke::new(1.0, p.border_strong), StrokeKind::Inside);
    }
    for (i, (option, glyph, text)) in options.iter().enumerate() {
        let s = seg(i);
        let r = ui.interact(s, response.id.with(i), Sense::click());
        let selected = *value == *option;
        if r.clicked() && !selected {
            *value = *option;
        }
        if !selected && r.hovered() {
            ui.painter().rect_filled(s, radius::CONTROL - 1, p.hover);
        }
        let color = if selected { p.text } else { p.weak };
        let galley = ui.painter().layout_no_wrap(text.to_string(), font.clone(), color);
        let lead = match glyph {
            Glyph::None => 0.0,
            Glyph::Icon(_) | Glyph::Tray(..) => 21.0,
            Glyph::Dot(_) => 16.0,
        };
        let width = galley.size().x + lead;
        let mut x = s.center().x - width / 2.0;
        match glyph {
            Glyph::None => {}
            Glyph::Icon(icon) => {
                let g = Rect::from_min_size(pos2(x, s.center().y - 8.0), Vec2::splat(16.0));
                anvil_ui::icons::paint(ui.painter(), g, *icon, color);
            }
            Glyph::Tray(kind, tint) => {
                let g = Rect::from_min_size(pos2(x, s.center().y - 8.0), Vec2::splat(16.0));
                let px = (16.0 * ui.ctx().pixels_per_point()).round() as u32;
                let texture = tray_glyph(ui.ctx(), *kind, *tint, ui.visuals().dark_mode, px);
                ui.painter().image(texture.id(), g, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }
            Glyph::Dot(tint) => {
                let [r, g, b] = tint.swatch();
                ui.painter().circle_filled(pos2(x + 5.0, s.center().y), 5.0, Color32::from_rgb(r, g, b));
            }
        }
        x += lead;
        ui.painter().galley(pos2(x, s.center().y - galley.size().y / 2.0), galley, color);
        anvil_ui::widgets::focus_ring(ui, s, &r, radius::CONTROL);
        r.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, selected, *text));
        r.on_hover_cursor(CursorIcon::PointingHand);
    }
}

/// Заголовок раздела окна у значка: полужирный 14, `text`, с полем 16.
fn section_title(ui: &mut Ui, text: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), HEADER_ROW), Sense::hover());
    let p = crate::contrast::palette(ui.ctx());
    ui.painter().text(
        pos2(rect.left() + PAD_X, rect.center().y + 2.0),
        Align2::LEFT_CENTER,
        text,
        anvil_ui::semibold(14.0),
        p.text,
    );
}

/// Черта между разделами: 4 + 1 + 4, с полями 16.
fn divider_row(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), DIVIDER), Sense::hover());
    let p = crate::contrast::palette(ui.ctx());
    let y = rect.center().y;
    ui.painter()
        .line_segment([pos2(rect.left() + PAD_X, y), pos2(rect.right() - PAD_X, y)], Stroke::new(1.0, p.border));
}

/// Содержимое с полями слева и справа.
fn padded<R>(ui: &mut Ui, x: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::new().inner_margin(egui::Margin::symmetric(x as i8, 0)).show(ui, add).inner
}

/// Пункт меню с отметкой текущего.
fn check_item(ui: &mut Ui, checked: bool, text: &str) -> bool {
    let icon = checked.then_some(anvil_ui::Icon::Check);
    anvil_ui::widgets::menu_item(ui, icon, text, None).clicked()
}

impl Flyout {
    /// Раздел направления: заголовок, список устройств, черта, заголовок программ — у вывода «Микшер
    /// громкости» с шестерёнкой, у ввода «Программы записи»; основное устройство «без звука» — полоса;
    /// затем программы (не на устройстве по умолчанию — с подписью «→ устройство»).
    fn section(
        &mut self,
        ui: &mut Ui,
        devs: &[&Device],
        title: &str,
        peaks: &HashMap<Target, f32>,
        snapshot: &Snapshot,
        output: bool,
    ) {
        section_title(ui, title);
        padded(ui, INSET, |ui| {
            for d in devs {
                let peak = peaks.get(&Target::Device(d.id.clone())).copied().unwrap_or(0.0);
                self.device_item(ui, d, peak);
                ui.add_space(ITEM_GAP);
            }
        });
        divider_row(ui);
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), MIXER_HEADER), Sense::hover());
        let p = crate::contrast::palette(ui.ctx());
        ui.painter().text(
            pos2(rect.left() + PAD_X, rect.center().y),
            Align2::LEFT_CENTER,
            if output { T::VolumeMixer.get() } else { T::RecordingApps.get() },
            anvil_ui::semibold(14.0),
            p.text,
        );
        if let Some(d) = default_muted(devs) {
            self.muted_banner(ui, d);
        }
        let apps = mixer_apps(devs);
        padded(ui, PAD_X, |ui| {
            if apps.is_empty() {
                note_row(ui, if output { T::NoApps.get() } else { T::NoRecordingApps.get() });
            }
            for (d, a) in apps {
                let peak = peaks.get(&Target::App(a.key.clone())).copied().unwrap_or(0.0);
                let dest = (!d.is_default).then_some(d.name.as_str());
                self.app_row(ui, a, d.flow, snapshot, peak, dest, d.muted);
            }
        });
    }

    /// Полоса «основное устройство без звука» (по образцу `banner` набора, в строку точной высоты):
    /// его программ не слышно, хотя их ползунки целы; «Включить» снимает «без звука» с устройства.
    fn muted_banner(&mut self, ui: &mut Ui, d: &Device) {
        let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), MUTED_ROW), Sense::hover());
        let p = crate::contrast::palette(ui.ctx());
        let rect = Rect::from_min_max(
            pos2(row.left() + PAD_X, row.top() + 2.0),
            pos2(row.right() - PAD_X, row.bottom() - 6.0),
        );
        let edge = Stroke::new(1.0, p.warning.gamma_multiply(0.35));
        ui.painter().rect(rect, radius::CONTROL, p.soft(p.warning), edge, StrokeKind::Inside);
        let glyph = Rect::from_center_size(pos2(rect.left() + 12.0 + 8.0, rect.center().y), Vec2::splat(16.0));
        let capture = d.flow == Flow::Capture;
        if capture {
            icons::device(ui.painter(), glyph, p.warning, icons::DeviceKind::Mic);
        } else {
            icons::speaker(ui.painter(), glyph, p.warning, 0.0, true);
        }
        let label = T::TurnOn.get();
        let galley = ui.painter().layout_no_wrap(label.to_string(), anvil_ui::semibold(13.0), p.accent_text);
        let button = Rect::from_min_max(
            pos2(rect.right() - 6.0 - galley.size().x - 20.0, rect.center().y - 14.0),
            pos2(rect.right() - 6.0, rect.center().y + 14.0),
        );
        let (title, sub) = if capture { (T::MicMuted, T::MicSilent) } else { (T::DefaultMuted, T::AppsSilent) };
        let left = glyph.right() + 10.0;
        let right = button.left() - 8.0;
        let top = Rect::from_min_max(pos2(left, rect.center().y - 17.0), pos2(right, rect.center().y));
        text_clipped(ui, top, title.get(), anvil_ui::semibold(13.0), p.text);
        let bottom = Rect::from_min_max(pos2(left, rect.center().y), pos2(right, rect.center().y + 16.0));
        text_clipped(ui, bottom, sub.get(), FontId::proportional(12.0), p.weak);
        let r = ui.interact(button, ui.id().with(("unmute", &d.id)), Sense::click());
        if r.hovered() {
            ui.painter().rect_filled(button, radius::CONTROL, p.hover);
        }
        ui.painter().galley(button.center() - galley.size() / 2.0, galley, p.accent_text);
        anvil_ui::widgets::focus_ring(ui, button, &r, radius::CONTROL);
        r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
        if r.on_hover_cursor(CursorIcon::PointingHand).clicked() {
            let _ = self.control.set_muted(Target::Device(d.id.clone()), false);
        }
    }

    /// Полоса внизу: «Другие параметры звука» со значком «наружу» (откроются Параметры Windows), справа
    /// «Профили» и кнопка устройств ввода — микрофон со стрелкой.
    fn footer(&mut self, ui: &mut Ui, rect: Rect) {
        let p = crate::contrast::palette(ui.ctx());
        // Внутри рамки окна: снизу углы скруглены, как у окна.
        let fill = Rect::from_min_max(rect.min + vec2(1.0, 0.0), rect.max - vec2(1.0, 1.0));
        let round = radius::WINDOW.saturating_sub(1);
        ui.painter().rect_filled(fill, CornerRadius { nw: 0, ne: 0, sw: round, se: round }, p.bg);
        ui.painter().line_segment([fill.left_top(), fill.right_top()], Stroke::new(1.0, p.border));
        let expanded = self.shared.expanded();
        let link = T::MoreSettings.get();
        let galley = ui.painter().layout_no_wrap(link.to_string(), FontId::proportional(13.0), p.text);
        let text_rect =
            Rect::from_min_size(pos2(rect.left() + PAD_X, rect.center().y - galley.size().y / 2.0), galley.size());
        let out = Rect::from_center_size(pos2(text_rect.right() + 6.0 + 6.0, rect.center().y), Vec2::splat(12.0));
        let link_rect = text_rect.union(out);
        let r = ui.interact(link_rect.expand(4.0), ui.id().with("more-settings"), Sense::click());
        let color = if r.hovered() { p.accent_text } else { p.text };
        ui.painter().galley(text_rect.min, galley, color);
        icons::external(ui.painter(), out, if r.hovered() { p.accent_text } else { p.weak });
        if r.hovered() {
            ui.painter().line_segment(
                [text_rect.left_bottom() + vec2(0.0, 1.0), text_rect.right_bottom() + vec2(0.0, 1.0)],
                Stroke::new(1.0, color),
            );
        }
        anvil_ui::widgets::focus_ring(ui, link_rect.expand(4.0), &r, radius::CONTROL);
        if r.on_hover_cursor(CursorIcon::PointingHand).clicked() {
            sys::open("ms-settings:sound");
        }
        let buttons =
            Rect::from_min_max(pos2(link_rect.right() + 8.0, rect.top()), pos2(rect.right() - 8.0, rect.bottom()));
        // Справа налево: кнопка устройств ввода в углу, черта, «Настройки», «Профили». Черта отделяет кнопку,
        // которая раскрывает окно, от двух, которые ведут на страницы.
        let (toggle, settings, profiles) = ui
            .scope_builder(
                egui::UiBuilder::new().max_rect(buttons).layout(egui::Layout::right_to_left(egui::Align::Center)),
                |ui| {
                    let t = inputs_toggle(ui, expanded).clicked();
                    let (line, _) = ui.allocate_exact_size(vec2(9.0, 18.0), Sense::hover());
                    ui.painter().line_segment([line.center_top(), line.center_bottom()], Stroke::new(1.0, p.border));
                    let s = anvil_ui::widgets::icon_button(ui, anvil_ui::Icon::Gear, T::Settings.get()).clicked();
                    let pr = anvil_ui::widgets::icon_button(ui, anvil_ui::Icon::Layers, T::Profiles.get()).clicked();
                    (t, s, pr)
                },
            )
            .inner;
        if toggle {
            let mut s = self.shared.settings.lock().unwrap();
            s.expanded = !s.expanded;
            s.save();
        }
        if profiles {
            self.page = Page::Profiles;
        }
        if settings {
            self.page = Page::Settings;
            self.focus_app = None;
        }
    }

    /// Строка списка устройств: значок, имя, под ним ползунок; справа число и «без звука». Устройство по
    /// умолчанию подсвечено и отмечено чертой акцента слева; щелчок по строке делает устройство основным.
    fn device_item(&mut self, ui: &mut Ui, d: &Device, peak: f32) {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ITEM_ROW), Sense::hover());
        let row = ui.interact(rect, ui.id().with(("devitem", &d.id)), Sense::click());
        let p = crate::contrast::palette(ui.ctx());
        if d.is_default || row.hovered() {
            ui.painter().rect_filled(rect, radius::CONTROL, p.raised);
        }
        if d.is_default {
            let pill = Rect::from_center_size(pos2(rect.left() + 1.5, rect.center().y), vec2(3.0, 16.0));
            ui.painter().rect_filled(pill, CornerRadius::same(2), p.accent);
        }
        // Значок в слоте 24 и «без звука» — в тех же колонках, что у программ ниже: поле строки
        // `ITEM_PAD` плюс отступ списка `INSET` = `PAD_X` программ.
        let kind = icons::device_kind(&d.name, d.flow == Flow::Capture);
        let slot = Rect::from_min_size(pos2(rect.left() + ITEM_PAD, rect.center().y - ICON / 2.0), Vec2::splat(ICON));
        icons::device(ui.painter(), slot, p.text, kind);
        let left = slot.right() + 10.0;
        let button =
            Rect::from_center_size(pos2(rect.right() - ITEM_PAD - BUTTON / 2.0, rect.center().y), Vec2::splat(BUTTON));
        let number = Rect::from_min_max(
            pos2(button.left() - 8.0 - NUMBER_W, rect.top()),
            pos2(button.left() - 8.0, rect.bottom()),
        );
        let font = if d.is_default { anvil_ui::semibold(14.0) } else { FontId::proportional(14.0) };
        let name_rect = Rect::from_min_max(pos2(left, rect.top() + 7.0), pos2(number.left() - 8.0, rect.top() + 27.0));
        text_clipped(ui, name_rect, &d.name, font, p.text);
        let track =
            Rect::from_min_max(pos2(left - 7.0, rect.top() + 30.0), pos2(number.left() - 10.0, rect.top() + 46.0));
        let target = Target::Device(d.id.clone());
        let shown = self.shown_value(&target, d.volume);
        let out = slider(ui, ui.id().with(("dev", &d.id)), track, shown, d.muted, peak);
        if let Some(v) = out.value {
            self.set_volume(target.clone(), v);
        }
        if out.released {
            sys::beep();
        }
        number_text(ui, number, shown, d.muted);
        let mute = mute_button(ui, button, ui.id().with(("devmute", &d.id)), shown, d.muted).clicked();
        if mute || out.toggle_mute {
            let _ = self.control.set_muted(target.clone(), !d.muted);
        }
        if row.clicked() && !d.is_default {
            let _ = self.control.set_default(&d.id, Roles::All);
        }
        if row.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
        self.targets.push((d.id.clone(), d.flow, rect));
        self.device_menu(&row, d);
    }

    /// Меню строки устройства (правый щелчок): по умолчанию, связь, «без звука», скрыть.
    fn device_menu(&mut self, row: &egui::Response, d: &Device) {
        let target = Target::Device(d.id.clone());
        row.context_menu(|ui| {
            if check_item(ui, d.is_default, T::MakeDefault.get()) {
                let _ = self.control.set_default(&d.id, Roles::All);
                ui.close();
            }
            if check_item(ui, d.is_comm, T::MakeComm.get()) {
                let _ = self.control.set_default(&d.id, Roles::Communications);
                ui.close();
            }
            anvil_ui::widgets::menu_separator(ui);
            let text = if d.muted { T::Unmute.get() } else { T::Mute.get() };
            if anvil_ui::widgets::menu_item(ui, None, text, Some("M")).clicked() {
                let _ = self.control.set_muted(target.clone(), !d.muted);
                ui.close();
            }
            if anvil_ui::widgets::menu_item(ui, None, T::Hide.get(), None).clicked() {
                let mut st = self.shared.settings.lock().unwrap();
                st.hidden_devices.insert(d.id.clone());
                st.save();
                ui.close();
            }
        });
    }

    fn device_row(&mut self, ui: &mut Ui, d: &Device, peak: f32) {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), DEVICE_ROW), Sense::hover());
        // Правый щелчок по строке — меню; ползунок и кнопка лежат поверх и ловят свои щелчки сами.
        let row = ui.interact(rect, ui.id().with(("devrow", &d.id)), Sense::click());
        let p = crate::contrast::palette(ui.ctx());
        let right = rect.right();
        let button = Rect::from_center_size(pos2(right - BUTTON / 2.0, rect.center().y), Vec2::splat(BUTTON));
        let number = Rect::from_min_max(
            pos2(button.left() - 8.0 - NUMBER_W, rect.top()),
            pos2(button.left() - 8.0, rect.bottom()),
        );
        // Шапка колонки: значок устройства в слоте 24 (как у программ ниже), имя; основное — черта акцента
        // у края карточки и метка «Основное».
        let kind = icons::device_kind(&d.name, d.flow == Flow::Capture);
        let slot = Rect::from_min_size(pos2(rect.left(), rect.top() + 4.0), Vec2::splat(ICON));
        icons::device(ui.painter(), slot, p.text, kind);
        let left = slot.right() + 10.0;
        let mut name_right = number.left() - 8.0;
        if d.is_default {
            // Черта — у левого края карточки (поле карточки набора — 16).
            let pill = Rect::from_center_size(pos2(rect.left() - 16.0 + 1.5, slot.center().y), vec2(3.0, 16.0));
            ui.painter().rect_filled(pill, CornerRadius::same(2), p.accent);
            let galley =
                ui.painter().layout_no_wrap(T::DefaultBadge.get().to_string(), anvil_ui::semibold(12.0), p.accent_text);
            let badge = Rect::from_min_max(
                pos2(name_right - galley.size().x - 14.0, slot.center().y - 10.0),
                pos2(name_right, slot.center().y + 10.0),
            );
            ui.painter().rect_filled(badge, 10, p.badge_fill(p.accent_text));
            ui.painter().galley(badge.center() - galley.size() / 2.0, galley, p.accent_text);
            name_right = badge.left() - 6.0;
        }
        let name_rect = Rect::from_min_max(pos2(left, slot.top()), pos2(name_right, slot.bottom()));
        text_clipped(ui, name_rect, &d.name, anvil_ui::semibold(14.5), p.text);
        let track =
            Rect::from_min_max(pos2(left - 7.0, rect.top() + 30.0), pos2(number.left() - 10.0, rect.top() + 46.0));
        let target = Target::Device(d.id.clone());
        let shown = self.shown_value(&target, d.volume);
        let out = slider(ui, ui.id().with(("dev", &d.id)), track, shown, d.muted, peak);
        if let Some(v) = out.value {
            self.set_volume(target.clone(), v);
        }
        if out.released {
            // Как у EarTrumpet: тихий системный звук — услышать новую громкость.
            sys::beep();
        }
        number_text(ui, number, shown, d.muted);
        let mute = mute_button(ui, button, ui.id().with(("devmute", &d.id)), shown, d.muted).clicked();
        if mute || out.toggle_mute {
            let _ = self.control.set_muted(target.clone(), !d.muted);
        }
        self.device_menu(&row, d);
    }

    /// Строка программы. `silent` — её устройство «без звука»: ползунок и число серые, как у выключенной.
    #[allow(clippy::too_many_arguments)]
    fn app_row(
        &mut self,
        ui: &mut Ui,
        a: &App,
        flow: Flow,
        snapshot: &Snapshot,
        peak: f32,
        dest: Option<&str>,
        silent: bool,
    ) {
        let p = crate::contrast::palette(ui.ctx());
        let ctx = ui.ctx().clone();
        // Подпись «→ устройство» усечена — при наведении на неё строка раскрывается и показывает её целиком (только в
        // окне у значка: в окне микшера высота не растёт). Раскрыта ли строка — в памяти egui, плавность — по кривой набора.
        let expandable = dest.is_some() && !self.mixer;
        let exp_id = ui.id().with(("appexp", key_id(&a.key)));
        let open = expandable
            && (std::env::var_os("KAMERTON_EXPAND").is_some()
                || ctx.data(|d| d.get_temp::<bool>(exp_id)).unwrap_or(false));
        let progress = if expandable && std::env::var_os("KAMERTON_EXPAND").is_some() {
            // Для снимков: раскрыто сразу, без движения.
            1.0
        } else if expandable {
            anvil_ui::motion::Motion::of(&ctx).value(
                &ctx,
                exp_id.with("progress"),
                if open { 1.0 } else { 0.0 },
                anvil_ui::motion::LAYOUT,
            )
        } else {
            0.0
        };
        let extra = APP_EXTRA * progress;
        self.row_extra_next += extra;
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), APP_ROW + extra), Sense::hover());
        // Значок, число и кнопка — по первым 50 px: они не уезжают вместе с раскрытием.
        let base = Rect::from_min_size(rect.min, vec2(rect.width(), APP_ROW));
        let row = ui.interact(rect, ui.id().with(("approw", key_id(&a.key))), Sense::click());
        if progress > 0.0 {
            ui.painter().rect_filled(rect, radius::CONTROL, p.hover.gamma_multiply(progress));
        }
        let icon_rect = Rect::from_min_size(pos2(base.left(), base.center().y - ICON / 2.0), Vec2::splat(ICON));
        if let Some(tex) = if a.system { None } else { self.icon_for(&ctx, a) } {
            ui.painter().image(tex.id(), icon_rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        } else {
            ui.painter().rect_filled(icon_rect, radius::CONTROL, p.raised);
            let inner = Rect::from_center_size(icon_rect.center(), Vec2::splat(16.0));
            if a.system {
                icons::speaker(ui.painter(), inner, p.weak, 1.0, false);
            } else {
                anvil_ui::icons::paint(ui.painter(), inner, anvil_ui::Icon::Window, p.weak);
            }
        }
        let button = Rect::from_center_size(pos2(base.right() - BUTTON / 2.0, base.center().y), Vec2::splat(BUTTON));
        let number = Rect::from_min_max(
            pos2(button.left() - 8.0 - NUMBER_W, base.top()),
            pos2(button.left() - 8.0, base.bottom()),
        );
        let left = icon_rect.right() + 10.0;
        let name = app_name(a);
        let name_rect = Rect::from_min_max(pos2(left, base.top() + 6.0), pos2(number.left() - 8.0, base.top() + 24.0));
        let quiet = a.muted || silent;
        let name_color = if quiet { p.weak } else { p.text };
        // Мелко после имени: «→ устройство» у программы не на основном, номер процесса — у программы без имени.
        let sub = match dest {
            Some(dest) => Some(format!("→ {dest}")),
            None if !a.system && a.name.is_empty() => a.pids.first().map(|pid| format!("PID {pid}")),
            None => None,
        };
        match sub {
            // Имя — не больше 60 % строки, остальное — подписи.
            Some(sub) => {
                let w = ui.painter().layout_no_wrap(name.clone(), FontId::proportional(14.0), name_color).size().x;
                let split = (name_rect.left() + w.min(name_rect.width() * 0.6)).round();
                let name_part = Rect::from_min_max(name_rect.min, pos2(split, name_rect.bottom()));
                text_clipped(ui, name_part, &name, FontId::proportional(14.0), name_color);
                let rest = Rect::from_min_max(pos2(split + 6.0, name_rect.top() + 1.0), name_rect.max);
                if expandable {
                    // Усечённая подпись растворяется, под именем проявляется полная; наведение на усечённую — раскрыть.
                    paint_clipped(ui, rest, &sub, FontId::proportional(11.5), p.weak.gamma_multiply(1.0 - progress));
                    if progress > 0.0 {
                        let line = Rect::from_min_max(
                            pos2(left, base.top() + 25.0 + (progress - 1.0) * 4.0),
                            pos2(rect.right() - 12.0, base.top() + 42.0 + (progress - 1.0) * 4.0),
                        );
                        text_clipped(ui, line, &sub, FontId::proportional(11.5), p.text.gamma_multiply(progress));
                    }
                    let pointer = ctx.input(|i| i.pointer.hover_pos());
                    let on_sub = pointer.is_some_and(|pt| rest.contains(pt)) && !open;
                    let in_row = pointer.is_some_and(|pt| rect.contains(pt));
                    let want_open = on_sub || (open && in_row);
                    if want_open != open {
                        ctx.data_mut(|d| d.insert_temp(exp_id, want_open));
                        ctx.request_repaint();
                    }
                } else {
                    text_clipped(ui, rest, &sub, FontId::proportional(11.5), p.weak);
                }
            }
            None => text_clipped(ui, name_rect, &name, FontId::proportional(14.0), name_color),
        }
        // Тянуть программу на другое устройство — за значок или имя.
        if snapshot.can_route && !a.system {
            // Весь значок и имя: значок выше и ниже строки имени.
            let handle = icon_rect.union(name_rect);
            let drag = ui.interact(handle, ui.id().with(("appdrag", key_id(&a.key))), Sense::drag());
            // Только левой кнопкой: правая — меню строки, она не должна начинать перенос.
            if drag.drag_started_by(egui::PointerButton::Primary) {
                egui::DragAndDrop::set_payload(&ctx, DragApp { key: a.key.clone(), name: name.clone(), flow });
            }
            if drag.hovered() || drag.dragged() {
                ctx.set_cursor_icon(if drag.dragged() { CursorIcon::Grabbing } else { CursorIcon::Grab });
            }
        }
        let track = Rect::from_min_max(
            pos2(left - 7.0, base.top() + 27.0 + extra),
            pos2(number.left() - 10.0, base.top() + 43.0 + extra),
        );
        let target = Target::App(a.key.clone());
        let shown = self.shown_value(&target, a.volume);
        let out = slider(ui, ui.id().with(("app", key_id(&a.key))), track, shown, quiet, peak);
        if let Some(v) = out.value {
            self.set_volume(target.clone(), v);
        }
        number_text(ui, number, shown, quiet);
        let mute = mute_button(ui, button, ui.id().with(("appmute", key_id(&a.key))), shown, a.muted).clicked();
        if mute || out.toggle_mute {
            let _ = self.control.set_muted(target.clone(), !a.muted);
        }
        if a.system {
            return;
        }
        let mut open_hotkeys = false;
        // Меню программы: сначала куда её перенести — устройства сразу в меню, без подменю (в подменю
        // трудно попасть), затем «без звука», сочетания, «Скрыть».
        row.context_menu(|ui| {
            anvil_ui::widgets::section_label(ui, T::Device.get());
            if !snapshot.can_route {
                ui.add_enabled(false, egui::Button::new(T::RouteUnavailable.get()));
            } else {
                if check_item(ui, a.pinned.is_none(), T::AsSystem.get()) {
                    self.move_app(a.key.clone(), None);
                    ui.close();
                }
                for d in snapshot.devices.iter().filter(|d| d.flow == flow) {
                    if check_item(ui, a.pinned.as_deref() == Some(d.id.as_str()), &d.name) {
                        self.move_app(a.key.clone(), Some(d.id.clone()));
                        ui.close();
                    }
                }
            }
            anvil_ui::widgets::menu_separator(ui);
            let text = if a.muted { T::Unmute.get() } else { T::Mute.get() };
            if anvil_ui::widgets::menu_item(ui, None, text, Some("M")).clicked() {
                let _ = self.control.set_muted(target.clone(), !a.muted);
                ui.close();
            }
            if !a.key.exe.starts_with("pid:")
                && anvil_ui::widgets::menu_item(ui, Some(anvil_ui::Icon::Gear), T::HotkeysMenu.get(), None).clicked()
            {
                open_hotkeys = true;
                ui.close();
            }
            if anvil_ui::widgets::menu_item(ui, None, T::Hide.get(), None).clicked() {
                let mut st = self.shared.settings.lock().unwrap();
                st.hidden_apps.insert(a.key.exe.clone());
                st.save();
                ui.close();
            }
        });
        if open_hotkeys {
            self.page = Page::Hotkeys;
            self.focus_app = Some(a.key.exe.clone());
        }
    }

    /// Закрепить программу за устройством (`None` — «как в системе») и запомнить выбор: Windows держит
    /// закрепление за процессом и забывает его вместе с ним.
    fn move_app(&self, key: AppKey, to: Option<String>) {
        let exe = key.exe.clone();
        if self.control.move_app(key, to.clone()).is_ok() && !exe.starts_with("pid:") {
            let mut st = self.shared.settings.lock().unwrap();
            match to {
                Some(dev) => st.app_routes.insert(exe, dev),
                None => st.app_routes.remove(&exe),
            };
            st.save();
        }
    }

    /// Перетаскивание программы: подсветить подходящие блоки, при отпускании — перенести.
    fn finish_drag(&mut self, ctx: &egui::Context) {
        let Some(drag) = egui::DragAndDrop::payload::<DragApp>(ctx) else { return };
        let pointer = ctx.input(|i| i.pointer.interact_pos());
        let p = crate::contrast::palette(ctx);
        let layer = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("kamerton-drag")));
        let mut over = None;
        for (id, flow, rect) in &self.targets {
            if *flow != drag.flow || *id == drag.key.device {
                continue;
            }
            let hit = pointer.is_some_and(|pt| rect.contains(pt));
            let fill = if hit { p.soft(p.accent) } else { p.soft(p.weak) };
            layer.rect_filled(rect.expand(2.0), radius::CONTROL, fill);
            if hit {
                layer.rect_stroke(rect.expand(2.0), radius::CONTROL, Stroke::new(1.5, p.accent), StrokeKind::Inside);
                over = Some(id.clone());
            }
        }
        if let Some(pt) = pointer {
            let galley = layer.layout_no_wrap(drag.name.clone(), FontId::proportional(13.0), p.text);
            let card = Rect::from_min_size(pt + vec2(14.0, 6.0), galley.size() + vec2(20.0, 12.0));
            layer.rect_filled(card, radius::CONTROL, p.card);
            layer.rect_stroke(card, radius::CONTROL, Stroke::new(1.0, p.border_strong), StrokeKind::Inside);
            layer.galley(card.min + vec2(10.0, 6.0), galley, p.text);
        }
        if ctx.input(|i| i.pointer.any_released()) {
            egui::DragAndDrop::clear_payload(ctx);
            if let Some(to) = over {
                self.move_app(drag.key.clone(), Some(to));
            }
        }
    }
}

/// Выдуманный снимок для проверки вида: колонки и двое одинаковых наушников, у двух программ звук идёт в разные.
fn demo_snapshot() -> Option<Snapshot> {
    std::env::var_os("KAMERTON_DEMO_MIXER")?;
    let app = |device: &str, exe: &str, name: &str, volume: f32, pinned: Option<&str>| App {
        key: AppKey { device: device.into(), exe: exe.into() },
        name: name.into(),
        path: None,
        volume,
        muted: false,
        system: false,
        pids: vec![100],
        pinned: pinned.map(str::to_string),
    };
    let device = |id: &str, name: &str, default: bool, flow: Flow, apps: Vec<App>| Device {
        id: id.into(),
        name: name.into(),
        flow,
        volume: 0.54,
        muted: false,
        is_default: default,
        is_comm: default,
        balance: None,
        apps,
    };
    Some(Snapshot {
        can_route: true,
        devices: vec![
            device(
                "spk",
                "Speakers (Demo Audio)",
                true,
                Flow::Render,
                vec![app("spk", "unity.exe", "Unity Editor", 1.0, None)],
            ),
            device(
                "hp5",
                "Headphones (WH-1000XM5 Stereo)",
                false,
                Flow::Render,
                vec![app("hp5", "spotify.exe", "Spotify", 0.62, Some("hp5"))],
            ),
            device(
                "hp4",
                "Headphones (WH-1000XM4 Stereo)",
                false,
                Flow::Render,
                vec![app("hp4", "discord.exe", "Discord", 0.35, Some("hp4"))],
            ),
            device("mic", "Microphone (Demo Audio)", true, Flow::Capture, vec![]),
        ],
    })
}

fn key_id(k: &AppKey) -> (String, String) {
    (k.device.clone(), k.exe.clone())
}

/// Текст в одну строку; длинный — с «…» (без подсказки). `true` — текст усечён.
fn paint_clipped(ui: &mut Ui, rect: Rect, text: &str, font: FontId, color: Color32) -> bool {
    let galley = ui.painter().layout_no_wrap(text.to_string(), font.clone(), color);
    if galley.size().x <= rect.width() {
        ui.painter().galley(pos2(rect.left(), rect.center().y - galley.size().y / 2.0), galley, color);
        return false;
    }
    let mut cut: String = text.to_string();
    while !cut.is_empty() {
        cut.pop();
        let g = ui.painter().layout_no_wrap(format!("{cut}…"), font.clone(), color);
        if g.size().x <= rect.width() {
            ui.painter().galley(pos2(rect.left(), rect.center().y - g.size().y / 2.0), g, color);
            break;
        }
    }
    true
}

/// Текст в одну строку; длинный — с «…», полный — в подсказке.
fn text_clipped(ui: &mut Ui, rect: Rect, text: &str, font: FontId, color: Color32) {
    if paint_clipped(ui, rect, text, font, color) {
        ui.interact(rect, ui.id().with(("tip", text)), Sense::hover()).on_hover_text(text);
    }
}

fn number_text(ui: &mut Ui, rect: Rect, v: f32, muted: bool) {
    let p = crate::contrast::palette(ui.ctx());
    let color = if muted { p.faint } else { p.weak };
    ui.painter().text(
        pos2(rect.right(), rect.center().y),
        Align2::RIGHT_CENTER,
        model::percent(v).to_string(),
        FontId::monospace(12.5),
        color,
    );
}

/// Кнопка «без звука»: динамик 17 в кнопке-значке 30, как `icon_button` набора.
fn mute_button(ui: &mut Ui, rect: Rect, id: egui::Id, level: f32, muted: bool) -> egui::Response {
    let p = crate::contrast::palette(ui.ctx());
    let response = ui.interact(rect, id, Sense::click());
    let hovered = response.hovered() || response.is_pointer_button_down_on();
    if hovered {
        ui.painter().rect_filled(rect, radius::CONTROL, p.hover);
    }
    let color = if muted {
        p.warning
    } else if hovered {
        p.text
    } else {
        p.weak
    };
    icons::speaker(ui.painter(), Rect::from_center_size(rect.center(), Vec2::splat(17.0)), color, level, muted);
    anvil_ui::widgets::focus_ring(ui, rect, &response, radius::CONTROL);
    let hint = if muted { T::Unmute.get() } else { T::Mute.get() };
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, hint));
    response.on_hover_cursor(CursorIcon::PointingHand).on_hover_text(hint)
}

/// Кнопка устройств ввода внизу окна: микрофон и стрелка (вниз — показать, вверх — скрыть) — понятно и
/// без подсказки. Раскрыто — подложка `raised`, как у выбранного.
fn inputs_toggle(ui: &mut Ui, expanded: bool) -> egui::Response {
    let p = crate::contrast::palette(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(vec2(TOGGLE_W, BUTTON), Sense::click());
    let hovered = response.hovered() || response.is_pointer_button_down_on();
    if hovered {
        ui.painter().rect_filled(rect, radius::CONTROL, p.hover);
    } else if expanded {
        ui.painter().rect_filled(rect, radius::CONTROL, p.raised);
    }
    let color = if hovered || expanded { p.text } else { p.weak };
    let mic = Rect::from_center_size(pos2(rect.left() + 6.0 + 8.0, rect.center().y), Vec2::splat(16.0));
    icons::device(ui.painter(), mic, color, icons::DeviceKind::Mic);
    let arrow = Rect::from_center_size(pos2(rect.right() - 6.0 - 7.0, rect.center().y), Vec2::splat(14.0));
    let icon = if expanded { anvil_ui::Icon::ChevronUp } else { anvil_ui::Icon::ArrowDown };
    anvil_ui::icons::paint(ui.painter(), arrow, icon, color);
    anvil_ui::widgets::focus_ring(ui, rect, &response, radius::CONTROL);
    let hint = if expanded { T::HideInputs.get() } else { T::ShowInputs.get() };
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, expanded, hint));
    response.on_hover_cursor(CursorIcon::PointingHand).on_hover_text(hint)
}

/// Что сделали с ползунком за кадр.
struct SliderOut {
    value: Option<f32>,
    /// Нажали M, пока ползунок в фокусе.
    toggle_mute: bool,
    /// Отпустили после перетаскивания.
    released: bool,
}

/// Ползунок громкости 0..1 с полоской уровня. Ловит только левую кнопку: правый щелчок уходит строке
/// (её меню).
fn slider(ui: &mut Ui, id: egui::Id, rect: Rect, value: f32, muted: bool, peak: f32) -> SliderOut {
    let log = ui.ctx().data(|d| d.get_temp::<bool>(egui::Id::new("kamerton-log-scale"))).unwrap_or(false);
    slider_scaled(ui, id, rect, value, muted, peak, log, 0.0)
}

/// Ползунок с явной шкалой: баланс всегда ровный (середина — в середине), громкость — по настройке.
/// `origin` — откуда идёт заливка: 0 — от левого края (громкость), 0.5 — от середины с меткой (баланс).
#[allow(clippy::too_many_arguments)]
fn slider_scaled(
    ui: &mut Ui,
    id: egui::Id,
    rect: Rect,
    value: f32,
    muted: bool,
    peak: f32,
    log: bool,
    origin: f32,
) -> SliderOut {
    let p = crate::contrast::palette(ui.ctx());
    let response = ui.interact(rect, id, Sense::drag());
    let track = Rect::from_min_max(
        pos2(rect.left() + 7.0, rect.center().y - 2.0),
        pos2(rect.right() - 7.0, rect.center().y + 2.0),
    );
    let mut out = None;
    let pressed = response.is_pointer_button_down_on() && ui.input(|i| i.pointer.primary_down());
    if pressed && let Some(pos) = response.interact_pointer_pos() {
        out = Some(model::step(model::from_pos((pos.x - track.left()) / track.width(), log), 0.0));
        response.request_focus();
    }
    if response.has_focus() {
        let delta = ui.input(|i| {
            let mut d = 0.0;
            if i.key_pressed(Key::ArrowRight)
                || i.key_pressed(Key::ArrowUp)
                || i.key_pressed(Key::Plus)
                || i.key_pressed(Key::Equals)
            {
                d += STEP;
            }
            if i.key_pressed(Key::ArrowLeft) || i.key_pressed(Key::ArrowDown) || i.key_pressed(Key::Minus) {
                d -= STEP;
            }
            if i.key_pressed(Key::PageUp) {
                d += PAGE;
            }
            if i.key_pressed(Key::PageDown) {
                d -= PAGE;
            }
            d
        });
        if delta != 0.0 {
            out = Some(model::step(value, delta));
        }
    }
    if response.hovered() {
        let notches: f32 = ui.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    Event::MouseWheel { unit: MouseWheelUnit::Line, delta, .. } => Some(delta.y),
                    Event::MouseWheel { delta, .. } => Some(delta.y / 50.0),
                    _ => None,
                })
                .sum()
        });
        if notches != 0.0 {
            out = Some(model::step(out.unwrap_or(value), notches.signum() * STEP * notches.abs().round().max(1.0)));
        }
    }
    let v = out.unwrap_or(value);
    // Положение на дорожке — по шкале; громкость и число — как есть.
    let pos = model::to_pos(v, log);
    let fill = if muted { p.faint } else { p.accent };
    ui.painter().rect_filled(track, CornerRadius::same(2), p.border_strong);
    if origin > 0.0 {
        let x = track.left() + track.width() * origin;
        ui.painter()
            .line_segment([pos2(x, track.center().y - 6.0), pos2(x, track.center().y + 6.0)], Stroke::new(2.0, p.weak));
    }
    let (from, to) = (origin.min(pos), origin.max(pos));
    let filled = Rect::from_min_max(
        pos2(track.left() + track.width() * from, track.top()),
        pos2(track.left() + track.width() * to, track.bottom()),
    );
    ui.painter().rect_filled(filled, CornerRadius::same(2), fill);
    if !muted && peak > 0.0 {
        let line = Rect::from_min_max(
            pos2(track.left(), track.bottom() + 2.0),
            pos2(track.left() + track.width() * model::to_pos(peak, log), track.bottom() + 4.0),
        );
        ui.painter().rect_filled(line, CornerRadius::same(1), p.accent_text);
    }
    let knob = pos2(track.left() + track.width() * pos, track.center().y);
    let r = if response.hovered() || response.dragged() { 8.0 } else { 7.0 };
    ui.painter().circle(knob, r, fill, Stroke::new(2.0, p.card));
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.expand(1.0),
            radius::CONTROL,
            Stroke::new(2.0, p.accent_text),
            StrokeKind::Outside,
        );
    }
    response.widget_info(|| egui::WidgetInfo::slider(true, (v * 100.0) as f64, ""));
    let toggle_mute = response.has_focus() && ui.input(|i| i.key_pressed(Key::M));
    let released = response.drag_stopped();
    let _ = response.on_hover_cursor(CursorIcon::PointingHand);
    SliderOut { value: out, toggle_mute, released }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::{App, AppKey};

    fn dev(id: &str, default: bool, apps: usize) -> Device {
        Device {
            id: id.into(),
            name: id.into(),
            flow: Flow::Render,
            volume: 0.5,
            muted: false,
            is_default: default,
            is_comm: false,
            balance: None,
            apps: (0..apps)
                .map(|i| App {
                    key: AppKey { device: id.into(), exe: format!("{i}.exe") },
                    name: format!("{i}"),
                    path: None,
                    volume: 1.0,
                    muted: false,
                    system: false,
                    pids: vec![i as u32],
                    pinned: None,
                })
                .collect(),
        }
    }

    #[test]
    fn default_device_first_and_inputs_hidden() {
        let mut mic = dev("mic", true, 0);
        mic.flow = Flow::Capture;
        let s = Snapshot { can_route: true, devices: vec![dev("a", false, 0), mic, dev("b", true, 0)] };
        let ids: Vec<&str> = outputs(&s).iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, vec!["b", "a"]);
    }

    #[test]
    fn height_counts_rows() {
        let mut mic = dev("mic", true, 1);
        mic.flow = Flow::Capture;
        mic.apps[0].system = true;
        let s = Snapshot { can_route: true, devices: vec![dev("a", true, 2), dev("b", false, 1), mic] };
        // Свёрнуто: оба вывода в списке, в микшере программы обоих (2 + 1).
        let outs = HEADER_ROW + 2.0 * (ITEM_ROW + ITEM_GAP) + DIVIDER + MIXER_HEADER + 3.0 * APP_ROW;
        assert_eq!(content_height(&s, false), TOP + outs + BOTTOM + FOOTER);
        // Развёрнуто: ещё раздел ввода с заголовком «Программы записи»; у микрофона «Системные звуки» не
        // показываются — «Микрофон никто не слушает».
        let ins = DIVIDER + HEADER_ROW + (ITEM_ROW + ITEM_GAP) + DIVIDER + MIXER_HEADER + EMPTY_ROW;
        assert_eq!(content_height(&s, true), TOP + outs + ins + BOTTOM + FOOTER);
        assert_eq!(content_height(&Snapshot::default(), true), TOP + EMPTY_ROW + BOTTOM + FOOTER);
    }

    #[test]
    fn muted_default_device_adds_banner() {
        let mut a = dev("a", true, 1);
        a.muted = true;
        let quiet = Snapshot { can_route: true, devices: vec![a.clone()] };
        a.muted = false;
        let loud = Snapshot { can_route: true, devices: vec![a] };
        assert_eq!(content_height(&quiet, false), content_height(&loud, false) + MUTED_ROW);
        // «Без звука» не у основного — полосы нет.
        let mut b = dev("b", false, 0);
        b.muted = true;
        let s = Snapshot { can_route: true, devices: vec![dev("a", true, 1), b] };
        assert!(default_muted(&outputs(&s)).is_none());
    }

    #[test]
    fn balance_in_words() {
        assert_eq!(balance_text(0.0), T::BalanceCenter.get());
        assert_eq!(balance_text(-0.2), format!("{} 20", T::BalanceL.get()));
        assert_eq!(balance_text(0.35), format!("{} 35", T::BalanceR.get()));
    }

    #[test]
    fn mixer_lists_default_device_apps_first() {
        let s = Snapshot { can_route: true, devices: vec![dev("a", false, 1), dev("b", true, 2)] };
        let order: Vec<&str> = mixer_apps(&outputs(&s)).iter().map(|(d, _)| d.id.as_str()).collect();
        assert_eq!(order, vec!["b", "b", "a"]);
        // «Системные звуки» — только у устройства по умолчанию.
        let mut a = dev("a", false, 1);
        let mut b = dev("b", true, 1);
        a.apps[0].system = true;
        b.apps[0].system = true;
        let s = Snapshot { can_route: true, devices: vec![a, b] };
        let order: Vec<&str> = mixer_apps(&outputs(&s)).iter().map(|(d, _)| d.id.as_str()).collect();
        assert_eq!(order, vec!["b"]);
    }
}
