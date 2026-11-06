//! Вкладка «Настройки»: язык, папки, тема, обслуживание и журнал.

use egui::{RichText, Vec2};

use crate::app::App;
use crate::config::ProxyKind;
use crate::i18n::{fill, pick, Lang};
use crate::theme::Palette;
use crate::ui::widgets as w;
use crate::util::human_size;
use crate::{theme, util};

impl App {
    pub fn settings_view(&mut self, ui: &mut egui::Ui) {
        let p = self.p;
        let busy = self.progress.is_some();
        let mut dirty = false;
        let mut apply_root = false;
        let mut pick_root = false;
        let mut clear_cache = false;
        let mut refresh_builds = false;
        let mut apply_proxy = false;
        let mut disable_proxy = false;

        egui::ScrollArea::vertical()
            .id_salt("settings_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // --- Язык -------------------------------------------------------
                settings_card(ui, &p, pick("ЯЗЫК ИНТЕРФЕЙСА", "INTERFACE LANGUAGE"), |ui| {
                    ui.label(
                        RichText::new(pick(
                            "Язык применяется сразу и сохраняется в config.json.",
                            "The language applies immediately and is saved to config.json.",
                        ))
                        .size(12.0)
                        .color(p.muted),
                    );
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        for lang in Lang::ALL {
                            let selected = self.cfg.lang == lang;
                            let label = RichText::new(format!(
                                "{}  {}",
                                lang.short(),
                                lang.native()
                            ))
                            .size(12.5);
                            if ui.selectable_label(selected, label).clicked() && !selected {
                                self.cfg.lang = lang;
                                crate::i18n::set(lang);
                                self.cfg.save();
                                self.log(pick("Язык интерфейса изменён", "Interface language changed"));
                            }
                        }
                    });
                });

                ui.add_space(12.0);

                // --- Хранилище -------------------------------------------------
                settings_card(ui, &p, pick("ХРАНИЛИЩЕ", "STORAGE"), |ui| {
                    ui.label(
                        RichText::new(pick(
                            "Корневой каталог лаунчера. В нём лежат games/, downloads/, cache/ \
                             и library.json. Можно указать отдельный диск для сборок.",
                            "The launcher's root folder. It contains games/, downloads/, cache/ \
                             and library.json. You can use a separate disk for builds.",
                        ))
                        .size(12.0)
                        .color(p.muted),
                    );
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        w::text_input_sized(
                            ui,
                            &mut self.root_input,
                            "C:\\Games\\YanLan",
                            Vec2::new(360.0, 32.0),
                        );
                        if w::soft(ui, pick("Обзор…", "Browse…"), &p, true).clicked() {
                            pick_root = true;
                        }
                        if w::primary(ui, pick("Применить", "Apply"), &p, !busy).clicked() {
                            apply_root = true;
                        }
                    });
                    ui.add_space(10.0);
                    w::key_value(
                        ui,
                        pick("Сборки", "Builds"),
                        &self.cfg.games_dir().display().to_string(),
                        &p,
                    );
                    w::key_value(
                        ui,
                        pick("Загрузки", "Downloads"),
                        &self.cfg.downloads_dir().display().to_string(),
                        &p,
                    );
                    w::key_value(
                        ui,
                        pick("Кэш", "Cache"),
                        &self.cfg.cache_dir().display().to_string(),
                        &p,
                    );
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if w::soft(ui, pick("Открыть папку сборок", "Open builds folder"), &p, true)
                            .clicked()
                        {
                            util::open_path(&self.cfg.games_dir());
                        }
                        if w::soft(ui, pick("Открыть загрузки", "Open downloads"), &p, true)
                            .clicked()
                        {
                            util::open_path(&self.cfg.downloads_dir());
                        }
                    });
                });

                ui.add_space(12.0);

                // --- Каталог сборок ---------------------------------------------
                settings_card(ui, &p, pick("КАТАЛОГ СБОРОК", "BUILD CATALOG"), |ui| {
                    ui.label(
                        RichText::new(pick(
                            "Список версий игры (2014–2026) берётся с официального сайта и \
                             зеркал archive.org. Обновление занимает несколько секунд.",
                            "The list of game versions (2014–2026) comes from the official website \
                             and archive.org mirrors. Updating takes a few seconds.",
                        ))
                        .size(12.0)
                        .color(p.muted),
                    );
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if w::soft(
                            ui,
                            pick("⟳  Обновить каталог", "⟳  Refresh catalog"),
                            &p,
                            !self.builds_loading,
                        )
                        .clicked()
                        {
                            refresh_builds = true;
                        }
                        if w::danger(ui, pick("Очистить кэш", "Clear cache"), &p, !busy).clicked() {
                            clear_cache = true;
                        }
                    });
                    ui.add_space(8.0);
                    w::key_value(
                        ui,
                        pick("Версий в каталоге", "Versions in catalog"),
                        &format!("{}", self.builds.len()),
                        &p,
                    );
                    if let Some(ts) = self.builds_fetched_at {
                        w::key_value(
                            ui,
                            pick("Последнее обновление", "Last update"),
                            &util::format_ts(ts),
                            &p,
                        );
                    }
                    if let Some(err) = &self.builds_error {
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(util::trim_to(err, 160))
                                .size(11.5)
                                .color(p.red),
                        );
                    }
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(pick(
                            "Кэш хранит список сборок, чтобы лаунчер запускался быстрее. \
                             Очистка безопасна: данные скачаются заново.",
                            "The cache keeps the build list so the launcher starts faster. \
                             Clearing it is safe: the data will be downloaded again.",
                        ))
                        .size(11.0)
                        .color(p.muted),
                    );
                });

                ui.add_space(12.0);

                // --- Поведение --------------------------------------------------
                settings_card(ui, &p, pick("ПОВЕДЕНИЕ", "BEHAVIOUR"), |ui| {
                    let before_theme = self.cfg.light_theme;
                    if w::toggle(
                        ui,
                        &mut self.cfg.keep_archives,
                        pick("Оставлять скачанные архивы", "Keep downloaded archives"),
                        &p,
                    )
                    .changed()
                    {
                        dirty = true;
                    }
                    ui.label(
                        RichText::new(pick(
                            "Пригодится, если одну и ту же версию нужно поставить в разные \
                             сборки: архив не придётся качать повторно.",
                            "Useful when the same version has to go into several builds: \
                             the archive won't need to be downloaded again.",
                        ))
                        .size(11.0)
                        .color(p.muted),
                    );
                    ui.add_space(4.0);
                    if w::toggle(
                        ui,
                        &mut self.cfg.confirm_delete,
                        pick(
                            "Спрашивать подтверждение перед удалением",
                            "Ask for confirmation before deleting",
                        ),
                        &p,
                    )
                    .changed()
                    {
                        dirty = true;
                    }
                    ui.add_space(4.0);
                    if w::toggle(
                        ui,
                        &mut self.cfg.auto_refresh,
                        pick(
                            "Проверять каталог сборок при запуске",
                            "Check the build catalog on startup",
                        ),
                        &p,
                    )
                    .changed()
                    {
                        dirty = true;
                    }
                    ui.add_space(4.0);
                    if w::toggle(
                        ui,
                        &mut self.cfg.light_theme,
                        pick("Светлая тема", "Light theme"),
                        &p,
                    )
                    .changed()
                    {
                        dirty = true;
                        if before_theme != self.cfg.light_theme {
                            self.p = theme::apply(&self.ctx, self.cfg.light_theme);
                        }
                    }
                });

                ui.add_space(12.0);

                // --- Сеть -------------------------------------------------------
                settings_card(ui, &p, pick("СЕТЬ", "NETWORK"), |ui| {
                    ui.label(
                        RichText::new(pick(
                            "Свой прокси для скачивания сборок и каталога.",
                            "A custom proxy for downloading builds and the catalog.",
                        ))
                        .size(12.0)
                        .color(p.muted),
                    );
                    ui.add_space(8.0);

                    let mut use_proxy = !self.cfg.proxy.trim().is_empty();
                    if w::toggle(
                        ui,
                        &mut use_proxy,
                        pick("Использовать прокси", "Use a proxy"),
                        &p,
                    )
                    .changed()
                    {
                        if use_proxy {
                            if self.proxy_input.trim().is_empty() {
                                self.toast(
                                    pick(
                                        "Сначала укажите адрес прокси",
                                        "Enter the proxy address first",
                                    ),
                                    false,
                                );
                            } else {
                                apply_proxy = true;
                            }
                        } else {
                            disable_proxy = true;
                        }
                    }
                    ui.add_space(6.0);

                    ui.horizontal(|ui| {
                        ui.spacing_mut().interact_size.y = 34.0;
                        egui::ComboBox::from_id_salt("proxy_kind")
                            .width(104.0)
                            .selected_text(RichText::new(self.proxy_kind.label()).size(12.5))
                            .show_ui(ui, |ui| {
                                for kind in [ProxyKind::Http, ProxyKind::Socks5] {
                                    if ui
                                        .selectable_label(self.proxy_kind == kind, kind.label())
                                        .clicked()
                                    {
                                        self.proxy_kind = kind;
                                    }
                                }
                            });
                        w::text_input_sized(
                            ui,
                            &mut self.proxy_input,
                            "127.0.0.1:8080",
                            Vec2::new(300.0, 34.0),
                        );
                        if w::primary(ui, pick("Применить", "Apply"), &p, !busy).clicked() {
                            apply_proxy = true;
                        }
                    });
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(pick(
                            "Адрес пишется без схемы: 127.0.0.1:8080. С логином и паролем — \
                             user:pass@127.0.0.1:8080. Выключите переключатель, чтобы отключить прокси.",
                            "Enter the address without a scheme: 127.0.0.1:8080. With a login and \
                             password: user:pass@127.0.0.1:8080. Turn the switch off to disable the proxy.",
                        ))
                        .size(11.0)
                        .color(p.muted),
                    );
                    ui.add_space(8.0);
                    if w::toggle(
                        ui,
                        &mut self.cfg.retry_forever,
                        pick(
                            "Повторять оборвавшуюся загрузку без ограничений",
                            "Retry interrupted downloads indefinitely",
                        ),
                        &p,
                    )
                    .changed()
                    {
                        dirty = true;
                    }
                    ui.label(
                        RichText::new(pick(
                            "Загрузка продолжится с места обрыва, пока вы не нажмёте «Отмена».",
                            "The download resumes where it stopped until you press “Cancel”.",
                        ))
                        .size(11.0)
                        .color(p.muted),
                    );
                });

                ui.add_space(12.0);

                // --- Журнал -----------------------------------------------------
                settings_card(ui, &p, pick("ЖУРНАЛ", "LOG"), |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(fill(
                                &pick(
                                    "Событий: {count} · кэш {size}",
                                    "Events: {count} · cache {size}",
                                ),
                                &[
                                    ("count", &self.log.len().to_string()),
                                    (
                                        "size",
                                        &human_size(util::dir_size(&self.cfg.cache_dir())),
                                    ),
                                ],
                            ))
                            .size(11.0)
                            .color(p.muted),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if w::soft(ui, pick("Очистить", "Clear"), &p, !self.log.is_empty())
                                .clicked()
                            {
                                self.log.clear();
                            }
                        });
                    });
                    ui.add_space(6.0);
                    egui::Frame::none()
                        .fill(p.bg)
                        .stroke(egui::Stroke::new(1.0_f32, p.border))
                        .rounding(8.0)
                        .inner_margin(egui::Margin::same(10.0))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            egui::ScrollArea::vertical()
                                .id_salt("log_scroll")
                                .max_height(190.0)
                                .stick_to_bottom(true)
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    if self.log.is_empty() {
                                        ui.label(
                                            RichText::new(pick("Журнал пуст", "The log is empty"))
                                                .size(11.5)
                                                .color(p.muted),
                                        );
                                    }
                                    for line in &self.log {
                                        ui.label(
                                            RichText::new(line)
                                                .monospace()
                                                .size(11.0)
                                                .color(p.muted),
                                        );
                                    }
                                });
                        });
                });

                ui.add_space(12.0);

                // --- О программе ------------------------------------------------
                settings_card(ui, &p, pick("О ПРОГРАММЕ", "ABOUT"), |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("YANLAN").size(15.0).strong().color(p.text));
                        w::tag(ui, &format!("v{}", env!("CARGO_PKG_VERSION")), p.accent, &p);
                    });
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(pick(
                            "Неофициальный лаунчер Yandere Simulator. Проект не связан с \
                             разработчиком игры: все файлы скачиваются с публичных зеркал, \
                             права принадлежат их авторам.",
                            "An unofficial Yandere Simulator launcher. The project is not affiliated \
                             with the game's developer: all files are downloaded from public mirrors \
                             and remain the property of their authors.",
                        ))
                        .size(12.0)
                        .color(p.muted),
                    );
                    ui.add_space(8.0);
                    ui.horizontal_wrapped(|ui| {
                        w::hyperlink(
                            ui,
                            "yanderesimulator.com",
                            "https://yanderesimulator.com/",
                            &p,
                        );
                        ui.label(RichText::new("·").size(11.0).color(p.muted));
                        w::hyperlink(ui, "archive.org", "https://archive.org/", &p);
                    });
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if w::soft(ui, pick("Папка конфигурации", "Config folder"), &p, true)
                            .clicked()
                        {
                            if let Some(parent) = crate::config::config_path().parent() {
                                util::open_path(parent);
                            }
                        }
                        if w::soft(
                            ui,
                            pick("Открыть папку лаунчера", "Open launcher folder"),
                            &p,
                            true,
                        )
                        .clicked()
                        {
                            util::open_path(&self.cfg.root_dir);
                        }
                    });
                });

                ui.add_space(6.0);
            });

        // --- Обработка действий после отрисовки ------------------------------
        if pick_root {
            if let Some(dir) = rfd::FileDialog::new()
                .set_title(pick("Корневой каталог лаунчера", "Launcher root folder"))
                .set_directory(&self.cfg.root_dir)
                .pick_folder()
            {
                self.root_input = dir.display().to_string();
            }
        }

        if apply_root {
            self.apply_root_dir();
            self.root_input = self.cfg.root_dir.display().to_string();
        }

        if refresh_builds {
            self.refresh_builds();
            self.log(pick(
                "Обновление каталога сборок",
                "Refreshing the build catalog",
            ));
        }

        if apply_proxy {
            self.apply_proxy();
        }
        if disable_proxy {
            self.disable_proxy(true);
        }

        if clear_cache {
            util::remove_dir_quiet(&self.cfg.cache_dir());
            self.cfg.ensure_dirs();
            self.builds_fetched_at = None;
            self.log(pick("Кэш каталога очищен", "Catalog cache cleared"));
            self.toast(pick("Кэш очищен", "Cache cleared"), true);
        }

        if dirty {
            self.cfg.save();
        }
    }

    fn apply_root_dir(&mut self) {
        let text = self.root_input.trim().to_string();
        if text.is_empty() {
            self.toast(pick("Укажите путь к папке", "Enter a folder path"), false);
            return;
        }
        if self.progress.is_some() {
            self.toast(
                pick(
                    "Дождитесь завершения текущей задачи",
                    "Wait for the current task to finish",
                ),
                false,
            );
            return;
        }
        let path = std::path::PathBuf::from(&text);
        if path == self.cfg.root_dir {
            self.toast(pick("Путь не изменился", "The path is unchanged"), false);
            return;
        }
        self.cfg.root_dir = path;
        self.cfg.ensure_dirs();
        self.cfg.save();
        self.reload_library();
        self.log(fill(
            &pick("Корневой каталог: {path}", "Root folder: {path}"),
            &[("path", &self.cfg.root_dir.display().to_string())],
        ));
        self.toast(pick("Каталог изменён", "Folder changed"), true);
    }
}

fn settings_card(ui: &mut egui::Ui, p: &Palette, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::none()
        .fill(p.card)
        .stroke(egui::Stroke::new(1.0_f32, p.border))
        .rounding(14.0)
        .inner_margin(egui::Margin::same(18.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            w::section_title(ui, title, p);
            ui.add_space(6.0);
            body(ui);
        });
}
