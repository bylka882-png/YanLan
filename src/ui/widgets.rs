//! Переиспользуемые элементы интерфейса.

use egui::{Color32, Frame, Margin, Response, RichText, Sense, Stroke, Ui, Vec2};

use crate::theme::Palette;

pub fn primary(ui: &mut Ui, text: &str, p: &Palette, enabled: bool) -> Response {
    let button = egui::Button::new(
        RichText::new(text)
            .color(if enabled { Color32::WHITE } else { p.muted })
            .size(14.0),
    )
    .fill(if enabled { p.accent } else { p.card })
    .stroke(Stroke::new(
        1.0_f32,
        if enabled { p.accent } else { p.border },
    ))
    .rounding(9.0)
    .min_size(Vec2::new(0.0, 34.0));
    ui.add_enabled(enabled, button)
}

pub fn soft(ui: &mut Ui, text: &str, p: &Palette, enabled: bool) -> Response {
    let button = egui::Button::new(RichText::new(text).color(p.text).size(13.5))
        .fill(p.card)
        .stroke(Stroke::new(1.0_f32, p.border))
        .rounding(9.0)
        .min_size(Vec2::new(0.0, 32.0));
    ui.add_enabled(enabled, button)
}

pub fn danger(ui: &mut Ui, text: &str, p: &Palette, enabled: bool) -> Response {
    let button = egui::Button::new(RichText::new(text).color(p.red).size(13.5))
        .fill(p.card)
        .stroke(Stroke::new(1.0_f32, p.red.gamma_multiply(0.5)))
        .rounding(9.0)
        .min_size(Vec2::new(0.0, 32.0));
    ui.add_enabled(enabled, button)
}

/// Компактная квадратная кнопка с иконкой (например, «⚙» для параметров запуска).
///
/// Рисуем её вручную: символ из запасного шрифта иначе оказывается смещён
/// относительно центра кнопки.
pub fn icon(ui: &mut Ui, glyph: &str, p: &Palette, active: bool, enabled: bool) -> Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(34.0, 32.0), sense);
    let hovered = enabled && response.hovered();
    let (bg, border, color) = if !enabled {
        (p.card, p.border, p.muted.gamma_multiply(0.6))
    } else if active {
        (
            p.accent.gamma_multiply(0.14),
            p.accent.gamma_multiply(0.6),
            p.accent,
        )
    } else if hovered {
        (p.card_hover, p.accent_dim, p.text)
    } else {
        (p.card, p.border, p.muted)
    };
    let painter = ui.painter();
    painter.rect(rect, 9.0, bg, Stroke::new(1.0_f32, border));
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        glyph,
        egui::FontId::proportional(15.0),
        color,
    );
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}

/// Поле ввода с текстом, выровненным по центру по вертикали.
pub fn text_input(ui: &mut Ui, text: &mut String, hint: &str, height: f32) -> Response {
    let width = ui.available_width();
    text_input_sized(ui, text, hint, Vec2::new(width, height))
}

/// То же, но с явным размером — для полей, которые стоят в одном ряду с кнопками.
pub fn text_input_sized(ui: &mut Ui, text: &mut String, hint: &str, size: Vec2) -> Response {
    ui.add_sized(
        size,
        egui::TextEdit::singleline(text)
            .hint_text(hint)
            .vertical_align(egui::Align::Center),
    )
}

pub fn section_title(ui: &mut Ui, text: &str, p: &Palette) {
    ui.label(RichText::new(text).size(11.0).color(p.muted).strong());
    ui.add_space(2.0);
}

pub fn muted(ui: &mut Ui, text: impl Into<String>, p: &Palette) {
    ui.label(RichText::new(text.into()).size(12.5).color(p.muted));
}

pub fn tag(ui: &mut Ui, text: &str, color: Color32, p: &Palette) {
    // В светлой теме исходные яркие цвета (особенно серый «Другое») на почти
    // белом фоне читаются плохо — подмешиваем цвет текста к чернилам тега.
    let (ink, fill, stroke) = tag_colors(ui, color, p);
    Frame::none()
        .fill(fill)
        .stroke(Stroke::new(1.0_f32, stroke))
        .rounding(6.0)
        .inner_margin(Margin::symmetric(7.0, 2.0))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(11.0).color(ink));
        });
}

pub fn tag_small(ui: &mut Ui, text: &str, color: Color32, p: &Palette) {
    // Тег рисуем вручную с фиксированной высотой: Frame подстраивается под
    // высоту строки шрифта, из-за чего тег получался почти квадратным.
    let (ink, fill, stroke) = tag_colors(ui, color, p);
    let font = egui::FontId::proportional(10.0);
    let galley = ui.fonts(|fonts| fonts.layout_no_wrap(text.to_owned(), font, ink));
    let width = galley.mesh_bounds.width() + 14.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 18.0), Sense::hover());
    let painter = ui.painter();
    painter.rect(rect, 4.0, fill, Stroke::new(1.0_f32, stroke));
    // Центрируем именно видимые глифы, а не всю строку с её полями.
    let pos = rect.center() - galley.mesh_bounds.center().to_vec2();
    painter.galley(pos, galley, ink);
}

fn tag_colors(ui: &Ui, color: Color32, p: &Palette) -> (Color32, Color32, Color32) {
    if ui.visuals().dark_mode {
        (color, color.gamma_multiply(0.16), color.gamma_multiply(0.5))
    } else {
        let ink = color.lerp_to_gamma(p.text, 0.45);
        (ink, color.gamma_multiply(0.12), ink.gamma_multiply(0.55))
    }
}

pub fn progress_bar(ui: &mut Ui, fraction: f32, p: &Palette, height: f32) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, height / 2.0, p.card);
    let fraction = fraction.clamp(0.0, 1.0);
    if fraction > 0.0 {
        let filled = egui::Rect::from_min_size(
            rect.min,
            Vec2::new((rect.width() * fraction).max(height), height),
        );
        painter.rect_filled(filled, height / 2.0, p.accent);
    }
}

pub fn divider(ui: &mut Ui, p: &Palette) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, p.border);
}

pub fn hyperlink(ui: &mut Ui, text: &str, url: &str, p: &Palette) -> Response {
    let response = ui.add(
        egui::Label::new(RichText::new(text).size(12.5).color(p.accent)).sense(Sense::click()),
    );
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if response.clicked() {
        crate::util::open_url(url);
    }
    response.on_hover_text(url)
}

pub fn key_value(ui: &mut Ui, key: &str, value: &str, p: &Palette) {
    ui.horizontal(|ui| {
        ui.add_sized(
            Vec2::new(170.0, 18.0),
            egui::Label::new(RichText::new(key).size(12.5).color(p.muted)),
        );
        ui.label(RichText::new(value).size(12.5).color(p.text));
    });
}

pub fn count_label(count: usize) -> String {
    crate::i18n::fill(
        crate::i18n::pick("{count} шт.", "{count} pcs"),
        &[("count", &count.to_string())],
    )
}
