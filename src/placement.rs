// Где поставить выпадающее окно: у значка в трее, прижатым к панели задач с её стороны, целиком в
// рабочей области монитора. Всё в физических пикселях; чистые функции, их проверяют тесты внизу.

/// Отступ от панели задач и от края экрана, логические пиксели (`docs/ui-guideline.md`).
pub const GAP: f32 = 12.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }
}

/// С какой стороны монитора панель задач.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Bottom,
    Top,
    Left,
    Right,
}

/// Сторона панели задач — где рабочая область уже всего монитора. Панель скрывается сама или её
/// нет на этом мониторе — считаем, что снизу.
pub fn taskbar_edge(monitor: Rect, work: Rect) -> Edge {
    let cut = [
        (monitor.bottom - work.bottom, Edge::Bottom),
        (work.top - monitor.top, Edge::Top),
        (work.left - monitor.left, Edge::Left),
        (monitor.right - work.right, Edge::Right),
    ];
    cut.iter().max_by_key(|(d, _)| *d).filter(|(d, _)| *d > 0).map(|(_, e)| *e).unwrap_or(Edge::Bottom)
}

/// Место окна: левый верхний угол и высота (может быть меньше желаемой — тогда список прокручивается).
/// `anchor` — прямоугольник значка (или точка курсора), `size` — желаемые ширина и высота, `gap` — отступ;
/// всё в физических пикселях.
pub fn place(anchor: Rect, work: Rect, edge: Edge, size: (i32, i32), gap: i32) -> (i32, i32, i32) {
    let (w, want_h) = size;
    let h = want_h.min(work.height() - 2 * gap).max(1);
    let cx = (anchor.left + anchor.right) / 2;
    let cy = (anchor.top + anchor.bottom) / 2;
    let clamp_x = |x: i32| x.clamp(work.left + gap, (work.right - gap - w).max(work.left + gap));
    let clamp_y = |y: i32| y.clamp(work.top + gap, (work.bottom - gap - h).max(work.top + gap));
    let (x, y) = match edge {
        Edge::Bottom => (clamp_x(cx - w / 2), work.bottom - gap - h),
        Edge::Top => (clamp_x(cx - w / 2), work.top + gap),
        Edge::Left => (work.left + gap, clamp_y(cy - h / 2)),
        Edge::Right => (work.right - gap - w, clamp_y(cy - h / 2)),
    };
    (x, y, h)
}

/// Место плашки громкости: по центру рабочей области вдоль панели задач, у её стороны (над панелью
/// внизу, под ней сверху); при боковой панели — внизу по центру. `gap` — отступ от края рабочей области.
pub fn place_osd(work: Rect, edge: Edge, size: (i32, i32), gap: i32) -> (i32, i32) {
    let (w, h) = size;
    let x = work.left + (work.right - work.left - w) / 2;
    let y = match edge {
        Edge::Top => work.top + gap,
        _ => work.bottom - gap - h,
    };
    (x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MON: Rect = Rect { left: 0, top: 0, right: 1920, bottom: 1080 };

    #[test]
    fn edge_follows_work_area() {
        assert_eq!(taskbar_edge(MON, Rect { bottom: 1032, ..MON }), Edge::Bottom);
        assert_eq!(taskbar_edge(MON, Rect { top: 48, ..MON }), Edge::Top);
        assert_eq!(taskbar_edge(MON, Rect { left: 62, ..MON }), Edge::Left);
        assert_eq!(taskbar_edge(MON, Rect { right: 1858, ..MON }), Edge::Right);
        assert_eq!(taskbar_edge(MON, MON), Edge::Bottom);
    }

    #[test]
    fn bottom_taskbar_icon_in_corner() {
        let work = Rect { bottom: 1032, ..MON };
        let icon = Rect { left: 1800, top: 1040, right: 1824, bottom: 1072 };
        assert_eq!(place(icon, work, Edge::Bottom, (360, 400), 12), (1920 - 12 - 360, 1032 - 12 - 400, 400));
    }

    #[test]
    fn top_taskbar_right_edge_gap() {
        let work = Rect { top: 48, ..MON };
        let icon = Rect { left: 1880, top: 8, right: 1904, bottom: 40 };
        let (x, y, _) = place(icon, work, Edge::Top, (360, 300), 12);
        assert_eq!((x + 360, y), (1920 - 12, 48 + 12));
    }

    #[test]
    fn side_taskbars_center_on_icon() {
        let left = Rect { left: 62, ..MON };
        let icon = Rect { left: 10, top: 500, right: 50, bottom: 540 };
        assert_eq!(place(icon, left, Edge::Left, (360, 200), 12), (74, 420, 200));
        let right = Rect { right: 1858, ..MON };
        let icon = Rect { left: 1870, top: 1040, right: 1910, bottom: 1070 };
        assert_eq!(place(icon, right, Edge::Right, (360, 200), 12), (1858 - 12 - 360, 1080 - 12 - 200, 200));
    }

    #[test]
    fn osd_is_centered_at_the_taskbar_side() {
        let work = Rect { bottom: 1032, ..MON };
        assert_eq!(place_osd(work, Edge::Bottom, (260, 56), 24), (830, 1032 - 24 - 56));
        let top = Rect { top: 48, ..MON };
        assert_eq!(place_osd(top, Edge::Top, (260, 56), 24), (830, 48 + 24));
        // Боковая панель: плашка внизу по центру рабочей области.
        let left = Rect { left: 62, ..MON };
        assert_eq!(place_osd(left, Edge::Left, (260, 56), 24), (62 + (1858 - 260) / 2, 1080 - 24 - 56));
    }

    #[test]
    fn scaled_monitor_and_too_tall_window() {
        // 150 %: отступ 18 физических пикселей, окно 540 шириной, хочет 3000 высотой.
        let mon = Rect { left: 1920, top: 0, right: 1920 + 2560, bottom: 1440 };
        let work = Rect { bottom: 1368, ..mon };
        let icon = Rect { left: 4300, top: 1380, right: 4336, bottom: 1428 };
        let (x, y, h) = place(icon, work, Edge::Bottom, (540, 3000), 18);
        assert_eq!(h, 1368 - 36);
        assert_eq!(y, 18);
        // Значок у самого края: окно упирается в отступ справа, а не вылезает за экран.
        assert_eq!(x, 4480 - 18 - 540);
    }
}
