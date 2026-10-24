// Значки окна: значок программы из её exe и «динамик», нарисованный линиями в сетке 24 × 24 набора.

use eframe::egui::{Color32, Painter, Pos2, Rect, Stroke, pos2};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject,
    GetDIBits, GetObjectW, HGDIOBJ,
};
use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
use windows::Win32::UI::Shell::{SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SHGetFileInfoW};
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, ICONINFO};
use windows::core::PCWSTR;

use crate::sys;

/// Значок exe: ширина, высота и RGBA (не premultiplied). `None` — у файла нет значка.
pub fn exe_icon(path: &str) -> Option<(usize, usize, Vec<u8>)> {
    let wpath = sys::wide(path);
    unsafe {
        let mut info = SHFILEINFOW::default();
        let ok = SHGetFileInfoW(
            PCWSTR(wpath.as_ptr()),
            FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(&mut info),
            size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_LARGEICON,
        );
        if ok == 0 || info.hIcon.is_invalid() {
            return None;
        }
        let out = icon_pixels(info);
        let _ = DestroyIcon(info.hIcon);
        out
    }
}

unsafe fn icon_pixels(info: SHFILEINFOW) -> Option<(usize, usize, Vec<u8>)> {
    unsafe {
        let mut ii = ICONINFO::default();
        GetIconInfo(info.hIcon, &mut ii).ok()?;
        let free = |ii: &ICONINFO| {
            let _ = DeleteObject(HGDIOBJ(ii.hbmColor.0));
            let _ = DeleteObject(HGDIOBJ(ii.hbmMask.0));
        };
        if ii.hbmColor.is_invalid() {
            free(&ii);
            return None;
        }
        let mut bm = BITMAP::default();
        GetObjectW(HGDIOBJ(ii.hbmColor.0), size_of::<BITMAP>() as i32, Some(&mut bm as *mut BITMAP as *mut _));
        let (w, h) = (bm.bmWidth, bm.bmHeight);
        if w <= 0 || h <= 0 {
            free(&ii);
            return None;
        }
        let mut bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bgra = vec![0u8; (w * h * 4) as usize];
        let dc = CreateCompatibleDC(None);
        let lines =
            GetDIBits(dc, ii.hbmColor, 0, h as u32, Some(bgra.as_mut_ptr() as *mut _), &mut bmi, DIB_RGB_COLORS);
        // Старые значки без альфы: прозрачность — из маски.
        let has_alpha = bgra.as_chunks::<4>().0.iter().any(|p| p[3] != 0);
        let mut mask = vec![0u8; (w * h * 4) as usize];
        if !has_alpha {
            GetDIBits(dc, ii.hbmMask, 0, h as u32, Some(mask.as_mut_ptr() as *mut _), &mut bmi, DIB_RGB_COLORS);
        }
        let _ = DeleteDC(dc);
        free(&ii);
        if lines == 0 {
            return None;
        }
        let mut rgba = Vec::with_capacity(bgra.len());
        for (i, p) in bgra.as_chunks::<4>().0.iter().enumerate() {
            let a = if has_alpha {
                p[3]
            } else if mask[i * 4] == 0 {
                255
            } else {
                0
            };
            rgba.extend_from_slice(&[p[2], p[1], p[0], a]);
        }
        Some((w as usize, h as usize, rgba))
    }
}

/// «Динамик» в сетке 24 × 24: корпус, раструб и волны по уровню (0..1); `muted` — крест вместо волн.
pub fn speaker(painter: &Painter, rect: Rect, color: Color32, level: f32, muted: bool) {
    let s = rect.width() / 24.0;
    let p = |x: f32, y: f32| -> Pos2 { pos2(rect.left() + x * s, rect.top() + y * s) };
    let stroke = Stroke::new((1.7 * s).max(1.2), color);
    let body = vec![p(3.0, 9.0), p(7.0, 9.0), p(12.0, 5.0), p(12.0, 19.0), p(7.0, 15.0), p(3.0, 15.0)];
    painter.add(eframe::egui::Shape::closed_line(body, stroke));
    if muted {
        painter.line_segment([p(15.5, 9.5), p(20.5, 14.5)], stroke);
        painter.line_segment([p(20.5, 9.5), p(15.5, 14.5)], stroke);
        return;
    }
    let waves = if level <= 0.0 {
        0
    } else if level <= 0.33 {
        1
    } else if level <= 0.66 {
        2
    } else {
        3
    };
    for i in 0..waves {
        let r = 3.5 + i as f32 * 3.0;
        let center = p(12.0, 12.0);
        let points: Vec<Pos2> = (0..=12)
            .map(|k| {
                let a = -0.9 + 1.8 * k as f32 / 12.0;
                pos2(center.x + (r * s) * a.cos(), center.y + (r * s) * a.sin())
            })
            .collect();
        painter.add(eframe::egui::Shape::line(points, stroke));
    }
}

/// «Наружу» в сетке 24 × 24: рамка со стрелкой из угла — ссылка открывает другое окно (Параметры Windows).
pub fn external(painter: &Painter, rect: Rect, color: Color32) {
    let s = rect.width() / 24.0;
    let p = |x: f32, y: f32| -> Pos2 { pos2(rect.left() + x * s, rect.top() + y * s) };
    let stroke = Stroke::new((2.0 * s).max(1.2), color);
    painter.add(eframe::egui::Shape::line(vec![p(14.0, 4.0), p(20.0, 4.0), p(20.0, 10.0)], stroke));
    painter.line_segment([p(20.0, 4.0), p(11.0, 13.0)], stroke);
    let frame = vec![p(18.0, 14.0), p(18.0, 20.0), p(4.0, 20.0), p(4.0, 6.0), p(10.0, 6.0)];
    painter.add(eframe::egui::Shape::line(frame, stroke));
}

/// Клавиатура в сетке 24 × 24: корпус, ряд клавиш и пробел — вход на страницу сочетаний.
pub fn keyboard(painter: &Painter, rect: Rect, color: Color32) {
    let s = rect.width() / 24.0;
    let p = |x: f32, y: f32| -> Pos2 { pos2(rect.left() + x * s, rect.top() + y * s) };
    let stroke = Stroke::new((1.6 * s).max(1.2), color);
    painter.rect_stroke(
        Rect::from_min_max(p(2.0, 6.0), p(22.0, 18.0)),
        eframe::egui::CornerRadius::same((2.0 * s) as u8),
        stroke,
        eframe::egui::StrokeKind::Middle,
    );
    for x in [6.0, 10.0, 14.0, 18.0] {
        painter.circle_filled(p(x, 10.0), 0.9 * s, color);
    }
    painter.line_segment([p(7.0, 14.0), p(17.0, 14.0)], stroke);
}

/// Вид устройства для значка в списке — по имени, которое дала Windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    Speakers,
    Headphones,
    Display,
    Mic,
}

pub fn device_kind(name: &str, capture: bool) -> DeviceKind {
    let n = name.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| n.contains(w));
    if capture {
        DeviceKind::Mic
    } else if has(&["headphone", "headset", "earphone", "наушник", "гарнитур"]) {
        DeviceKind::Headphones
    } else if has(&["hdmi", "displayport", "monitor", "display", "монитор", "дисплей"]) {
        DeviceKind::Display
    } else {
        DeviceKind::Speakers
    }
}

/// Значок устройства линиями в сетке 24 × 24: колонка, наушники, монитор, микрофон.
pub fn device(painter: &Painter, rect: Rect, color: Color32, kind: DeviceKind) {
    let s = rect.width() / 24.0;
    let p = |x: f32, y: f32| -> Pos2 { pos2(rect.left() + x * s, rect.top() + y * s) };
    let stroke = Stroke::new((1.6 * s).max(1.2), color);
    let r = |x0: f32, y0: f32, x1: f32, y1: f32, round: f32| {
        painter.rect_stroke(
            Rect::from_min_max(p(x0, y0), p(x1, y1)),
            eframe::egui::CornerRadius::same((round * s) as u8),
            stroke,
            eframe::egui::StrokeKind::Middle,
        );
    };
    let arc = |cx: f32, cy: f32, rad: f32, from: f32, to: f32| {
        let pts: Vec<Pos2> = (0..=16)
            .map(|k| {
                let a = from + (to - from) * k as f32 / 16.0;
                let c = p(cx, cy);
                pos2(c.x + rad * s * a.cos(), c.y + rad * s * a.sin())
            })
            .collect();
        painter.add(eframe::egui::Shape::line(pts, stroke));
    };
    match kind {
        DeviceKind::Speakers => {
            // Корпус по центру сетки (5…19 × 2…22), верхний динамик и нижний на одной оси x = 12.
            r(5.0, 2.0, 19.0, 22.0, 2.5);
            painter.circle_stroke(p(12.0, 7.0), 1.4 * s, stroke);
            painter.circle_stroke(p(12.0, 15.0), 4.0 * s, stroke);
        }
        DeviceKind::Headphones => {
            arc(12.0, 13.0, 8.0, std::f32::consts::PI, 2.0 * std::f32::consts::PI);
            r(3.0, 14.0, 7.0, 20.0, 1.5);
            r(17.0, 14.0, 21.0, 20.0, 1.5);
        }
        DeviceKind::Display => {
            r(3.0, 4.0, 21.0, 16.0, 2.0);
            painter.line_segment([p(9.0, 20.0), p(15.0, 20.0)], stroke);
            painter.line_segment([p(12.0, 16.0), p(12.0, 20.0)], stroke);
        }
        DeviceKind::Mic => {
            r(9.0, 3.0, 15.0, 14.0, 3.0);
            arc(12.0, 11.0, 6.0, 0.0, std::f32::consts::PI);
            painter.line_segment([p(12.0, 17.0), p(12.0, 21.0)], stroke);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DeviceKind, device_kind};

    #[test]
    fn kind_from_name() {
        assert_eq!(device_kind("Наушники (USB Headset)", false), DeviceKind::Headphones);
        assert_eq!(device_kind("Monitor (HDMI Audio)", false), DeviceKind::Display);
        assert_eq!(device_kind("Speakers (Demo Audio)", false), DeviceKind::Speakers);
        assert_eq!(device_kind("Headset Microphone", true), DeviceKind::Mic);
    }
}
