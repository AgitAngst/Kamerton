// Растеризатор кадров значков: знак-камертон (плитка, зубцы, ручки) и колонка — в RGBA нужного размера, без
// сторонних крейтов. Геометрия — та же, что у `assets/icon/kamerton*.svg` и эталона `reference/mark-motion.html`
// изменения OpenSpec `stage-0-9-mark-motion`. Покрытие — подвыборки: у знака до 48 px — 16 × 16, крупнее — 8 × 8 и
// 16 × 16 на краях (`mark`), у колонки — 4 × 4 (`speaker`).
// Цвета значка от темы окна не зависят: это картинка, а не элементы набора.

use crate::mark::{MarkFrame, SpeakerFrame};

/// Цвет значка в трее (`tray_color`). На exe, окне микшера и в «О программе» — всегда пурпур.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IconColor {
    #[default]
    Purple,
    Azure,
}

impl IconColor {
    /// Плитка знака: верх и низ вертикального градиента.
    fn tile(self) -> ([f32; 3], [f32; 3]) {
        match self {
            IconColor::Purple => (rgb(0xC24FAE), rgb(0x931A82)),
            IconColor::Azure => (rgb(0x0792B3), rgb(0x03647B)),
        }
    }

    /// Колонка: акцентный текст под тёмную или светлую панель задач (контраст с панелью ≥ 4,5:1).
    fn speaker(self, dark_taskbar: bool) -> [f32; 3] {
        match (self, dark_taskbar) {
            (IconColor::Purple, true) => rgb(0xDE93CD),
            (IconColor::Purple, false) => rgb(0x962A85),
            (IconColor::Azure, true) => rgb(0x49C0E5),
            (IconColor::Azure, false) => rgb(0x036A83),
        }
    }
}

/// Ручки знака при ошибке.
const DANGER: u32 = 0xF2676B;
/// Колонка при ошибке: под тёмную и светлую панель.
const SPEAKER_ERROR: [u32; 2] = [0xF2676B, 0xC4303A];

const fn rgb(hex: u32) -> [f32; 3] {
    [((hex >> 16) & 0xFF) as f32, ((hex >> 8) & 0xFF) as f32, (hex & 0xFF) as f32]
}

/// Подвыборок на пиксель по каждой оси.
const SS: usize = 4;

// ─── Знак ────────────────────────────────────────────────────────────────────

/// Геометрия знака в единицах плитки 1024.
struct Geom {
    w: f32,
    track: f32,
    gap: f32,
    top: f32,
    c: f32,
    stem: f32,
    end: f32,
    r: f32,
    ring: f32,
    pale: f32,
    dx: f32,
    dy: f32,
    /// Ручки как в файле знака (y).
    rest: [f32; 2],
}

/// Полный знак — от 49 px.
const FULL: Geom = Geom {
    w: 104.0,
    track: 56.0,
    gap: 136.0,
    top: 170.0,
    c: 610.0,
    stem: 80.0,
    end: 872.0,
    r: 96.0,
    ring: 32.0,
    pale: 0.65,
    dx: 2.0,
    dy: -21.0,
    rest: [360.0, 500.0],
};

/// Малый знак — до 48 px (трей).
const SMALL: Geom = Geom {
    w: 112.0,
    track: 80.0,
    gap: 136.0,
    top: 180.0,
    c: 606.0,
    stem: 88.0,
    end: 862.0,
    r: 104.0,
    ring: 0.0,
    pale: 0.70,
    dx: 1.0,
    dy: -14.0,
    rest: [370.0, 505.0],
};

/// До этого размера включительно — малый знак.
pub const SMALL_UP_TO: u32 = 48;

fn geom(size: u32) -> &'static Geom {
    if size <= SMALL_UP_TO { &SMALL } else { &FULL }
}

impl Geom {
    /// Низ и верх хода ручки.
    fn lo(&self) -> f32 {
        self.c - 40.0
    }

    fn hi(&self) -> f32 {
        self.top + 34.0
    }

    fn knob_y(&self, v: f32) -> f32 {
        self.lo() + (self.hi() - self.lo()) * v
    }
}

/// Ручки как в файле знака (доля хода) для знака этого размера.
pub fn home(size: u32) -> [f32; 2] {
    let g = geom(size);
    g.rest.map(|y| (g.lo() - y) / (g.lo() - g.hi()))
}

/// Кадр знака в положениях файла (значок окна, «О программе» в покое).
pub fn rest(size: u32) -> MarkFrame {
    let [l, r] = home(size);
    MarkFrame { l, r, ang: 0.0, alpha: 1.0, error: false }
}

/// Контур плитки — squircle из `kamerton.svg`, развёрнутый в многоугольник (по 10 точек на кривую и дугу; посчитан
/// один раз скриптом при выборе знака). Правка контура в SVG — пересчитать точки; сверяет тест
/// `rest_frame_matches_the_file`.
#[rustfmt::skip]
const TILE: [(f32, f32); 124] = [
    (601.62, 0.00), (643.81, 0.03), (681.91, 0.23), (716.25, 0.78), (747.17, 1.84), (775.01, 3.60),
    (800.11, 6.21), (822.80, 9.87), (843.43, 14.73), (862.34, 20.97), (879.86, 28.77), (894.39, 36.76),
    (908.40, 45.65), (921.81, 55.39), (934.59, 65.97), (946.68, 77.32), (958.03, 89.41), (968.61, 102.19),
    (978.35, 115.60), (987.24, 129.61), (995.23, 144.14), (1003.03, 161.66), (1009.27, 180.57), (1014.13, 201.20),
    (1017.79, 223.89), (1020.40, 248.99), (1022.16, 276.83), (1023.22, 307.75), (1023.77, 342.09), (1023.97, 380.19),
    (1024.00, 422.38), (1024.00, 601.62), (1023.97, 643.81), (1023.77, 681.91), (1023.22, 716.25), (1022.16, 747.17),
    (1020.40, 775.01), (1017.79, 800.11), (1014.13, 822.80), (1009.27, 843.43), (1003.03, 862.34), (995.23, 879.86),
    (987.24, 894.39), (978.35, 908.40), (968.61, 921.81), (958.03, 934.59), (946.68, 946.68), (934.59, 958.03),
    (921.81, 968.61), (908.40, 978.35), (894.39, 987.24), (879.86, 995.23), (862.34, 1003.03), (843.43, 1009.27),
    (822.80, 1014.13), (800.11, 1017.79), (775.01, 1020.40), (747.17, 1022.16), (716.25, 1023.22), (681.91, 1023.77),
    (643.81, 1023.97), (601.62, 1024.00), (422.38, 1024.00), (380.19, 1023.97), (342.09, 1023.77), (307.75, 1023.22),
    (276.83, 1022.16), (248.99, 1020.40), (223.89, 1017.79), (201.20, 1014.13), (180.57, 1009.27), (161.66, 1003.03),
    (144.14, 995.23), (129.61, 987.24), (115.60, 978.35), (102.19, 968.61), (89.41, 958.03), (77.32, 946.68),
    (65.97, 934.59), (55.39, 921.81), (45.65, 908.40), (36.76, 894.39), (28.77, 879.86), (20.97, 862.34),
    (14.73, 843.43), (9.87, 822.80), (6.21, 800.11), (3.60, 775.01), (1.84, 747.17), (0.78, 716.25),
    (0.23, 681.91), (0.03, 643.81), (0.00, 601.62), (0.00, 422.38), (0.03, 380.19), (0.23, 342.09),
    (0.78, 307.75), (1.84, 276.83), (3.60, 248.99), (6.21, 223.89), (9.87, 201.20), (14.73, 180.57),
    (20.97, 161.66), (28.77, 144.14), (36.76, 129.61), (45.65, 115.60), (55.39, 102.19), (65.97, 89.41),
    (77.32, 77.32), (89.41, 65.97), (102.19, 55.39), (115.60, 45.65), (129.61, 36.76), (144.14, 28.77),
    (161.66, 20.97), (180.57, 14.73), (201.20, 9.87), (223.89, 6.21), (248.99, 3.60), (276.83, 1.84),
    (307.75, 0.78), (342.09, 0.23), (380.19, 0.03), (422.38, 0.00),
];

/// Где строка `y` пересекает плитку: контур выпуклый — один отрезок `[x0, x1]`.
fn tile_span(y: f32) -> Option<(f32, f32)> {
    let (mut x0, mut x1) = (f32::MAX, f32::MIN);
    for i in 0..TILE.len() {
        let (ax, ay) = TILE[i];
        let (bx, by) = TILE[(i + 1) % TILE.len()];
        if (ay <= y) != (by <= y) {
            let x = ax + (y - ay) / (by - ay) * (bx - ax);
            x0 = x0.min(x);
            x1 = x1.max(x);
        }
    }
    (x0 <= x1).then_some((x0, x1))
}

/// Пиксель целиком внутри плитки: плитка выпуклая — достаточно четырёх углов.
fn inside_tile(px: usize, py: usize, k: f32) -> bool {
    let (x0, x1) = (px as f32 * k, (px + 1) as f32 * k);
    [py as f32 * k, (py + 1) as f32 * k].into_iter().all(|y| tile_span(y).is_some_and(|(a, b)| a <= x0 && x1 <= b))
}

/// Слой знака в предумноженных цветах: закрасить поверх цветом `c` с непрозрачностью `a`.
#[derive(Clone, Copy, Default)]
struct Layer {
    c: [f32; 3],
    a: f32,
}

impl Layer {
    fn paint(&mut self, c: [f32; 3], a: f32) {
        for (own, new) in self.c.iter_mut().zip(c) {
            *own = new * a + *own * (1.0 - a);
        }
        self.a = a + self.a * (1.0 - a);
    }
}

const WHITE: [f32; 3] = [255.0, 255.0, 255.0];

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// Зубец: дорожка, залитая часть, ручка. `x`, `y` — точка в системе знака до наклона.
#[allow(clippy::too_many_arguments)]
fn prong(layer: &mut Layer, g: &Geom, cx: f32, ky: f32, x: f32, y: f32, knob: [f32; 3], ring: [f32; 3]) {
    let tr = g.track / 2.0;
    let pale = ((x - cx).abs() <= tr && y >= g.top + tr && y <= ky)
        || (y < g.top + tr && (x - cx).hypot(y - g.top - tr) <= tr);
    if pale {
        layer.paint(WHITE, g.pale);
    }
    if (x - cx).abs() <= g.w / 2.0 && y >= ky && y <= g.c + 2.0 {
        layer.paint(WHITE, 1.0);
    }
    let d = (x - cx).hypot(y - ky);
    if g.ring > 0.0 {
        if d <= g.r - g.ring / 2.0 {
            layer.paint(knob, 1.0);
        } else if d <= g.r + g.ring / 2.0 {
            layer.paint(ring, 1.0);
        }
    } else if d <= g.r {
        layer.paint(knob, 1.0);
    }
}

/// Кадр знака в RGBA (прямая альфа), `size` × `size`. Малый знак (трей) — подвыборки 16 × 16 на каждый пиксель;
/// крупнее — 8 × 8, а где они расходятся или пиксель лежит на краю плитки — заново 16 × 16. Тонкую полоску у края
/// фигуры редкая сетка не видит вовсе, поэтому мелким размерам — сразу частая.
pub fn mark(f: &MarkFrame, size: u32, color: IconColor) -> Vec<u8> {
    let g = geom(size);
    let (top, bottom) = color.tile();
    let knob = if f.error { rgb(DANGER) } else { WHITE };
    let n = size as usize;
    let k = 1024.0 / size as f32;
    let tan = f.ang.to_radians().tan();
    let (yl, yr) = (g.knob_y(f.l), g.knob_y(f.r));
    let (ro, ri, h) = (g.gap / 2.0 + g.w, g.gap / 2.0, g.stem / 2.0);
    let ys = g.c + (ro * ro - h * h).sqrt();
    let off = g.gap / 2.0 + g.w / 2.0;
    // Цвет точки (единицы плитки) или `None` — вне плитки; `span` — отрезок её строки внутри плитки.
    let sample = |x: f32, y: f32, span: Option<(f32, f32)>| -> Option<[f32; 3]> {
        let (x0, x1) = span?;
        if x < x0 || x > x1 {
            return None;
        }
        let tile = mix(top, bottom, (y / 1024.0).clamp(0.0, 1.0));
        let (gx, gy) = (x - g.dx, y - g.dy);
        let mut layer = Layer::default();
        // Дуга и ножка.
        let d = (gx - 512.0).hypot(gy - g.c);
        let stem = (gx - 512.0).abs() <= h && gy >= ys && gy <= g.end - h;
        let cap = (gx - 512.0).hypot(gy - (g.end - h)) <= h;
        if (gy >= g.c && d >= ri && d <= ro) || stem || cap {
            layer.paint(WHITE, 1.0);
        }
        // Зубцы с ручками; наклон — обратным преобразованием точки вокруг основания зубца.
        for (cx, ky, sign) in [(512.0 - off, yl, 1.0), (512.0 + off, yr, -1.0)] {
            let lx = gx - sign * tan * (gy - g.c);
            prong(&mut layer, g, cx, ky, lx, gy, knob, tile);
        }
        let ga = layer.a * f.alpha;
        Some(std::array::from_fn(|i| layer.c[i] * f.alpha + tile[i] * (1.0 - ga)))
    };
    // Отрезки плитки для строк подвыборок 4 × 4 и 8 × 8.
    let spans = |ss: usize| -> Vec<Option<(f32, f32)>> {
        (0..n * ss).map(|row| tile_span((row as f32 + 0.5) / ss as f32 * k)).collect()
    };
    let (cs, fs) = if size <= SMALL_UP_TO { (16, 16) } else { (8, 16) };
    let (coarse, fine) = (spans(cs), spans(fs));
    // Сумма подвыборок пикселя (r, g, b, сколько внутри) и сошлись ли они.
    let pixel = |px: usize, py: usize, ss: usize, rows: &[Option<(f32, f32)>]| -> ([f32; 4], bool) {
        let mut acc = [0.0f32; 4];
        let (mut lo, mut hi) = ([f32::MAX; 4], [f32::MIN; 4]);
        for sy in 0..ss {
            let row = py * ss + sy;
            let y = (row as f32 + 0.5) / ss as f32 * k;
            for sx in 0..ss {
                let x = ((px * ss + sx) as f32 + 0.5) / ss as f32 * k;
                let s = sample(x, y, rows[row]).map_or([0.0; 4], |c| [c[0], c[1], c[2], 1.0]);
                for i in 0..4 {
                    acc[i] += s[i];
                    lo[i] = lo[i].min(s[i]);
                    hi[i] = hi[i].max(s[i]);
                }
            }
        }
        // Градиент плитки внутри пикселя — до нескольких единиц цвета: это не край.
        let same = (0..3).all(|i| hi[i] - lo[i] <= 8.0) && hi[3] == lo[3];
        (acc, same)
    };
    let mut out = vec![0u8; n * n * 4];
    for py in 0..n {
        for px in 0..n {
            let (mut acc, same) = pixel(px, py, cs, &coarse);
            let mut count = (cs * cs) as f32;
            if cs != fs && !(same && inside_tile(px, py, k)) {
                acc = pixel(px, py, fs, &fine).0;
                count = (fs * fs) as f32;
            }
            if acc[3] > 0.0 {
                let o = (py * n + px) * 4;
                for i in 0..3 {
                    out[o + i] = (acc[i] / acc[3]).round().clamp(0.0, 255.0) as u8;
                }
                out[o + 3] = (acc[3] / count * 255.0).round() as u8;
            }
        }
    }
    out
}

// ─── Колонка ─────────────────────────────────────────────────────────────────

/// Корпус и диффузор (сетка 32), обводка 1,2 со скруглёнными углами.
const BODY: [(f32, f32); 6] = [(3.5, 12.0), (9.0, 12.0), (15.5, 6.5), (15.5, 25.5), (9.0, 20.0), (3.5, 20.0)];
const BODY_HALF_STROKE: f32 = 0.6;
/// Вокруг этой точки диффузор растёт в толчке.
const PIVOT: (f32, f32) = (9.0, 16.0);
/// Центр волн, их радиусы, полуугол, полутолщина штриха.
const WAVE_CENTER: (f32, f32) = (15.5, 16.0);
const RADII: [f32; 3] = [5.0, 9.5, 14.0];
const SPAN_DEG: f32 = 50.0;
const HALF_STROKE: f32 = 1.15;
/// Крестик «без звука»: два штриха, каждый растёт от своего начала.
const CROSS: [((f32, f32), (f32, f32)); 2] = [((21.0, 12.0), (29.0, 20.0)), ((29.0, 12.0), (21.0, 20.0))];

fn segment_distance(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
    (p.0 - a.0 - t * dx).hypot(p.1 - a.1 - t * dy)
}

fn in_polygon(p: (f32, f32), poly: &[(f32, f32)]) -> bool {
    let mut inside = false;
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        if (a.1 > p.1) != (b.1 > p.1) && p.0 < a.0 + (p.1 - a.1) / (b.1 - a.1) * (b.0 - a.0) {
            inside = !inside;
        }
    }
    inside
}

/// Расстояние от точки до дуги волны радиуса `r` (±`SPAN_DEG`, концы — круглые).
fn arc_distance(p: (f32, f32), r: f32) -> f32 {
    let (dx, dy) = (p.0 - WAVE_CENTER.0, p.1 - WAVE_CENTER.1);
    let span = SPAN_DEG.to_radians();
    if dy.atan2(dx).abs() <= span {
        return (dx.hypot(dy) - r).abs();
    }
    let end = |s: f32| (WAVE_CENTER.0 + r * (s * span).cos(), WAVE_CENTER.1 + r * (s * span).sin());
    let (a, b) = (end(-1.0), end(1.0));
    (p.0 - a.0).hypot(p.1 - a.1).min((p.0 - b.0).hypot(p.1 - b.1))
}

/// Кадр колонки в RGBA (прямая альфа), `size` × `size`, без плитки.
pub fn speaker(f: &SpeakerFrame, size: u32, dark_taskbar: bool, color: IconColor) -> Vec<u8> {
    let c = if f.error { rgb(SPEAKER_ERROR[if dark_taskbar { 0 } else { 1 }]) } else { color.speaker(dark_taskbar) };
    let n = size as usize;
    let k = 32.0 / size as f32;
    let mut out = vec![0u8; n * n * 4];
    for py in 0..n {
        for px in 0..n {
            let mut sum = 0.0;
            for sy in 0..SS {
                for sx in 0..SS {
                    let p = (
                        (px as f32 + (sx as f32 + 0.5) / SS as f32) * k,
                        (py as f32 + (sy as f32 + 0.5) / SS as f32) * k,
                    );
                    let mut clear = 1.0f32;
                    let b = (PIVOT.0 + (p.0 - PIVOT.0) / f.thump, PIVOT.1 + (p.1 - PIVOT.1) / f.thump);
                    let body = in_polygon(b, &BODY)
                        || (0..BODY.len())
                            .any(|i| segment_distance(b, BODY[i], BODY[(i + 1) % BODY.len()]) <= BODY_HALF_STROKE);
                    if body {
                        clear = 0.0;
                    }
                    for (i, &r) in RADII.iter().enumerate() {
                        let w = f.waves[i];
                        if w > 0.001 && arc_distance(p, r * (0.82 + 0.18 * w)) <= HALF_STROKE {
                            clear *= 1.0 - w;
                        }
                    }
                    if let Some((r, a)) = f.ripple
                        && arc_distance(p, r) <= HALF_STROKE
                    {
                        clear *= 1.0 - a;
                    }
                    if f.cross > 0.001 {
                        for (a, e) in CROSS {
                            let e = (a.0 + (e.0 - a.0) * f.cross, a.1 + (e.1 - a.1) * f.cross);
                            if segment_distance(p, a, e) <= HALF_STROKE {
                                clear = 0.0;
                            }
                        }
                    }
                    sum += 1.0 - clear;
                }
            }
            let a = sum / (SS * SS) as f32;
            if a > 0.0 {
                let o = (py * n + px) * 4;
                for i in 0..3 {
                    out[o + i] = c[i] as u8;
                }
                out[o + 3] = (a * 255.0).round() as u8;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rest(size: u32) -> MarkFrame {
        let [l, r] = home(size);
        MarkFrame { l, r, ang: 0.0, alpha: 1.0, error: false }
    }

    fn px(img: &[u8], size: u32, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * size + x) * 4) as usize;
        [img[i], img[i + 1], img[i + 2], img[i + 3]]
    }

    /// Доля пикселей, где предумноженные каналы расходятся больше чем на `tol` из 255.
    fn mismatch(a: &[u8], b: &[u8], tol: f32) -> f32 {
        let bad = a
            .chunks(4)
            .zip(b.chunks(4))
            .filter(|(p, q)| {
                let pm = |c: &[u8], i: usize| c[i] as f32 * c[3] as f32 / 255.0;
                (0..3).any(|i| (pm(p, i) - pm(q, i)).abs() > tol) || (p[3] as f32 - q[3] as f32).abs() > tol
            })
            .count();
        bad as f32 / (a.len() / 4) as f32
    }

    #[test]
    fn rest_frame_matches_the_file() {
        // Образцы — PNG из `scripts/render-icon.ps1` (resvg) в сырых RGBA.
        for (size, file) in [
            (16, &include_bytes!("../assets/icon/kamerton-16.rgba")[..]),
            (32, &include_bytes!("../assets/icon/kamerton-32.rgba")[..]),
            (64, &include_bytes!("../assets/icon/kamerton-64.rgba")[..]),
        ] {
            let img = mark(&rest(size), size, IconColor::Purple);
            let bad = mismatch(&img, file, 8.0);
            assert!(bad <= 0.01, "{size} px: {:.1} % of pixels differ", bad * 100.0);
        }
    }

    #[test]
    fn corners_clear_center_solid() {
        for size in [16, 20, 24, 32, 56, 112] {
            let img = mark(&rest(size), size, IconColor::Purple);
            assert_eq!(px(&img, size, 0, 0)[3], 0, "{size}");
            assert_eq!(px(&img, size, size / 2, size / 2)[3], 255, "{size}");
        }
    }

    #[test]
    fn error_knobs_are_red_and_fade_touches_only_the_glyph() {
        let size = 64;
        let mut f = rest(size);
        let [l, _] = home(size);
        // Центр левой ручки полного знака: x = 512 − 120 + 2, y по ходу − 21.
        let (x, y) = (((392.0 + 2.0) / 16.0) as u32, ((FULL.knob_y(l) - 21.0) / 16.0) as u32);
        assert_eq!(px(&mark(&f, size, IconColor::Purple), size, x, y)[..3], [255, 255, 255]);
        f.error = true;
        let red = px(&mark(&f, size, IconColor::Purple), size, x, y);
        assert!(red[0] > 200 && red[1] < 140, "{red:?}");
        f.error = false;
        f.alpha = 0.55;
        let faded = mark(&f, size, IconColor::Purple);
        let full = mark(&rest(size), size, IconColor::Purple);
        assert_eq!(px(&faded, size, 4, 32), px(&full, size, 4, 32), "the tile does not fade");
        assert!(px(&faded, size, x, y)[1] < 255);
    }

    #[test]
    fn azure_tile() {
        let img = mark(&rest(32), 32, IconColor::Azure);
        let p = px(&img, 32, 3, 16);
        assert!(p[2] > p[0] && p[3] == 255, "{p:?}");
    }

    #[test]
    fn speaker_draws_waves_cross_and_error() {
        let size = 32;
        let all = SpeakerFrame { waves: [1.0; 3], cross: 0.0, thump: 1.0, ripple: None, error: false };
        let img = speaker(&all, size, true, IconColor::Purple);
        // Корпус, третья волна на оси, пусто у края.
        assert_eq!(px(&img, size, 6, 16)[3], 255);
        assert_eq!(px(&img, size, 29, 16), [0xDE, 0x93, 0xCD, 255]);
        assert_eq!(px(&img, size, 0, 0)[3], 0);
        let none = SpeakerFrame { waves: [0.0; 3], cross: 1.0, ..all };
        let img = speaker(&none, size, false, IconColor::Azure);
        assert_eq!(px(&img, size, 29, 16)[3], 0, "no third wave");
        assert_eq!(px(&img, size, 25, 16), [0x03, 0x6A, 0x83, 255], "cross centre");
        let err = SpeakerFrame { error: true, ..none };
        assert_eq!(px(&speaker(&err, size, true, IconColor::Purple), size, 6, 16)[..3], [0xF2, 0x67, 0x6B]);
    }

    /// Лист для проверки вида значков трея (`KAMERTON_SHEET=<файл.png>`): знак и колонка, пурпур и лазурь,
    /// 16 · 20 · 24 · 32 px на тёмной и светлой панели задач — громкость, «без звука», ошибка, кадры движения.
    #[test]
    #[ignore]
    fn tray_sheet() {
        let Ok(path) = std::env::var("KAMERTON_SHEET") else { return };
        let sizes = [16u32, 20, 24, 32];
        let marks = [
            MarkFrame { l: 0.4, r: 0.4, ang: 0.0, alpha: 1.0, error: false },
            MarkFrame { l: 0.85, r: 0.55, ang: 0.0, alpha: 1.0, error: false },
            MarkFrame { l: 0.0, r: 0.0, ang: 0.0, alpha: 0.55, error: false },
            MarkFrame { l: 0.4, r: 0.4, ang: 0.0, alpha: 1.0, error: true },
            MarkFrame { l: 0.95, r: 0.2, ang: 0.0, alpha: 1.0, error: false },
            MarkFrame { l: 0.5, r: 0.5, ang: 4.0, alpha: 1.0, error: false },
        ];
        let speakers = [
            SpeakerFrame { waves: [1.0, 1.0, 0.0], cross: 0.0, thump: 1.0, ripple: None, error: false },
            SpeakerFrame { waves: [1.0; 3], cross: 0.0, thump: 1.0, ripple: None, error: false },
            SpeakerFrame { waves: [0.0; 3], cross: 1.0, thump: 1.0, ripple: None, error: false },
            SpeakerFrame { waves: [0.0; 3], cross: 0.0, thump: 1.0, ripple: None, error: true },
            SpeakerFrame { waves: [1.0, 0.4, 0.0], cross: 0.0, thump: 1.1, ripple: None, error: false },
            SpeakerFrame { waves: [1.0, 1.0, 0.0], cross: 0.0, thump: 1.07, ripple: Some((15.4, 0.5)), error: false },
        ];
        let cell = 40usize;
        let (cols, rows) = (marks.len() * sizes.len(), 8usize);
        let (w, h) = (cols * cell, rows * cell);
        let mut img = vec![0u8; w * h * 4];
        for row in 0..rows {
            let dark = row % 2 == 0;
            let bg = if dark { [0x20, 0x20, 0x20] } else { [0xF3, 0xF3, 0xF3] };
            for y in row * cell..(row + 1) * cell {
                for x in 0..w {
                    img[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&[bg[0], bg[1], bg[2], 255]);
                }
            }
            let color = if row / 2 % 2 == 0 { IconColor::Purple } else { IconColor::Azure };
            for (state, (m, s)) in marks.iter().zip(&speakers).enumerate() {
                for (k, &size) in sizes.iter().enumerate() {
                    let icon = if row < 4 { mark(m, size, color) } else { speaker(s, size, dark, color) };
                    let (ox, oy) = ((state * sizes.len() + k) * cell + 4, row * cell + 4);
                    for y in 0..size as usize {
                        for x in 0..size as usize {
                            let p = &icon[(y * size as usize + x) * 4..][..4];
                            let o = ((oy + y) * w + ox + x) * 4;
                            let a = p[3] as f32 / 255.0;
                            for i in 0..3 {
                                img[o + i] = (p[i] as f32 * a + img[o + i] as f32 * (1.0 - a)).round() as u8;
                            }
                        }
                    }
                }
            }
        }
        std::fs::write(path, crate::snap::png(w, h, &img)).expect("sheet");
    }
}
