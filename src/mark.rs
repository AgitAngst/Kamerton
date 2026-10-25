// Ход значков Kamerton — знака-камертона и колонки. Чистые машины состояний без egui: события с отметкой
// времени (секунды от часов программы) → кадр в момент `now`. Числа и ход — эталон `reference/mark-motion.html`
// изменения OpenSpec `stage-0-9-mark-motion` (в `openspec/changes/`, после архивации — в `openspec/changes/archive/`;
// классы `MarkScene`, `SpeakerScene`); где расходятся — прав эталон. Кадр рисует `mark_render`.

use anvil_ui::motion::machine::{LONG_AGO, Scene};
use anvil_ui::motion::{self, ease_in, ease_out, ease_standard};

/// Ручки едут к новой громкости.
pub const SLIDE: f32 = motion::LAYOUT;
/// «Без звука»: знак бледнеет.
pub const FADE: f32 = motion::STATE;
/// Непрозрачность знака без звука.
pub const MUTED_ALPHA: f32 = 0.55;
/// Звон при смене устройства.
pub const STRIKE: f32 = motion::WAVE;
/// Размах наклона зубцов, градусы.
pub const STRIKE_DEG: f32 = 5.0;
/// Затухание звона, с.
pub const STRIKE_TAU: f32 = 0.18;
/// Частота звона там, где кадров хватает (крупный знак эталона; в «О программе» звона нет).
pub const STRIKE_HZ: f32 = 7.0;
/// Частота звона в трее: 10 кадров в секунду не покажут 7 Гц.
pub const STRIKE_HZ_TRAY: f32 = 2.5;
/// Фейдеры: вниз, вверх до упора, на место; правая ручка позже левой.
pub const SWEEP_DOWN: f32 = 0.18;
pub const SWEEP_UP: f32 = 0.30;
pub const SWEEP_BACK: f32 = 0.30;
pub const SWEEP_STAGGER: f32 = 0.12;
const SWEEP: f32 = SWEEP_DOWN + SWEEP_UP + SWEEP_BACK;
/// «Поиграть» (щелчок по знаку в «О программе»): ручки плавно ездят туда-сюда навстречу друг другу.
pub const PLAY_LEN: f32 = 2.2;
pub const PLAY_CYCLES: f32 = 2.0;
/// Размах от положения покоя (доля хода ручки).
pub const PLAY_AMP: f32 = 0.42;

/// Смещение левой ручки (правой — со знаком минус) через `u` секунд после щелчка: размах плавно нарастает и сходит на нет
/// (`sin²`), так что ручки стартуют и встают на место без рывка.
pub fn play_offset(u: f32) -> f32 {
    if !(0.0..PLAY_LEN).contains(&u) {
        return 0.0;
    }
    let k = u / PLAY_LEN;
    let envelope = (std::f32::consts::PI * k).sin().powi(2);
    PLAY_AMP * envelope * (std::f32::consts::TAU * PLAY_CYCLES * k).sin()
}
/// Потолок кадров трея.
pub const TRAY_FPS: f64 = 10.0;

/// Колонка: волна появляется или гаснет.
pub const WAVE_IN: f32 = motion::LAYOUT;
/// Колонка: крестик «без звука» прорисовывается.
pub const CROSS: f32 = motion::STATE;
/// Колонка: толчок диффузора и насколько он вырастает.
pub const THUMP: f32 = 0.30;
pub const THUMP_SCALE: f32 = 0.14;
/// Колонка: волна уходит наружу; её радиус — от и до (сетка 32).
pub const RIPPLE: f32 = motion::WAVE;
pub const RIPPLE_FROM: f32 = 5.0;
pub const RIPPLE_TO: f32 = 16.0;
/// Колонка при запуске: первая волна после толчка и шаг между волнами.
pub const START_DELAY: f32 = 0.15;
pub const START_STEP: f32 = 0.15;
const START_LEN: f32 = START_DELAY + 2.0 * START_STEP + WAVE_IN;

/// Переход числа от `a` к `b`, начатый в `t0`.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Tween {
    a: f32,
    b: f32,
    t0: f64,
}

impl Tween {
    fn still(v: f32) -> Self {
        Tween { a: v, b: v, t0: LONG_AGO }
    }

    fn at(&self, now: f64, secs: f32, ease: fn(f32) -> f32) -> f32 {
        if now >= self.t0 + secs as f64 {
            return self.b;
        }
        let s = (((now - self.t0) / secs as f64) as f32).max(0.0);
        self.a + (self.b - self.a) * ease(s)
    }

    fn moving(&self, now: f64, secs: f32) -> bool {
        self.a != self.b && now < self.t0 + secs as f64
    }
}

// ─── Знак ────────────────────────────────────────────────────────────────────

/// Кадр знака: ручки — доля хода 0..1 (0 — низ дорожки), наклон зубцов в градусах, непрозрачность знака.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MarkFrame {
    pub l: f32,
    pub r: f32,
    pub ang: f32,
    pub alpha: f32,
    pub error: bool,
}

/// Знак-камертон: ручки = громкость каналов, «без звука», звон, фейдеры, ошибка.
#[derive(Debug, Clone)]
pub struct MarkScene {
    knob: [Tween; 2],
    vol: [f32; 2],
    muted: bool,
    error: bool,
    fade: Tween,
    strike_at: f64,
    sweep_at: f64,
    play_at: f64,
    hz: f32,
    /// «О программе»: в покое знак как в файле.
    file_rest: bool,
}

impl MarkScene {
    /// `home` — ручки, пока громкость неизвестна; `hz` — частота звона; `file_rest` — в покое знак как файл.
    pub fn new(home: [f32; 2], hz: f32, file_rest: bool) -> Self {
        MarkScene {
            knob: home.map(Tween::still),
            vol: home,
            muted: false,
            error: false,
            fade: Tween::still(1.0),
            strike_at: LONG_AGO,
            sweep_at: LONG_AGO,
            play_at: LONG_AGO,
            hz,
            file_rest,
        }
    }

    fn knobs_at(&self, now: f64) -> [f32; 2] {
        self.knob.map(|k| k.at(now, SLIDE, ease_out))
    }

    fn aim(&mut self, now: f64, still: bool) {
        if self.error {
            return;
        }
        let cur = self.knobs_at(now);
        let to = if self.muted { [0.0, 0.0] } else { self.vol };
        for i in 0..2 {
            self.knob[i] = Tween { a: if still { to[i] } else { cur[i] }, b: to[i], t0: now };
        }
    }

    /// Громкость каналов (доля хода ручки).
    pub fn volume(&mut self, now: f64, l: f32, r: f32, still: bool) {
        if self.vol == [l, r] {
            return;
        }
        self.vol = [l, r];
        self.aim(now, still);
    }

    pub fn mute(&mut self, now: f64, on: bool, still: bool) {
        if on == self.muted {
            return;
        }
        let f = self.fade.at(now, FADE, ease_out);
        let b = if on { MUTED_ALPHA } else { 1.0 };
        self.muted = on;
        self.fade = Tween { a: if still { b } else { f }, b, t0: now };
        self.aim(now, still);
    }

    /// Нет устройства вывода или служба звука пропала: ручки стоят, звона и фейдеров нет.
    pub fn fail(&mut self, now: f64, on: bool, still: bool) {
        if on == self.error {
            return;
        }
        if on {
            self.knob = self.knobs_at(now).map(|v| Tween { a: v, b: v, t0: now });
            self.error = true;
        } else {
            self.error = false;
            self.aim(now, still);
        }
    }

    /// Смена основного устройства вывода.
    pub fn strike(&mut self, now: f64) {
        if !self.error {
            self.strike_at = now;
        }
    }

    /// Запуск (трей), открытие «О программе».
    pub fn sweep(&mut self, now: f64) {
        if !self.error {
            self.sweep_at = now;
        }
    }

    /// Щелчок по знаку в «О программе»: ручки плавно ездят туда-сюда (`play_offset`).
    pub fn play(&mut self, now: f64) {
        if !self.error {
            self.play_at = now;
        }
    }
}

impl Scene for MarkScene {
    type Frame = MarkFrame;
    type Key = [i32; 4];

    fn frame(&self, now: f64, still: bool) -> MarkFrame {
        let mut v = self.knobs_at(now);
        let mut ang = 0.0;
        if !self.error && !still {
            for (i, x) in v.iter_mut().enumerate() {
                let u = (now - self.sweep_at) as f32 - i as f32 * SWEEP_STAGGER;
                if !(0.0..SWEEP).contains(&u) {
                    continue;
                }
                *x = if u < SWEEP_DOWN {
                    *x * (1.0 - ease_in(u / SWEEP_DOWN))
                } else if u < SWEEP_DOWN + SWEEP_UP {
                    ease_standard((u - SWEEP_DOWN) / SWEEP_UP)
                } else {
                    1.0 + (*x - 1.0) * ease_out((u - SWEEP_DOWN - SWEEP_UP) / SWEEP_BACK)
                };
            }
            let o = play_offset((now - self.play_at) as f32);
            v[0] = (v[0] + o).clamp(0.0, 1.0);
            v[1] = (v[1] - o).clamp(0.0, 1.0);
            let s = (now - self.strike_at) as f32;
            if (0.0..STRIKE).contains(&s) {
                ang = STRIKE_DEG * (-s / STRIKE_TAU).exp() * (std::f32::consts::TAU * self.hz * s).sin();
            }
        }
        MarkFrame { l: v[0], r: v[1], ang, alpha: self.fade.at(now, FADE, ease_out), error: self.error }
    }

    fn wants_frames(&self, now: f64, still: bool) -> bool {
        if still || self.error {
            return false;
        }
        self.knob.iter().any(|k| k.moving(now, SLIDE))
            || self.fade.moving(now, FADE)
            || now - self.strike_at < STRIKE as f64
            || now - self.sweep_at < (SWEEP + SWEEP_STAGGER) as f64
            || now - self.play_at < PLAY_LEN as f64
    }

    fn at_rest(&self, now: f64, still: bool) -> bool {
        self.file_rest && !self.error && !self.muted && !self.wants_frames(now, still)
    }

    fn key(&self) -> [i32; 4] {
        let k = self.knob.map(|k| (k.b * 1000.0).round() as i32);
        [k[0], k[1], self.muted as i32, self.error as i32]
    }
}

// ─── Колонка ─────────────────────────────────────────────────────────────────

/// Число волн по громкости — как у прежнего глифа: 0 % или «без звука» — 0, ≤ 33 % — 1, ≤ 66 % — 2, иначе 3.
pub fn waves(volume: f32, muted: bool) -> usize {
    let p = (volume.clamp(0.0, 1.0) * 100.0).round() as u32;
    match () {
        _ if muted || p == 0 => 0,
        _ if p <= 33 => 1,
        _ if p <= 66 => 2,
        _ => 3,
    }
}

/// Кадр колонки: видимость волн 0..1, доля крестика, масштаб диффузора, уходящая волна (радиус, непрозрачность).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpeakerFrame {
    pub waves: [f32; 3],
    pub cross: f32,
    pub thump: f32,
    pub ripple: Option<(f32, f32)>,
    pub error: bool,
}

/// Колонка: волны по общей громкости, крестик «без звука», толчок при смене устройства, волны по одной при
/// запуске, красная при ошибке.
#[derive(Debug, Clone)]
pub struct SpeakerScene {
    vol: f32,
    muted: bool,
    error: bool,
    wave: [Tween; 3],
    cross: Tween,
    thump_at: f64,
    start_at: f64,
}

impl SpeakerScene {
    pub fn new(volume: f32) -> Self {
        let n = waves(volume, false);
        SpeakerScene {
            vol: volume,
            muted: false,
            error: false,
            wave: std::array::from_fn(|i| Tween::still((i < n) as u8 as f32)),
            cross: Tween::still(0.0),
            thump_at: LONG_AGO,
            start_at: LONG_AGO,
        }
    }

    fn aim(&mut self, now: f64, still: bool) {
        let n = if self.error { 0 } else { waves(self.vol, self.muted) };
        for (i, w) in self.wave.iter_mut().enumerate() {
            let b = (i < n) as u8 as f32;
            if w.b != b {
                *w = Tween { a: if still { b } else { w.at(now, WAVE_IN, ease_out) }, b, t0: now };
            }
        }
        let c = if self.muted && !self.error { 1.0 } else { 0.0 };
        if self.cross.b != c {
            self.cross = Tween { a: if still { c } else { self.cross.at(now, CROSS, ease_out) }, b: c, t0: now };
        }
    }

    /// Общая громкость устройства 0..1.
    pub fn volume(&mut self, now: f64, volume: f32, still: bool) {
        self.vol = volume;
        self.aim(now, still);
    }

    pub fn mute(&mut self, now: f64, on: bool, still: bool) {
        self.muted = on;
        self.aim(now, still);
    }

    pub fn fail(&mut self, now: f64, on: bool, still: bool) {
        self.error = on;
        self.aim(now, still);
    }

    /// Смена устройства: толчок и уходящая волна.
    pub fn strike(&mut self, now: f64) {
        if !self.error {
            self.thump_at = now;
        }
    }

    /// Запуск: толчок и волны по одной.
    pub fn sweep(&mut self, now: f64) {
        if !self.error {
            self.start_at = now;
        }
    }
}

impl Scene for SpeakerScene {
    type Frame = SpeakerFrame;
    type Key = [i32; 3];

    fn frame(&self, now: f64, still: bool) -> SpeakerFrame {
        let mut p = self.wave.map(|w| w.at(now, WAVE_IN, ease_out));
        let mut thump = 1.0;
        let mut ripple = None;
        if !self.error && !still {
            let s = (now - self.start_at) as f32;
            let n = waves(self.vol, self.muted);
            if (0.0..START_LEN).contains(&s) {
                for (i, x) in p.iter_mut().enumerate().take(n) {
                    *x = ease_out(((s - START_DELAY - i as f32 * START_STEP) / WAVE_IN).clamp(0.0, 1.0));
                }
            }
            let k = ((now - self.thump_at) as f32).min(if s >= 0.0 { s } else { f32::MAX });
            if (0.0..THUMP).contains(&k) {
                thump = 1.0 + THUMP_SCALE * (std::f32::consts::PI * k / THUMP).sin();
            }
            let r = (now - self.thump_at) as f32;
            if (0.0..RIPPLE).contains(&r) {
                ripple =
                    Some((RIPPLE_FROM + (RIPPLE_TO - RIPPLE_FROM) * ease_out(r / RIPPLE), 0.9 * (1.0 - r / RIPPLE)));
            }
        }
        SpeakerFrame { waves: p, cross: self.cross.at(now, CROSS, ease_out), thump, ripple, error: self.error }
    }

    fn wants_frames(&self, now: f64, still: bool) -> bool {
        if still || self.error {
            return false;
        }
        self.wave.iter().any(|w| w.moving(now, WAVE_IN))
            || self.cross.moving(now, CROSS)
            || now - self.thump_at < RIPPLE as f64
            || now - self.start_at < START_LEN as f64
    }

    fn at_rest(&self, _now: f64, _still: bool) -> bool {
        false
    }

    fn key(&self) -> [i32; 3] {
        let n = if self.error { 0 } else { waves(self.vol, self.muted) };
        [n as i32, self.muted as i32, self.error as i32]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Сценарии эталона: громкость в начале — 0,4 на оба канала; события — те же секунды.
    const START: [f32; 2] = [0.4, 0.4];

    fn close(a: f32, b: f32, eps: f32) -> bool {
        (a - b).abs() <= eps
    }

    fn mark_at(events: &[(f64, &str, f32, f32)], now: f64, still: bool) -> (MarkScene, MarkFrame) {
        let mut m = MarkScene::new(START, STRIKE_HZ, false);
        for &(at, op, a, b) in events {
            if at > now {
                break;
            }
            match op {
                "volume" => m.volume(at, a, b, still),
                "mute" => m.mute(at, a > 0.0, still),
                "fail" => m.fail(at, a > 0.0, still),
                "strike" => m.strike(at),
                "sweep" => m.sweep(at),
                _ => unreachable!(),
            }
        }
        let f = m.frame(now, still);
        (m, f)
    }

    fn speaker_at(events: &[(f64, &str, f32)], now: f64) -> SpeakerFrame {
        let mut s = SpeakerScene::new(START[0]);
        for &(at, op, a) in events {
            if at > now {
                break;
            }
            match op {
                "volume" => s.volume(at, a, false),
                "mute" => s.mute(at, a > 0.0, false),
                "strike" => s.strike(at),
                "sweep" => s.sweep(at),
                _ => unreachable!(),
            }
        }
        s.frame(now, false)
    }

    #[test]
    fn mark_matches_reference() {
        // Числа — из эталона (`?scenario=…&at=…`, строка внизу страницы).
        let (_, f) = mark_at(&[(0.5, "volume", 0.8, 0.8)], 0.6, false);
        assert!(close(f.l, 0.7887, 0.001) && close(f.r, 0.7887, 0.001), "{f:?}");
        let (_, f) = mark_at(&[(0.5, "mute", 1.0, 0.0)], 0.6, false);
        assert!(close(f.l, 0.0113, 0.001) && close(f.alpha, 0.5580, 0.001), "{f:?}");
        let (_, f) = mark_at(&[(0.5, "strike", 0.0, 0.0), (0.5, "volume", 0.7, 0.7)], 0.56, false);
        assert!(close(f.l, 0.6632, 0.001) && close(f.ang, 1.726, 0.01), "{f:?}");
        let (_, f) = mark_at(&[(0.3, "sweep", 0.0, 0.0)], 0.6, false);
        assert!(close(f.l, 0.6136, 0.001) && close(f.r, 0.0, 0.001), "{f:?}");
        let ev = [(0.5, "volume", 0.8, 0.8), (1.5, "volume", 0.25, 0.25), (2.5, "volume", 0.6, 0.35)];
        let (_, f) = mark_at(&ev, 2.6, false);
        assert!(close(f.l, 0.5901, 0.001) && close(f.r, 0.3472, 0.001), "{f:?}");
        let (m, f) = mark_at(&[(0.5, "fail", 1.0, 0.0)], 1.2, false);
        assert!(f.error && close(f.l, 0.4, 0.001) && !m.wants_frames(1.2, false));
    }

    #[test]
    fn about_sweep_returns_to_the_file() {
        // «О программе»: дом — положения файла полного знака (y 360 и 500 на ходе 570…204).
        let home = [(570.0 - 360.0) / 366.0, (570.0 - 500.0) / 366.0];
        let mut m = MarkScene::new(home, STRIKE_HZ, true);
        assert!(m.at_rest(0.0, false));
        m.sweep(0.3);
        let f = m.frame(0.62, false);
        assert!(close(f.l, 0.7300, 0.001) && close(f.r, 0.0106, 0.001), "{f:?}");
        assert!(!m.at_rest(0.62, false));
        let f = m.frame(2.0, false);
        assert!(close(f.l, home[0], 1e-6) && close(f.r, home[1], 1e-6));
        assert!(m.at_rest(2.0, false));
    }

    #[test]
    fn mark_at_rest_asks_no_frames() {
        let mut m = MarkScene::new(START, STRIKE_HZ_TRAY, false);
        assert!(!m.wants_frames(10.0, false));
        m.volume(10.0, 0.5, 0.5, false);
        assert!(m.wants_frames(10.1, false));
        assert!(!m.wants_frames(10.0 + SLIDE as f64, false));
        // Та же громкость ещё раз — ход не начинается заново.
        m.volume(11.0, 0.5, 0.5, false);
        assert!(!m.wants_frames(11.0, false));
    }

    #[test]
    fn reduced_motion_jumps() {
        let (m, f) = mark_at(&[(0.5, "volume", 0.8, 0.8), (0.5, "strike", 0.0, 0.0)], 0.6, true);
        assert!(close(f.l, 0.8, 1e-6) && f.ang == 0.0 && !m.wants_frames(0.6, true));
        let (_, f) = mark_at(&[(0.5, "mute", 1.0, 0.0)], 0.5, true);
        assert!(close(f.alpha, MUTED_ALPHA, 1e-6) && close(f.l, 0.0, 1e-6));
    }

    #[test]
    fn error_freezes_and_silences() {
        let mut m = MarkScene::new(START, STRIKE_HZ, false);
        m.volume(0.0, 0.9, 0.9, false);
        m.fail(0.1, true, false);
        let frozen = m.frame(0.1, false).l;
        m.strike(0.2);
        m.sweep(0.2);
        m.volume(0.3, 0.1, 0.1, false);
        let f = m.frame(0.5, false);
        assert!(f.error && f.ang == 0.0 && close(f.l, frozen, 1e-6));
        m.fail(1.0, false, false);
        assert!(close(m.frame(1.0 + SLIDE as f64, false).l, 0.1, 1e-6));
    }

    #[test]
    fn speaker_matches_reference() {
        let f = speaker_at(&[(0.5, "volume", 0.8)], 0.6);
        assert!(close(f.waves[1], 1.0, 1e-6) && close(f.waves[2], 0.9718, 0.001), "{f:?}");
        let f = speaker_at(&[(0.5, "mute", 1.0)], 0.6);
        assert!(close(f.waves[0], 0.0282, 0.001) && close(f.cross, 0.9821, 0.001), "{f:?}");
        let f = speaker_at(&[(0.5, "strike", 0.0), (0.5, "volume", 0.7)], 0.75);
        let (r, a) = f.ripple.expect("ripple");
        assert!(close(f.thump, 1.07, 0.001) && close(r, 15.413, 0.01) && close(a, 0.525, 0.001), "{f:?}");
        let f = speaker_at(&[(0.3, "sweep", 0.0)], 0.4);
        assert!(f.waves == [0.0; 3] && close(f.thump, 1.1212, 0.001), "{f:?}");
    }

    #[test]
    fn mute_and_back() {
        let mut m = MarkScene::new(START, STRIKE_HZ_TRAY, false);
        m.mute(1.0, true, false);
        m.mute(2.0, false, false);
        let f = m.frame(2.0 + SLIDE as f64, false);
        assert!(close(f.l, START[0], 1e-6) && close(f.r, START[1], 1e-6) && close(f.alpha, 1.0, 1e-6), "{f:?}");
        let mut s = SpeakerScene::new(START[0]);
        s.mute(1.0, true, false);
        assert!(close(s.frame(1.0 + WAVE_IN as f64, false).cross, 1.0, 1e-6));
        s.mute(2.0, false, false);
        let f = s.frame(2.0 + WAVE_IN as f64, false);
        assert!(f.waves == [1.0, 1.0, 0.0] && f.cross == 0.0, "{f:?}");
    }

    #[test]
    fn speaker_waves_follow_the_old_glyph() {
        assert_eq!(
            [waves(0.0, false), waves(0.33, false), waves(0.34, false), waves(0.66, false), waves(0.67, false)],
            [0, 1, 2, 2, 3]
        );
        assert_eq!(waves(1.0, true), 0);
        let mut s = SpeakerScene::new(0.5);
        s.fail(1.0, true, false);
        let f = s.frame(2.0, false);
        assert!(f.error && f.waves == [0.0; 3] && f.cross == 0.0);
        assert!(!s.wants_frames(2.0, false));
    }

    #[test]
    fn play_moves_the_knobs_against_each_other_and_returns() {
        let mut s = MarkScene::new([0.5, 0.5], 7.0, true);
        s.play(10.0);
        let rest = s.frame(9.0, false);
        assert_eq!((rest.l, rest.r), (0.5, 0.5));
        let mut moved = false;
        for i in 1..44 {
            let f = s.frame(10.0 + i as f64 * 0.05, false);
            if (f.l - 0.5).abs() > 0.1 {
                moved = true;
            }
            // Навстречу друг другу: что одна ручка прошла вверх, то другая вниз.
            assert!(((f.l - 0.5) + (f.r - 0.5)).abs() < 1e-5);
        }
        assert!(moved);
        // Начало и конец без рывка: смещение и его скорость на краях нулевые.
        assert_eq!(play_offset(0.0), 0.0);
        assert!(play_offset(0.01).abs() < 1e-3 && play_offset(PLAY_LEN - 0.01).abs() < 1e-3);
        let end = s.frame(10.0 + PLAY_LEN as f64 + 0.01, false);
        assert_eq!((end.l, end.r), (0.5, 0.5));
        assert!(s.wants_frames(10.5, false) && !s.wants_frames(14.0, false));
        // Без анимации Windows и при ошибке знак стоит.
        assert_eq!(s.frame(10.5, true).l, 0.5);
        s.fail(11.0, true, false);
        s.play(12.0);
        assert!(!s.wants_frames(12.5, false));
    }
}
