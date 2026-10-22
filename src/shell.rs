// Свой цикл окна вместо eframe: winit + egui-winit для окна и ввода, рисовальщик D3D11 (`d3d.rs`).
// eframe тянул контекст OpenGL (~90 МБ своей памяти на маленькое окно), D3D11 обходится в единицы МБ.

use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, ViewportCommand, ViewportId, ViewportInfo};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::platform::windows::WindowAttributesExtWindows;
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::{Window, WindowId, WindowLevel};

use crate::d3d::Painter;

/// Что рисует окно: создаётся, когда окно уже есть (нужен HWND), и рисует кадр в корневой `Ui`.
pub trait Content {
    fn ui(&mut self, ui: &mut egui::Ui);
    /// Цвет, которым чистится окно перед кадром.
    fn clear(&self, ctx: &egui::Context) -> Color32;
    /// Кадр показан (`Present`): можно проявить окно — оно уже нарисовано и на месте.
    fn after_present(&mut self) {}
    /// Окно просят закрыть (крестик, Alt+F4, `WM_CLOSE`); `false` — не закрывать.
    fn close_requested(&mut self) -> bool {
        true
    }
}

pub struct Config {
    pub title: &'static str,
    /// Логический размер при создании.
    pub size: [f32; 2],
    pub decorations: bool,
    pub resizable: bool,
    pub always_on_top: bool,
    /// Без кнопки на панели задач.
    pub skip_taskbar: bool,
    /// Где создать, физические пиксели; `None` — где решит система.
    pub position: Option<[i32; 2]>,
    /// Давать окну фокус при показе.
    pub active: bool,
}

impl Config {
    /// Всплывающее окно: без рамки, поверх всех, без кнопки на панели задач, создаётся за экранами —
    /// место ставит содержимое на первом кадре.
    pub fn popup(title: &'static str, size: [f32; 2]) -> Config {
        Config {
            title,
            size,
            decorations: false,
            resizable: false,
            always_on_top: true,
            skip_taskbar: true,
            position: Some([-32000, -32000]),
            active: false,
        }
    }
}

type Make = Box<dyn FnOnce(&egui::Context, isize) -> Result<Box<dyn Content>, String>>;

/// Пробуждение из других потоков: egui просит кадр (`request_repaint`) через `EventLoopProxy`.
struct Wake(Instant);

struct Shell {
    config: Config,
    make: Option<Make>,
    ctx: egui::Context,
    window: Option<Window>,
    state: Option<egui_winit::State>,
    painter: Option<Painter>,
    content: Option<Box<dyn Content>>,
    next: Option<Instant>,
    pending: Vec<egui::Event>,
    /// Сведения об окне для egui (фокус, размер, масштаб) — их заполняет eframe, у нас — мы сами.
    info: ViewportInfo,
    shown: bool,
    error: Option<String>,
}

pub fn run(config: Config, make: Make) -> Result<(), String> {
    let event_loop = EventLoop::<Wake>::with_user_event().build().map_err(|e| e.to_string())?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let ctx = egui::Context::default();
    let wake = event_loop.create_proxy();
    ctx.set_request_repaint_callback(move |info| {
        let _ = wake.send_event(Wake(Instant::now() + info.delay));
    });
    let mut shell = Shell {
        config,
        make: Some(make),
        ctx,
        window: None,
        state: None,
        painter: None,
        content: None,
        next: None,
        pending: Vec::new(),
        info: ViewportInfo::default(),
        shown: false,
        error: None,
    };
    event_loop.run_app(&mut shell).map_err(|e| e.to_string())?;
    match shell.error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

impl Shell {
    fn fail(&mut self, el: &ActiveEventLoop, e: String) {
        self.error = Some(e);
        el.exit();
    }

    fn redraw(&mut self, el: &ActiveEventLoop) {
        let (Some(window), Some(state), Some(painter), Some(content)) =
            (&self.window, &mut self.state, &mut self.painter, &mut self.content)
        else {
            return;
        };
        let mut input = state.take_egui_input(window);
        input.events.append(&mut self.pending);
        // Без этого `viewport().focused` всегда пуст и потерю фокуса не увидеть.
        egui_winit::update_viewport_info(&mut self.info, &self.ctx, window, !self.shown);
        input.viewports.insert(ViewportId::ROOT, self.info.clone());
        let out = self.ctx.run_ui(input, |ui| content.ui(ui));
        state.handle_platform_output(window, out.platform_output);
        let mut capture = None;
        if let Some(v) = out.viewport_output.get(&ViewportId::ROOT) {
            for cmd in &v.commands {
                match cmd {
                    ViewportCommand::Close => el.exit(),
                    ViewportCommand::Screenshot(data) => capture = Some(data.clone()),
                    _ => {}
                }
            }
        }
        let prims = self.ctx.tessellate(out.shapes, out.pixels_per_point);
        let size = window.inner_size();
        if std::env::var_os("KAMERTON_TRACE").is_some() {
            eprintln!("trace: paint {}x{} ppp {}", size.width, size.height, out.pixels_per_point);
        }
        let clear = content.clear(&self.ctx);
        let mut textures = out.textures_delta;
        let painted =
            painter.paint([size.width, size.height], out.pixels_per_point, clear, &prims, &textures, capture.is_some());
        // egui требует отметить изменения текстур применёнными.
        textures.clear();
        let shot = match painted {
            Ok(s) => s,
            Err(e) => return self.fail(el, e),
        };
        if let (Some(user_data), Some(image)) = (capture, shot) {
            self.pending.push(egui::Event::Screenshot {
                viewport_id: ViewportId::ROOT,
                user_data,
                image: Arc::new(image),
            });
            window.request_redraw();
        }
        // Окно создано спрятанным: показать после первого нарисованного кадра — уже на своём месте.
        if !self.shown {
            self.shown = true;
            window.set_visible(true);
        }
        content.after_present();
    }
}

impl ApplicationHandler<Wake> for Shell {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let c = &self.config;
        let mut attrs = Window::default_attributes()
            .with_title(c.title)
            .with_decorations(c.decorations)
            .with_resizable(c.resizable)
            .with_visible(false)
            .with_inner_size(LogicalSize::new(c.size[0], c.size[1]))
            .with_skip_taskbar(c.skip_taskbar)
            .with_active(c.active);
        if c.always_on_top {
            attrs = attrs.with_window_level(WindowLevel::AlwaysOnTop);
        }
        if let Some([x, y]) = c.position {
            attrs = attrs.with_position(PhysicalPosition::new(x, y));
        }
        let window = match el.create_window(attrs) {
            Ok(w) => w,
            Err(e) => return self.fail(el, e.to_string()),
        };
        let hwnd = match window.window_handle().map(|h| h.as_raw()) {
            Ok(RawWindowHandle::Win32(h)) => h.hwnd.get(),
            _ => return self.fail(el, "no window handle".into()),
        };
        let painter = match Painter::new(hwnd) {
            Ok(p) => p,
            Err(e) => return self.fail(el, e),
        };
        let state = egui_winit::State::new(
            self.ctx.clone(),
            ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            window.theme(),
            Some(8192),
        );
        let content = match self.make.take().map(|m| m(&self.ctx, hwnd)) {
            Some(Ok(c)) => c,
            Some(Err(e)) => return self.fail(el, e),
            None => return,
        };
        self.window = Some(window);
        self.state = Some(state);
        self.painter = Some(painter);
        self.content = Some(content);
        // Невидимому окну Windows не шлёт WM_PAINT — первый кадр рисуем сами, он и покажет окно.
        self.redraw(el);
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::RedrawRequested => self.redraw(el),
            WindowEvent::CloseRequested => {
                if self.content.as_mut().is_none_or(|c| c.close_requested()) {
                    el.exit();
                }
            }
            WindowEvent::Destroyed => el.exit(),
            other => {
                if let (Some(window), Some(state)) = (&self.window, &mut self.state)
                    && state.on_window_event(window, &other).repaint
                {
                    window.request_redraw();
                }
            }
        }
    }

    fn user_event(&mut self, _el: &ActiveEventLoop, wake: Wake) {
        if wake.0 <= Instant::now() + Duration::from_millis(1) {
            if let Some(w) = &self.window {
                w.request_redraw();
            }
        } else {
            self.next = Some(self.next.map_or(wake.0, |n| n.min(wake.0)));
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        match self.next {
            Some(t) if t <= Instant::now() => {
                self.next = None;
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
                el.set_control_flow(ControlFlow::Wait);
            }
            Some(t) => el.set_control_flow(ControlFlow::WaitUntil(t)),
            None => el.set_control_flow(ControlFlow::Wait),
        }
    }
}
