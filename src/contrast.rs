// Высокий контраст Windows: когда он включён, окно, страницы и плашка рисуются цветами системы
// (фон окна, текст, серый текст, выделение, ссылка), а не палитрой набора. Своих hex в интерфейсе нет:
// цвета берутся из системы; образцы тем — только для проверки вида (`KAMERTON_CONTRAST=black|white`).
//
// Палитра набора (`anvil_ui::Palette::new`) — таблица цветов без способа подменить её из программы, поэтому
// «свои» части окна берут палитру через `palette`, а виджеты набора получают системные цвета через стиль egui
// и акцент (`apply`). Полное покрытие виджетов набора — правка набора (подмена палитры в контексте), по команде.

use anvil_ui::theme::Swatch;
use anvil_ui::{Accent, Palette, ThemeChoice};
use eframe::egui::{self, Color32, Context, Stroke, Theme};

/// Цвета системы при включённом высоком контрасте (RGB).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sys {
    pub window: [u8; 3],
    pub text: [u8; 3],
    /// Серый текст (`COLOR_GRAYTEXT`): недоступное.
    pub gray: [u8; 3],
    pub highlight: [u8; 3],
    pub highlight_text: [u8; 3],
    /// Ссылка (`COLOR_HOTLIGHT`).
    pub hotlight: [u8; 3],
}

fn c(v: [u8; 3]) -> Color32 {
    Color32::from_rgb(v[0], v[1], v[2])
}

/// Образцы тем высокого контраста для проверки вида. Это не точные цвета тем Windows, а похожие.
fn sample(name: &str) -> Option<Sys> {
    match name {
        "black" => Some(Sys {
            window: [0, 0, 0],
            text: [255, 255, 255],
            gray: [63, 242, 63],
            highlight: [26, 235, 255],
            highlight_text: [0, 0, 0],
            hotlight: [255, 255, 0],
        }),
        "white" => Some(Sys {
            window: [255, 255, 255],
            text: [0, 0, 0],
            gray: [96, 0, 0],
            highlight: [55, 0, 110],
            highlight_text: [255, 255, 255],
            hotlight: [0, 0, 159],
        }),
        _ => None,
    }
}

/// Включён ли высокий контраст, и цвета системы; `None` — не включён.
pub fn read() -> Option<Sys> {
    if let Ok(v) = std::env::var("KAMERTON_CONTRAST") {
        return sample(&v);
    }
    #[cfg(windows)]
    {
        use windows::Win32::Graphics::Gdi::{
            COLOR_GRAYTEXT, COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT, COLOR_HOTLIGHT, COLOR_WINDOW, COLOR_WINDOWTEXT,
            GetSysColor, SYS_COLOR_INDEX,
        };
        use windows::Win32::UI::WindowsAndMessaging::{
            SPI_GETHIGHCONTRAST, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
        };
        /// `HIGHCONTRASTW` из Win32 (его модуль в `windows` — отдельная функция сборки, а нужны два поля).
        #[repr(C)]
        struct HighContrast {
            size: u32,
            flags: u32,
            scheme: *mut u16,
        }
        const ON: u32 = 0x1; // HCF_HIGHCONTRASTON
        let mut hc = HighContrast { size: size_of::<HighContrast>() as u32, flags: 0, scheme: std::ptr::null_mut() };
        let ok = unsafe {
            SystemParametersInfoW(
                SPI_GETHIGHCONTRAST,
                hc.size,
                Some(&mut hc as *mut HighContrast as *mut _),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            )
        }
        .is_ok();
        if !ok || hc.flags & ON == 0 {
            return None;
        }
        let rgb = |i: SYS_COLOR_INDEX| {
            let v = unsafe { GetSysColor(i) };
            [v as u8, (v >> 8) as u8, (v >> 16) as u8]
        };
        return Some(Sys {
            window: rgb(COLOR_WINDOW),
            text: rgb(COLOR_WINDOWTEXT),
            gray: rgb(COLOR_GRAYTEXT),
            highlight: rgb(COLOR_HIGHLIGHT),
            highlight_text: rgb(COLOR_HIGHLIGHTTEXT),
            hotlight: rgb(COLOR_HOTLIGHT),
        });
    }
    #[allow(unreachable_code)]
    None
}

/// Яркость цвета 0..1 (ITU-R BT.709 по линейным каналам).
fn luminance(v: [u8; 3]) -> f32 {
    let lin = |x: u8| {
        let s = x as f32 / 255.0;
        if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * lin(v[0]) + 0.7152 * lin(v[1]) + 0.0722 * lin(v[2])
}

/// Отношение контраста двух цветов (WCAG).
#[cfg(test)]
fn ratio(a: [u8; 3], b: [u8; 3]) -> f32 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

fn mix(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    [m(a[0], b[0]), m(a[1], b[1]), m(a[2], b[2])]
}

impl Sys {
    /// Тёмная ли тема системы: по фону окна.
    pub fn dark(&self) -> bool {
        luminance(self.window) < 0.18
    }

    /// Палитра окна: одна заливка, рамки и текст — цветом текста, выбранное — цветом выделения; цвета
    /// состояний (успех, предупреждение, ошибка) в высоком контрасте не различаются цветом — только значком.
    pub fn palette(&self) -> Palette {
        let window = c(self.window);
        let text = c(self.text);
        Palette {
            dark: self.dark(),
            bg: window,
            surface: window,
            card: window,
            raised: window,
            hover: c(mix(self.window, self.text, 0.25)),
            border: text,
            border_strong: text,
            field: window,
            text,
            weak: text,
            faint: c(self.gray),
            accent: c(self.highlight),
            on_accent: c(self.highlight_text),
            accent_text: c(self.hotlight),
            success: text,
            warning: text,
            danger: text,
        }
    }

    /// Акцент для виджетов набора: выделение и ссылка системы в обеих темах.
    fn accent(&self) -> Accent {
        let swatch = Swatch { fill: c(self.highlight), on_fill: c(self.highlight_text), text: c(self.hotlight) };
        Accent { name: "Contrast", dark: swatch, light: swatch }
    }
}

fn state_id() -> egui::Id {
    egui::Id::new("kamerton-contrast")
}

/// Включён ли высокий контраст в этом окне (по последнему `follow`).
pub fn active(ctx: &Context) -> bool {
    ctx.data(|d| d.get_temp::<Option<Sys>>(state_id())).flatten().is_some()
}

/// Палитра окна: у высокого контраста — системная, иначе — набора.
pub fn palette(ctx: &Context) -> Palette {
    match ctx.data(|d| d.get_temp::<Option<Sys>>(state_id())).flatten() {
        Some(sys) => sys.palette(),
        None => Palette::of_ctx(ctx),
    }
}

/// Звать каждый кадр: если высокий контраст включили или выключили, перекрасить стиль egui и акцент.
/// `prefer` — тема из настроек, к ней возвращаемся, когда контраст выключен.
pub fn follow(ctx: &Context, prefer: ThemeChoice) {
    let now = read();
    let known = ctx.data(|d| d.get_temp::<Option<Sys>>(state_id())).flatten();
    if now == known {
        return;
    }
    ctx.data_mut(|d| d.insert_temp(state_id(), now));
    match now {
        Some(sys) => {
            // Акцент пересобирает стиль обеих тем; поверх него — цвета системы.
            anvil_ui::set_accent(ctx, sys.accent());
            let (window, text) = (c(sys.window), c(sys.text));
            let line = Stroke::new(1.0, text);
            for theme in [Theme::Dark, Theme::Light] {
                ctx.style_mut_of(theme, |s| {
                    let v = &mut s.visuals;
                    v.override_text_color = Some(text);
                    v.panel_fill = window;
                    v.window_fill = window;
                    v.extreme_bg_color = window;
                    v.faint_bg_color = window;
                    v.window_stroke = line;
                    for w in [
                        &mut v.widgets.noninteractive,
                        &mut v.widgets.inactive,
                        &mut v.widgets.hovered,
                        &mut v.widgets.active,
                        &mut v.widgets.open,
                    ] {
                        w.bg_fill = window;
                        w.weak_bg_fill = window;
                        w.bg_stroke = line;
                        w.fg_stroke = line;
                    }
                    v.widgets.hovered.bg_fill = c(mix(sys.window, sys.text, 0.25));
                    v.widgets.hovered.weak_bg_fill = c(mix(sys.window, sys.text, 0.25));
                    v.selection.bg_fill = c(sys.highlight);
                    v.selection.stroke = Stroke::new(1.0, c(sys.highlight_text));
                });
            }
            anvil_ui::choose(ctx, if sys.dark() { ThemeChoice::Dark } else { ThemeChoice::Light });
        }
        None => {
            anvil_ui::set_accent(ctx, Accent::BLUE);
            for theme in [Theme::Dark, Theme::Light] {
                ctx.style_mut_of(theme, |s| s.visuals.override_text_color = None);
            }
            anvil_ui::choose(ctx, prefer);
        }
    }
    ctx.request_repaint();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_are_dark_and_light() {
        assert!(sample("black").unwrap().dark());
        assert!(!sample("white").unwrap().dark());
        assert!(sample("other").is_none());
    }

    #[test]
    fn text_and_links_stay_readable() {
        for name in ["black", "white"] {
            let s = sample(name).unwrap();
            assert!(ratio(s.text, s.window) >= 7.0, "{name}: text");
            assert!(ratio(s.hotlight, s.window) >= 4.5, "{name}: link");
            assert!(ratio(s.highlight_text, s.highlight) >= 4.5, "{name}: selected");
            assert!(ratio(s.text, mix(s.window, s.text, 0.25)) >= 4.5, "{name}: hover");
        }
    }

    #[test]
    fn palette_uses_only_system_colors() {
        let s = sample("black").unwrap();
        let p = s.palette();
        assert_eq!(p.bg, Color32::BLACK);
        assert_eq!(p.card, p.bg);
        assert_eq!(p.border_strong, p.text);
        assert_eq!(p.danger, p.text);
        assert_eq!(p.accent, c(s.highlight));
        assert!(p.dark);
    }
}
