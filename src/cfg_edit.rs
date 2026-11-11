//! Редактор конфигов модов: INI-подобные файлы BepInEx и AutoTranslator.
//!
//! Файл разбирается построчно: комментарии и порядок строк сохраняются,
//! меняются только значения. Логические `true`/`false` показываются
//! переключателем, а перечисление из `Acceptable values` — выпадающим списком.

use std::path::{Path, PathBuf};

use egui::{RichText, Vec2};

use crate::app::App;
use crate::i18n::{fill, is_ru, pick};
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
    /// Исходный текст файла и режим «обычный текст».
    raw: String,
    text_mode: bool,
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
            raw: text.to_string(),
            text_mode: false,
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
                        let label = if self.text_mode {
                            pick("Визуальный редактор", "Visual editor")
                        } else {
                            pick("Обычный текст", "Plain text")
                        };
                        if w::soft(ui, label, p, true).clicked() {
                            if self.text_mode {
                                self.apply_raw();
                            } else {
                                self.raw = self.render_lines();
                            }
                            self.text_mode = !self.text_mode;
                            self.error = None;
                        }
                    });
                });
                ui.add_space(12.0);
                if self.text_mode {
                    egui::ScrollArea::vertical()
                        .id_salt("mod_cfg_raw_scroll")
                        .max_height(max_height)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::multiline(&mut self.raw)
                                    .code_editor()
                                    .desired_width(f32::INFINITY)
                                    .desired_rows(20),
                            );
                        });
                } else {
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
                                        w::section_title(ui, &section_label(&section), p);
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
                }

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

    fn render_lines(&self) -> String {
        let mut text = self.lines.join(&self.newline);
        if self.trailing_newline {
            text.push_str(&self.newline);
        }
        text
    }

    /// Перечитывает визуальные настройки из текста.
    fn apply_raw(&mut self) {
        self.newline = if self.raw.contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        }
        .to_string();
        self.trailing_newline = self.raw.ends_with('\n');
        self.lines = self.raw.lines().map(str::to_string).collect();
        self.settings = parse(&self.lines);
    }

    fn save(&mut self) -> Result<(), String> {
        if self.text_mode {
            return crate::util::write_atomic(&self.path, &self.raw)
                .map_err(|e| format!("{}: {e}", self.path.display()));
        }
        for setting in &self.settings {
            let Some(line) = self.lines.get_mut(setting.line) else {
                continue;
            };
            // Pose Mod пишет `ключ:значение`, BepInEx — `ключ = значение`.
            let Some(pos) = line.find('=').into_iter().chain(line.find(':')).min() else {
                continue;
            };
            let after = &line[pos + 1..];
            let spaced = after.starts_with(' ') || after.starts_with('\t');
            let head = line[..=pos].trim_end().to_string();
            let mut new_line = head;
            if !setting.value.is_empty() {
                // Пробел после разделителя сохраняем как в исходном файле.
                if spaced {
                    new_line.push(' ');
                }
                new_line.push_str(&setting.value);
            }
            new_line.push_str(&setting.trailing);
            *line = new_line;
        }
        let mut text = self.lines.join(&self.newline);
        if self.trailing_newline {
            text.push_str(&self.newline);
        }
        crate::util::write_atomic(&self.path, text)
            .map_err(|e| format!("{}: {e}", self.path.display()))?;
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

/// Подписи известных параметров конфигов; остальные остаются как в файле.
fn setting_label(key: &str) -> String {
    if let Some(rest) = key.strip_prefix("LoadOrder_") {
        return fill(
            &pick("Порядок загрузки · {name}", "Load order · {name}"),
            &[("name", rest)],
        );
    }
    let label = match key {
        // Pose Mod
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
        // BepInEx
        "EnableAssemblyCache" => pick("Кэш метаданных сборок", "Assembly metadata cache"),
        "HideManagerGameObject" => pick("Скрывать Manager GameObject", "Hide Manager GameObject"),
        "LogChannels" => pick("Каналы логов", "Log channels"),
        "UnityLogListening" => pick("Слушать лог Unity", "Listen to the Unity log"),
        "LogConsoleToUnityLog" => pick(
            "Писать консоль в лог Unity",
            "Write console to the Unity log",
        ),
        "Enabled" => pick("Включено", "Enabled"),
        "PreventClose" => pick("Запретить закрытие консоли", "Prevent closing the console"),
        "ShiftJisEncoding" => pick("Кодировка Shift-JIS", "Shift-JIS encoding"),
        "StandardOutType" => pick("Тип стандартного вывода", "Standard output type"),
        "LogLevels" => pick("Уровни логов", "Log levels"),
        "WriteUnityLog" => pick("Писать в лог Unity", "Write to the Unity log"),
        "AppendLog" => pick("Дописывать лог", "Append to the log"),
        "ApplyRuntimePatches" => pick("Патчи во время выполнения", "Apply runtime patches"),
        "HarmonyBackend" => pick("Бэкенд Harmony", "Harmony backend"),
        "DumpAssemblies" => pick("Сохранять сборки", "Dump assemblies"),
        "LoadDumpedAssemblies" => pick("Загружать сохранённые сборки", "Load dumped assemblies"),
        "BreakBeforeLoadAssemblies" => pick(
            "Пауза перед загрузкой сборок",
            "Break before loading assemblies",
        ),
        "Assembly" => pick("Сборка", "Assembly"),
        "Type" => pick("Тип", "Type"),
        "Method" => pick("Метод", "Method"),
        // Resource Unload Optimizations
        "Disable Resource Unload" => pick("Отключить выгрузку ресурсов", "Disable resource unload"),
        "Optimize Memory Usage" => pick("Оптимизировать память", "Optimize memory usage"),
        "Optimize Memory Threshold" => {
            pick("Порог оптимизации памяти", "Optimize memory threshold")
        }
        // Configuration Manager
        "Show advanced" => pick("Показывать расширенные", "Show advanced"),
        "Show keybinds" => pick("Показывать горячие клавиши", "Show keybinds"),
        "Show settings" => pick("Показывать настройки", "Show settings"),
        "Show config manager" => pick("Показывать менеджер конфигурации", "Show config manager"),
        "Hide single sections" => pick("Скрывать одиночные разделы", "Hide single sections"),
        "Plugin collapsed default" => pick(
            "Плагины свёрнуты по умолчанию",
            "Plugin collapsed by default",
        ),
        // XUnity.AutoTranslator
        "Endpoint" => pick("Сервис перевода", "Translation service"),
        "FallbackEndpoint" => pick("Резервный сервис", "Fallback service"),
        "Language" => pick("Язык перевода", "Translation language"),
        "FromLanguage" => pick("Исходный язык", "Source language"),
        "Directory" => pick("Папка переводов", "Translations folder"),
        "OutputFile" => pick("Файл вывода", "Output file"),
        "SubstitutionFile" => pick("Файл подстановок", "Substitution file"),
        "PreprocessorsFile" => pick("Файл препроцессоров", "Preprocessors file"),
        "PostprocessorsFile" => pick("Файл постпроцессоров", "Postprocessors file"),
        "EnableIMGUI" => pick("Перевод IMGUI", "Translate IMGUI"),
        "EnableUGUI" => pick("Перевод UGUI", "Translate UGUI"),
        "EnableUIElements" => pick("Перевод UI Elements", "Translate UI Elements"),
        "EnableNGUI" => pick("Перевод NGUI", "Translate NGUI"),
        "EnableTextMeshPro" => pick("Перевод TextMeshPro", "Translate TextMeshPro"),
        "EnableTextMesh" => pick("Перевод TextMesh", "Translate TextMesh"),
        "EnableFairyGUI" => pick("Перевод FairyGUI", "Translate FairyGUI"),
        "MaxCharactersPerTranslation" => pick(
            "Максимум символов на перевод",
            "Max characters per translation",
        ),
        "IgnoreWhitespaceInDialogue" => pick(
            "Игнорировать пробелы в диалогах",
            "Ignore whitespace in dialogue",
        ),
        "MinDialogueChars" => pick("Минимум символов в диалоге", "Min dialogue characters"),
        "ForceSplitTextAfterCharacters" => pick(
            "Разбивать текст после N символов",
            "Force split text after characters",
        ),
        "CopyToClipboard" => pick("Копировать в буфер обмена", "Copy to clipboard"),
        "MaxClipboardCopyCharacters" => pick(
            "Максимум символов для копирования",
            "Max clipboard copy characters",
        ),
        "ClipboardDebounceTime" => pick("Задержка буфера обмена", "Clipboard debounce time"),
        "EnableUIResizing" => pick("Менять размер интерфейса", "Enable UI resizing"),
        "EnableBatching" => pick("Пакетный перевод", "Enable batching"),
        "UseStaticTranslations" => pick("Статические переводы", "Use static translations"),
        "OverrideFont" => pick("Заменить шрифт", "Override font"),
        "OverrideFontSize" => pick("Заменить размер шрифта", "Override font size"),
        "OverrideFontTextMeshPro" => {
            pick("Заменить шрифт TextMeshPro", "Override TextMeshPro font")
        }
        "FallbackFontTextMeshPro" => {
            pick("Резервный шрифт TextMeshPro", "Fallback TextMeshPro font")
        }
        "ResizeUILineSpacingScale" => {
            pick("Масштаб межстрочного интервала", "UI line spacing scale")
        }
        "ForceUIResizing" => pick("Принудительно менять размер UI", "Force UI resizing"),
        "IgnoreTextStartingWith" => pick(
            "Игнорировать текст, начинающийся с",
            "Ignore text starting with",
        ),
        "TextGetterCompatibilityMode" => pick(
            "Режим совместимости текста",
            "Text getter compatibility mode",
        ),
        "GameLogTextPaths" => pick("Пути текста в логе игры", "Game log text paths"),
        "RomajiPostProcessing" => pick("Постобработка ромадзи", "Romaji post-processing"),
        "TranslationPostProcessing" => {
            pick("Постобработка перевода", "Translation post-processing")
        }
        "RegexPostProcessing" => pick("Постобработка регулярками", "Regex post-processing"),
        "CacheRegexPatternResults" => {
            pick("Кэшировать результаты regex", "Cache regex pattern results")
        }
        "PersistRichTextMode" => pick("Режим сохранения Rich Text", "Persist rich text mode"),
        "CacheRegexLookups" => pick("Кэшировать поиск regex", "Cache regex lookups"),
        "CacheWhitespaceDifferences" => pick(
            "Кэшировать различия пробелов",
            "Cache whitespace differences",
        ),
        "GenerateStaticSubstitutionTranslations" => pick(
            "Создавать статические подстановки",
            "Generate static substitution translations",
        ),
        "GeneratePartialTranslations" => pick(
            "Создавать частичные переводы",
            "Generate partial translations",
        ),
        "EnableTranslationScoping" => {
            pick("Область видимости переводов", "Enable translation scoping")
        }
        "EnableSilentMode" => pick("Тихий режим", "Enable silent mode"),
        "BlacklistedIMGUIPlugins" => {
            pick("Заблокированные IMGUI-плагины", "Blacklisted IMGUI plugins")
        }
        "EnableTextPathLogging" => pick("Логировать пути текста", "Enable text path logging"),
        "OutputUntranslatableText" => {
            pick("Выводить непереводимый текст", "Output untranslatable text")
        }
        "IgnoreVirtualTextSetterCallingRules" => pick(
            "Игнорировать правила виртуального текста",
            "Ignore virtual text setter rules",
        ),
        "MaxTextParserRecursion" => pick("Глубина разбора текста", "Max text parser recursion"),
        "HtmlEntityPreprocessing" => pick("Обработка HTML-сущностей", "HTML entity preprocessing"),
        "HandleRichText" => pick("Обрабатывать Rich Text", "Handle rich text"),
        "EnableTranslationHelper" => pick("Помощник перевода", "Enable translation helper"),
        "ForceMonoModHooks" => pick("Принудительные хуки MonoMod", "Force MonoMod hooks"),
        "InitializeHarmonyDetourBridge" => pick(
            "Инициализировать HarmonyDetourBridge",
            "Initialize HarmonyDetourBridge",
        ),
        "RedirectedResourceDetectionStrategy" => {
            pick("Стратегия поиска ресурсов", "Redirected resource detection")
        }
        "OutputTooLongText" => pick("Выводить слишком длинный текст", "Output too long text"),
        "TemplateAllNumberAway" => pick("Шаблон для всех чисел", "Template all numbers away"),
        "ReloadTranslationsOnFileChange" => pick(
            "Перезагружать переводы при изменении",
            "Reload translations on file change",
        ),
        "DisableTextMeshProScrollInEffects" => pick(
            "Отключить эффекты TextMeshPro",
            "Disable TextMeshPro scroll effects",
        ),
        "CacheParsedTranslations" => pick(
            "Кэшировать разобранные переводы",
            "Cache parsed translations",
        ),
        "TextureDirectory" => pick("Папка текстур", "Texture directory"),
        "EnableTextureTranslation" => pick("Перевод текстур", "Enable texture translation"),
        "EnableTextureDumping" => pick("Выгрузка текстур", "Enable texture dumping"),
        "EnableTextureToggling" => pick("Переключение текстур", "Enable texture toggling"),
        "EnableTextureScanOnSceneLoad" => pick(
            "Сканировать текстуры при загрузке",
            "Scan textures on scene load",
        ),
        "EnableSpriteRendererHooking" => {
            pick("Перехват SpriteRenderer", "Enable SpriteRenderer hooking")
        }
        "LoadUnmodifiedTextures" => pick(
            "Загружать неизменённые текстуры",
            "Load unmodified textures",
        ),
        "DetectDuplicateTextureNames" => pick(
            "Искать дубликаты имён текстур",
            "Detect duplicate texture names",
        ),
        "DuplicateTextureNames" => pick("Дубликаты имён текстур", "Duplicate texture names"),
        "EnableLegacyTextureLoading" => {
            pick("Старая загрузка текстур", "Enable legacy texture loading")
        }
        "TextureHashGenerationStrategy" => {
            pick("Стратегия хэша текстур", "Texture hash generation")
        }
        "CacheTexturesInMemory" => pick("Кэшировать текстуры в памяти", "Cache textures in memory"),
        "EnableSpriteHooking" => pick("Перехват спрайтов", "Enable sprite hooking"),
        "PreferredStoragePath" => pick("Предпочитаемый путь хранения", "Preferred storage path"),
        "EnableTextAssetRedirector" => {
            pick("Перенаправление TextAsset", "Enable TextAsset redirector")
        }
        "LogAllLoadedResources" => {
            pick("Логировать загруженные ресурсы", "Log all loaded resources")
        }
        "EnableDumping" => pick("Включить выгрузку", "Enable dumping"),
        "CacheMetadataForAllFiles" => pick(
            "Кэшировать метаданные файлов",
            "Cache metadata for all files",
        ),
        "UserAgent" => pick("User-Agent", "User-Agent"),
        "DisableCertificateValidation" => pick(
            "Отключить проверку сертификата",
            "Disable certificate validation",
        ),
        "Width" => pick("Ширина", "Width"),
        "Height" => pick("Высота", "Height"),
        "EnabledTranslators" => pick("Включённые переводчики", "Enabled translators"),
        "EnableConsole" => pick("Консоль", "Enable console"),
        "Enable" => pick("Включить", "Enable"),
        "Tag" => pick("Тег", "Tag"),
        "BaiduAppId" => pick("Baidu App ID", "Baidu App ID"),
        "BaiduAppSecret" => pick("Baidu App Secret", "Baidu App Secret"),
        "DelaySeconds" => pick("Задержка (сек)", "Delay (seconds)"),
        "OcpApimSubscriptionKey" => pick("Ключ Azure", "Azure subscription key"),
        "Url" => pick("Адрес", "URL"),
        "EnableShortDelay" => pick("Короткая задержка", "Enable short delay"),
        "DisableSpamChecks" => pick("Отключить проверку спама", "Disable spam checks"),
        "ExecutableLocation" => pick("Путь к программе", "Executable location"),
        "MinDelaySeconds" => pick("Минимальная задержка (сек)", "Min delay (seconds)"),
        "MaxDelaySeconds" => pick("Максимальная задержка (сек)", "Max delay (seconds)"),
        "ApiKey" => pick("API-ключ", "API key"),
        "Free" => pick("Бесплатный режим", "Free mode"),
        "InstallationPath" => pick("Путь установки", "Installation path"),
        "ServiceUrl" => pick("Адрес сервиса", "Service URL"),
        "RPCID" => pick("RPC ID", "RPC ID"),
        "VERSION" => pick("Версия", "Version"),
        "UseSimplest" => pick("Простой режим", "Use simplest"),
        "GoogleAPIKey" => pick("Google API-ключ", "Google API key"),
        "LingoCloudToken" => pick("Токен LingoCloud", "LingoCloud token"),
        "Key" => pick("Ключ", "Key"),
        "YandexAPIKey" => pick("Яндекс API-ключ", "Yandex API key"),
        _ => return key.to_string(),
    };
    label.to_string()
}

/// Русские названия разделов известных конфигов.
fn section_label(section: &str) -> String {
    let label = match section {
        "Caching" => pick("Кэширование", "Caching"),
        "Chainloader" => pick("Загрузчик", "Chainloader"),
        "Harmony.Logger" => pick("Лог Harmony", "Harmony logger"),
        "Logging" => pick("Логирование", "Logging"),
        "Logging.Console" => pick("Консоль", "Console"),
        "Logging.Disk" => pick("Лог на диск", "Disk log"),
        "Preloader" => pick("Прелоадер", "Preloader"),
        "Preloader.Entrypoint" => pick("Точка входа", "Entrypoint"),
        "Unload Throttling" => pick("Ограничение выгрузки", "Unload throttling"),
        "Filtering" => pick("Фильтрация", "Filtering"),
        "General" => pick("Общие", "General"),
        "Service" => pick("Сервис", "Service"),
        "Files" => pick("Файлы", "Files"),
        "TextFrameworks" => pick("Текстовые фреймворки", "Text frameworks"),
        "Behaviour" => pick("Поведение", "Behaviour"),
        "Texture" => pick("Текстуры", "Texture"),
        "ResourceRedirector" => pick("Перенаправление ресурсов", "Resource redirector"),
        "Http" => pick("HTTP", "HTTP"),
        "TranslationAggregator" => pick("Агрегатор переводов", "Translation aggregator"),
        "Debug" => pick("Отладка", "Debug"),
        "Migrations" => pick("Миграции", "Migrations"),
        "Custom" => pick("Свои настройки", "Custom"),
        "Baidu" => pick("Baidu", "Baidu"),
        "BingLegitimate" => pick("Bing", "Bing"),
        "DeepL" => pick("DeepL", "DeepL"),
        "DeepLLegitimate" => pick("DeepL", "DeepL"),
        "ezTrans" => pick("ezTrans", "ezTrans"),
        "Google" => pick("Google", "Google"),
        "GoogleV2" => pick("Google V2", "Google V2"),
        "GoogleLegitimate" => pick("Google", "Google"),
        "LecPowerTranslator15" => pick("LEC Power Translator", "LEC Power Translator"),
        "LingoCloud" => pick("LingoCloud", "LingoCloud"),
        "Watson" => pick("Watson", "Watson"),
        "Yandex" => pick("Яндекс", "Yandex"),
        _ => return section.to_string(),
    };
    label.to_string()
}

/// Русские описания параметров из комментариев известных конфигов.
fn note_label(note: &str) -> &str {
    let ru = match note {
        // BepInEx
        "Enable/disable assembly metadata cache" => {
            "Включает или отключает кэш метаданных сборок"
        }
        "Enabling this will speed up discovery of plugins and patchers by caching the metadata of all types BepInEx discovers." => {
            "Ускоряет поиск плагинов и патчеров: метаданные всех найденных типов кэшируются."
        }
        "If enabled, hides BepInEx Manager GameObject from Unity." => {
            "Скрывает объект BepInEx Manager от Unity."
        }
        "This can fix loading issues in some games that attempt to prevent BepInEx from being loaded." => {
            "Помогает, если игра мешает загрузке BepInEx."
        }
        "Use this only if you know what this option means, as it can affect functionality of some older plugins." => {
            "Включайте, только если понимаете последствия: это может нарушить работу старых плагинов."
        }
        "Specifies which Harmony log channels to listen to." => {
            "Какие каналы логов Harmony слушать."
        }
        "NOTE: IL channel dumps the whole patch methods, use only when needed!" => {
            "Канал IL выводит целые методы патчей — включайте только при необходимости!"
        }
        "Multiple values can be set at the same time by separating them with , (e.g. Debug, Warning)" => {
            "Можно указать несколько значений через запятую, например Debug, Warning."
        }
        "Enables showing unity log messages in the BepInEx logging system." => {
            "Показывает сообщения лога Unity в журнале BepInEx."
        }
        "If enabled, writes Standard Output messages to Unity log" => {
            "Записывает сообщения стандартного вывода в лог Unity."
        }
        "NOTE: By default, Unity does so automatically. Only use this option if no console messages are visible in Unity log" => {
            "Обычно Unity делает это сама. Включайте, только если сообщений консоли нет в логе Unity."
        }
        "Enables showing a console for log output." => "Показывает консоль с логами.",
        "If enabled, will prevent closing the console (either by deleting the close button or in other platform-specific way)." => {
            "Запрещает закрывать консоль."
        }
        "If true, console is set to the Shift-JIS encoding, otherwise UTF-8 encoding." => {
            "Кодировка консоли: Shift-JIS вместо UTF-8."
        }
        "Hints console manager on what handle to assign as StandardOut. Possible values:" => {
            "Какой поток назначать стандартным выводом. Варианты:"
        }
        "Auto - lets BepInEx decide how to redirect console output" => {
            "Auto — BepInEx сам выбирает способ перенаправления вывода."
        }
        "ConsoleOut - prefer redirecting to console output; if possible, closes original standard output" => {
            "ConsoleOut — вывод в консоль; исходный стандартный вывод закрывается, если возможно."
        }
        "StandardOut - prefer redirecting to standard output; if possible, closes console out" => {
            "StandardOut — вывод в стандартный поток; консоль закрывается, если возможно."
        }
        "Which log levels to show in the console output." => {
            "Какие уровни логов показывать в консоли."
        }
        "Include unity log messages in log file output." => {
            "Добавлять сообщения лога Unity в файл журнала."
        }
        "Appends to the log file instead of overwriting, on game startup." => {
            "Дописывать журнал при запуске игры, а не перезаписывать."
        }
        "Enables writing log messages to disk." => "Записывать журнал на диск.",
        "Which log leves are saved to the disk log output." => {
            "Какие уровни логов сохранять в файл."
        }
        "Enables or disables runtime patches." => {
            "Включает или отключает патчи во время выполнения."
        }
        "This should always be true, unless you cannot start the game due to a Harmony related issue (such as running .NET Standard runtime) or you know what you're doing." => {
            "Обычно должно быть включено. Отключайте только при сбоях Harmony или если понимаете последствия."
        }
        "Specifies which MonoMod backend to use for Harmony patches. Auto uses the best available backend." => {
            "Бэкенд MonoMod для патчей Harmony. Auto выбирает лучший доступный."
        }
        "This setting should only be used for development purposes (e.g. debugging in dnSpy). Other code might override this setting." => {
            "Только для разработки и отладки в dnSpy: другие плагины могут переопределить значение."
        }
        "If enabled, BepInEx will save patched assemblies into BepInEx/DumpedAssemblies." => {
            "Сохраняет пропатченные сборки в BepInEx/DumpedAssemblies."
        }
        "This can be used by developers to inspect and debug preloader patchers." => {
            "Нужно разработчикам для проверки и отладки патчеров прелоадера."
        }
        "If enabled, BepInEx will load patched assemblies from BepInEx/DumpedAssemblies instead of memory." => {
            "Загружает пропатченные сборки из BepInEx/DumpedAssemblies вместо памяти."
        }
        "This can be used to be able to load patched assemblies into debuggers like dnSpy." => {
            "Позволяет открывать пропатченные сборки в отладчиках вроде dnSpy."
        }
        "If set to true, will override DumpAssemblies." => {
            "При включении переопределяет DumpAssemblies."
        }
        "If enabled, BepInEx will call Debugger.Break() once before loading patched assemblies." => {
            "Вызывает Debugger.Break() перед загрузкой пропатченных сборок."
        }
        "This can be used with debuggers like dnSpy to install breakpoints into patched assemblies before they are loaded." => {
            "Позволяет ставить точки останова в dnSpy до загрузки сборок."
        }
        "The local filename of the assembly to target." => "Имя целевой сборки.",
        "The name of the type in the entrypoint assembly to search for the entrypoint method." => {
            "Имя типа в сборке точки входа, где ищется метод входа."
        }
        "The name of the method in the specified entrypoint assembly and type to hook and load Chainloader from." => {
            "Имя метода, с которого загружается Chainloader."
        }
        // Resource Unload Optimizations
        "Disables all resource unloading. Requires large amounts of RAM or will likely crash your game. NOT RECOMMENDED FOR NORMAL USE" => {
            "Полностью отключает выгрузку ресурсов. Требует много ОЗУ, иначе игра, скорее всего, упадёт. Не рекомендуется для обычной игры."
        }
        "Use more memory (if available) in order to load the game faster and reduce random stutter." => {
            "Использовать больше памяти для ускорения загрузки и уменьшения рывков."
        }
        "Minimum amount of memory to be used before resource unloading will run." => {
            "Минимальный объём используемой памяти до запуска выгрузки ресурсов."
        }
        // Configuration Manager
        "The shortcut used to toggle the config manager window on and off." => {
            "Горячая клавиша для открытия и закрытия менеджера конфигурации."
        }
        "The key can be overridden by a game-specific plugin if necessary, in that case this setting is ignored." => {
            "Плагин игры может переопределить клавишу — тогда эта настройка игнорируется."
        }
        "Show section title for plugins with only one section" => {
            "Показывать заголовок раздела у плагинов с одним разделом."
        }
        "If set to true plugins will be collapsed when opening the configuration manager window" => {
            "Сворачивать плагины при открытии менеджера конфигурации."
        }
        _ => return note,
    };
    if is_ru() {
        ru
    } else {
        note
    }
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
        ui.label(RichText::new(note_label(note)).size(10.5).color(p.muted));
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

        let Some(rel) = crate::util::safe_relative(&cfg) else {
            self.toast(
                fill(
                    &pick(
                        "Некорректный путь к настройкам: {path}",
                        "Invalid settings path: {path}",
                    ),
                    &[("path", &cfg)],
                ),
                false,
            );
            return;
        };
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

        match Editor::open(name, &rel.to_string_lossy(), &path) {
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
        let dir = std::env::temp_dir().join(format!("yanlan_cfg_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Settings.txt");
        std::fs::write(&path, "UIColor:0.62,0,0,1\nPosingMenu:F9\n").unwrap();
        let mut editor = Editor::open("test", "Settings.txt", &path).unwrap();
        editor.settings[0].value = "0.5,0,0,1".to_string();
        editor.save().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text, "UIColor:0.5,0,0,1\nPosingMenu:F9\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_keeps_bepinex_spacing() {
        let dir = std::env::temp_dir().join(format!("yanlan_cfg_eq_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("BepInEx.cfg");
        std::fs::write(&path, "Enabled = true\n").unwrap();
        let mut editor = Editor::open("test", "BepInEx.cfg", &path).unwrap();
        editor.settings[0].value = "false".to_string();
        editor.save().unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "Enabled = false\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn known_notes_are_translated() {
        crate::i18n::set(crate::i18n::Lang::Ru);
        assert_eq!(
            note_label("Enables showing a console for log output."),
            "Показывает консоль с логами."
        );
        assert_eq!(note_label("Some plugin note"), "Some plugin note");
    }
}
