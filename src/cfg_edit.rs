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

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Bool,
    Choice,
    Text,
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
            let Some(eq) = line.find('=') else {
                continue;
            };
            let head = line[..=eq].trim_end().to_string();
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
        let Some(eq) = raw.find('=') else {
            pending.clear();
            continue;
        };
        let key = raw[..eq].trim().to_string();
        if key.is_empty() {
            pending.clear();
            continue;
        }
        let raw_value = &raw[eq + 1..];
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
        let kind = if bool_type || is_bool_value {
            Kind::Bool
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
        });
        pending.clear();
    }
    out
}

fn setting_row(ui: &mut egui::Ui, p: &Palette, setting: &mut Setting) {
    match setting.kind {
        Kind::Bool => {
            let mut on = setting.value.eq_ignore_ascii_case("true");
            if w::toggle(ui, &mut on, &setting.key, p).changed() {
                setting.value = match (on, setting.upper) {
                    (true, true) => "True".to_string(),
                    (true, false) => "true".to_string(),
                    (false, true) => "False".to_string(),
                    (false, false) => "false".to_string(),
                };
            }
        }
        Kind::Choice => {
            ui.label(RichText::new(&setting.key).size(12.5).color(p.text));
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
        Kind::Text => {
            ui.label(RichText::new(&setting.key).size(12.5).color(p.text));
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
    /// Открывает редактор конфига установленного мода.
    pub fn open_mod_settings(&mut self, key: &str) {
        let Some(item) = self.library.mods.iter().find(|m| m.key == key).cloned() else {
            self.toast(
                pick("Запись о моде не найдена", "Mod record not found"),
                false,
            );
            return;
        };
        let Some(game) = self
            .games
            .iter()
            .find(|g| g.meta.id == item.game_id)
            .cloned()
        else {
            self.toast(
                pick("Сборка мода не найдена", "The mod's build was not found"),
                false,
            );
            return;
        };

        // У модов, установленных до появления поля `cfg`, путь можно взять
        // из актуального каталога.
        let mut cfg = item.cfg.trim().to_string();
        if cfg.is_empty() {
            cfg = self
                .catalog
                .iter()
                .find(|group| group.id == item.mod_id)
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
        if cfg.is_empty() {
            self.toast(
                fill(
                    &pick(
                        "У мода «{name}» нет настроек",
                        "The mod “{name}” has no settings",
                    ),
                    &[("name", &item.name)],
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

        match Editor::open(&item.name, &rel, &path) {
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
}
