//! Оформление: палитра, шрифты, стили.

use egui::{Color32, Context, FontFamily, FontId, Rounding, Stroke, TextStyle};

#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: Color32,
    pub panel: Color32,
    pub card: Color32,
    pub card_hover: Color32,
    pub border: Color32,
    pub accent: Color32,
    pub accent_dim: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub green: Color32,
    pub red: Color32,
}

pub const DARK: Palette = Palette {
    bg: Color32::from_rgb(0x12, 0x0A, 0x0F),
    panel: Color32::from_rgb(0x1A, 0x0F, 0x16),
    card: Color32::from_rgb(0x23, 0x13, 0x1C),
    card_hover: Color32::from_rgb(0x2D, 0x18, 0x23),
    border: Color32::from_rgb(0x3A, 0x22, 0x2E),
    accent: Color32::from_rgb(0xFF, 0x4D, 0x8D),
    accent_dim: Color32::from_rgb(0xC2, 0x2F, 0x67),
    text: Color32::from_rgb(0xF4, 0xE9, 0xEE),
    muted: Color32::from_rgb(0x9E, 0x86, 0x92),
    green: Color32::from_rgb(0x5A, 0xD6, 0x8E),
    red: Color32::from_rgb(0xF8, 0x71, 0x71),
};

pub const LIGHT: Palette = Palette {
    bg: Color32::from_rgb(0xFA, 0xF2, 0xF5),
    panel: Color32::from_rgb(0xFF, 0xFB, 0xFC),
    card: Color32::from_rgb(0xF6, 0xE7, 0xED),
    card_hover: Color32::from_rgb(0xEF, 0xDA, 0xE3),
    border: Color32::from_rgb(0xE2, 0xC7, 0xD3),
    accent: Color32::from_rgb(0xD6, 0x1E, 0x62),
    accent_dim: Color32::from_rgb(0xAD, 0x18, 0x4F),
    text: Color32::from_rgb(0x2A, 0x12, 0x1C),
    muted: Color32::from_rgb(0x7A, 0x5E, 0x6A),
    green: Color32::from_rgb(0x1F, 0x9D, 0x5C),
    red: Color32::from_rgb(0xC0, 0x2A, 0x2A),
};

pub fn palette(light: bool) -> Palette {
    if light {
        LIGHT
    } else {
        DARK
    }
}

fn install_fonts(ctx: &Context) {
    let mut fonts = egui::FontDefinitions::default();
    // Comic Sans MS — основной шрифт интерфейса. Если его нет в системе,
    // остаются встроенные шрифты egui, поэтому лаунчер всё равно запустится.
    let ui_fonts = [
        "C:\\Windows\\Fonts\\comic.ttf",
        "C:\\Windows\\Fonts\\comicbd.ttf",
        "C:\\Windows\\Fonts\\comici.ttf",
    ];
    for path in ui_fonts {
        if let Ok(bytes) = std::fs::read(path) {
            fonts
                .font_data
                .insert("ui-text".to_owned(), egui::FontData::from_owned(bytes));
            for family in [FontFamily::Proportional, FontFamily::Monospace] {
                fonts
                    .families
                    .entry(family)
                    .or_default()
                    .insert(0, "ui-text".to_owned());
            }
            break;
        }
    }
    // В Comic Sans нет части геометрических знаков (◆, ▤, ☰), поэтому
    // подключаем символьный шрифт как запасной.
    let symbols = [
        "C:\\Windows\\Fonts\\seguisym.ttf",
        "C:\\Windows\\Fonts\\SegoeUISymbol.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    ];
    for path in symbols {
        if let Ok(bytes) = std::fs::read(path) {
            fonts
                .font_data
                .insert("ui-symbols".to_owned(), egui::FontData::from_owned(bytes));
            for family in [FontFamily::Proportional, FontFamily::Monospace] {
                fonts
                    .families
                    .entry(family)
                    .or_default()
                    .push("ui-symbols".to_owned());
            }
            break;
        }
    }
    ctx.set_fonts(fonts);
}

pub fn apply(ctx: &Context, light: bool) -> Palette {
    let p = palette(light);
    install_fonts(ctx);

    // Жёстко фиксируем выбранную тему: по умолчанию egui следует системной,
    // из-за чего светлые стили перетирали кастомные поля ввода и чекбоксы.
    ctx.set_theme(if light {
        egui::ThemePreference::Light
    } else {
        egui::ThemePreference::Dark
    });

    let mut visuals = if light {
        egui::Visuals::light()
    } else {
        egui::Visuals::dark()
    };
    visuals.override_text_color = Some(p.text);
    visuals.panel_fill = p.bg;
    visuals.window_fill = p.panel;
    visuals.extreme_bg_color = p.card;
    visuals.faint_bg_color = p.card;
    visuals.hyperlink_color = p.accent;
    visuals.window_stroke = Stroke::new(1.0_f32, p.border);
    visuals.selection.bg_fill = p.accent.gamma_multiply(0.4);
    visuals.selection.stroke = Stroke::new(1.0_f32, p.text);
    visuals.window_rounding = Rounding::same(14.0);

    let rounding = Rounding::same(9.0);
    visuals.widgets.noninteractive.bg_fill = p.panel;
    visuals.widgets.noninteractive.weak_bg_fill = p.panel;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, p.border);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, p.muted);
    visuals.widgets.noninteractive.rounding = rounding;

    visuals.widgets.inactive.bg_fill = p.card;
    visuals.widgets.inactive.weak_bg_fill = p.card;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, p.border);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, p.text);
    visuals.widgets.inactive.rounding = rounding;

    visuals.widgets.hovered.bg_fill = p.card_hover;
    visuals.widgets.hovered.weak_bg_fill = p.card_hover;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, p.accent_dim);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, p.text);
    visuals.widgets.hovered.rounding = rounding;

    visuals.widgets.active.bg_fill = p.accent_dim;
    visuals.widgets.active.weak_bg_fill = p.accent_dim;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, p.accent);
    visuals.widgets.active.fg_stroke = Stroke::new(1.5_f32, p.text);
    visuals.widgets.active.rounding = rounding;

    visuals.widgets.open.bg_fill = p.card_hover;
    visuals.widgets.open.weak_bg_fill = p.card_hover;
    visuals.widgets.open.bg_stroke = Stroke::new(1.0_f32, p.accent_dim);
    visuals.widgets.open.fg_stroke = Stroke::new(1.0_f32, p.text);

    // При наведении egui по умолчанию «раздувает» рамку кнопки, но текст
    // остаётся прежним — кнопка выглядит неаккуратно. Отключаем расширение.
    visuals.widgets.noninteractive.expansion = 0.0;
    visuals.widgets.inactive.expansion = 0.0;
    visuals.widgets.hovered.expansion = 0.0;
    visuals.widgets.active.expansion = 0.0;
    visuals.widgets.open.expansion = 0.0;

    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(10.0, 10.0);
    style.spacing.button_padding = egui::vec2(14.0, 8.0);
    style.spacing.scroll.bar_width = 9.0;
    style.spacing.scroll.floating = true;
    style.spacing.interact_size.y = 30.0;
    style.text_styles = [
        (
            TextStyle::Heading,
            FontId::new(22.0, FontFamily::Proportional),
        ),
        (TextStyle::Body, FontId::new(14.5, FontFamily::Proportional)),
        (
            TextStyle::Button,
            FontId::new(14.5, FontFamily::Proportional),
        ),
        (
            TextStyle::Small,
            FontId::new(12.0, FontFamily::Proportional),
        ),
        (
            TextStyle::Monospace,
            FontId::new(12.5, FontFamily::Monospace),
        ),
        (
            TextStyle::Name("h1".into()),
            FontId::new(28.0, FontFamily::Proportional),
        ),
        (
            TextStyle::Name("h2".into()),
            FontId::new(19.0, FontFamily::Proportional),
        ),
        (
            TextStyle::Name("tiny".into()),
            FontId::new(11.0, FontFamily::Proportional),
        ),
    ]
    .into();
    ctx.set_style(style);
    p
}

/// Иконка приложения из `assets/icon.png` (сконвертирована из .ico).
pub fn app_icon() -> egui::IconData {
    const BYTES: &[u8] = include_bytes!("../assets/icon.png");
    match image::load_from_memory_with_format(BYTES, image::ImageFormat::Png) {
        Ok(img) => {
            let rgba = img.to_rgba8();
            let (width, height) = rgba.dimensions();
            egui::IconData {
                rgba: rgba.into_raw(),
                width,
                height,
            }
        }
        // Иконка не критична для работы — не роняем приложение из-за неё.
        Err(_) => egui::IconData {
            rgba: vec![0, 0, 0, 0],
            width: 1,
            height: 1,
        },
    }
}

/// Красит системный заголовок окна в тон теме (Windows 10/11).
#[cfg(windows)]
pub fn apply_window_chrome(frame: &eframe::Frame, light: bool) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let Ok(handle) = frame.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return;
    };
    let hwnd = win32.hwnd.get() as *mut std::ffi::c_void;
    unsafe { set_window_theme(hwnd, !light) }
}

#[cfg(not(windows))]
pub fn apply_window_chrome(_frame: &eframe::Frame, _light: bool) {}

#[cfg(windows)]
unsafe fn set_window_theme(hwnd: *mut std::ffi::c_void, dark: bool) {
    #[link(name = "dwmapi")]
    extern "system" {
        fn DwmSetWindowAttribute(
            hwnd: *mut std::ffi::c_void,
            attribute: u32,
            value: *const std::ffi::c_void,
            size: u32,
        ) -> i32;
    }
    #[link(name = "user32")]
    extern "system" {
        fn SetWindowPos(
            hwnd: *mut std::ffi::c_void,
            after: *mut std::ffi::c_void,
            x: i32,
            y: i32,
            cx: i32,
            cy: i32,
            flags: u32,
        ) -> i32;
    }

    let enabled: i32 = if dark { 1 } else { 0 };
    let value = &enabled as *const i32 as *const std::ffi::c_void;
    // 20 — актуальный атрибут, 19 — старые сборки Windows 10.
    let _ = DwmSetWindowAttribute(hwnd, 20, value, 4);
    let _ = DwmSetWindowAttribute(hwnd, 19, value, 4);

    // Цвета в формате COLORREF (0x00BBGGRR); 0xFFFFFFFF — системный по умолчанию.
    let (caption, border, text) = if dark {
        (0x0016_0F1A_u32, 0x002E_223A_u32, 0x00EE_E9F4_u32)
    } else {
        // Светлая тема: светлый заголовок и тёмный текст, иначе подпись окна
        // становится белой на белом.
        (0x00F5_F2FA, 0x00D3_C7E2, 0x001C_122A)
    };
    for (attribute, color) in [(35_u32, caption), (34, border), (36, text)] {
        let _ = DwmSetWindowAttribute(
            hwnd,
            attribute,
            &color as *const u32 as *const std::ffi::c_void,
            4,
        );
    }

    // Перерисовать нестандартную рамку после смены темы.
    const SWP_NOSIZE: u32 = 0x0001;
    const SWP_NOMOVE: u32 = 0x0002;
    const SWP_NOZORDER: u32 = 0x0004;
    const SWP_FRAMECHANGED: u32 = 0x0020;
    let _ = SetWindowPos(
        hwnd,
        std::ptr::null_mut(),
        0,
        0,
        0,
        0,
        SWP_NOSIZE | SWP_NOMOVE | SWP_NOZORDER | SWP_FRAMECHANGED,
    );
}
