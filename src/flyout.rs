// Выпадающее окно микшера. Живёт в процессе трея с самого старта (как у EarTrumpet): спрятано и закрыто
// DWM-плащом (cloak), по щелчку встаёт на место, рисует кадр и проявляется сразу после него — без создания
// окна и устройства на щелчок. Пока окно закрыто, кадров нет; пока открыто — снимок и уровни каждый кадр.
// `kamerton --flyout` — то же окно отдельным процессом, для проверки вида.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anvil_ui::Palette;
use anvil_ui::theme::radius;
use eframe::egui::{
    self, Align2, Color32, CornerRadius, CursorIcon, Event, FontId, Key, MouseWheelUnit, Rect, Sense, Stroke,
    StrokeKind, TextureHandle, TextureOptions, Ui, Vec2, pos2, vec2,
};

use crate::audio::{App, AppKey, Audio, Control, Device, Flow, Roles, Snapshot, Target, model};
use crate::config::{LangPref, Settings, ThemePref};
use crate::hotkeys::{Action, AppAction, Combo};
use crate::icons;
use crate::placement::{self, Edge, Rect as PxRect};
use crate::sys;
use crate::texts::T;

/// Размеры окна и строк, логические пиксели (`docs/ui-guideline.md`).
pub const WIDTH: f32 = 360.0;
pub const PAD_X: f32 = 16.0;
pub const PAD_Y: f32 = 12.0;
pub const DEVICE_ROW: f32 = 52.0;
pub const APP_ROW: f32 = 44.0;
pub const EMPTY_ROW: f32 = 30.0;
/// Между блоками устройств: отступ, линия, отступ.
pub const SECTION_GAP: f32 = 6.0 * 2.0 + 1.0;
/// Подпись «ЗАПИСЬ» над устройствами записи.
pub const LABEL_ROW: f32 = 28.0;
/// Низ окна: отступ и кнопка «Все устройства» / «Только основное».
pub const FOOTER: f32 = 6.0 + 30.0;
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
/// Окно микшера: ширина колонки устройства и строка баланса.
const MIXER_COLUMN: f32 = 340.0;
const BALANCE_ROW: f32 = 40.0;
const SLIDE_SECS: f32 = 0.18;
/// Окно закрылось — щелчок по значку в эти мс его не открывает: это тот же щелчок, что снял фокус.
const REOPEN_GUARD_MS: u64 = 300;

const HIDDEN: u8 = 0;
const OPENING: u8 = 1;
const OPEN: u8 = 2;

/// Что стоит в окне сверху вниз.
#[derive(Debug, Clone, PartialEq)]
pub enum Block<'a> {
    Device(&'a Device),
    /// Подпись «ЗАПИСЬ» перед устройствами записи.
    RecordingLabel,
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

/// Блоки окна. Свёрнуто — только устройство вывода по умолчанию; развёрнуто — все устройства вывода,
/// затем подпись и устройства записи.
pub fn blocks(s: &Snapshot, expanded: bool) -> Vec<Block<'_>> {
    let outs = outputs(s);
    if !expanded {
        return outs.into_iter().take(1).map(Block::Device).collect();
    }
    let mut v: Vec<Block> = outs.into_iter().map(Block::Device).collect();
    let ins = of_flow(s, Flow::Capture);
    if !ins.is_empty() {
        v.push(Block::RecordingLabel);
        v.extend(ins.into_iter().map(Block::Device));
    }
    v
}

/// Программы устройства в окне: у записи — без «Системных звуков».
fn shown_apps(d: &Device) -> Vec<&App> {
    d.apps.iter().filter(|a| !(d.flow == Flow::Capture && a.system)).collect()
}

/// Высота содержимого окна: от неё место окна считается ещё до кадра.
pub fn content_height(s: &Snapshot, expanded: bool) -> f32 {
    let list = blocks(s, expanded);
    if list.is_empty() {
        return 2.0 * PAD_Y + EMPTY_ROW + FOOTER;
    }
    let mut h = 0.0;
    let mut prev_device = false;
    for b in &list {
        match b {
            Block::Device(d) => {
                if prev_device {
                    h += SECTION_GAP;
                }
                let n = shown_apps(d).len();
                h += DEVICE_ROW + if n == 0 { EMPTY_ROW } else { n as f32 * APP_ROW };
                prev_device = true;
            }
            Block::RecordingLabel => {
                h += SECTION_GAP + LABEL_ROW;
                prev_device = false;
            }
        }
    }
    2.0 * PAD_Y + h + FOOTER
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
        };
        s.wheel_outside.store(wheel, Ordering::SeqCst);
        s
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
            _ => {}
        }
    }

    /// Открыть окно у `anchor`. Спрятанному окну Windows не шлёт WM_PAINT, поэтому сначала показать его
    /// под плащом (невидимым, без фокуса), потом попросить кадр.
    pub fn open(&self, anchor: PxRect, by_click: bool) {
        if std::env::var_os("KAMERTON_TRACE").is_some() {
            eprintln!("trace: open requested at {} ms", sys::now_ms() % 100_000);
        }
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
    /// Блоки устройств на этом кадре — цели перетаскивания.
    targets: Vec<(String, Flow, Rect)>,
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
            Ok("settings") => f.page = Page::Settings,
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
            targets: Vec::new(),
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
        let mut f = Flyout::make(audio, hwnd, shared, true, 0.0);
        f.mixer = true;
        Ok(f)
    }

    /// Окно микшера: колонка на устройство — строка устройства, баланс, программы.
    fn mixer_ui(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let Ok(all) = self.control.snapshot() else { return };
        let snapshot = self.shared.visible(&all);
        self.all = all;
        let peaks: HashMap<Target, f32> = self.control.peaks().into_iter().collect();
        let p = Palette::of_ctx(&ctx);
        self.targets.clear();
        let frame = egui::Frame::new().fill(p.bg).inner_margin(egui::Margin::same(16));
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing = vec2(12.0, 0.0);
                    let mut devs: Vec<&Device> = outputs(&snapshot);
                    devs.extend(of_flow(&snapshot, Flow::Capture));
                    for d in devs {
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
                                        self.app_row(ui, a, d.flow, &snapshot, peak);
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
                });
            });
        });
        self.finish_drag(&ctx);
        if self.snap_check(&ctx) {
            return;
        }
        ctx.request_repaint_after(Duration::from_millis(33));
    }

    /// Баланс устройства: «Л» — ползунок — «П».
    fn balance_row(&mut self, ui: &mut Ui, d: &Device, b: f32) {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), BALANCE_ROW), Sense::hover());
        let p = Palette::of(ui);
        ui.painter().text(
            pos2(rect.left(), rect.top() + 9.0),
            Align2::LEFT_CENTER,
            T::Balance.get(),
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
        let target = Target::Device(format!("balance:{}", d.id));
        let shown = self.shown_value(&target, (b + 1.0) / 2.0);
        let out = slider(ui, ui.id().with(("balance", &d.id)), track, shown, false, 0.0);
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
        let (mut x, mut y, h) =
            placement::place(s.anchor, s.work, s.edge, (w, (self.want_h * s.scale).round() as i32), gap);
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
        let p = Palette::of_ctx(ctx);
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
        // Окно микшера: запомнить место и размер.
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
        let log = self.shared.settings.lock().unwrap().log_scale;
        ctx.data_mut(|d| d.insert_temp(egui::Id::new("kamerton-log-scale"), log));
        if self.mixer {
            self.mixer_ui(ui);
            return;
        }
        let opening = self.shared.pending.lock().unwrap().take();
        if let Some((anchor, by_click)) = opening {
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
        // Первый кадр в трее рисуем целиком (окно за экранами и под плащом): прогрев — значки программ,
        // глифы шрифтов, текстуры. Иначе первый щелчок ждал бы их ~0,2 с.
        let warming = !self.started && !self.standalone;
        if self.shared.state.load(Ordering::SeqCst) == HIDDEN && !warming {
            return;
        }
        self.frames += 1;
        let focused = ctx.input(|i| i.viewport().focused).unwrap_or(false);
        let lost = self.by_click && self.was_focused && !focused;
        let asked = self.shared.close.load(Ordering::SeqCst);
        let esc = self.capture.is_none() && ctx.input(|i| i.key_pressed(Key::Escape));
        if !warming && (lost || asked || esc) {
            self.close(ctx);
            return;
        }
        self.was_focused |= focused;

        let Ok(all) = self.control.snapshot() else { return };
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
        let want =
            if self.page != Page::Mixer { self.page_h } else { content_height(&snapshot, self.shared.expanded()) };
        if (self.want_h - want).abs() > 0.5 {
            self.want_h = want;
            self.layout();
        }

        let p = Palette::of_ctx(ctx);
        let frame = egui::Frame::new()
            .fill(p.card)
            .stroke(Stroke::new(1.0, p.border_strong))
            .corner_radius(radius::WINDOW)
            .inner_margin(egui::Margin::symmetric(PAD_X as i8, PAD_Y as i8));
        self.targets.clear();
        if self.page != Page::Mixer {
            let shown = egui::CentralPanel::default()
                .frame(frame)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| match self.page {
                            Page::Profiles => self.profiles_page(ui, &snapshot),
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
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                let list: Vec<Block> = blocks(&snapshot, self.shared.expanded());
                if list.is_empty() {
                    note_row(ui, T::NoDevices.get());
                }
                let mut prev_device = false;
                for b in &list {
                    match b {
                        Block::RecordingLabel => {
                            gap(ui);
                            label_row(ui, T::Recording.get());
                            prev_device = false;
                        }
                        Block::Device(d) => {
                            if prev_device {
                                gap(ui);
                            }
                            prev_device = true;
                            let top = ui.cursor().top();
                            let peak = peaks.get(&Target::Device(d.id.clone())).copied().unwrap_or(0.0);
                            self.device_row(ui, d, peak);
                            let apps = shown_apps(d);
                            if apps.is_empty() {
                                note_row(ui, T::NoApps.get());
                            }
                            for a in apps {
                                let peak = peaks.get(&Target::App(a.key.clone())).copied().unwrap_or(0.0);
                                self.app_row(ui, a, d.flow, &snapshot, peak);
                            }
                            let block = Rect::from_min_max(
                                pos2(ui.min_rect().left(), top),
                                pos2(ui.min_rect().right(), ui.cursor().top()),
                            );
                            self.targets.push((d.id.clone(), d.flow, block));
                        }
                    }
                }
                self.footer(ui);
            });
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
    let p = Palette::of(ui);
    ui.painter().text(
        pos2(rect.left(), rect.center().y),
        Align2::LEFT_CENTER,
        text,
        FontId::proportional(13.0),
        p.weak,
    );
}

/// Что показывает окно.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Mixer,
    Hotkeys,
    Profiles,
    Settings,
}

/// Поле сочетания: общее действие или действие программы.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Slot {
    Global(Action),
    App(AppAction, String),
}

impl Slot {
    /// Ключ в `settings.txt` — он же ключ состояния регистрации.
    fn key(&self) -> String {
        match self {
            Slot::Global(a) => format!("hotkey.{}", a.key()),
            Slot::App(a, exe) => format!("apphotkey.{}.{}", a.key(), exe),
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
        {
            let mut s = self.shared.settings.lock().unwrap();
            match (&slot, new) {
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
            }
            s.save();
        }
        self.stop_capture();
    }

    /// Строка: имя действия и поле сочетания набора.
    fn hotkey_row(&mut self, ui: &mut Ui, name: &str, slot: Slot, combo: Option<Combo>) {
        ui.add_space(4.0);
        ui.label(egui::RichText::new(name).size(14.0));
        let capturing = self.capture.as_ref() == Some(&slot);
        let parts: Vec<String> = combo.map(|c| c.parts()).unwrap_or_default();
        let keys: Vec<&str> = parts.iter().map(String::as_str).collect();
        let free = self.shared.hotkey_free.lock().unwrap().get(&slot.key()).copied();
        let status = match (capturing, combo, free) {
            (true, _, _) => Some((true, T::CaptureHint.get())),
            (false, Some(_), Some(false)) => Some((false, T::Taken.get())),
            (false, Some(_), _) => Some((true, T::Free.get())),
            (false, None, _) => None,
        };
        let change = if capturing { T::PressKeys.get() } else { T::Change.get() };
        let shown_keys: Vec<&str> = if keys.is_empty() && !capturing { vec![T::NotSet.get()] } else { keys };
        if anvil_ui::widgets::hotkey_field(ui, &shown_keys, change, status).clicked() && !capturing {
            self.stop_capture();
            self.start_capture(slot);
        }
        ui.add_space(4.0);
    }

    fn settings_page(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        ui.horizontal(|ui| {
            if anvil_ui::widgets::icon_button(ui, anvil_ui::Icon::ArrowLeft, T::Back.get()).clicked() {
                self.page = Page::Mixer;
            }
            ui.label(egui::RichText::new(T::Settings.get()).font(anvil_ui::semibold(14.5)));
        });
        ui.add_space(6.0);
        let mut st = self.shared.settings.lock().unwrap().clone();
        let before = st.clone();

        anvil_ui::widgets::section_label(ui, T::Appearance.get());
        anvil_ui::chrome::setting_row(ui, T::Theme.get(), |ui| {
            anvil_ui::widgets::segmented(
                ui,
                &mut st.theme,
                &[
                    (ThemePref::System, Some(anvil_ui::Icon::Monitor), T::ThemeSystem.get()),
                    (ThemePref::Light, Some(anvil_ui::Icon::Sun), T::ThemeLight.get()),
                    (ThemePref::Dark, Some(anvil_ui::Icon::Moon), T::ThemeDark.get()),
                ],
            );
        });
        anvil_ui::chrome::setting_row(ui, T::Language.get(), |ui| {
            anvil_ui::widgets::segmented(
                ui,
                &mut st.lang,
                &[
                    (LangPref::System, Some(anvil_ui::Icon::Monitor), T::ThemeSystem.get()),
                    (LangPref::Ru, None, "Русский"),
                    (LangPref::En, None, "English"),
                ],
            );
        });

        ui.add_space(10.0);
        anvil_ui::widgets::section_label(ui, T::WindowSection.get());
        anvil_ui::widgets::switch(ui, &mut st.animate, T::Animation.get());
        anvil_ui::widgets::switch(ui, &mut st.wheel_outside, T::WheelOutside.get());
        anvil_ui::widgets::switch(ui, &mut st.log_scale, T::LogScale.get());

        ui.add_space(10.0);
        anvil_ui::widgets::section_label(ui, T::ThemeSystem.get());
        let mut auto = sys::autostart();
        if anvil_ui::widgets::switch(ui, &mut auto, T::Autostart.get()).changed() {
            sys::set_autostart(auto);
        }

        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if anvil_ui::widgets::button(ui, anvil_ui::Kind::Secondary, None, T::Hotkeys.get()).clicked() {
                self.page = Page::Hotkeys;
            }
            if anvil_ui::widgets::button(ui, anvil_ui::Kind::Secondary, Some(anvil_ui::Icon::Layers), T::Profiles.get())
                .clicked()
            {
                self.page = Page::Profiles;
            }
        });

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
                .map(|a| a.name.clone())
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

        if st != before {
            if st.theme != before.theme {
                anvil_ui::choose(&ctx, theme_choice(st.theme));
            }
            if st.lang != before.lang {
                crate::texts::apply(st.lang);
                anvil_ui::lang::set_language(&ctx, kit_lang());
            }
            self.shared.wheel_outside.store(st.wheel_outside, Ordering::SeqCst);
            st.save();
            *self.shared.settings.lock().unwrap() = st;
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
            let track = Rect::from_min_max(row.min, pos2(number.left() - 10.0, row.bottom()));
            let out = slider(ui, ui.id().with("newapp"), track, value, false, 0.0);
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

    fn hotkeys_page(&mut self, ui: &mut Ui, snapshot: &Snapshot) {
        self.capture_keys(ui);
        ui.horizontal(|ui| {
            if anvil_ui::widgets::icon_button(ui, anvil_ui::Icon::ArrowLeft, T::Back.get()).clicked() {
                self.stop_capture();
                self.page = Page::Mixer;
            }
            ui.label(egui::RichText::new(T::Hotkeys.get()).font(anvil_ui::semibold(14.5)));
        });
        ui.add_space(6.0);
        anvil_ui::widgets::section_label(ui, T::General.get());
        let settings = self.shared.settings.lock().unwrap().clone();
        for a in Action::ALL {
            self.hotkey_row(ui, action_text(a), Slot::Global(a), settings.hotkeys.get(&a).copied());
        }
        // Программы: у которых уже есть сочетания, и та, чьё меню открыло страницу.
        let mut exes: Vec<String> = settings.app_hotkeys.keys().map(|(_, e)| e.clone()).collect();
        if let Some(e) = &self.focus_app {
            exes.push(e.clone());
        }
        exes.sort();
        exes.dedup();
        for exe in exes {
            ui.add_space(8.0);
            anvil_ui::widgets::divider(ui);
            ui.add_space(6.0);
            let name = snapshot
                .devices
                .iter()
                .flat_map(|d| d.apps.iter())
                .find(|a| a.key.exe == exe)
                .map(|a| a.name.clone())
                .unwrap_or_else(|| exe.rsplit(['\\', '/']).next().unwrap_or(&exe).to_string());
            anvil_ui::widgets::section_label(ui, &name);
            for a in AppAction::ALL {
                let combo = settings.app_hotkeys.get(&(a, exe.clone())).copied();
                self.hotkey_row(ui, app_action_text(a), Slot::App(a, exe.clone()), combo);
            }
        }
    }
}

fn gap(ui: &mut Ui) {
    ui.add_space(6.0);
    anvil_ui::widgets::divider(ui);
    ui.add_space(6.0);
}

/// Подпись раздела, как `section_label` набора, но в строке точной высоты.
fn label_row(ui: &mut Ui, text: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), LABEL_ROW), Sense::hover());
    let p = Palette::of(ui);
    ui.painter().text(
        pos2(rect.left(), rect.center().y),
        Align2::LEFT_CENTER,
        text.to_uppercase(),
        anvil_ui::semibold(11.0),
        p.weak,
    );
}

/// Пункт меню с отметкой текущего.
fn check_item(ui: &mut Ui, checked: bool, text: &str) -> bool {
    let icon = checked.then_some(anvil_ui::Icon::Check);
    anvil_ui::widgets::menu_item(ui, icon, text, None).clicked()
}

impl Flyout {
    fn footer(&mut self, ui: &mut Ui) {
        ui.add_space(6.0);
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
        let expanded = self.shared.expanded();
        let (icon, text) = if expanded {
            (anvil_ui::Icon::ChevronUp, T::DefaultOnly.get())
        } else {
            (anvil_ui::Icon::ArrowDown, T::AllDevices.get())
        };
        let (clicked, gear) = ui
            .scope_builder(
                egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::left_to_right(egui::Align::Center)),
                |ui| {
                    let c = anvil_ui::widgets::button(ui, anvil_ui::Kind::Ghost, Some(icon), text).clicked();
                    let g = ui
                        .with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let g =
                                anvil_ui::widgets::icon_button(ui, anvil_ui::Icon::Gear, T::Settings.get()).clicked();
                            if anvil_ui::widgets::icon_button(ui, anvil_ui::Icon::Layers, T::Profiles.get()).clicked() {
                                self.page = Page::Profiles;
                            }
                            g
                        })
                        .inner;
                    (c, g)
                },
            )
            .inner;
        if clicked {
            let mut s = self.shared.settings.lock().unwrap();
            s.expanded = !s.expanded;
            s.save();
        }
        if gear {
            self.page = Page::Settings;
            self.focus_app = None;
        }
    }

    fn device_row(&mut self, ui: &mut Ui, d: &Device, peak: f32) {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), DEVICE_ROW), Sense::hover());
        // Правый щелчок по строке — меню; ползунок и кнопка лежат поверх и ловят свои щелчки сами.
        let row = ui.interact(rect, ui.id().with(("devrow", &d.id)), Sense::click());
        let p = Palette::of(ui);
        let right = rect.right();
        let button = Rect::from_center_size(pos2(right - BUTTON / 2.0, rect.center().y), Vec2::splat(BUTTON));
        let number = Rect::from_min_max(
            pos2(button.left() - 8.0 - NUMBER_W, rect.top()),
            pos2(button.left() - 8.0, rect.bottom()),
        );
        let name_rect = Rect::from_min_max(rect.min + vec2(0.0, 6.0), pos2(number.left() - 8.0, rect.top() + 26.0));
        text_clipped(ui, name_rect, &d.name, anvil_ui::semibold(14.5), p.text);
        let track =
            Rect::from_min_max(pos2(rect.left(), rect.top() + 30.0), pos2(number.left() - 10.0, rect.top() + 46.0));
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

    fn app_row(&mut self, ui: &mut Ui, a: &App, flow: Flow, snapshot: &Snapshot, peak: f32) {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), APP_ROW), Sense::hover());
        let row = ui.interact(rect, ui.id().with(("approw", key_id(&a.key))), Sense::click());
        let p = Palette::of(ui);
        let ctx = ui.ctx().clone();
        let icon_rect = Rect::from_min_size(pos2(rect.left(), rect.center().y - ICON / 2.0), Vec2::splat(ICON));
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
        let button = Rect::from_center_size(pos2(rect.right() - BUTTON / 2.0, rect.center().y), Vec2::splat(BUTTON));
        let number = Rect::from_min_max(
            pos2(button.left() - 8.0 - NUMBER_W, rect.top()),
            pos2(button.left() - 8.0, rect.bottom()),
        );
        let left = icon_rect.right() + 10.0;
        let name = if a.system { T::SystemSounds.get().to_string() } else { a.name.clone() };
        let name_rect = Rect::from_min_max(pos2(left, rect.top() + 4.0), pos2(number.left() - 8.0, rect.top() + 22.0));
        text_clipped(ui, name_rect, &name, FontId::proportional(14.0), p.text);
        // Тянуть программу на другое устройство — за значок или имя.
        if snapshot.can_route && !a.system {
            let handle = Rect::from_min_max(icon_rect.min, name_rect.max);
            let drag = ui.interact(handle, ui.id().with(("appdrag", key_id(&a.key))), Sense::drag());
            if drag.drag_started() {
                egui::DragAndDrop::set_payload(&ctx, DragApp { key: a.key.clone(), name: name.clone(), flow });
            }
            if drag.hovered() || drag.dragged() {
                ctx.set_cursor_icon(if drag.dragged() { CursorIcon::Grabbing } else { CursorIcon::Grab });
            }
        }
        let track = Rect::from_min_max(pos2(left, rect.top() + 24.0), pos2(number.left() - 10.0, rect.top() + 40.0));
        let target = Target::App(a.key.clone());
        let shown = self.shown_value(&target, a.volume);
        let out = slider(ui, ui.id().with(("app", key_id(&a.key))), track, shown, a.muted, peak);
        if let Some(v) = out.value {
            self.set_volume(target.clone(), v);
        }
        number_text(ui, number, shown, a.muted);
        let mute = mute_button(ui, button, ui.id().with(("appmute", key_id(&a.key))), shown, a.muted).clicked();
        if mute || out.toggle_mute {
            let _ = self.control.set_muted(target.clone(), !a.muted);
        }
        if a.system {
            return;
        }
        let mut open_hotkeys = false;
        row.context_menu(|ui| {
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
            anvil_ui::widgets::menu_separator(ui);
            if !snapshot.can_route {
                ui.add_enabled(
                    false,
                    egui::Button::new(format!("{} ({})", T::Device.get(), T::RouteUnavailable.get())),
                );
                return;
            }
            ui.menu_button(T::Device.get(), |ui| {
                if check_item(ui, a.pinned.is_none(), T::AsSystem.get()) {
                    let _ = self.control.move_app(a.key.clone(), None);
                    ui.close();
                }
                anvil_ui::widgets::menu_separator(ui);
                for d in snapshot.devices.iter().filter(|d| d.flow == flow) {
                    if check_item(ui, a.pinned.as_deref() == Some(d.id.as_str()), &d.name) {
                        let _ = self.control.move_app(a.key.clone(), Some(d.id.clone()));
                        ui.close();
                    }
                }
            });
        });
        if open_hotkeys {
            self.page = Page::Hotkeys;
            self.focus_app = Some(a.key.exe.clone());
        }
    }

    /// Перетаскивание программы: подсветить подходящие блоки, при отпускании — перенести.
    fn finish_drag(&mut self, ctx: &egui::Context) {
        let Some(drag) = egui::DragAndDrop::payload::<DragApp>(ctx) else { return };
        let pointer = ctx.input(|i| i.pointer.interact_pos());
        let p = Palette::of_ctx(ctx);
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
            layer.text(pt + vec2(14.0, 4.0), Align2::LEFT_TOP, &drag.name, FontId::proportional(13.0), p.text);
        }
        if ctx.input(|i| i.pointer.any_released()) {
            egui::DragAndDrop::clear_payload(ctx);
            if let Some(to) = over {
                let _ = self.control.move_app(drag.key.clone(), Some(to));
            }
        }
    }
}

fn key_id(k: &AppKey) -> (String, String) {
    (k.device.clone(), k.exe.clone())
}

/// Текст в одну строку; длинный — с «…», полный — в подсказке.
fn text_clipped(ui: &mut Ui, rect: Rect, text: &str, font: FontId, color: Color32) {
    let galley = ui.painter().layout_no_wrap(text.to_string(), font.clone(), color);
    if galley.size().x <= rect.width() {
        ui.painter().galley(pos2(rect.left(), rect.center().y - galley.size().y / 2.0), galley, color);
        return;
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
    ui.interact(rect, ui.id().with(("tip", text)), Sense::hover()).on_hover_text(text);
}

fn number_text(ui: &mut Ui, rect: Rect, v: f32, muted: bool) {
    let p = Palette::of(ui);
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
    let p = Palette::of(ui);
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
    let p = Palette::of(ui);
    let log = ui.ctx().data(|d| d.get_temp::<bool>(egui::Id::new("kamerton-log-scale"))).unwrap_or(false);
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
    ui.painter().rect_filled(track, CornerRadius::same(2), p.raised);
    let filled = Rect::from_min_max(track.min, pos2(track.left() + track.width() * pos, track.bottom()));
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
        let s = Snapshot { can_route: true, devices: vec![dev("a", true, 2), dev("b", false, 0), mic] };
        let collapsed = 2.0 * PAD_Y + (DEVICE_ROW + 2.0 * APP_ROW) + FOOTER;
        assert_eq!(content_height(&s, false), collapsed);
        // Развёрнуто: оба вывода, подпись записи, микрофон без «Системных звуков» — «нет программ».
        let expanded = 2.0 * PAD_Y
            + (DEVICE_ROW + 2.0 * APP_ROW)
            + SECTION_GAP
            + (DEVICE_ROW + EMPTY_ROW)
            + SECTION_GAP
            + LABEL_ROW
            + (DEVICE_ROW + EMPTY_ROW)
            + FOOTER;
        assert_eq!(content_height(&s, true), expanded);
        assert_eq!(content_height(&Snapshot::default(), true), 2.0 * PAD_Y + EMPTY_ROW + FOOTER);
    }

    #[test]
    fn collapsed_shows_default_output_only() {
        let mut mic = dev("mic", true, 0);
        mic.flow = Flow::Capture;
        let s = Snapshot { can_route: true, devices: vec![dev("a", false, 0), dev("b", true, 0), mic] };
        let ids = |v: Vec<Block>| -> Vec<String> {
            v.into_iter()
                .map(|b| match b {
                    Block::Device(d) => d.id.clone(),
                    Block::RecordingLabel => "-".into(),
                })
                .collect()
        };
        assert_eq!(ids(blocks(&s, false)), vec!["b"]);
        assert_eq!(ids(blocks(&s, true)), vec!["b", "a", "-", "mic"]);
    }
}
