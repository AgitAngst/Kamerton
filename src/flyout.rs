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

use crate::audio::{App, AppKey, Audio, Control, Device, Flow, Snapshot, Target, model};
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
const NUMBER_W: f32 = 34.0;
const BUTTON: f32 = 30.0;
const ICON: f32 = 24.0;
/// Шаг клавиш и колеса над ползунком.
const STEP: f32 = 0.02;
const PAGE: f32 = 0.10;
/// Сколько держать своё значение ползунка после движения, пока Windows не сообщила новое.
const HOLD: Duration = Duration::from_millis(400);
/// Окно закрылось — щелчок по значку в эти мс его не открывает: это тот же щелчок, что снял фокус.
const REOPEN_GUARD_MS: u64 = 300;

const HIDDEN: u8 = 0;
const OPENING: u8 = 1;
const OPEN: u8 = 2;

/// Устройства вывода: по умолчанию — первым.
pub fn outputs(s: &Snapshot) -> Vec<&Device> {
    let mut v: Vec<&Device> = s.devices.iter().filter(|d| d.flow == Flow::Render).collect();
    v.sort_by_key(|d| !d.is_default);
    v
}

/// Высота содержимого окна: от неё место окна считается ещё до первого кадра.
pub fn content_height(s: &Snapshot) -> f32 {
    let devs = outputs(s);
    if devs.is_empty() {
        return 2.0 * PAD_Y + EMPTY_ROW;
    }
    let blocks: f32 = devs
        .iter()
        .map(|d| DEVICE_ROW + if d.apps.is_empty() { EMPTY_ROW } else { d.apps.len() as f32 * APP_ROW })
        .sum();
    2.0 * PAD_Y + blocks + (devs.len() - 1) as f32 * SECTION_GAP
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
}

impl Shared {
    pub fn new(ctx: egui::Context, hwnd: isize) -> Shared {
        Shared {
            hwnd: AtomicIsize::new(hwnd),
            state: AtomicU8::new(HIDDEN),
            hidden_at: AtomicU64::new(0),
            pending: Mutex::new(None),
            close: AtomicBool::new(false),
            exiting: AtomicBool::new(false),
            ctx,
        }
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
}

fn install_look(ctx: &egui::Context) {
    let theme = match std::env::var("KAMERTON_THEME").ok().as_deref() {
        Some("light") => anvil_ui::ThemeChoice::Light,
        Some("dark") => anvil_ui::ThemeChoice::Dark,
        _ => anvil_ui::ThemeChoice::System,
    };
    anvil_ui::install(ctx, anvil_ui::Accent::BLUE, theme);
    anvil_ui::motion::install(ctx);
}

impl Flyout {
    /// Окно в процессе трея: создаётся при старте спрятанным под плащом и ждёт щелчка.
    pub fn in_tray(ctx: &egui::Context, hwnd: isize, audio: Audio, want_h: f32, shared: Arc<Shared>) -> Flyout {
        install_look(ctx);
        sys::cloak(hwnd, true);
        Flyout::make(audio, hwnd, shared, false, want_h)
    }

    /// Отдельный процесс `--flyout`: открыто сразу. `anchor` — прямоугольник значка; `None` — угол рабочей
    /// области основного монитора. `passive` — для проверки вида: не брать фокус и не закрываться без него.
    pub fn standalone(
        ctx: &egui::Context,
        hwnd: isize,
        anchor: Option<PxRect>,
        passive: bool,
    ) -> Result<Flyout, String> {
        install_look(ctx);
        let wake_ctx = ctx.clone();
        let (audio, snapshot) = Audio::start_quiet(move || wake_ctx.request_repaint()).map_err(|e| e.to_string())?;
        let by_click = anchor.is_some() && !passive;
        let anchor = anchor.unwrap_or_else(|| {
            let (_, work, _) = sys::monitor_at(0, 0);
            PxRect { left: work.right - 1, top: work.bottom - 1, right: work.right, bottom: work.bottom }
        });
        let shared = Arc::new(Shared::new(ctx.clone(), hwnd));
        *shared.pending.lock().unwrap() = Some((anchor, by_click));
        shared.state.store(OPENING, Ordering::SeqCst);
        Ok(Flyout::make(audio, hwnd, shared, true, content_height(&snapshot)))
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
        }
    }

    /// Поставить окно по месту и желаемой высоте.
    fn layout(&self) {
        let Some(s) = &self.spot else { return };
        let w = (WIDTH * s.scale).round() as i32;
        let gap = (placement::GAP * s.scale).round() as i32;
        let (x, y, h) = placement::place(s.anchor, s.work, s.edge, (w, (self.want_h * s.scale).round() as i32), gap);
        if std::env::var_os("KAMERTON_TRACE").is_some() {
            eprintln!(
                "trace: flyout at {x},{y} size {w}x{h}, anchor {:?}, edge {:?}, scale {}",
                s.anchor, s.edge, s.scale
            );
        }
        sys::move_to(self.hwnd, x, y, w, h);
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
        self.held.clear();
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
        Palette::of_ctx(ctx).card
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
            sys::cloak(self.hwnd, false);
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
        let opening = self.shared.pending.lock().unwrap().take();
        if let Some((anchor, by_click)) = opening {
            self.spot = Some(Spot::at(anchor));
            self.by_click = by_click;
            self.was_focused = false;
            self.frames = 0;
            self.reveal = true;
            if let Ok(s) = self.control.snapshot() {
                self.want_h = content_height(&s);
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
        if !warming && (lost || asked || ctx.input(|i| i.key_pressed(Key::Escape))) {
            self.close(ctx);
            return;
        }
        self.was_focused |= focused;

        let Ok(snapshot) = self.control.snapshot() else { return };
        let peaks: HashMap<Target, f32> = self.control.peaks().into_iter().collect();
        let want = content_height(&snapshot);
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
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                let devs: Vec<Device> = outputs(&snapshot).into_iter().cloned().collect();
                if devs.is_empty() {
                    note_row(ui, T::NoDevices.get());
                }
                for (i, d) in devs.iter().enumerate() {
                    if i > 0 {
                        ui.add_space(6.0);
                        anvil_ui::widgets::divider(ui);
                        ui.add_space(6.0);
                    }
                    self.device_row(ui, d, peaks.get(&Target::Device(d.id.clone())).copied().unwrap_or(0.0));
                    if d.apps.is_empty() {
                        note_row(ui, T::NoApps.get());
                    }
                    for a in &d.apps {
                        let peak = peaks.get(&Target::App(a.key.clone())).copied().unwrap_or(0.0);
                        self.app_row(ui, a, peak);
                    }
                }
            });
        });
        // Снимок для проверки вида: через несколько кадров после показа.
        if let Ok(path) = std::env::var("KAMERTON_SNAP") {
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
                return;
            }
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

impl Flyout {
    fn device_row(&mut self, ui: &mut Ui, d: &Device, peak: f32) {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), DEVICE_ROW), Sense::hover());
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
        if let Some(v) = slider(ui, ui.id().with(("dev", &d.id)), track, shown, d.muted, peak) {
            self.set_volume(target.clone(), v);
        }
        number_text(ui, number, shown, d.muted);
        if mute_button(ui, button, ui.id().with(("devmute", &d.id)), shown, d.muted).clicked() {
            let _ = self.control.set_muted(target, !d.muted);
        }
    }

    fn app_row(&mut self, ui: &mut Ui, a: &App, peak: f32) {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), APP_ROW), Sense::hover());
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
        let track = Rect::from_min_max(pos2(left, rect.top() + 24.0), pos2(number.left() - 10.0, rect.top() + 40.0));
        let target = Target::App(a.key.clone());
        let shown = self.shown_value(&target, a.volume);
        if let Some(v) = slider(ui, ui.id().with(("app", key_id(&a.key))), track, shown, a.muted, peak) {
            self.set_volume(target.clone(), v);
        }
        number_text(ui, number, shown, a.muted);
        if mute_button(ui, button, ui.id().with(("appmute", key_id(&a.key))), shown, a.muted).clicked() {
            let _ = self.control.set_muted(target, !a.muted);
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

/// Ползунок громкости 0..1 с полоской уровня. Возвращает новое значение, если его сдвинули.
fn slider(ui: &mut Ui, id: egui::Id, rect: Rect, value: f32, muted: bool, peak: f32) -> Option<f32> {
    let p = Palette::of(ui);
    let response = ui.interact(rect, id, Sense::click_and_drag());
    let track = Rect::from_min_max(
        pos2(rect.left() + 7.0, rect.center().y - 2.0),
        pos2(rect.right() - 7.0, rect.center().y + 2.0),
    );
    let mut out = None;
    if (response.dragged() || response.clicked())
        && let Some(pos) = response.interact_pointer_pos()
    {
        out = Some(model::step(((pos.x - track.left()) / track.width()).clamp(0.0, 1.0), 0.0));
        response.request_focus();
    }
    if response.has_focus() {
        let delta = ui.input(|i| {
            let mut d = 0.0;
            if i.key_pressed(Key::ArrowRight) || i.key_pressed(Key::ArrowUp) {
                d += STEP;
            }
            if i.key_pressed(Key::ArrowLeft) || i.key_pressed(Key::ArrowDown) {
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
    let fill = if muted { p.faint } else { p.accent };
    ui.painter().rect_filled(track, CornerRadius::same(2), p.raised);
    let filled = Rect::from_min_max(track.min, pos2(track.left() + track.width() * v, track.bottom()));
    ui.painter().rect_filled(filled, CornerRadius::same(2), fill);
    if !muted && peak > 0.0 {
        let line = Rect::from_min_max(
            pos2(track.left(), track.bottom() + 2.0),
            pos2(track.left() + track.width() * peak.clamp(0.0, 1.0), track.bottom() + 4.0),
        );
        ui.painter().rect_filled(line, CornerRadius::same(1), p.accent_text);
    }
    let knob = pos2(track.left() + track.width() * v, track.center().y);
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
    let _ = response.on_hover_cursor(CursorIcon::PointingHand);
    out
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
            apps: (0..apps)
                .map(|i| App {
                    key: AppKey { device: id.into(), exe: format!("{i}.exe") },
                    name: format!("{i}"),
                    path: None,
                    volume: 1.0,
                    muted: false,
                    system: false,
                    pids: vec![i as u32],
                })
                .collect(),
        }
    }

    #[test]
    fn default_device_first_and_inputs_hidden() {
        let mut mic = dev("mic", true, 0);
        mic.flow = Flow::Capture;
        let s = Snapshot { devices: vec![dev("a", false, 0), mic, dev("b", true, 0)] };
        let ids: Vec<&str> = outputs(&s).iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, vec!["b", "a"]);
    }

    #[test]
    fn height_counts_rows() {
        let s = Snapshot { devices: vec![dev("a", true, 2), dev("b", false, 0)] };
        let want = 2.0 * PAD_Y + (DEVICE_ROW + 2.0 * APP_ROW) + SECTION_GAP + (DEVICE_ROW + EMPTY_ROW);
        assert_eq!(content_height(&s), want);
        assert_eq!(content_height(&Snapshot::default()), 2.0 * PAD_Y + EMPTY_ROW);
    }
}
