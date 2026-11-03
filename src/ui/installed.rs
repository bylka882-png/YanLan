//! Вкладка «Установлено»: моды, установленные в сборки.

use egui::{RichText, Vec2};

use crate::app::{App, ConfirmAction, Tab};
use crate::i18n::{fill, pick};
use crate::jobs;
use crate::library::{self, InstalledMod};
use crate::theme::Palette;
use crate::ui::widgets as w;
use crate::util::{format_ts, trim_to};

enum InstalledAction {
    UninstallMod {
        key: String,
        name: String,
        game_id: String,
    },
    InstallLocalFile,
    GoBuilds,
}

impl App {
    pub fn installed_view(&mut self, ui: &mut egui::Ui) {
        let p = self.p;
        let games = self.games.clone();
        let mods = self.library.mods.clone();
        let current = self.current_game().map(|g| g.display_name());
        let mut action: Option<InstalledAction> = None;

        egui::ScrollArea::vertical()
            .id_salt("installed_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.allocate_ui_with_layout(
                    Vec2::new(ui.available_width(), 32.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.label(
                            RichText::new(pick("МОДЫ", "MODS"))
                                .size(11.0)
                                .strong()
                                .color(p.muted),
                        );
                        ui.label(
                            RichText::new(w::count_label(mods.len()))
                                .size(11.0)
                                .color(p.muted),
                        );
                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                if w::soft(
                                    ui,
                                    pick(
                                        "+  Установить из файла…",
                                        "+  Install from file…",
                                    ),
                                    &p,
                                    !games.is_empty(),
                                )
                                .clicked()
                                {
                                    action = Some(InstalledAction::InstallLocalFile);
                                }
                            },
                        );
                    },
                );
                // Название активной сборки — отдельной строкой: длинное имя
                // больше не сдвигает и не обрезает кнопку установки.
                if let Some(name) = &current {
                    ui.add_space(2.0);
                    ui.add(
                        egui::Label::new(
                            RichText::new(fill(
                                &pick("активная сборка: {name}", "active build: {name}"),
                                &[("name", name)],
                            ))
                            .size(11.0)
                            .color(p.accent),
                        )
                        .truncate(),
                    );
                }
                ui.add_space(10.0);

                if games.is_empty() {
                    empty_card(
                        ui,
                        &p,
                        pick("Сначала создайте сборку", "Create a build first"),
                        pick(
                            "Моды устанавливаются в конкретную сборку. Создайте её на вкладке \
                             «Сборки» — после этого сюда можно будет ставить архивы и моды из каталога.",
                            "Mods are installed into a specific build. Create one on the \
                             “Builds” tab — after that you can install archives and catalog mods here.",
                        ),
                    );
                    ui.add_space(10.0);
                    ui.vertical_centered(|ui| {
                        if w::primary(ui, pick("Перейти к сборкам", "Go to builds"), &p, true)
                            .clicked()
                        {
                            action = Some(InstalledAction::GoBuilds);
                        }
                    });
                } else if mods.is_empty() {
                    empty_card(
                        ui,
                        &p,
                        pick("Моды не установлены", "No mods installed"),
                        pick(
                            "Каталог модов ещё в разработке. Пока можно установить архив \
                             zip/rar/7z вручную — заменённые файлы игры сохранятся в .yanlan \
                             и восстановятся при удалении мода.",
                            "The mod catalog is still in development. For now you can install a \
                             zip/rar/7z archive manually — replaced game files are saved in .yanlan \
                             and restored when the mod is removed.",
                        ),
                    );
                } else {
                    // Активная сборка — первой в списке, чтобы её моды были
                    // видны без прокрутки.
                    let selected = self.selected_game.clone();
                    let mut sections: Vec<&library::GameInstall> = games
                        .iter()
                        .filter(|g| mods.iter().any(|m| m.game_id == g.meta.id))
                        .collect();
                    sections.sort_by_key(|g| {
                        if selected.as_deref() == Some(g.meta.id.as_str()) {
                            0
                        } else {
                            1
                        }
                    });
                    let mut shown = false;
                    for game in sections {
                        let items: Vec<&InstalledMod> =
                            mods.iter().filter(|m| m.game_id == game.meta.id).collect();
                        if items.is_empty() {
                            continue;
                        }
                        shown = true;
                        let active = selected.as_deref() == Some(game.meta.id.as_str());
                        ui.add_space(2.0);
                        ui.horizontal(|ui| {
                            let name = game.display_name();
                            ui.add(
                                egui::Label::new(
                                    RichText::new(trim_to(&name, 48))
                                        .size(13.0)
                                        .strong()
                                        .color(p.text),
                                )
                                .truncate(),
                            )
                            .on_hover_text(&name);
                            if !game.meta.version.is_empty()
                                && !name.contains(&game.meta.version)
                            {
                                w::tag(ui, &game.meta.version, p.accent, &p);
                            }
                            if active {
                                w::tag(ui, pick("выбрана", "selected"), p.green, &p);
                            }
                            ui.label(
                                RichText::new(fill(
                                    &pick(
                                        "· {count} в этой сборке",
                                        "· {count} in this build",
                                    ),
                                    &[("count", &w::count_label(items.len()))],
                                ))
                                .size(11.0)
                                .color(p.muted),
                            );
                        });
                        ui.add_space(4.0);
                        for item in items {
                            if let Some(act) = mod_row(ui, &p, item) {
                                action = Some(act);
                            }
                            ui.add_space(4.0);
                        }
                        ui.add_space(4.0);
                    }
                    if !shown {
                        empty_card(
                            ui,
                            &p,
                            pick("Моды не установлены", "No mods installed"),
                            pick(
                                "Записи о модах есть в библиотеке, но их сборки не найдены. \
                                 Убедитесь, что папки игр не удалены вручную.",
                                "The library has mod records, but their builds were not found. \
                                 Make sure the game folders were not deleted manually.",
                            ),
                        );
                    }
                }

                ui.add_space(16.0);
                w::divider(ui, &p);
                ui.add_space(8.0);
                ui.label(
                    RichText::new(pick(
                        "Удаление мода возвращает файлы игры в состояние до установки. \
                         Сами сборки и резервные копии хранятся в папке .yanlan внутри игры.",
                        "Removing a mod restores the game files to their state before installation. \
                         Builds and backups are kept in the .yanlan folder inside the game.",
                    ))
                    .size(11.0)
                    .color(p.muted),
                );
            });

        if let Some(action) = action {
            self.handle_installed_action(action);
        }
    }

    fn handle_installed_action(&mut self, action: InstalledAction) {
        match action {
            InstalledAction::UninstallMod { key, name, game_id } => {
                let message = fill(
                    &pick(
                        "Удалить мод «{name}» и восстановить заменённые файлы?",
                        "Remove the mod “{name}” and restore the replaced files?",
                    ),
                    &[("name", &name)],
                );
                self.confirm = Some((message, ConfirmAction::UninstallMod { key, name, game_id }));
            }
            InstalledAction::GoBuilds => {
                self.tab = Tab::Builds;
            }
            InstalledAction::InstallLocalFile => {
                let Some(game) = self.current_game().cloned() else {
                    self.toast(
                        pick("Сначала выберите сборку", "Select a build first"),
                        false,
                    );
                    return;
                };
                let Some(path) = rfd::FileDialog::new()
                    .add_filter(pick("Архивы", "Archives"), &["zip", "rar", "7z"])
                    .set_title(pick(
                        "Архив с модом или патчем",
                        "Archive with a mod or patch",
                    ))
                    .set_directory(self.cfg.downloads_dir())
                    .pick_file()
                else {
                    return;
                };
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| pick("мод", "mod").to_string());
                let c = jobs::Ctx2 {
                    ctx: self.ctx.clone(),
                    tx: self.tx.clone(),
                };
                let progress =
                    jobs::install_local_archive(c, self.cfg.clone(), game.clone(), path.clone());
                self.start_parallel_job(
                    fill(
                        &pick("Установка {name}", "Installing {name}"),
                        &[("name", &name)],
                    ),
                    progress,
                );
                self.log(fill(
                    &pick(
                        "Установка локального архива {path} в {build}",
                        "Installing local archive {path} into {build}",
                    ),
                    &[
                        ("path", &path.display().to_string()),
                        ("build", &game.display_name()),
                    ],
                ));
            }
        }
    }
}

fn empty_card(ui: &mut egui::Ui, p: &Palette, title: &str, text: &str) {
    egui::Frame::none()
        .fill(p.panel)
        .stroke(egui::Stroke::new(1.0_f32, p.border))
        .rounding(14.0)
        .inner_margin(egui::Margin::same(20.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).size(15.0).strong().color(p.text));
            ui.add_space(4.0);
            ui.label(RichText::new(text).size(12.0).color(p.muted));
        });
}

fn mod_row(ui: &mut egui::Ui, p: &Palette, item: &InstalledMod) -> Option<InstalledAction> {
    let mut action = None;
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 56.0), egui::Sense::hover());
    ui.painter()
        .rect(rect, 10.0, p.panel, egui::Stroke::new(1.0_f32, p.border));

    // Кнопка «Удалить» — ровно по центру вертикали карточки.
    let btn_w = 86.0;
    let btn_h = 34.0;
    let btn_rect = egui::Rect::from_center_size(
        egui::pos2(rect.right() - 14.0 - btn_w / 2.0, rect.center().y),
        Vec2::new(btn_w, btn_h),
    );
    // Текст занимает всё слева от кнопки, поэтому длинные строки обрезаются,
    // не заезжая под неё.
    let text_rect = egui::Rect::from_min_max(
        rect.min + Vec2::new(14.0, 5.0),
        egui::pos2(btn_rect.left() - 12.0, rect.max.y - 5.0),
    );
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(text_rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
        |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.horizontal(|ui| {
                ui.add(
                    egui::Label::new(
                        RichText::new(trim_to(&item.name, 40))
                            .size(13.0)
                            .strong()
                            .color(p.text),
                    )
                    .truncate(),
                );
                w::tag_small(
                    ui,
                    library::kind_label(&item.category),
                    library::kind_color(&item.category),
                    p,
                );
            });
            let mut meta = String::new();
            if !item.author.is_empty() {
                meta.push_str(&item.author);
                meta.push_str("  ·  ");
            }
            meta.push_str(&fill(
                &pick("файлов: {count}", "files: {count}"),
                &[("count", &item.files.len().to_string())],
            ));
            if item.installed_at > 0 {
                meta.push_str(&format!("  ·  {}", format_ts(item.installed_at)));
            }
            ui.add(egui::Label::new(RichText::new(meta).size(10.5).color(p.muted)).truncate());
        },
    );

    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(btn_rect)
            .layout(egui::Layout::centered_and_justified(
                egui::Direction::LeftToRight,
            )),
        |ui| {
            if w::danger(ui, pick("Удалить", "Remove"), p, true).clicked() {
                action = Some(InstalledAction::UninstallMod {
                    key: item.key.clone(),
                    name: item.name.clone(),
                    game_id: item.game_id.clone(),
                });
            }
        },
    );

    action
}
