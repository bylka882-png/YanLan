//! Переиспользуемые элементы интерфейса.

use egui::{pos2, Align2, Color32, Frame, Margin, Response, RichText, Sense, Stroke, Ui, Vec2};

use crate::mods::LinkKind;
use crate::theme::Palette;

#[derive(Clone, Copy)]
enum ButtonKind {
    Primary,
    Soft,
    Danger,
}

/// Кнопка с текстом, которая при наведении увеличивается целиком — вместе
/// с текстом, а не только рамка.
fn text_button(ui: &mut Ui, text: &str, p: &Palette, enabled: bool, kind: ButtonKind) -> Response {
    let (base, min_height, color) = match kind {
        ButtonKind::Primary => (14.0, 34.0, if enabled { Color32::WHITE } else { p.muted }),
        ButtonKind::Soft => (13.5, 32.0, p.text),
        ButtonKind::Danger => (13.5, 32.0, p.red),
    };
    let font = egui::FontId::proportional(base);
    let galley = ui.fonts(|fonts| fonts.layout_no_wrap(text.to_owned(), font, color));
    let size = Vec2::new(
        galley.size().x + 28.0,
        (galley.size().y + 10.0).max(min_height),
    );
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    let hovered = enabled && response.hovered();

    let (fill, border) = match kind {
        ButtonKind::Primary => {
            if !enabled {
                (p.card, p.border)
            } else if hovered {
                (p.accent.lerp_to_gamma(Color32::WHITE, 0.10), p.accent)
            } else {
                (p.accent, p.accent)
            }
        }
        ButtonKind::Soft | ButtonKind::Danger => {
            let idle_border = match kind {
                ButtonKind::Danger => p.red.gamma_multiply(0.5),
                _ => p.border,
            };
            if !enabled {
                (p.card, p.border)
            } else if hovered {
                (
                    p.card_hover,
                    match kind {
                        ButtonKind::Danger => p.red,
                        _ => p.accent_dim,
                    },
                )
            } else {
                (p.card, idle_border)
            }
        }
    };

    // Рамка и текст растут одновременно: кнопка выглядит цельной.
    let grow = if hovered { 2.0 } else { 0.0 };
    ui.painter()
        .rect(rect.expand(grow), 9.0, fill, Stroke::new(1.0_f32, border));
    let font_size = if hovered { base + 1.0 } else { base };
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        egui::FontId::proportional(font_size),
        color,
    );
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}

pub fn primary(ui: &mut Ui, text: &str, p: &Palette, enabled: bool) -> Response {
    text_button(ui, text, p, enabled, ButtonKind::Primary)
}

pub fn soft(ui: &mut Ui, text: &str, p: &Palette, enabled: bool) -> Response {
    text_button(ui, text, p, enabled, ButtonKind::Soft)
}

pub fn danger(ui: &mut Ui, text: &str, p: &Palette, enabled: bool) -> Response {
    text_button(ui, text, p, enabled, ButtonKind::Danger)
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
        (p.card_hover, p.accent, p.text)
    } else {
        (p.card, p.border, p.muted)
    };
    let painter = ui.painter();
    let grow = if hovered { 2.0 } else { 0.0 };
    painter.rect(rect.expand(grow), 9.0, bg, Stroke::new(1.0_f32, border));
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        glyph,
        egui::FontId::proportional(if hovered { 16.0 } else { 15.0 }),
        color,
    );
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}

/// Крестик закрытия: обычная кнопка, которая заметно растёт при наведении.
pub fn close(ui: &mut Ui, p: &Palette) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::click());
    let hovered = response.hovered();
    let grown = rect.expand(if hovered { 3.0 } else { 0.0 });
    let painter = ui.painter();
    painter.circle_filled(
        grown.center(),
        grown.width() / 2.0,
        if hovered { p.card_hover } else { p.card },
    );
    painter.text(
        grown.center(),
        Align2::CENTER_CENTER,
        "✕",
        egui::FontId::proportional(if hovered { 17.0 } else { 14.0 }),
        if hovered { p.text } else { p.muted },
    );
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}

/// Дорожка и ручка переключателя. Общая для `toggle` и `switch`.
fn paint_switch(ui: &mut Ui, track: egui::Rect, t: f32, hovered: bool, p: &Palette) {
    let fill = p.card.lerp_to_gamma(p.accent, t);
    let border = if hovered && t < 0.5 {
        p.accent_dim
    } else {
        p.border.lerp_to_gamma(p.accent, t)
    };
    ui.painter()
        .rect(track, 12.0, fill, Stroke::new(1.0_f32, border));
    let knob_x = egui::lerp((track.left() + 12.0)..=(track.right() - 12.0), t);
    ui.painter().circle_filled(
        pos2(knob_x, track.center().y),
        if hovered { 8.5 } else { 7.6 },
        p.muted.lerp_to_gamma(Color32::WHITE, t),
    );
}

/// Розовый переключатель-ползунок: сам переключатель слева, подпись справа.
/// Возвращает `changed()` при щелчке.
pub fn toggle(ui: &mut Ui, value: &mut bool, text: &str, p: &Palette) -> Response {
    let width = ui.available_width();
    let (rect, mut response) = ui.allocate_exact_size(Vec2::new(width, 30.0), Sense::click());
    let hovered = response.hovered();

    let track = egui::Rect::from_min_size(
        pos2(rect.left() + 2.0, rect.center().y - 12.0),
        Vec2::new(44.0, 24.0),
    );
    let text_left = track.right() + 10.0;
    let font = egui::FontId::proportional(13.0);
    let galley = ui.fonts(|fonts| {
        fonts.layout(
            text.to_owned(),
            font,
            p.text,
            (rect.right() - text_left - 2.0).max(40.0),
        )
    });
    ui.painter().galley(
        pos2(text_left, rect.center().y - galley.size().y / 2.0),
        galley,
        p.text,
    );

    // Плавно едет не только ручка, но и заливка дорожки.
    let t = ui.ctx().animate_bool_with_time(response.id, *value, 0.16);
    paint_switch(ui, track, t, hovered, p);
    if t > 0.01 && t < 0.99 {
        ui.ctx().request_repaint();
    }

    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if response.clicked() {
        *value = !*value;
        response.mark_changed();
    }
    response
}

/// Переключатель без подписи: для строк, где место занято кнопками.
pub fn switch(ui: &mut Ui, value: &mut bool, p: &Palette) -> Response {
    let (rect, mut response) = ui.allocate_exact_size(Vec2::new(44.0, 24.0), Sense::click());
    let hovered = response.hovered();
    let t = ui.ctx().animate_bool_with_time(response.id, *value, 0.16);
    paint_switch(ui, rect, t, hovered, p);
    if t > 0.01 && t < 0.99 {
        ui.ctx().request_repaint();
    }
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if response.clicked() {
        *value = !*value;
        response.mark_changed();
    }
    response
}

/// Шестерёнка для строки установленного мода: рисуется вручную, чтобы
/// символ не зависел от шрифта.
pub fn gear(ui: &mut Ui, p: &Palette, enabled: bool) -> Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(34.0, 32.0), sense);
    let hovered = enabled && response.hovered();
    let (bg, border, ink) = if !enabled {
        (p.card, p.border, p.muted.gamma_multiply(0.6))
    } else if hovered {
        (p.card_hover, p.accent, p.text)
    } else {
        (p.card, p.border, p.muted)
    };
    let painter = ui.painter();
    let grow = if hovered { 2.0 } else { 0.0 };
    painter.rect(rect.expand(grow), 9.0, bg, Stroke::new(1.0_f32, border));
    let c = rect.center();
    let stroke = Stroke::new(1.6_f32, ink);
    for index in 0..8 {
        let angle = index as f32 * std::f32::consts::TAU / 8.0;
        let (sin, cos) = angle.sin_cos();
        painter.line_segment(
            [
                pos2(c.x + cos * 5.6, c.y + sin * 5.6),
                pos2(c.x + cos * 8.4, c.y + sin * 8.4),
            ],
            stroke,
        );
    }
    painter.circle_stroke(c, 5.4, stroke);
    painter.circle_filled(c, 1.8, ink);
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}

/// Компактная кнопка со стрелкой наружу — открывает внешнюю ссылку.
pub fn external(ui: &mut Ui, url: &str, p: &Palette) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(26.0, 22.0), Sense::click());
    let hovered = response.hovered();
    let grown = rect.expand(if hovered { 2.0 } else { 0.0 });
    let painter = ui.painter();
    painter.rect(
        grown,
        6.0,
        if hovered { p.card_hover } else { p.card },
        Stroke::new(1.0_f32, if hovered { p.accent } else { p.border }),
    );
    let ink = if hovered { p.accent } else { p.muted };
    let c = grown.center();
    let stroke = Stroke::new(1.5_f32, ink);
    painter.line_segment(
        [pos2(c.x - 4.5, c.y + 4.5), pos2(c.x + 4.0, c.y - 4.0)],
        stroke,
    );
    painter.line_segment(
        [pos2(c.x - 0.5, c.y - 4.0), pos2(c.x + 4.0, c.y - 4.0)],
        stroke,
    );
    painter.line_segment(
        [pos2(c.x + 4.0, c.y - 4.0), pos2(c.x + 4.0, c.y + 0.5)],
        stroke,
    );
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if response.clicked() {
        crate::util::open_url(url);
    }
    response.on_hover_text(url)
}

/// Кнопка ссылки из поля `credits`: фирменный значок и название сервиса.
pub fn credit(ui: &mut Ui, kind: LinkKind, url: &str, p: &Palette) -> Response {
    let font = egui::FontId::proportional(11.5);
    let text = kind.label();
    let galley = ui.fonts(|fonts| fonts.layout_no_wrap(text.to_owned(), font.clone(), p.text));
    let width = 14.0 + 6.0 + galley.size().x + 18.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 26.0), Sense::click());
    let hovered = response.hovered();
    let grown = rect.expand(if hovered { 1.5 } else { 0.0 });
    let painter = ui.painter();
    painter.rect(
        grown,
        7.0,
        if hovered {
            p.accent.gamma_multiply(0.16)
        } else {
            p.card
        },
        Stroke::new(1.0_f32, if hovered { p.accent } else { p.border }),
    );
    let icon = egui::Rect::from_min_size(
        pos2(rect.left() + 9.0, rect.center().y - 7.0),
        Vec2::splat(14.0),
    );
    brand_icon(painter, icon, kind, p);
    painter.text(
        pos2(icon.right() + 6.0, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        font,
        p.text,
    );
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if response.clicked() {
        crate::util::open_url(url);
    }
    response.on_hover_text(url)
}

fn brand_icon(painter: &egui::Painter, rect: egui::Rect, kind: LinkKind, p: &Palette) {
    let c = rect.center();
    match kind {
        LinkKind::YouTube => {
            painter.rect_filled(rect, 3.5, Color32::from_rgb(0xE5, 0x39, 0x35));
            painter.add(egui::Shape::convex_polygon(
                vec![
                    pos2(c.x - 3.0, c.y - 4.2),
                    pos2(c.x - 3.0, c.y + 4.2),
                    pos2(c.x + 4.2, c.y),
                ],
                Color32::WHITE,
                Stroke::NONE,
            ));
        }
        LinkKind::Discord => {
            painter.rect_filled(
                rect,
                rect.height() / 2.0,
                Color32::from_rgb(0x58, 0x65, 0xF2),
            );
            painter.circle_filled(pos2(c.x - 3.0, c.y - 0.5), 1.7, Color32::WHITE);
            painter.circle_filled(pos2(c.x + 3.0, c.y - 0.5), 1.7, Color32::WHITE);
        }
        LinkKind::GitHub => {
            let bg_sum = p.bg.r() as u32 + p.bg.g() as u32 + p.bg.b() as u32;
            let dark_bg = bg_sum < 220;
            let ink = if dark_bg {
                Color32::WHITE
            } else {
                Color32::from_rgb(0x24, 0x18, 0x1E)
            };
            painter.circle_filled(pos2(c.x, c.y + 1.2), 5.4, ink);
            painter.add(egui::Shape::convex_polygon(
                vec![
                    pos2(c.x - 6.0, c.y - 1.0),
                    pos2(c.x - 4.0, c.y - 7.0),
                    pos2(c.x - 1.0, c.y - 4.0),
                ],
                ink,
                Stroke::NONE,
            ));
            painter.add(egui::Shape::convex_polygon(
                vec![
                    pos2(c.x + 6.0, c.y - 1.0),
                    pos2(c.x + 4.0, c.y - 7.0),
                    pos2(c.x + 1.0, c.y - 4.0),
                ],
                ink,
                Stroke::NONE,
            ));
        }
        LinkKind::Web => {
            let stroke = Stroke::new(1.6_f32, p.muted);
            painter.line_segment(
                [pos2(c.x - 4.0, c.y + 4.0), pos2(c.x + 4.0, c.y - 4.0)],
                stroke,
            );
            painter.line_segment(
                [pos2(c.x - 0.5, c.y - 4.0), pos2(c.x + 4.0, c.y - 4.0)],
                stroke,
            );
            painter.line_segment(
                [pos2(c.x + 4.0, c.y - 4.0), pos2(c.x + 4.0, c.y + 0.5)],
                stroke,
            );
        }
    }
}

/// Компактный чип-фильтр с закруглением; используется в каталоге и «Установлено».
pub fn chip(
    ui: &mut Ui,
    p: &Palette,
    text: &str,
    active: bool,
    enabled: bool,
    color: Color32,
) -> Response {
    let font = egui::FontId::proportional(12.0);
    let galley = ui.fonts(|fonts| {
        fonts.layout_no_wrap(
            text.to_owned(),
            font,
            if active { Color32::WHITE } else { color },
        )
    });
    let size = Vec2::new(galley.size().x + 24.0, 28.0);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    let hovered = enabled && response.hovered();
    let (fill, border) = if active {
        (color.gamma_multiply(0.85), color)
    } else if hovered {
        (p.card_hover, color.gamma_multiply(0.7))
    } else {
        (p.card, p.border)
    };
    ui.painter()
        .rect(rect, 14.0, fill, Stroke::new(1.0_f32, border));
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        egui::FontId::proportional(12.0),
        if active { Color32::WHITE } else { color },
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
