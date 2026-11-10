//! Вкладка «Каталог»: моды YanMod из GitHub-репозитория сообщества.
//!
//! Каталог читается из `info.yaml` (или `index.yaml`) и кэшируется на шесть
//! часов. Карточка показывает обложку, совместимость с выбранной сборкой и
//! кнопку установки; подробности, выбор версии и зависимости открываются в
//! отдельных окнах.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use egui::{Align, Color32, Layout, Rect, RichText, Sense, Stroke, Vec2};

use crate::app::{App, CatalogPending, CoverAnim};
use crate::i18n::{fill, pick};
use crate::jobs;
use crate::library::{GameInstall, Library};
use crate::mods::{self, CatalogSort, Compat, ModGroup, ModType, ModVariant};
use crate::theme::Palette;
use crate::ui::widgets as w;
use crate::util::trim_to;

enum CatalogAction {
    Details(String),
    Install {
        group: String,
        variant: usize,
    },
    Build {
        group: String,
        variant: usize,
    },
    /// Открыть ролик YouTube во встроенном плеере.
    Watch(String),
    Refresh,
}

impl App {
    pub fn catalog_view(&mut self, ui: &mut egui::Ui) {
        let p = self.p;
        let catalog = self.catalog.clone();
        let game = self.current_game().cloned();
        let total = catalog.len();
        let mut action: Option<CatalogAction> = None;

        ui.allocate_ui_with_layout(
            Vec2::new(ui.available_width(), 34.0),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.label(
                    RichText::new(pick("КАТАЛОГ YANMOD", "YANMOD CATALOG"))
                        .size(11.0)
                        .strong()
                        .color(p.muted),
                );
                ui.add_space(2.0);
                w::external(ui, "https://github.com/bylka882-png/YanMod", &p)
                    .on_hover_text(pick("Открыть YanMod на GitHub", "Open YanMod on GitHub"));
                ui.add_space(6.0);
                let found = match &self.catalog_fetched_at {
                    Some(ts) => fill(
                        &pick(
                            "{count} модов · обновлено {time}",
                            "{count} mods · updated {time}",
                        ),
                        &[
                            ("count", &total.to_string()),
                            ("time", &crate::util::format_ts(*ts)),
                        ],
                    ),
                    None => w::count_label(total),
                };
                ui.label(RichText::new(found).size(11.0).color(p.muted));

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if w::soft(
                        ui,
                        pick("⟳  Обновить", "⟳  Refresh"),
                        &p,
                        !self.catalog_loading,
                    )
                    .clicked()
                    {
                        action = Some(CatalogAction::Refresh);
                    }
                    ui.add_space(10.0);
                    let active = game.as_ref().map(|g| g.display_name()).unwrap_or_else(|| {
                        pick("Сборка не выбрана", "No build selected").to_string()
                    });
                    ui.label(
                        RichText::new(trim_to(&active, 40))
                            .size(11.0)
                            .color(if game.is_some() { p.text } else { p.muted }),
                    )
                    .on_hover_text(active);
                    ui.label(
                        RichText::new(pick("Активная сборка", "Active build"))
                            .size(11.0)
                            .color(p.muted),
                    );
                });
            },
        );

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let sort_width = 170.0;
            // Подпись «Сортировка» стоит слева от списка, поэтому её ширина
            // тоже входит в резерв: иначе она наезжает на поле поиска.
            let label_width = 72.0;
            let search_width = (ui.available_width() - sort_width - label_width - 16.0).max(220.0);
            w::text_input_sized(
                ui,
                &mut self.catalog_query,
                pick(
                    "Поиск по названию, автору, описанию…",
                    "Search by name, author, description…",
                ),
                Vec2::new(search_width, 32.0),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().interact_size.y = 32.0;
                egui::ComboBox::from_id_salt("catalog_sort")
                    .width(sort_width)
                    .selected_text(RichText::new(self.catalog_sort.label()).size(12.0))
                    .show_ui(ui, |ui| {
                        for option in CatalogSort::ALL {
                            let chosen = self.catalog_sort == option;
                            if ui
                                .selectable_label(chosen, option.label())
                                .on_hover_text(option.hint())
                                .clicked()
                            {
                                self.catalog_sort = option;
                            }
                        }
                    })
                    .response
                    .on_hover_text(pick("Сортировка каталога", "Catalog sorting"));
                ui.label(
                    RichText::new(pick("Сортировка", "Sort"))
                        .size(11.0)
                        .color(p.muted),
                );
            });
        });

        ui.add_space(8.0);
        let query = self.catalog_query.trim().to_lowercase();
        let matched: Vec<&ModGroup> = catalog
            .iter()
            .filter(|group| catalog_matches(group, &query))
            .collect();
        let mut counts: HashMap<ModType, usize> = HashMap::new();
        for group in &matched {
            for ty in group.type_list() {
                *counts.entry(ty).or_insert(0) += 1;
            }
        }

        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
            let all = fill(
                &pick("Все · {count}", "All · {count}"),
                &[("count", &matched.len().to_string())],
            );
            if w::chip(ui, &p, &all, self.catalog_section.is_none(), true, p.accent).clicked() {
                self.catalog_section = None;
            }
            for ty in ModType::ALL {
                let active = self.catalog_section == Some(ty);
                let text = format!("{} · {}", ty.code(), counts.get(&ty).copied().unwrap_or(0));
                if w::chip(ui, &p, &text, active, true, ty.color())
                    .on_hover_text(ty.label())
                    .clicked()
                {
                    self.catalog_section = if active { None } else { Some(ty) };
                }
            }
        });

        ui.add_space(8.0);
        w::divider(ui, &p);

        let mut list: Vec<&ModGroup> = catalog
            .iter()
            .filter(|group| self.catalog_section.map_or(true, |ty| group.has_type(ty)))
            .filter(|group| catalog_matches(group, &query))
            .collect();
        sort_groups(&mut list, self.catalog_sort, game.as_ref());

        let installed: HashSet<String> = match &game {
            Some(build) => {
                let mut set: HashSet<String> = self
                    .library
                    .for_game(&build.meta.id)
                    .map(|m| m.mod_id.to_lowercase())
                    .collect();
                // Моды из `ioa` уже входят в сборку — показываем «Установлено».
                for entry in &build.meta.ioa {
                    let (id, _) = mods::dep_spec(entry);
                    if !id.is_empty() {
                        set.insert(id.to_lowercase());
                    }
                }
                set
            }
            None => HashSet::new(),
        };
        let covers = self.catalog_covers.clone();
        let failed = self.catalog_cover_failed.clone();
        let mut requests: Vec<(String, String)> = Vec::new();

        egui::ScrollArea::vertical()
            .id_salt("catalog_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add_space(10.0);
                if catalog.is_empty() {
                    empty_state(
                        ui,
                        &p,
                        self.catalog_loading,
                        self.loading.as_ref(),
                        self.catalog_error.as_deref(),
                    );
                } else if list.is_empty() {
                    ui.add_space(30.0);
                    ui.vertical_centered(|ui| {
                        ui.label(
                            RichText::new(pick("Ничего не найдено", "Nothing found"))
                                .size(15.0)
                                .strong()
                                .color(p.text),
                        );
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(pick(
                                "Попробуйте другой запрос или раздел.",
                                "Try another query or section.",
                            ))
                            .size(11.5)
                            .color(p.muted),
                        );
                    });
                } else {
                    let width = ui.available_width();
                    let columns: usize = if width >= 930.0 {
                        3
                    } else if width >= 630.0 {
                        2
                    } else {
                        1
                    };
                    let gap = 12.0;
                    let card_width = ((width - gap * (columns.saturating_sub(1)) as f32)
                        / columns as f32)
                        .floor();

                    for row in list.chunks(columns) {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = gap;
                            for group in row {
                                let is_installed = installed.contains(&group.id.to_lowercase());
                                if let Some(act) = catalog_card(
                                    ui,
                                    &p,
                                    group,
                                    game.as_ref(),
                                    card_width,
                                    &covers,
                                    &failed,
                                    is_installed,
                                    &mut requests,
                                ) {
                                    action = Some(act);
                                }
                            }
                        });
                        ui.add_space(gap);
                    }
                }
            });

        // Обложки качаются в фоне по одному разу: ключ — хэш пути к файлу.
        let mut seen = HashSet::new();
        let mut started = 0usize;
        for (key, url) in requests {
            if started >= 8 {
                break;
            }
            if !seen.insert(key.clone())
                || self.catalog_covers.contains_key(&key)
                || self.catalog_cover_pending.contains(&key)
                || self.catalog_cover_failed.contains(&key)
            {
                continue;
            }
            started += 1;
            self.catalog_cover_pending.insert(key.clone());
            let c = jobs::Ctx2 {
                ctx: self.ctx.clone(),
                tx: self.tx.clone(),
            };
            jobs::fetch_cover(c, self.cfg.cache_dir(), key, url);
        }

        if let Some(action) = action {
            self.handle_catalog_action(action);
        }
    }

    fn handle_catalog_action(&mut self, action: CatalogAction) {
        match action {
            CatalogAction::Refresh => self.refresh_catalog(),
            CatalogAction::Details(id) => {
                self.catalog_selected = Some(id.clone());
                self.catalog_shot = 0;
                self.catalog_info = false;
                self.catalog_variant = self
                    .catalog
                    .iter()
                    .find(|group| group.id == id)
                    .map(|group| group.best_variant(self.current_game()))
                    .unwrap_or(0);
            }
            CatalogAction::Install { group, variant } => {
                let selected = self
                    .catalog
                    .iter()
                    .find(|item| item.id == group)
                    .and_then(|item| item.variants.get(variant))
                    .cloned();
                self.catalog_selected = None;
                if let Some(variant) = selected {
                    self.prepare_catalog_install(variant);
                }
            }
            CatalogAction::Build { group, variant } => {
                let selected = self
                    .catalog
                    .iter()
                    .find(|item| item.id == group)
                    .and_then(|item| item.variants.get(variant))
                    .cloned();
                self.catalog_selected = None;
                if let Some(variant) = selected {
                    self.open_catalog_build(variant);
                }
            }
            CatalogAction::Watch(id) => self.watch_youtube(id),
        }
    }

    fn open_catalog_build(&mut self, variant: ModVariant) {
        self.catalog_build_name = variant.name.clone();
        self.catalog_build = Some(variant);
        self.catalog_build_focus = true;
    }

    /// Проверяет зависимости и конфликты перед установкой.
    ///
    /// Если всё в порядке, установка стартует сразу; иначе открывается окно с
    /// предложением скачать зависимости и предупреждением о замене файлов.
    fn prepare_catalog_install(&mut self, variant: ModVariant) {
        if variant.is_build() {
            self.open_catalog_build(variant);
            return;
        }
        let Some(game) = self.current_game().cloned() else {
            self.toast(
                pick(
                    "Сначала создайте или выберите сборку",
                    "Create or select a build first",
                ),
                false,
            );
            return;
        };
        if variant.soon {
            self.toast(
                fill(
                    &pick(
                        "У мода «{name}» пока нет ссылки на скачивание",
                        "The mod “{name}” has no download link yet",
                    ),
                    &[("name", &variant.name)],
                ),
                false,
            );
            return;
        }

        let mut deps = Vec::new();
        let mut missing = Vec::new();
        let mut seen = HashSet::new();
        seen.insert(variant.id.to_lowercase());
        for dep in &variant.dependencies {
            resolve_dependency(
                &self.catalog,
                &self.library,
                &game,
                dep,
                &mut seen,
                &mut deps,
                &mut missing,
            );
        }

        let mut conflicts = Vec::new();
        if !variant.affected_files.is_empty() {
            let affected: HashSet<String> = variant
                .affected_files
                .iter()
                .map(|path| normalize_path(path))
                .filter(|path| !path.is_empty())
                .collect();
            for installed in self.library.for_game(&game.meta.id) {
                if installed
                    .files
                    .iter()
                    .any(|file| affected.contains(&normalize_path(&file.path)))
                {
                    conflicts.push(installed.name.clone());
                }
            }
            conflicts.sort();
            conflicts.dedup();
        }

        if deps.is_empty() && missing.is_empty() && conflicts.is_empty() {
            self.start_catalog_install(game, vec![variant]);
        } else {
            self.catalog_pending = Some(CatalogPending {
                variant,
                deps,
                missing,
                conflicts,
            });
        }
    }

    fn start_catalog_install(&mut self, game: GameInstall, items: Vec<ModVariant>) {
        let Some(last) = items.last().cloned() else {
            return;
        };
        let c = jobs::Ctx2 {
            ctx: self.ctx.clone(),
            tx: self.tx.clone(),
        };
        let title = if items.len() > 1 {
            fill(
                &pick(
                    "Установка «{name}» и зависимостей",
                    "Installing “{name}” and dependencies",
                ),
                &[("name", &last.name)],
            )
        } else {
            fill(
                &pick("Установка «{name}»", "Installing “{name}”"),
                &[("name", &last.name)],
            )
        };
        let progress = jobs::install_catalog_mods(c, self.cfg.clone(), game.clone(), items);
        self.start_parallel_job(title.clone(), progress);
        self.log(fill(
            &pick(
                "Установка «{name}» в {build}",
                "Installing “{name}” into {build}",
            ),
            &[("name", &last.name), ("build", &game.display_name())],
        ));
        self.catalog_pending = None;
        self.catalog_selected = None;
    }

    fn start_catalog_build(&mut self, variant: ModVariant, name: String) {
        let label = name.trim().to_string();
        if label.is_empty() {
            self.toast(pick("Укажите название сборки", "Enter a build name"), false);
            return;
        }
        if self.name_taken(&label) {
            self.toast(
                pick(
                    "Сборка с таким названием уже существует",
                    "A build with this name already exists",
                ),
                false,
            );
            return;
        }
        let c = jobs::Ctx2 {
            ctx: self.ctx.clone(),
            tx: self.tx.clone(),
        };
        let progress =
            jobs::install_catalog_build(c, self.cfg.clone(), variant.clone(), label.clone());
        self.start_parallel_job(
            fill(
                &pick("Создание сборки «{name}»", "Creating build “{name}”"),
                &[("name", &label)],
            ),
            progress,
        );
        self.log(fill(
            &pick(
                "Создание сборки «{name}» из каталога YanMod",
                "Creating build “{name}” from the YanMod catalog",
            ),
            &[("name", &label)],
        ));
        self.catalog_build = None;
        self.catalog_build_focus = false;
        self.catalog_selected = None;
        self.tab = crate::app::Tab::Builds;
    }

    /// Окно с подробностями мода: скриншоты, описание и выбор версии.
    pub fn draw_catalog_details(&mut self, ctx: &egui::Context) {
        let Some(id) = self.catalog_selected.clone() else {
            return;
        };
        let Some(group) = self.catalog.iter().find(|group| group.id == id).cloned() else {
            self.catalog_selected = None;
            return;
        };
        let p = self.p;
        let game = self.current_game().cloned();
        let mut variant_index = self
            .catalog_variant
            .min(group.variants.len().saturating_sub(1));
        let mut shot = self.catalog_shot;
        let mut show_info = self.catalog_info;
        let mut shots = variant_shots(&group, variant_index);
        if shot >= shots.len() {
            shot = 0;
        }
        let header_credits = group.variants[variant_index].credits.clone();
        let installed = match &game {
            Some(build) => {
                !group.is_build()
                    && (self
                        .library
                        .for_game(&build.meta.id)
                        .any(|m| m.mod_id.eq_ignore_ascii_case(&group.id))
                        || build
                            .meta
                            .ioa
                            .iter()
                            .any(|e| mods::dep_spec(e).0.eq_ignore_ascii_case(&group.id)))
            }
            None => false,
        };
        let covers = self.catalog_covers.clone();
        let failed = self.catalog_cover_failed.clone();
        let mut requests: Vec<(String, String)> = Vec::new();
        let mut action: Option<CatalogAction> = None;
        let mut close = false;

        let frame = modal_frame(&p, self.cfg.light_theme);
        egui::Window::new("catalog_details")
            .id(egui::Id::new(("catalog_details", id.as_str())))
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::new(0.0, -4.0))
            .frame(frame)
            .show(ctx, |ui| {
                ui.set_width(720.0);
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&group.name).size(20.0).strong().color(p.text));
                            for ty in group.type_list().into_iter().take(3) {
                                w::tag(ui, ty.label(), ty.color(), &p);
                            }
                        });
                        let author = if group.author.trim().is_empty() {
                            pick("автор неизвестен", "unknown author").to_string()
                        } else {
                            format!("by {}", group.author)
                        };
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(author).size(11.5).color(p.muted));
                            ui.spacing_mut().item_spacing.x = 5.0;
                            for link in mods::credit_links(&header_credits) {
                                w::credit(ui, link.kind, &link.url, &p);
                            }
                        });
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                        if w::close(ui, &p)
                            .on_hover_text(pick("Закрыть", "Close"))
                            .clicked()
                        {
                            close = true;
                        }
                    });
                });
                ui.add_space(12.0);

                egui::ScrollArea::vertical()
                    .id_salt("catalog_details_scroll")
                    .max_height(520.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        // Большой скриншот и миниатюры.
                        let width = ui.available_width();
                        let big = Rect::from_min_size(
                            ui.cursor().min,
                            Vec2::new(width, (width * 9.0 / 16.0).round()),
                        );
                        ui.allocate_exact_size(big.size(), Sense::hover());
                        let video = first_video(&group, variant_index);
                        let icon = group_icon(&group, variant_index);
                        paint_group_cover(
                            ui,
                            big,
                            shots.get(shot),
                            video.as_deref(),
                            icon.as_deref(),
                            &covers,
                            &failed,
                            &p,
                            group.ty,
                            &mut requests,
                        );

                        if shots.len() > 1 {
                            ui.add_space(8.0);
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
                                for (index, path) in shots.iter().enumerate() {
                                    let (rect, response) = ui
                                        .allocate_exact_size(Vec2::new(82.0, 46.0), Sense::click());
                                    paint_shot(
                                        ui,
                                        rect,
                                        Some(path),
                                        &covers,
                                        &failed,
                                        &p,
                                        group.ty,
                                        &mut requests,
                                    );
                                    let border = if index == shot { p.accent } else { p.border };
                                    ui.painter().rect_stroke(
                                        rect,
                                        6.0,
                                        Stroke::new(1.5_f32, border),
                                    );
                                    if response.clicked() {
                                        shot = index;
                                    }
                                }
                            });
                        }

                        let video_ids =
                            mods::youtube_ids(&group.variants[variant_index].description);
                        if !video_ids.is_empty() {
                            ui.add_space(10.0);
                            w::section_title(
                                ui,
                                pick("ВИДЕО ИЗ ОПИСАНИЯ", "VIDEO FROM THE DESCRIPTION"),
                                &p,
                            );
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing = Vec2::new(10.0, 8.0);
                                for id in video_ids.into_iter().take(10) {
                                    let (rect, response) = ui.allocate_exact_size(
                                        Vec2::new(220.0, 124.0),
                                        Sense::click(),
                                    );
                                    paint_video(ui, rect, &id, &covers, &failed, &p, &mut requests);
                                    if response.hovered() {
                                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                    }
                                    if response.clicked() {
                                        action = Some(CatalogAction::Watch(id.clone()));
                                    }
                                }
                            });
                        }

                        ui.add_space(12.0);
                        let variant = &group.variants[variant_index];
                        let description = variant.description_text();
                        ui.horizontal(|ui| {
                            let label = if show_info {
                                pick("Показать описание", "Show description")
                            } else {
                                pick("Показать info.yaml", "Show info.yaml")
                            };
                            if w::soft(ui, label, &p, true).clicked() {
                                show_info = !show_info;
                            }
                        });
                        ui.add_space(6.0);
                        if show_info {
                            let text = if variant.manifest_text.trim().is_empty() {
                                pick(
                                    "info.yaml ещё не загружен — обновите каталог модов.",
                                    "info.yaml is not loaded yet — refresh the mod catalog.",
                                )
                            } else {
                                variant.manifest_text.trim()
                            };
                            ui.label(RichText::new(text).monospace().size(11.0).color(p.text));
                            ui.add_space(10.0);
                        } else if !description.is_empty() {
                            ui.label(RichText::new(description).size(12.5).color(p.text));
                            ui.add_space(10.0);
                        }

                        let mut meta = String::new();
                        if !variant.version.is_empty() {
                            meta.push_str(&fill(
                                &pick("версия {v}", "version {v}"),
                                &[("v", &variant.version)],
                            ));
                        }
                        if !variant.game_v.is_empty() {
                            if !meta.is_empty() {
                                meta.push_str("  ·  ");
                            }
                            meta.push_str(&format!("gameV: {}", variant.game_v));
                        }
                        if !meta.is_empty() {
                            ui.label(RichText::new(meta).size(11.0).color(p.muted));
                            ui.add_space(8.0);
                        }

                        if !variant.dependencies.is_empty() {
                            ui.label(
                                RichText::new(fill(
                                    &pick("Зависимости: {list}", "Dependencies: {list}"),
                                    &[("list", &variant.dependencies.join(", "))],
                                ))
                                .size(11.0)
                                .color(p.muted),
                            );
                        }
                        if !variant.affected_files.is_empty() {
                            let shown = variant
                                .affected_files
                                .iter()
                                .take(6)
                                .cloned()
                                .collect::<Vec<_>>()
                                .join(", ");
                            let extra = variant.affected_files.len().saturating_sub(6);
                            let text = if extra > 0 {
                                fill(
                                    &pick(
                                        "Заменяет файлы: {list} и ещё {count}",
                                        "Replaces files: {list} and {count} more",
                                    ),
                                    &[("list", &shown), ("count", &extra.to_string())],
                                )
                            } else {
                                fill(
                                    &pick("Заменяет файлы: {list}", "Replaces files: {list}"),
                                    &[("list", &shown)],
                                )
                            };
                            ui.label(RichText::new(text).size(10.5).color(p.muted));
                        }

                        ui.add_space(12.0);
                        let label_of = |variant: &ModVariant| -> String {
                            let mut text = if variant.version.is_empty() {
                                variant.name.clone()
                            } else {
                                format!("v{}", variant.version)
                            };
                            if !variant.game_v.is_empty() {
                                text.push_str("  ·  ");
                                text.push_str(&variant.game_v);
                            }
                            if !variant.is_build() {
                                let (compat, _) = compat_tag(variant.compat(game.as_ref()), &p);
                                text.push_str("  ·  ");
                                text.push_str(compat);
                            }
                            if variant.soon {
                                text.push_str("  ·  ");
                                text.push_str(pick("скоро", "soon"));
                            }
                            text
                        };
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(fill(
                                    &pick("ВЕРСИИ · {count}", "VERSIONS · {count}"),
                                    &[("count", &group.variants.len().to_string())],
                                ))
                                .size(10.5)
                                .strong()
                                .color(p.muted),
                            );
                            ui.add_space(8.0);
                            let before = variant_index;
                            egui::ComboBox::from_id_salt(("catalog_versions", group.id.as_str()))
                                .width(320.0)
                                .selected_text(
                                    RichText::new(label_of(&group.variants[variant_index]))
                                        .size(12.0),
                                )
                                .show_ui(ui, |ui| {
                                    for (index, variant) in group.variants.iter().enumerate() {
                                        if ui
                                            .selectable_label(
                                                index == variant_index,
                                                label_of(variant),
                                            )
                                            .clicked()
                                        {
                                            variant_index = index;
                                        }
                                    }
                                });
                            if variant_index != before {
                                shots = variant_shots(&group, variant_index);
                                shot = 0;
                            }
                        });
                    });

                ui.add_space(10.0);
                w::divider(ui, &p);
                ui.add_space(10.0);

                let variant = &group.variants[variant_index];
                let is_build = variant.is_build();
                let enabled = !installed && !variant.soon && (is_build || game.is_some());
                let label = if installed {
                    pick("Установлено", "Installed")
                } else if variant.soon {
                    pick("Скоро", "Coming soon")
                } else if is_build {
                    pick("Создать сборку", "Create build")
                } else {
                    pick("Установить", "Install")
                };
                ui.horizontal(|ui| {
                    if w::primary(ui, label, &p, enabled).clicked() {
                        action = Some(if is_build {
                            CatalogAction::Build {
                                group: group.id.clone(),
                                variant: variant_index,
                            }
                        } else {
                            CatalogAction::Install {
                                group: group.id.clone(),
                                variant: variant_index,
                            }
                        });
                    }
                    if w::soft(ui, pick("Закрыть", "Close"), &p, true).clicked() {
                        close = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let note = if installed {
                            (
                                pick(
                                    "Мод уже установлен в эту сборку",
                                    "Already installed in this build",
                                ),
                                p.muted,
                            )
                        } else if variant.soon {
                            (
                                pick("Ссылка на архив ещё не добавлена", "No download link yet"),
                                p.muted,
                            )
                        } else if is_build {
                            (
                                pick(
                                    "Будет создана отдельная сборка в games/",
                                    "A separate build will be created in games/",
                                ),
                                p.muted,
                            )
                        } else if game.is_none() {
                            (
                                pick("Сначала выберите сборку", "Select a build first"),
                                p.red,
                            )
                        } else {
                            (variant.compat(game.as_ref()).label(), p.muted)
                        };
                        ui.label(RichText::new(note.0).size(11.0).color(note.1));
                    });
                });
            });

        self.catalog_variant = variant_index;
        self.catalog_shot = shot;
        self.catalog_info = show_info;
        self.spawn_cover_requests(requests);
        if let Some(action) = action {
            self.handle_catalog_action(action);
        } else if close {
            self.catalog_selected = None;
        }
    }

    /// Окно подтверждения: зависимости, отсутствующие моды и конфликты файлов.
    pub fn draw_catalog_pending(&mut self, ctx: &egui::Context) {
        let Some(pending) = self.catalog_pending.clone() else {
            return;
        };
        let p = self.p;
        let game = self.current_game().cloned();
        let mut confirm = false;
        let mut cancel = false;
        let mut download: Option<ModVariant> = None;

        let frame = modal_frame(&p, self.cfg.light_theme);
        egui::Window::new("catalog_pending_dialog")
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::new(0.0, -6.0))
            .frame(frame)
            .show(ctx, |ui| {
                ui.set_width(540.0);
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.label(
                            RichText::new(pick("Перед установкой", "Before installing"))
                                .size(19.0)
                                .strong()
                                .color(p.text),
                        );
                        ui.label(
                            RichText::new(&pending.variant.name)
                                .size(12.5)
                                .color(p.accent),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                        if w::close(ui, &p)
                            .on_hover_text(pick("Закрыть", "Close"))
                            .clicked()
                        {
                            cancel = true;
                        }
                    });
                });
                ui.add_space(12.0);

                if let Some(build) = &game {
                    ui.label(
                        RichText::new(fill(
                            &pick("Сборка: {name}", "Build: {name}"),
                            &[("name", &build.display_name())],
                        ))
                        .size(11.5)
                        .color(p.muted),
                    );
                    ui.add_space(8.0);
                }

                if !pending.deps.is_empty() {
                    ui.label(
                        RichText::new(fill(
                            &pick("Зависимости ({count}):", "Dependencies ({count}):"),
                            &[("count", &pending.deps.len().to_string())],
                        ))
                        .size(12.0)
                        .strong()
                        .color(p.text),
                    );
                    for dep in &pending.deps {
                        let mut line = dep.name.clone();
                        if !dep.version.is_empty() {
                            line.push_str(&format!("  ·  {}", dep.version));
                        }
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!("•  {line}"))
                                    .size(11.5)
                                    .color(p.muted),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if w::soft(ui, pick("Скачать", "Download"), &p, true).clicked()
                                    {
                                        download = Some(dep.clone());
                                    }
                                },
                            );
                        });
                    }
                    ui.add_space(8.0);
                }

                if !pending.missing.is_empty() {
                    ui.label(
                        RichText::new(fill(
                            &pick("Нет в каталоге: {list}", "Not found in the catalog: {list}"),
                            &[("list", &pending.missing.join(", "))],
                        ))
                        .size(11.5)
                        .color(p.red),
                    );
                    ui.label(
                        RichText::new(pick(
                            "Установите их вручную или проверьте ссылки в манифестах.",
                            "Install them manually or check the links in the manifests.",
                        ))
                        .size(10.5)
                        .color(p.muted),
                    );
                    ui.add_space(8.0);
                }

                if !pending.conflicts.is_empty() {
                    ui.label(
                        RichText::new(fill(
                            &pick(
                                "Файлы пересекаются с установленными модами: {list}",
                                "Files overlap with installed mods: {list}",
                            ),
                            &[("list", &pending.conflicts.join(", "))],
                        ))
                        .size(11.5)
                        .color(p.red),
                    );
                    ui.label(
                        RichText::new(pick(
                            "Заменённые файлы будут сохранены в резервной копии — \
                             мод можно удалить и вернуть всё как было.",
                            "Replaced files are saved in a backup — you can remove the mod \
                             and restore everything.",
                        ))
                        .size(10.5)
                        .color(p.muted),
                    );
                    ui.add_space(8.0);
                }

                ui.add_space(4.0);
                w::divider(ui, &p);
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let enabled = game.is_some() && !pending.variant.soon;
                    let label = if pending.deps.is_empty() {
                        pick("Установить", "Install")
                    } else {
                        pick(
                            "Установить мод и все зависимости",
                            "Install the mod and all dependencies",
                        )
                    };
                    if w::primary(ui, label, &p, enabled).clicked() {
                        confirm = true;
                    }
                    if w::soft(ui, pick("Отмена", "Cancel"), &p, true).clicked() {
                        cancel = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if pending.variant.soon {
                            ui.label(
                                RichText::new(pick(
                                    "Ссылка на архив ещё не добавлена",
                                    "No download link yet",
                                ))
                                .size(10.5)
                                .color(p.red),
                            );
                        }
                    });
                });
            });

        if let Some(dep) = download {
            let c = jobs::Ctx2 {
                ctx: self.ctx.clone(),
                tx: self.tx.clone(),
            };
            let progress = jobs::download_catalog_mod(c, self.cfg.clone(), dep.clone());
            self.start_parallel_job(
                fill(
                    &pick("Скачивание {name}", "Downloading {name}"),
                    &[("name", &dep.name)],
                ),
                progress,
            );
        }

        if confirm {
            if let Some(game) = game {
                let mut items = pending.deps.clone();
                items.push(pending.variant.clone());
                self.catalog_pending = None;
                self.start_catalog_install(game, items);
            } else {
                self.catalog_pending = None;
                self.toast(
                    pick("Сначала выберите сборку", "Select a build first"),
                    false,
                );
            }
        } else if cancel {
            self.catalog_pending = None;
        }
    }

    /// Окно названия для мода типа `Build`: он устанавливается как отдельная сборка.
    pub fn draw_catalog_build(&mut self, ctx: &egui::Context) {
        let Some(variant) = self.catalog_build.clone() else {
            return;
        };
        let p = self.p;
        let name = self.catalog_build_name.trim().to_string();
        let taken = !name.is_empty() && self.name_taken(&name);
        let can_create = !name.is_empty() && !taken;
        let mut create = false;
        let mut cancel = false;

        let frame = modal_frame(&p, self.cfg.light_theme);
        egui::Window::new("catalog_build_dialog")
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::new(0.0, -6.0))
            .frame(frame)
            .show(ctx, |ui| {
                ui.set_width(460.0);
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.label(
                            RichText::new(pick("Новая сборка из каталога", "New build from the catalog"))
                                .size(19.0)
                                .strong()
                                .color(p.text),
                        );
                        ui.label(
                            RichText::new(&variant.name)
                                .size(12.5)
                                .color(p.accent),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                        if w::close(ui, &p)
                            .on_hover_text(pick("Закрыть", "Close"))
                            .clicked()
                        {
                            cancel = true;
                        }
                    });
                });
                ui.add_space(14.0);
                ui.label(
                    RichText::new(pick("НАЗВАНИЕ СБОРКИ", "BUILD NAME"))
                        .size(10.5)
                        .strong()
                        .color(p.muted),
                );
                ui.add_space(4.0);
                let response = w::text_input(
                    ui,
                    &mut self.catalog_build_name,
                    pick(
                        "Например: 80s Student Council",
                        "For example: 80s Student Council",
                    ),
                    34.0,
                );
                if self.catalog_build_focus {
                    response.request_focus();
                    self.catalog_build_focus = false;
                }
                ui.add_space(5.0);
                let (status, color) = if taken {
                    (
                        pick(
                            "Сборка с таким названием уже существует",
                            "A build with this name already exists",
                        ),
                        p.red,
                    )
                } else if name.is_empty() {
                    (
                        pick("Введите название сборки", "Enter a build name"),
                        p.muted,
                    )
                } else {
                    (
                        pick(
                            "Архив будет скачан и распакован в отдельную папку games/",
                            "The archive will be downloaded and unpacked into a separate games/ folder",
                        ),
                        p.muted,
                    )
                };
                ui.allocate_ui_with_layout(
                    Vec2::new(ui.available_width(), 16.0),
                    egui::Layout::left_to_right(egui::Align::Min),
                    |ui| {
                        ui.label(RichText::new(status).size(11.0).color(color));
                    },
                );
                ui.add_space(10.0);
                w::divider(ui, &p);
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if w::primary(ui, pick("Создать сборку", "Create build"), &p, can_create)
                        .clicked()
                    {
                        create = true;
                    }
                    if w::soft(ui, pick("Отмена", "Cancel"), &p, true).clicked() {
                        cancel = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(pick(
                                "Enter — создать, Esc — отмена",
                                "Enter — create, Esc — cancel",
                            ))
                            .size(10.5)
                            .color(p.muted),
                        );
                    });
                });
                if can_create && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    create = true;
                }
            });

        if create {
            self.start_catalog_build(variant, name);
        } else if cancel {
            self.catalog_build = None;
            self.catalog_build_focus = false;
        }
    }

    fn spawn_cover_requests(&mut self, requests: Vec<(String, String)>) {
        let mut seen = HashSet::new();
        let mut started = 0usize;
        for (key, url) in requests {
            if started >= 8 {
                break;
            }
            if !seen.insert(key.clone())
                || self.catalog_covers.contains_key(&key)
                || self.catalog_cover_pending.contains(&key)
                || self.catalog_cover_failed.contains(&key)
            {
                continue;
            }
            started += 1;
            self.catalog_cover_pending.insert(key.clone());
            let c = jobs::Ctx2 {
                ctx: self.ctx.clone(),
                tx: self.tx.clone(),
            };
            jobs::fetch_cover(c, self.cfg.cache_dir(), key, url);
        }
    }
}

fn catalog_card(
    ui: &mut egui::Ui,
    p: &Palette,
    group: &ModGroup,
    game: Option<&GameInstall>,
    width: f32,
    covers: &HashMap<String, CoverAnim>,
    failed: &HashSet<String>,
    installed: bool,
    requests: &mut Vec<(String, String)>,
) -> Option<CatalogAction> {
    let inner_width = width - 24.0;
    let cover_h = (inner_width * 9.0 / 16.0).round();
    let height = cover_h + 170.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    let hovered = response.hovered();
    let border = if hovered { p.accent_dim } else { p.border };
    ui.painter()
        .rect(rect, 14.0, p.card, Stroke::new(1.0_f32, border));
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    let mut action = None;
    if response.clicked() {
        action = Some(CatalogAction::Details(group.id.clone()));
    }

    let inner = rect.shrink(12.0);
    let mut ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(Layout::top_down(Align::Min)),
    );
    ui.spacing_mut().item_spacing.y = 3.0;

    let best_index = group.best_variant(game);
    let best = &group.variants[best_index];
    let compat = best.compat(game);
    let cover_path = best
        .screenshots
        .first()
        .or_else(|| group.variants.iter().find_map(|v| v.screenshots.first()));
    let video = first_video(group, best_index);
    let icon = group_icon(group, best_index);
    let cover_rect = Rect::from_min_size(ui.cursor().min, Vec2::new(inner.width(), cover_h));
    ui.allocate_exact_size(cover_rect.size(), Sense::hover());
    paint_group_cover(
        &ui,
        cover_rect,
        cover_path,
        video.as_deref(),
        icon.as_deref(),
        covers,
        failed,
        p,
        group.ty,
        requests,
    );

    ui.add_space(7.0);
    ui.add(
        egui::Label::new(
            RichText::new(trim_to(&group.name, 42))
                .size(15.0)
                .strong()
                .color(p.text),
        )
        .truncate(),
    )
    .on_hover_text(&group.name);

    let author = if group.author.trim().is_empty() {
        pick("автор неизвестен", "unknown author").to_string()
    } else {
        format!("by {}", group.author)
    };
    ui.add(
        egui::Label::new(
            RichText::new(trim_to(&author, 44))
                .size(10.5)
                .color(p.muted),
        )
        .truncate(),
    );

    if group.is_build() {
        let included = best
            .ioa
            .iter()
            .map(|entry| {
                let (id, version) = mods::dep_spec(entry);
                if version.is_empty() {
                    id
                } else {
                    format!("{id} {version}")
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        if !included.is_empty() {
            ui.add(
                egui::Label::new(
                    RichText::new(fill(
                        &pick(
                            "В этой сборке установлены: {list}",
                            "Included in this build: {list}",
                        ),
                        &[("list", &included)],
                    ))
                    .size(10.0)
                    .color(p.accent_dim),
                )
                .truncate(),
            )
            .on_hover_text(included);
        }
    }

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        for ty in group.type_list().into_iter().take(3) {
            w::tag_small(ui, ty.code(), ty.color(), p);
        }
        if !best.version.is_empty() {
            w::tag_small(ui, &format!("v{}", trim_to(&best.version, 12)), p.muted, p);
        }
        if game.is_some() && !group.is_build() {
            let (label, color) = compat_tag(compat, p);
            w::tag_small(ui, label, color, p);
        }
        if installed {
            w::tag_small(ui, pick("установлен", "installed"), p.green, p);
        }
    });

    let footer = Rect::from_min_max(egui::pos2(inner.left(), inner.bottom() - 34.0), inner.max);
    let description = if best.description_text().is_empty() {
        group.description()
    } else {
        best.description_text()
    };
    let desc_rect = Rect::from_min_max(
        egui::pos2(inner.left(), ui.cursor().min.y + 4.0),
        egui::pos2(inner.right(), footer.top() - 4.0),
    );
    if desc_rect.height() > 10.0 {
        let mut desc_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(desc_rect)
                .layout(Layout::top_down(Align::Min)),
        );
        // Нельзя заменять область отсечения целиком: описание карточки,
        // прокрученной за пределы списка, иначе рисуется поверх шапки и
        // нижней панели. Пересекаем с областью списка.
        desc_ui.set_clip_rect(ui.clip_rect().intersect(desc_rect));
        if description.is_empty() {
            desc_ui.label(
                RichText::new(pick("Описание не добавлено", "No description yet"))
                    .size(11.0)
                    .color(p.muted),
            );
        } else {
            desc_ui.add(
                egui::Label::new(
                    RichText::new(trim_to(&description, 150))
                        .size(11.0)
                        .color(p.muted),
                )
                .wrap(),
            );
        }
    }

    let mut footer_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(footer)
            .layout(Layout::right_to_left(Align::Center)),
    );
    footer_ui.spacing_mut().item_spacing.x = 6.0;
    let is_build = group.is_build();
    let already = installed && !is_build;
    let enabled = !already && !best.soon && (is_build || game.is_some());
    let label = if already {
        pick("Установлено", "Installed")
    } else if best.soon {
        pick("Скоро", "Coming soon")
    } else if is_build {
        pick("Создать сборку", "Create build")
    } else {
        pick("Установить", "Install")
    };
    if w::primary(&mut footer_ui, label, p, enabled).clicked() {
        action = Some(if is_build {
            CatalogAction::Build {
                group: group.id.clone(),
                variant: best_index,
            }
        } else {
            CatalogAction::Install {
                group: group.id.clone(),
                variant: best_index,
            }
        });
    }
    let note = if already {
        ("", p.muted)
    } else if best.soon {
        (pick("нет ссылки", "no link"), p.muted)
    } else if !is_build && game.is_none() {
        (pick("выберите сборку", "select a build"), p.red)
    } else if !is_build && compat == Compat::No {
        (pick("не подходит", "incompatible"), p.red)
    } else {
        ("", p.muted)
    };
    if !note.0.is_empty() {
        footer_ui.add(egui::Label::new(RichText::new(note.0).size(10.5).color(note.1)).truncate());
    }

    action
}

/// Скругление обложек: как у карточки мода.
const COVER_ROUNDING: f32 = 14.0;

/// Превью первого видео из описания — запасная обложка.
fn first_video(group: &ModGroup, index: usize) -> Option<String> {
    group
        .variants
        .get(index)
        .and_then(|variant| mods::youtube_ids(&variant.description).into_iter().next())
}

/// Иконка `ico.webp` нужной версии (или любой версии мода).
fn group_icon(group: &ModGroup, index: usize) -> Option<String> {
    group
        .variants
        .get(index)
        .map(|variant| variant.icon.as_str())
        .filter(|icon| !icon.is_empty())
        .or_else(|| {
            group
                .variants
                .iter()
                .map(|variant| variant.icon.as_str())
                .find(|icon| !icon.is_empty())
        })
        .map(str::to_string)
}

/// Обложка по приоритету: скриншот → превью видео → иконка.
fn paint_group_cover(
    ui: &egui::Ui,
    rect: Rect,
    shot: Option<&String>,
    video: Option<&str>,
    icon: Option<&str>,
    covers: &HashMap<String, CoverAnim>,
    failed: &HashSet<String>,
    p: &Palette,
    ty: ModType,
    requests: &mut Vec<(String, String)>,
) {
    let covered = if let Some(path) = shot {
        paint_shot(ui, rect, Some(path), covers, failed, p, ty, requests);
        true
    } else if let Some(id) = video {
        paint_thumb(ui, rect, id, covers, failed, p, requests);
        true
    } else {
        false
    };
    if covered {
        if let Some(icon) = icon {
            paint_icon_badge(ui, rect, icon, covers, failed, p, requests);
        }
    } else if let Some(icon) = icon {
        paint_image(ui, rect, icon, covers, failed, p, requests);
    } else {
        paint_shot(ui, rect, None, covers, failed, p, ty, requests);
    }
}

fn paint_video(
    ui: &egui::Ui,
    rect: Rect,
    id: &str,
    covers: &HashMap<String, CoverAnim>,
    failed: &HashSet<String>,
    p: &Palette,
    requests: &mut Vec<(String, String)>,
) {
    paint_thumb(ui, rect, id, covers, failed, p, requests);

    let painter = ui.painter_at(rect);
    let c = rect.center();
    painter.rect_filled(rect, COVER_ROUNDING, Color32::from_black_alpha(36));
    painter.circle_filled(c, 21.0, Color32::from_black_alpha(160));
    painter.circle_stroke(c, 21.0, Stroke::new(1.5_f32, Color32::WHITE));
    painter.add(egui::Shape::convex_polygon(
        vec![
            egui::pos2(c.x - 6.0, c.y - 9.0),
            egui::pos2(c.x - 6.0, c.y + 9.0),
            egui::pos2(c.x + 10.0, c.y),
        ],
        Color32::WHITE,
        Stroke::NONE,
    ));
}

/// Превью ролика без кнопки воспроизведения.
fn paint_thumb(
    ui: &egui::Ui,
    rect: Rect,
    id: &str,
    covers: &HashMap<String, CoverAnim>,
    failed: &HashSet<String>,
    p: &Palette,
    requests: &mut Vec<(String, String)>,
) {
    let key = format!("yt-{id}");
    let texture = covers.get(&key);
    if texture.is_none() && !failed.contains(&key) {
        requests.push((key, mods::youtube_thumb_url(id)));
    }

    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, COVER_ROUNDING, p.card_hover);
    if let Some(cover) = texture {
        paint_cover(ui, &painter, rect, cover);
    }
}

/// Картинка из репозитория (иконка), когда обложки нет: крупная по центру.
fn paint_image(
    ui: &egui::Ui,
    rect: Rect,
    path: &str,
    covers: &HashMap<String, CoverAnim>,
    failed: &HashSet<String>,
    p: &Palette,
    requests: &mut Vec<(String, String)>,
) {
    let key = mods::cover_key(path);
    let texture = covers.get(&key);
    if texture.is_none() && !failed.contains(&key) {
        requests.push((key, mods::raw_url(path)));
    }

    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, COVER_ROUNDING, p.card_hover);
    if let Some(cover) = texture {
        let side = (rect.height() * 0.55)
            .min(rect.width() * 0.55)
            .clamp(56.0, 140.0);
        let area = Rect::from_center_size(rect.center(), Vec2::splat(side));
        paint_cover(ui, &painter, area, cover);
    }
}

/// Иконка мода в левом нижнем углу обложки — крупная, без рамки и отступа.
fn paint_icon_badge(
    ui: &egui::Ui,
    cover: Rect,
    path: &str,
    covers: &HashMap<String, CoverAnim>,
    failed: &HashSet<String>,
    _p: &Palette,
    requests: &mut Vec<(String, String)>,
) {
    let key = mods::cover_key(path);
    let texture = covers.get(&key);
    if texture.is_none() && !failed.contains(&key) {
        requests.push((key, mods::raw_url(path)));
    }
    let Some(icon) = texture else {
        return;
    };
    let side = (cover.height() * 0.34)
        .clamp(40.0, 104.0)
        .min(cover.width() * 0.42);
    let badge = Rect::from_min_size(
        egui::pos2(cover.left(), cover.bottom() - side),
        Vec2::splat(side),
    );
    let painter = ui.painter_at(cover);
    paint_cover(ui, &painter, badge, icon);
}

/// Рисует текущий кадр обложки и просит перерисовку к следующему кадру.
fn paint_cover(ui: &egui::Ui, painter: &egui::Painter, rect: Rect, cover: &CoverAnim) {
    let time = ui.ctx().input(|i| i.time);
    let (index, remaining) = cover.frame(time);
    let Some(texture) = cover.frames.get(index) else {
        return;
    };
    let size = texture.size_vec2();
    if size.x <= 0.0 || size.y <= 0.0 {
        return;
    }
    let scale = (rect.width() / size.x).max(rect.height() / size.y);
    let draw = Rect::from_center_size(rect.center(), size * scale);
    rounded_image(painter, rect, texture, draw);
    if cover.frames.len() > 1 {
        ui.ctx()
            .request_repaint_after(Duration::from_secs_f32(remaining.max(0.01)));
    }
}

/// Текстура с обрезкой по скруглённому прямоугольнику.
///
/// `painter.image` рисует прямоугольник, поэтому скругление делается сеткой:
/// вершины идут по контуру, UV считаются из полного прямоугольника картинки.
fn rounded_image(painter: &egui::Painter, rect: Rect, texture: &egui::TextureHandle, draw: Rect) {
    let radius = COVER_ROUNDING.min(rect.width().min(rect.height()) * 0.5);
    if radius <= 1.0 || draw.width() <= 0.0 || draw.height() <= 0.0 {
        painter.image(
            texture.id(),
            draw,
            Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        return;
    }

    let steps = 6;
    let arcs = [
        (
            egui::pos2(rect.left() + radius, rect.top() + radius),
            std::f32::consts::PI,
        ),
        (
            egui::pos2(rect.right() - radius, rect.top() + radius),
            1.5 * std::f32::consts::PI,
        ),
        (
            egui::pos2(rect.right() - radius, rect.bottom() - radius),
            0.0,
        ),
        (
            egui::pos2(rect.left() + radius, rect.bottom() - radius),
            0.5 * std::f32::consts::PI,
        ),
    ];
    let mut points = Vec::with_capacity(4 * (steps + 1));
    for (center, start) in arcs {
        for step in 0..=steps {
            let angle = start + 0.5 * std::f32::consts::PI * step as f32 / steps as f32;
            points.push(egui::pos2(
                center.x + radius * angle.cos(),
                center.y + radius * angle.sin(),
            ));
        }
    }

    let uv = |p: egui::Pos2| {
        egui::pos2(
            (p.x - draw.left()) / draw.width(),
            (p.y - draw.top()) / draw.height(),
        )
    };
    let mut mesh = egui::Mesh::default();
    mesh.texture_id = texture.id();
    mesh.vertices.push(egui::epaint::Vertex {
        pos: rect.center(),
        uv: uv(rect.center()),
        color: Color32::WHITE,
    });
    for point in &points {
        mesh.vertices.push(egui::epaint::Vertex {
            pos: *point,
            uv: uv(*point),
            color: Color32::WHITE,
        });
    }
    for index in 0..points.len() as u32 {
        let next = (index + 1) % points.len() as u32;
        mesh.indices.extend_from_slice(&[0, 1 + index, 1 + next]);
    }
    painter.add(egui::Shape::mesh(mesh));
}

fn paint_shot(
    ui: &egui::Ui,
    rect: Rect,
    path: Option<&String>,
    covers: &HashMap<String, CoverAnim>,
    failed: &HashSet<String>,
    p: &Palette,
    ty: ModType,
    requests: &mut Vec<(String, String)>,
) {
    let mut cover = None;
    if let Some(path) = path {
        let key = mods::cover_key(path);
        cover = covers.get(&key);
        if cover.is_none() && !failed.contains(&key) {
            requests.push((key, mods::raw_url(path)));
        }
    }

    let painter = ui.painter_at(rect);
    if let Some(cover) = cover {
        paint_cover(ui, &painter, rect, cover);
    } else {
        painter.rect_filled(rect, COVER_ROUNDING, p.card_hover);
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            ty.code(),
            egui::FontId::proportional(24.0),
            ty.color().gamma_multiply(0.9),
        );
    }
}

fn compat_tag(compat: Compat, p: &Palette) -> (&'static str, Color32) {
    match compat {
        Compat::Yes => (pick("подходит", "compatible"), p.green),
        Compat::No => (pick("не подходит", "incompatible"), p.red),
        Compat::Unknown => (pick("неизвестно", "unknown"), p.muted),
    }
}

fn variant_shots(group: &ModGroup, index: usize) -> Vec<String> {
    if let Some(variant) = group.variants.get(index) {
        if !variant.screenshots.is_empty() {
            return variant.screenshots.clone();
        }
    }
    group
        .variants
        .iter()
        .find_map(|variant| {
            if variant.screenshots.is_empty() {
                None
            } else {
                Some(variant.screenshots.clone())
            }
        })
        .unwrap_or_default()
}

fn catalog_matches(group: &ModGroup, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let mut haystack = format!(
        "{} {} {} {}",
        group.name,
        group.id,
        group.author,
        group.ty.code()
    );
    for variant in &group.variants {
        haystack.push(' ');
        haystack.push_str(&variant.name);
        haystack.push_str(&variant.author);
        haystack.push_str(&variant.description);
        haystack.push_str(&variant.version);
        haystack.push_str(&variant.game_v);
        haystack.push_str(&variant.folder);
        haystack.push_str(&variant.dependencies.join(" "));
    }
    haystack.to_lowercase().contains(query)
}

fn sort_groups(list: &mut [&ModGroup], sort: CatalogSort, game: Option<&GameInstall>) {
    match sort {
        CatalogSort::Name => list.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
        CatalogSort::Compat => list.sort_by(|a, b| {
            compat_rank(a, game)
                .cmp(&compat_rank(b, game))
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        }),
    }
}

fn compat_rank(group: &ModGroup, game: Option<&GameInstall>) -> u8 {
    let Some(game) = game else {
        return 1;
    };
    match group.variants[group.best_variant(Some(game))].compat(Some(game)) {
        Compat::Yes => 0,
        Compat::Unknown => 1,
        Compat::No => 2,
    }
}

// Зависимость пишется как `"id; версия"` (версия может отсутствовать).
fn resolve_dependency(
    catalog: &[ModGroup],
    library: &Library,
    game: &GameInstall,
    dep: &str,
    seen: &mut HashSet<String>,
    out: &mut Vec<ModVariant>,
    missing: &mut Vec<String>,
) {
    let (id, version) = mods::dep_spec(dep);
    let id = id.trim();
    if id.is_empty() || !seen.insert(format!("{}|{}", id.to_lowercase(), version.to_lowercase())) {
        return;
    }
    let wanted = |installed: &str| {
        version.trim().is_empty()
            || installed.trim().is_empty()
            || installed.eq_ignore_ascii_case(version.trim())
    };
    // Уже установлен (или входит в сборку) — вторую копию не ставим.
    if library
        .for_game(&game.meta.id)
        .any(|item| item.mod_id.eq_ignore_ascii_case(id) && wanted(&item.version))
    {
        return;
    }
    if game.meta.ioa.iter().any(|entry| {
        let (bundled, bundled_version) = mods::dep_spec(entry);
        bundled.eq_ignore_ascii_case(id) && wanted(&bundled_version)
    }) {
        return;
    }
    let Some(group) = catalog
        .iter()
        .find(|group| group.id.eq_ignore_ascii_case(id))
    else {
        missing.push(dep_label(id, &version));
        return;
    };
    let variant = if version.is_empty() {
        group.variants[group.best_variant(Some(game))].clone()
    } else {
        match group
            .variants
            .iter()
            .find(|variant| variant.version.eq_ignore_ascii_case(version.trim()))
        {
            Some(variant) => variant.clone(),
            None => {
                missing.push(dep_label(id, &version));
                return;
            }
        }
    };
    if variant.soon {
        missing.push(variant.name.clone());
        return;
    }
    for dep in variant.dependencies.clone() {
        resolve_dependency(catalog, library, game, &dep, seen, out, missing);
    }
    out.push(variant);
}

fn dep_label(id: &str, version: &str) -> String {
    if version.is_empty() {
        id.to_string()
    } else {
        format!("{id} {version}")
    }
}

fn normalize_path(path: &str) -> String {
    path.replace('\\', "/")
        .trim_start_matches('/')
        .to_ascii_lowercase()
}

fn empty_state(
    ui: &mut egui::Ui,
    p: &Palette,
    loading: bool,
    anim: Option<&crate::loading::Anim>,
    error: Option<&str>,
) {
    ui.add_space(26.0);
    egui::Frame::none()
        .fill(p.card)
        .stroke(Stroke::new(1.0_f32, p.border))
        .rounding(16.0)
        .inner_margin(egui::Margin::same(30.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.vertical_centered(|ui| {
                if loading {
                    ui.add_space(14.0);
                    if let Some(anim) = anim {
                        anim.show(ui, 64.0);
                    } else {
                        ui.spinner();
                    }
                    ui.add_space(10.0);
                    ui.label(
                        RichText::new(pick(
                            "Загружаем каталог YanMod…",
                            "Loading the YanMod catalog…",
                        ))
                        .size(13.0)
                        .color(p.text),
                    );
                    ui.add_space(14.0);
                } else if let Some(error) = error {
                    ui.label(
                        RichText::new("!")
                            .size(34.0)
                            .color(p.red.gamma_multiply(0.8)),
                    );
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(pick(
                            "Не удалось загрузить каталог",
                            "Could not load the catalog",
                        ))
                        .size(17.0)
                        .strong()
                        .color(p.text),
                    );
                    ui.add_space(6.0);
                    ui.label(RichText::new(trim_to(error, 260)).size(11.5).color(p.muted));
                    ui.add_space(12.0);
                    ui.label(
                        RichText::new(pick(
                            "Нажмите «Обновить» в шапке, чтобы попробовать снова.",
                            "Press “Refresh” at the top to try again.",
                        ))
                        .size(11.0)
                        .color(p.muted),
                    );
                } else {
                    ui.label(
                        RichText::new("◆")
                            .size(40.0)
                            .color(p.accent.gamma_multiply(0.7)),
                    );
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(pick("Каталог пока пуст", "The catalog is empty"))
                            .size(18.0)
                            .strong()
                            .color(p.text),
                    );
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(pick(
                            "Добавьте info.yaml в репозиторий YanMod и нажмите «Обновить».",
                            "Add info.yaml files to the YanMod repository and press “Refresh”.",
                        ))
                        .size(11.5)
                        .color(p.muted),
                    );
                }
            });
        });
}

fn modal_frame(p: &Palette, light: bool) -> egui::Frame {
    egui::Frame::none()
        .fill(p.panel)
        .stroke(Stroke::new(1.0_f32, p.border))
        .rounding(16.0)
        .inner_margin(egui::Margin::same(22.0))
        .shadow(egui::epaint::Shadow {
            offset: Vec2::new(0.0, 12.0),
            blur: 30.0,
            spread: 0.0,
            color: Color32::from_black_alpha(if light { 60 } else { 150 }),
        })
}
