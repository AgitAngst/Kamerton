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
