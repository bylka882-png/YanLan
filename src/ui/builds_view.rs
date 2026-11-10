//! Вкладка «Сборки»: отдельные копии игры и создание новых.

use egui::{Align, Layout, RichText, Sense, Stroke, Vec2};

use crate::app::{App, ConfirmAction};
use crate::config::SortBy;
use crate::i18n::{fill, pick};
use crate::jobs;
use crate::library::GameInstall;
use crate::theme::Palette;
use crate::ui::widgets as w;
use crate::util::{format_date, format_ts, human_size, trim_to};

enum BuildAction {
    Play(String),
    OpenFolder(String),
    Select(String),
    Rename(String),
    LaunchOptions(String),
    Delete(String),
}

impl App {
    pub fn builds_view(&mut self, ui: &mut egui::Ui) {
        let p = self.p;
        let busy = self.progress.is_some();
        let mut games = self.games.clone();
        sort_games(&mut games, self.cfg.builds_sort);
        let selected = self.selected_game.clone();
        let mut action: Option<BuildAction> = None;
        let mut create = false;

        // Шапка списка не прокручивается вместе с карточками и не задевает полосу
        // прокрутки: так кнопка создания всегда целиком видна.
        ui.allocate_ui_with_layout(
            Vec2::new(ui.available_width(), 34.0),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.label(
                    RichText::new(pick("СБОРКИ", "BUILDS"))
                        .size(11.0)
                        .strong()
                        .color(p.muted),
                );
                let total_size: u64 = games.iter().map(|g| g.size).sum();
                ui.label(
                    RichText::new(format!(
                        "{} · {}",
                        w::count_label(games.len()),
                        human_size(total_size)
                    ))
                    .size(11.0)
                    .color(p.muted),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if w::primary(ui, pick("+  Создать сборку", "+  Create build"), &p, !busy)
                        .clicked()
                    {
                        create = true;
                    }
                    ui.add_space(10.0);
                    // ComboBox берёт высоту из interact_size, поэтому задаём её
                    // равной высоте кнопки — тогда центры совпадают пиксель в пиксель.
                    ui.spacing_mut().interact_size.y = 34.0;
                    egui::ComboBox::from_id_salt("builds_sort")
                        .width(132.0)
                        .selected_text(RichText::new(self.cfg.builds_sort.label()).size(12.5))
                        .show_ui(ui, |ui| {
                            for option in [SortBy::Version, SortBy::Size, SortBy::Name] {
                                let chosen = self.cfg.builds_sort == option;
                                if ui
                                    .selectable_label(chosen, option.label())
                                    .on_hover_text(option.hint())
                                    .clicked()
                                {
                                    self.cfg.builds_sort = option;
                                    // Сохраняем сразу: выбор не должен теряться
                                    // при аварийном закрытии окна.
                                    self.cfg.save();
                                }
                            }
                        })
                        .response
                        .on_hover_text(pick("Сортировка списка сборок", "Build list sorting"));
                    ui.label(
                        RichText::new(pick("Сортировка", "Sort"))
                            .size(11.0)
                            .color(p.muted),
                    );
                });
            },
        );
        ui.add_space(12.0);

        egui::ScrollArea::vertical()
            .id_salt("builds_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if games.is_empty() {
                    empty_state(ui, &p, busy, &mut create);
                } else {
                    for game in &games {
                        let is_selected = selected.as_deref() == Some(game.meta.id.as_str());
                        ui.spacing_mut().item_spacing.y = 10.0;
                        if let Some(act) = build_card(ui, &p, game, is_selected, busy) {
                            action = Some(act);
                        }
                    }

                    ui.add_space(8.0);
                    w::divider(ui, &p);
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(pick(
                            "Каждая сборка хранится в отдельной папке внутри games/. \
                             Моды, установленные в одну сборку, не затрагивают остальные.",
                            "Each build lives in its own folder inside games/. \
                             Mods installed into one build do not affect the others.",
                        ))
                        .size(11.0)
                        .color(p.muted),
                    );
                }
            });

        if create {
            self.open_create();
        }
        if let Some(action) = action {
            self.handle_build_action(action);
        }
    }

    fn handle_build_action(&mut self, action: BuildAction) {
        match action {
            BuildAction::Play(id) => {
                self.selected_game = Some(id);
                self.play_game();
            }
            BuildAction::Select(id) => {
                self.selected_game = Some(id);
                if let Some(game) = self.current_game() {
                    let name = game.display_name();
                    self.log(fill(
                        &pick("Выбрана сборка {name}", "Build {name} selected"),
                        &[("name", &name)],
                    ));
                }
            }
            BuildAction::Rename(id) => {
                if let Some(game) = self.games.iter().find(|g| g.meta.id == id) {
                    let name = game.display_name();
                    self.rename = Some((id.clone(), name));
                    self.rename_focus = true;
                }
            }
            BuildAction::LaunchOptions(id) => {
                if let Some(game) = self.games.iter().find(|g| g.meta.id == id) {
                    self.launch_opts = Some((id.clone(), game.meta.args.clone()));
                    self.launch_opts_focus = true;
                }
            }
            BuildAction::OpenFolder(id) => {
                let path = self.cfg.games_dir().join(id);
                if path.exists() {
                    crate::runner::reveal(&path);
                } else {
                    self.toast(
                        pick("Папка сборки не найдена", "Build folder not found"),
                        false,
                    );
                }
            }
            BuildAction::Delete(id) => {
                let Some(game) = self.games.iter().find(|g| g.meta.id == id).cloned() else {
                    self.toast(pick("Сборка не найдена", "Build not found"), false);
                    return;
                };
                let message = fill(
                    &pick(
                        "Удалить сборку «{name}»? Вместе с ней будут удалены установленные в неё моды.",
                        "Delete the build “{name}”? Mods installed into it will be deleted as well.",
                    ),
                    &[("name", &game.display_name())],
                );
                if self.cfg.confirm_delete {
                    self.confirm = Some((message, ConfirmAction::DeleteGame(game)));
                } else {
                    self.delete_game_now(game);
                }
            }
        }
    }

    /// Удаляет сборку сразу, без диалога подтверждения.
    pub(crate) fn delete_game_now(&mut self, game: GameInstall) {
        let c = jobs::Ctx2 {
            ctx: self.ctx.clone(),
            tx: self.tx.clone(),
        };
        let name = game.display_name();
        let progress = jobs::delete_game(c, self.cfg.clone(), game);
        // Удаление может идти параллельно скачиванию другой сборки.
        self.start_parallel_job(
            fill(
                &pick("Удаление {name}", "Deleting {name}"),
                &[("name", &name)],
            ),
            progress,
        );
        self.log(fill(
            &pick("Удаление сборки {name}", "Deleting build {name}"),
            &[("name", &name)],
        ));
    }
}

fn sort_games(games: &mut [GameInstall], by: SortBy) {
    match by {
        SortBy::Version => games.sort_by(|a, b| {
            b.meta
                .date
                .cmp(&a.meta.date)
                .then_with(|| b.meta.version.cmp(&a.meta.version))
                .then_with(|| b.meta.installed_at.cmp(&a.meta.installed_at))
                .then_with(|| a.display_name().cmp(&b.display_name()))
        }),
        SortBy::Size => games.sort_by(|a, b| {
            b.size
                .cmp(&a.size)
                .then_with(|| a.display_name().cmp(&b.display_name()))
        }),
        SortBy::Name => games.sort_by(|a, b| {
            a.display_name()
                .to_lowercase()
                .cmp(&b.display_name().to_lowercase())
        }),
    }
}

fn empty_state(ui: &mut egui::Ui, p: &Palette, busy: bool, create: &mut bool) {
    ui.add_space(26.0);
    egui::Frame::none()
        .fill(p.card)
        .stroke(egui::Stroke::new(1.0_f32, p.border))
        .rounding(16.0)
        .inner_margin(egui::Margin::same(30.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new("♥")
                        .size(46.0)
                        .color(p.accent.gamma_multiply(0.7)),
                );
                ui.add_space(6.0);
                ui.label(
                    RichText::new(pick("Сборок пока нет", "No builds yet"))
                        .size(19.0)
                        .strong()
                        .color(p.text),
                );
                ui.add_space(6.0);
                ui.label(
                    RichText::new(pick(
                        "Создайте сборку: выберите любую версию игры с 2014 по 2026 год \
                         и задайте название — например, «Для модов» или «Чистая 2025».",
                        "Create a build: choose any game version from 2014 to 2026 \
                         and give it a name — for example, “For mods” or “Clean 2025”.",
                    ))
                    .size(12.5)
                    .color(p.muted),
                );
                ui.add_space(14.0);
                if w::primary(ui, pick("+  Создать сборку", "+  Create build"), p, !busy).clicked()
                {
                    *create = true;
                }
            });
        });
}

fn build_card(
    ui: &mut egui::Ui,
    p: &Palette,
    game: &GameInstall,
    selected: bool,
    _busy: bool,
) -> Option<BuildAction> {
    let mut action = None;
    let width = ui.available_width();
    // Фон рисуем сами: клик по любому месту карточки выбирает сборку, а кнопки
    // внутри добавляются позже и поэтому получают клики первыми.
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 118.0), Sense::click());
    // Подсвечивается только выбранная сборка. Наведение на карточку ничего не
    // меняет: клик по ней и так выбирает сборку, а лишняя подсветка мешала.
    let bg = p.card;
    let border = if selected { p.accent } else { p.border };
    ui.painter()
        .rect(rect, 14.0, bg, Stroke::new(1.0_f32, border));
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if response.clicked() {
        action = Some(BuildAction::Select(game.meta.id.clone()));
    }

    let inner = rect.shrink(16.0);
    // Важно: new_child, а не scope_builder — scope сдвигает курсор родителя по
    // фактическому содержимому, из-за чего карточки наезжали друг на друга.
    let mut ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(Layout::top_down(Align::Min)),
    );
    {
        // Верхняя строка: название и теги слева, кнопки справа. Метаданные
        // и путь идут ниже во всю ширину карточки, поэтому длинные строки
        // больше не упираются в кнопки и не переносятся на вторую строку.
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.allocate_ui_with_layout(
            Vec2::new(ui.available_width(), 36.0),
            Layout::left_to_right(Align::Center),
            |ui| {
                let reserve = 404.0;
                ui.allocate_ui_with_layout(
                    Vec2::new((ui.available_width() - reserve).max(200.0), 36.0),
                    Layout::left_to_right(Align::Center),
                    |ui| {
                        let name = game.display_name();
                        ui.add(
                            egui::Label::new(
                                RichText::new(&name).size(16.0).strong().color(p.text),
                            )
                            .truncate(),
                        )
                        .on_hover_text(&name);
                        if !game.meta.version.is_empty() && !name.contains(&game.meta.version) {
                            w::tag(ui, &game.meta.version, p.accent, p);
                        }
                        if selected {
                            w::tag(ui, pick("выбрана", "selected"), p.green, p);
                        }
                        if game.mods > 0 {
                            w::tag(
                                ui,
                                &fill(
                                    &pick("модов: {count}", "mods: {count}"),
                                    &[("count", &game.mods.to_string())],
                                ),
                                p.green,
                                p,
                            );
                        }
                        if game.exe.is_none() {
                            w::tag(ui, pick("нет .exe", "no .exe"), p.red, p);
                        }
                    },
                );

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if w::danger(ui, pick("Удалить", "Delete"), p, true).clicked() {
                        action = Some(BuildAction::Delete(game.meta.id.clone()));
                    }
                    if w::soft(ui, pick("Папка", "Folder"), p, true).clicked() {
                        action = Some(BuildAction::OpenFolder(game.meta.id.clone()));
                    }
                    // Играть можно даже во время скачивания другой сборки:
                    // установка идёт в отдельную папку и не мешает игре.
                    if w::primary(ui, pick("▶  Играть", "▶  Play"), p, game.exe.is_some()).clicked()
                    {
                        action = Some(BuildAction::Play(game.meta.id.clone()));
                    }
                    let has_args = !game.meta.args.trim().is_empty();
                    let opts = w::icon(ui, "⚙", p, has_args, true);
                    let hint = if has_args {
                        fill(
                            &pick("Параметры запуска: {args}", "Launch options: {args}"),
                            &[("args", &game.meta.args)],
                        )
                    } else {
                        pick("Параметры запуска", "Launch options").to_string()
                    };
                    if opts.on_hover_text(hint).clicked() {
                        action = Some(BuildAction::LaunchOptions(game.meta.id.clone()));
                    }
                    // Переименовать можно любую сборку, а не только безымянную.
                    if w::soft(ui, pick("✎  Имя", "✎  Name"), p, true).clicked() {
                        action = Some(BuildAction::Rename(game.meta.id.clone()));
                    }
                });
            },
        );

        ui.add_space(2.0);

        let mut meta = String::new();
        if let Some(date) = &game.meta.date {
            meta.push_str(&format_date(date));
            meta.push_str("  ·  ");
        }
        meta.push_str(&human_size(game.size));
        if let Some(unity) = game.unity_label() {
            meta.push_str("  ·  ");
            meta.push_str(&unity);
        }
        if !game.meta.source.is_empty() {
            meta.push_str("  ·  ");
            meta.push_str(&crate::builds::source_label(&game.meta.source));
        }
        if game.meta.installed_at > 0 {
            meta.push_str(&fill(
                &pick("  ·  создано {date}", "  ·  created {date}"),
                &[("date", &format_ts(game.meta.installed_at))],
            ));
        }
        ui.add(
            egui::Label::new(RichText::new(trim_to(&meta, 180)).size(11.5).color(p.muted))
                .truncate(),
        )
        .on_hover_text(meta);

        let path = game.path.display().to_string();
        ui.add(
            egui::Label::new(RichText::new(trim_to(&path, 140)).size(10.5).color(p.muted))
                .truncate(),
        )
        .on_hover_text(path);
    }
    action
}
