//! Редактор конфигов модов: INI-подобные файлы BepInEx и AutoTranslator.
//!
//! Файл разбирается построчно: комментарии и порядок строк сохраняются,
//! меняются только значения. Логические `true`/`false` показываются
//! переключателем, а перечисление из `Acceptable values` — выпадающим списком.

use std::path::{Path, PathBuf};

use egui::{RichText, Vec2};

use crate::app::App;
use crate::i18n::{fill, pick};
use crate::theme::Palette;
use crate::ui::widgets as w;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Bool,
    Choice,
    Text,
    /// Клавиша Pose Mod: клик и нажатие клавиши.
    Key,
    /// Цвет Pose Mod: компоненты RGBA через запятую.
    Color,
}

pub struct Setting {
    /// Номер строки `key = value` в исходном файле.
    line: usize,
    section: String,
    key: String,
    value: String,
    /// Хвостовой комментарий после значения, если он был.
    trailing: String,
    kind: Kind,
    notes: Vec<String>,
    default: Option<String>,
    options: Vec<String>,
    /// Значение записывалось как `True`/`False`, а не `true`/`false`.
    upper: bool,
    /// Ожидает нажатия клавиши для поля `Kind::Key`.
    capturing: bool,
    /// Сколько компонент цвета было в файле: 3 (RGB) или 4 (RGBA).
    color_components: usize,
}

impl Setting {
    fn matches(&self, query: &str) -> bool {
        if query.is_empty() {
            return true;
        }
        let haystack = format!(
            "{} {} {} {}",
            self.section,
            self.key,
            self.value,
            self.notes.join(" ")
        )
        .to_lowercase();
        haystack.contains(query)
    }
}

pub struct Editor {
    pub title: String,
    pub path: PathBuf,
    /// Путь к файлу относительно папки сборки — для подписи в окне.
    pub rel: String,
    lines: Vec<String>,
    settings: Vec<Setting>,
    newline: String,
    trailing_newline: bool,
    error: Option<String>,
    query: String,
}

impl Editor {
    /// Идёт ли сейчас захват клавиши: Esc должен отменить его, а не закрыть окно.
    pub fn capturing(&self) -> bool {
        self.settings.iter().any(|setting| setting.capturing)
    }

    pub fn open(title: &str, rel: &str, path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let text = String::from_utf8_lossy(&bytes);
        let text = text.trim_start_matches('\u{feff}');
        let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
        let trailing_newline = text.ends_with('\n');
        let lines: Vec<String> = text.lines().map(str::to_string).collect();
        let settings = parse(&lines);
        Ok(Self {
            title: title.to_string(),
            path: path.to_path_buf(),
            rel: rel.to_string(),
            lines,
            settings,
            newline: newline.to_string(),
            trailing_newline,
            error: None,
            query: String::new(),
        })
    }

    /// Рисует окно. `Some(true)` — сохранено и закрыто, `Some(false)` —
    /// закрыто без сохранения, `None` — окно остаётся открытым.
    pub fn show(&mut self, ctx: &egui::Context, p: &Palette) -> Option<bool> {
        let screen = ctx.screen_rect();
        let width = (screen.width() - 80.0).clamp(420.0, 680.0);
        let max_height = (screen.height() * 0.56).clamp(240.0, 560.0);
        let query = self.query.trim().to_lowercase();
        let mut save = false;
        let mut close = false;

        let frame = egui::Frame::none()
            .fill(p.panel)
            .stroke(egui::Stroke::new(1.0_f32, p.border))
            .rounding(16.0)
            .inner_margin(egui::Margin::same(22.0))
            .shadow(egui::epaint::Shadow {
                offset: Vec2::new(0.0, 12.0),
                blur: 30.0,
                spread: 0.0,
                color: egui::Color32::from_black_alpha(if p.bg.r() > 180 { 60 } else { 150 }),
            });

        egui::Window::new("mod_cfg_dialog")
            .id(egui::Id::new(("mod_cfg_dialog", self.path.as_path())))
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .frame(frame)
            .show(ctx, |ui| {
                ui.set_width(width);
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.label(
                            RichText::new(fill(
                                &pick("Настройки · {name}", "Settings · {name}"),
                                &[("name", &self.title)],
                            ))
                            .size(18.0)
                            .strong()
                            .color(p.text),
                        );
                        ui.label(RichText::new(&self.rel).size(10.5).color(p.muted));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                        if w::close(ui, p)
                            .on_hover_text(pick("Закрыть", "Close"))
                            .clicked()
                        {
                            close = true;
                        }
                    });
                });
                ui.add_space(12.0);
                w::text_input(
                    ui,
                    &mut self.query,
                    pick("Поиск по параметрам", "Search settings"),
                    30.0,
                );
                ui.add_space(8.0);

                egui::ScrollArea::vertical()
                    .id_salt("mod_cfg_scroll")
                    .max_height(max_height)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        let mut section = String::new();
                        let mut shown = 0usize;
                        for setting in self.settings.iter_mut() {
                            if !setting.matches(&query) {
                                continue;
                            }
                            if setting.section != section {
                                section = setting.section.clone();
                                if shown > 0 {
                                    ui.add_space(10.0);
                                }
                                if !section.is_empty() {
                                    w::section_title(ui, &section, p);
                                }
                            }
                            shown += 1;
                            setting_row(ui, p, setting);
                            ui.add_space(7.0);
                        }
                        if shown == 0 {
                            ui.add_space(10.0);
                            w::muted(ui, pick("Ничего не найдено", "Nothing found"), p);
                        }
                    });

                if let Some(error) = &self.error {
                    ui.add_space(8.0);
                    ui.label(RichText::new(error).size(11.0).color(p.red));
                }

                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if w::primary(ui, pick("Сохранить", "Save"), p, true).clicked() {
                        save = true;
                    }
                    if w::soft(ui, pick("Отмена", "Cancel"), p, true).clicked() {
                        close = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(pick(
                                "Комментарии и порядок строк сохраняются",
                                "Comments and line order are preserved",
                            ))
                            .size(10.5)
                            .color(p.muted),
                        );
                    });
                });
            });

        if save {
            match self.save() {
                Ok(()) => return Some(true),
                Err(error) => self.error = Some(error),
            }
        }
        if close {
            return Some(false);
        }
        None
    }

    fn save(&mut self) -> Result<(), String> {
        for setting in &self.settings {
            let Some(line) = self.lines.get_mut(setting.line) else {
                continue;
            };
            // Pose Mod пишет `ключ:значение`, BepInEx — `ключ = значение`.
            let Some(pos) = line.find('=').into_iter().chain(line.find(':')).min() else {
                continue;
            };
            let head = line[..=pos].trim_end().to_string();
            let mut new_line = head;
            if !setting.value.is_empty() {
                new_line.push(' ');
                new_line.push_str(&setting.value);
            }
            new_line.push_str(&setting.trailing);
            *line = new_line;
        }
        let mut text = self.lines.join(&self.newline);
        if self.trailing_newline {
            text.push_str(&self.newline);
        }
        std::fs::write(&self.path, text).map_err(|e| format!("{}: {e}", self.path.display()))?;
        Ok(())
    }
}

fn parse(lines: &[String]) -> Vec<Setting> {
    let mut out = Vec::new();
    let mut section = String::new();
    let mut pending: Vec<String> = Vec::new();

    for (index, raw) in lines.iter().enumerate() {
        let line = raw.trim();
        if line.is_empty() {
            pending.clear();
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].trim().to_string();
            pending.clear();
            continue;
        }
        if line.starts_with('#') || line.starts_with(';') {
            let note = line.trim_start_matches(|c| c == '#' || c == ';').trim();
            if !note.is_empty() {
                pending.push(note.to_string());
            }
            continue;
        }
        let Some((sep, key, raw_value)) = split_setting(raw) else {
            pending.clear();
            continue;
        };
        let (value, trailing) = match raw_value.find(" #") {
            Some(pos) => (
                raw_value[..pos].trim().to_string(),
                raw_value[pos..].to_string(),
            ),
            None => (raw_value.trim().to_string(), String::new()),
        };

        let mut options = Vec::new();
        let mut notes = Vec::new();
        let mut default = None;
        let mut bool_type = false;
        for note in &pending {
            let lower = note.to_ascii_lowercase();
            if let Some(rest) = lower.strip_prefix("setting type:") {
                bool_type = rest.trim() == "boolean";
                continue;
            }
            if let Some(rest) = lower.strip_prefix("default value:") {
                let _ = rest;
                default = Some(
                    note[note.find(':').map(|i| i + 1).unwrap_or(0)..]
                        .trim()
                        .to_string(),
                );
                continue;
            }
            if let Some(pos) = lower.find("acceptable values:") {
                let rest = &note[pos + "acceptable values:".len()..];
                for item in rest.split(',') {
                    let item = item.trim();
                    if !item.is_empty() {
                        options.push(item.to_string());
                    }
                }
                continue;
            }
            notes.push(note.clone());
        }

        let is_bool_value =
            value.eq_ignore_ascii_case("true") || value.eq_ignore_ascii_case("false");
        let color = parse_color(&value);
        let kind = if bool_type || is_bool_value {
            Kind::Bool
        } else if color.is_some() {
            Kind::Color
        } else if sep == ':' && is_key(&value) {
            Kind::Key
        } else if !options.is_empty() {
            Kind::Choice
        } else {
            Kind::Text
        };
        if kind == Kind::Choice && !value.is_empty() && !options.iter().any(|v| v == &value) {
            options.insert(0, value.clone());
        }

        out.push(Setting {
            line: index,
            section: section.clone(),
            key,
            value: value.clone(),
            trailing,
            kind,
            notes,
            default,
            options,
            upper: value.starts_with('T') || value.starts_with('F'),
            capturing: false,
            color_components: color.map_or(0, |c| c.len()),
        });
        pending.clear();
    }
    out
}

/// Делит строку на ключ и значение по первому `=` или `:`.
fn split_setting(raw: &str) -> Option<(char, String, &str)> {
    let pos = raw.find('=').into_iter().chain(raw.find(':')).min()?;
    let key = raw[..pos].trim().to_string();
    if key.is_empty() {
        return None;
    }
    Some((raw.as_bytes()[pos] as char, key, &raw[pos + 1..]))
}

/// `0.62,0,0,1` → компоненты цвета. Только числа в диапазоне 0..=1.
fn parse_color(value: &str) -> Option<Vec<f32>> {
    let parts: Vec<&str> = value.split(',').map(str::trim).collect();
    if parts.len() != 3 && parts.len() != 4 {
        return None;
    }
    let mut out = Vec::with_capacity(parts.len());
    for part in parts {
        let number: f32 = part.parse().ok()?;
        if !(0.0..=1.0).contains(&number) {
            return None;
        }
        out.push(number);
    }
    Some(out)
}

/// Похоже ли значение на клавишу Pose Mod.
fn is_key(value: &str) -> bool {
    let value = value.trim();
    if value.is_empty() {
        return false;
    }
    if value.len() == 1 && value.chars().all(|c| c.is_ascii_alphanumeric()) {
        return true;
    }
    let upper = value.to_ascii_uppercase();
    if let Some(number) = upper.strip_prefix('F') {
        if let Ok(number) = number.parse::<u8>() {
            if (1..=35).contains(&number) {
                return true;
            }
        }
    }
    matches!(
        upper.as_str(),
        "SPACE"
            | "TAB"
            | "ESCAPE"
            | "RETURN"
            | "ENTER"
            | "BACKSPACE"
            | "DELETE"
            | "INSERT"
            | "HOME"
            | "END"
            | "PAGEUP"
            | "PAGEDOWN"
            | "UPARROW"
            | "DOWNARROW"
            | "LEFTARROW"
            | "RIGHTARROW"
            | "CAPSLOCK"
            | "LEFTSHIFT"
            | "RIGHTSHIFT"
            | "LEFTCONTROL"
            | "RIGHTCONTROL"
            | "LEFTALT"
            | "RIGHTALT"
            | "NUMLOCK"
            | "SCROLLLOCK"
            | "PAUSE"
            | "PRINTSCREEN"
    )
}

/// Имя клавиши в нотации Unity `KeyCode` (её понимает Pose Mod).
fn key_name(key: egui::Key) -> String {
    match key.name() {
        "Up" => "UpArrow".to_string(),
        "Down" => "DownArrow".to_string(),
        "Left" => "LeftArrow".to_string(),
        "Right" => "RightArrow".to_string(),
        "Enter" => "Return".to_string(),
        "Backtick" => "BackQuote".to_string(),
        "OpenBracket" => "LeftBracket".to_string(),
        "CloseBracket" => "RightBracket".to_string(),
        name => name.to_string(),
    }
}

fn format_color(rgba: &[f32; 4], components: usize) -> String {
    let format = |value: f32| {
        if value <= 0.0 {
            "0".to_string()
        } else if value >= 1.0 {
            "1".to_string()
        } else {
            let text = format!("{value:.3}");
            text.trim_end_matches('0').trim_end_matches('.').to_string()
        }
    };
    let mut parts: Vec<String> = rgba[..3.min(components.max(3))]
        .iter()
        .map(|value| format(*value))
        .collect();
    if components >= 4 {
        parts.push(format(rgba[3]));
    }
    parts.join(",")
}

/// Подписи известных параметров Pose Mod; остальные остаются как в файле.
fn setting_label(key: &str) -> String {
    if let Some(rest) = key.strip_prefix("LoadOrder_") {
        return fill(
            &pick("Порядок загрузки · {name}", "Load order · {name}"),
            &[("name", rest)],
        );
    }
    let label = match key {
        "UIColor" => pick("Цвет интерфейса", "Interface color"),
        "MiniMenuTextColor" => pick("Цвет текста мини-меню", "Mini-menu text color"),
        "MiniMenuTextOutlineColor" => pick("Обводка текста мини-меню", "Mini-menu text outline"),
        "PosingMenu" => pick("Меню позирования", "Posing menu"),
        "RefnameViewer" => pick("Просмотр имён", "Refname viewer"),
        "FreeCam" => pick("Свободная камера", "Free camera"),
        "Console" => pick("Консоль", "Console"),
        "MiniMenu" => pick("Мини-меню", "Mini menu"),
        "ArrowXColor" => pick("Цвет стрелки X", "Arrow X color"),
        "ArrowYColor" => pick("Цвет стрелки Y", "Arrow Y color"),
        "ArrowZColor" => pick("Цвет стрелки Z", "Arrow Z color"),
        "ArrowSeleted" | "ArrowSelected" => pick("Выбранная стрелка", "Selected arrow"),
        "BackgroundImage" => pick("Фоновое изображение", "Background image"),
        "PointerImage" => pick("Изображение курсора", "Pointer image"),
        "MenuColor" => pick("Цвет меню", "Menu color"),
        "TextColor" => pick("Цвет текста", "Text color"),
        "TextShadowColor" => pick("Тень текста", "Text shadow color"),
        _ => return key.to_string(),
    };
    label.to_string()
}

fn setting_row(ui: &mut egui::Ui, p: &Palette, setting: &mut Setting) {
    let label = setting_label(&setting.key);
    match setting.kind {
        Kind::Bool => {
            let mut on = setting.value.eq_ignore_ascii_case("true");
            if w::toggle(ui, &mut on, &label, p).changed() {
                setting.value = match (on, setting.upper) {
                    (true, true) => "True".to_string(),
                    (true, false) => "true".to_string(),
                    (false, true) => "False".to_string(),
                    (false, false) => "false".to_string(),
                };
            }
        }
        Kind::Choice => {
            ui.label(RichText::new(&label).size(12.5).color(p.text));
            ui.add_space(3.0);
            egui::ComboBox::from_id_salt(("mod_cfg_choice", setting.line))
                .selected_text(RichText::new(&setting.value).size(12.0))
                .width(240.0)
                .show_ui(ui, |ui| {
                    for option in &setting.options {
                        if ui
                            .selectable_label(&setting.value == option, option)
                            .clicked()
                        {
                            setting.value = option.clone();
                        }
                    }
                });
        }
        Kind::Key => {
            ui.label(RichText::new(&label).size(12.5).color(p.text));
            ui.add_space(3.0);
            if setting.capturing {
                let text = pick(
                    "Нажмите клавишу… (Esc — отмена)",
                    "Press a key… (Esc to cancel)",
                );
                w::primary(ui, text, p, true);
                if let Some(key) = ui.input(|i| {
                    i.events.iter().find_map(|event| match event {
                        egui::Event::Key {
                            key, pressed: true, ..
                        } => Some(*key),
                        _ => None,
                    })
                }) {
                    if key == egui::Key::Escape {
                        setting.capturing = false;
                    } else {
                        setting.value = key_name(key);
                        setting.capturing = false;
                    }
                }
            } else {
                let text = if setting.value.is_empty() {
                    pick("не задано", "not set").to_string()
                } else {
                    setting.value.clone()
                };
                if w::soft(ui, &text, p, true)
                    .on_hover_text(pick(
                        "Нажмите, чтобы назначить клавишу",
                        "Click to assign a key",
                    ))
                    .clicked()
                {
                    setting.capturing = true;
                    ui.ctx().memory_mut(|memory| memory.stop_text_input());
                }
            }
        }
        Kind::Color => {
            let mut rgba = [0.0, 0.0, 0.0, 1.0];
            if let Some(color) = parse_color(&setting.value) {
                for (slot, value) in rgba.iter_mut().zip(color) {
                    *slot = value;
                }
            }
            ui.horizontal(|ui| {
                ui.label(RichText::new(&label).size(12.5).color(p.text));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.color_edit_button_rgba_unmultiplied(&mut rgba).changed() {
                        setting.value = format_color(&rgba, setting.color_components);
                    }
                    ui.label(RichText::new(&setting.value).size(10.5).color(p.muted));
                });
            });
        }
        Kind::Text => {
            ui.label(RichText::new(&label).size(12.5).color(p.text));
            ui.add_space(3.0);
            w::text_input(ui, &mut setting.value, "", 28.0);
        }
    }

    if let Some(default) = &setting.default {
        if setting.kind != Kind::Bool {
            ui.add_space(2.0);
            ui.label(
                RichText::new(fill(
                    &pick("По умолчанию: {value}", "Default: {value}"),
                    &[("value", default)],
                ))
                .size(10.5)
                .color(p.muted),
            );
        }
    }
    for note in &setting.notes {
        ui.add_space(1.0);
        ui.label(RichText::new(note).size(10.5).color(p.muted));
    }
}

impl App {
    /// Открывает конфиг по пути из каталога или библиотеки.
    pub fn open_mod_cfg(&mut self, game_id: &str, mod_id: &str, name: &str, cfg: &str) {
        let Some(game) = self.games.iter().find(|g| g.meta.id == game_id).cloned() else {
            self.toast(
                pick("Сборка мода не найдена", "The mod's build was not found"),
                false,
            );
            return;
        };

        // У модов, установленных до появления поля `cfg`, путь можно взять
        // из актуального каталога.
        let mut cfg = cfg.trim().to_string();
        if cfg.is_empty() {
            cfg = self
                .catalog
                .iter()
                .find(|group| group.id == mod_id)
                .and_then(|group| {
                    group
                        .variants
                        .iter()
                        .map(|variant| variant.cfg.trim())
                        .find(|cfg| !cfg.is_empty())
                })
                .unwrap_or("")
                .to_string();
        }
        // Старые версии Pose Mod: у них в манифесте путь мог не указаться.
        if cfg.is_empty() && mod_id.eq_ignore_ascii_case("PoseMod") {
            cfg = [
                "PoseMod/Settings.ini",
                "YandereSimulator_Data/StreamingAssets/PoseMod/Settings.txt",
            ]
            .into_iter()
            .find(|rel| game.path.join(rel).is_file())
            .unwrap_or("")
            .to_string();
        }
        if cfg.is_empty() {
            self.toast(
                fill(
                    &pick(
                        "У мода «{name}» нет настроек",
                        "The mod “{name}” has no settings",
                    ),
                    &[("name", name)],
                ),
                false,
            );
            return;
        }

        let rel = cfg.replace('\\', "/").trim_start_matches('/').to_string();
        let path = game.path.join(&rel);
        if !path.is_file() {
            self.toast(
                fill(
                    &pick(
                        "Файл настроек не найден: {path}",
                        "Settings file not found: {path}",
                    ),
                    &[("path", &path.display().to_string())],
                ),
                false,
            );
            return;
        }

        match Editor::open(name, &rel, &path) {
            Ok(editor) => self.mod_cfg = Some(editor),
            Err(err) => self.toast(err, false),
        }
    }

    pub fn draw_mod_settings(&mut self, ctx: &egui::Context) {
        let p = self.p;
        let Some(editor) = self.mod_cfg.as_mut() else {
            return;
        };
        let title = editor.title.clone();
        match editor.show(ctx, &p) {
            Some(true) => {
                self.mod_cfg = None;
                self.toast(
                    fill(
                        &pick(
                            "Настройки «{name}» сохранены",
                            "Settings for “{name}” saved",
                        ),
                        &[("name", &title)],
                    ),
                    true,
                );
                self.log(fill(
                    &pick(
                        "Сохранён конфиг мода «{name}»",
                        "Saved the config of “{name}”",
                    ),
                    &[("name", &title)],
                ));
            }
            Some(false) => self.mod_cfg = None,
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
[General]

## Enable the cache
# Setting type: Boolean
# Default value: true
EnableCache = true

# Setting type: LogChannel
# Default value: Warn, Error
# Acceptable values: None, Info, IL, Warn, Error, Debug, All
LogChannels = Warn, Error

# Setting type: String
# Default value:
Name = test
";

    fn settings() -> Vec<Setting> {
        let lines: Vec<String> = SAMPLE.lines().map(str::to_string).collect();
        parse(&lines)
    }

    #[test]
    fn bepinex_notes_select_the_control() {
        let settings = settings();
        assert_eq!(settings.len(), 3);
        assert!(matches!(settings[0].kind, Kind::Bool));
        assert!(settings[0]
            .notes
            .iter()
            .any(|note| note.contains("Enable the cache")));

        assert!(matches!(settings[1].kind, Kind::Choice));
        assert!(settings[1].options.iter().any(|value| value == "Debug"));
        assert_eq!(settings[1].value, "Warn, Error");

        assert!(matches!(settings[2].kind, Kind::Text));
        assert_eq!(settings[2].default.as_deref(), Some(""));
    }

    #[test]
    fn bool_spelling_is_preserved() {
        let lines: Vec<String> = "Enable=True\n".lines().map(str::to_string).collect();
        let settings = parse(&lines);
        assert!(matches!(settings[0].kind, Kind::Bool));
        assert!(settings[0].upper);
    }

    #[test]
    fn pose_settings_use_colon_and_pickers() {
        let text = "UIColor:0.62,0,0,1\n\
                    PosingMenu:F9\n\
                    MiniMenu:x\n\
                    MenuColor:1,.75,1,1\n\
                    BackgroundImage:\n\
                    PointerImage:PoseMod/Pointer.png\n";
        let lines: Vec<String> = text.lines().map(str::to_string).collect();
        let settings = parse(&lines);
        assert_eq!(settings.len(), 6);
        assert!(matches!(settings[0].kind, Kind::Color));
        assert_eq!(settings[0].color_components, 4);
        assert!(matches!(settings[1].kind, Kind::Key));
        assert!(matches!(settings[2].kind, Kind::Key));
        assert!(matches!(settings[3].kind, Kind::Color));
        assert!(matches!(settings[4].kind, Kind::Text));
        assert!(matches!(settings[5].kind, Kind::Text));
        assert_eq!(settings[1].key, "PosingMenu");
    }

    #[test]
    fn colon_values_keep_no_space_on_save() {
        let mut lines: Vec<String> = "UIColor:0.62,0,0,1\n".lines().map(str::to_string).collect();
        let settings = parse(&lines);
        assert_eq!(settings[0].kind, Kind::Color);
        let pos = lines[0].find(':').unwrap();
        let head = lines[0][..=pos].trim_end().to_string();
        assert_eq!(format!("{head}0.5,0,0,1"), "UIColor:0.5,0,0,1");
        // Значение через `=` осталось бы с пробелами, как в BepInEx.
        lines[0] = format!("{head}{}", format_color(&[0.5, 0.0, 0.0, 1.0], 4));
        assert_eq!(lines[0], "UIColor:0.5,0,0,1");
    }
}
