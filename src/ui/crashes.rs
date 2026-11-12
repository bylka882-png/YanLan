//! Вкладка «Краши»: история падений и разбор логов.
//!
//! Анализ запускается сразу после завершения игры, но отчёты остаются здесь,
//! чтобы к ним можно было вернуться и сравнить разные сборки.

use egui::{RichText, Vec2};

use crate::app::App;
use crate::diagnose::CrashReport;
use crate::i18n::{fill, pick};
use crate::theme::Palette;
use crate::ui::widgets as w;
use crate::util::{format_ts_secs, human_eta, human_ms, trim_to};

enum CrashAction {
    OpenLogs,
    OpenBuild(String),
    Copy(String),
}

impl App {
    pub fn crashes_view(&mut self, ui: &mut egui::Ui) {
        let p = self.p;
        let reports = self.crashes.clone();
        let progress = self.crash_progress.clone();
        let mut action: Option<CrashAction> = None;

        egui::ScrollArea::vertical()
            .id_salt("crashes_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.allocate_ui_with_layout(
                    Vec2::new(ui.available_width(), 32.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.label(
                            RichText::new(pick("КРАШИ", "CRASHES"))
                                .size(11.0)
                                .strong()
                                .color(p.muted),
                        );
                        ui.label(
                            RichText::new(fill(
                                &pick("{count} шт.", "{count} total"),
                                &[("count", &reports.len().to_string())],
                            ))
                            .size(11.0)
                            .color(p.muted),
                        );
                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                if w::soft(
                                    ui,
                                    pick("Открыть папку логов", "Open logs folder"),
                                    &p,
                                    true,
                                )
                                .clicked()
                                {
                                    action = Some(CrashAction::OpenLogs);
                                }
                            },
                        );
                    },
                );
                ui.add_space(10.0);

                if let Some(progress) = &progress {
                    loading_card(ui, &p, progress, self.loading.as_ref());
                    ui.add_space(10.0);
                }

                if reports.is_empty() {
                    if progress.is_none() {
                        empty_card(
                            ui,
                            &p,
                            pick("Падений пока нет", "No crashes yet"),
                            pick(
                                "Запустите игру из лаунчера. Если она упадёт, лаунчер сам найдёт \
                                 логи и покажет здесь вероятную причину с уверенностью в процентах.",
                                "Launch the game from the launcher. If it crashes, the launcher \
                                 will find the logs and show the probable cause here with a \
                                 confidence percentage.",
                            ),
                        );
                    }
                    return;
                }

                // Вкладки сборок: только те, чьи краши есть в истории. Порядок —
                // как в списке отчётов, новые сверху.
                let mut builds: Vec<(String, String, usize)> = Vec::new();
                for report in &reports {
                    match builds.iter_mut().find(|item| item.0 == report.game_id) {
                        Some(item) => item.2 += 1,
                        None => builds.push((
                            report.game_id.clone(),
                            report.game_name.clone(),
                            1,
                        )),
                    }
                }
                if let Some(id) = &self.crash_filter {
                    if !builds.iter().any(|item| &item.0 == id) {
                        self.crash_filter = None;
                    }
                }
                let filter = self.crash_filter.clone();
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
                    let all = fill(
                        &pick("Все · {count}", "All · {count}"),
                        &[("count", &reports.len().to_string())],
                    );
                    if w::chip(ui, &p, &all, filter.is_none(), true, p.accent).clicked() {
                        self.crash_filter = None;
                    }
                    for (id, name, count) in &builds {
                        let text = format!("{} · {}", trim_to(name, 30), count);
                        let active = filter.as_deref() == Some(id.as_str());
                        if w::chip(ui, &p, &text, active, true, p.accent)
                            .on_hover_text(name)
                            .clicked()
                        {
                            self.crash_filter = if active {
                                None
                            } else {
                                Some(id.clone())
                            };
                        }
                    }
                });
                ui.add_space(10.0);

                let only = self.crash_filter.clone();
                let selected = self.crash_selected.clone();
                let shown: Vec<&CrashReport> = reports
                    .iter()
                    .filter(|report| {
                        only.as_deref()
                            .map_or(true, |id| id == report.game_id)
                    })
                    .collect();

                for report in &shown {
                    let open = selected.as_deref() == Some(report.id.as_str());
                    let (clicked, inner) = report_card(ui, &p, report, open);
                    if clicked {
                        self.crash_selected = if open {
                            None
                        } else {
                            Some(report.id.clone())
                        };
                    }
                    if let Some(inner) = inner {
                        action = Some(inner);
                    }
                    ui.add_space(8.0);
                }
            });

        if let Some(action) = action {
            match action {
                CrashAction::OpenLogs => crate::util::open_path(&crate::diagnose::logs_dir()),
                CrashAction::OpenBuild(id) => {
                    if let Some(game) = self.games.iter().find(|game| game.meta.id == id) {
                        crate::util::open_path(&game.path);
                    }
                }
                CrashAction::Copy(text) => {
                    ui.output_mut(|out| out.copied_text = text);
                    self.toast(pick("Отчёт скопирован", "Report copied"), true);
                }
            }
        }
    }
}

fn loading_card(
    ui: &mut egui::Ui,
    p: &Palette,
    progress: &crate::progress::Progress,
    anim: Option<&crate::loading::Anim>,
) {
    let snap = progress.snapshot();
    let fraction = if snap.total > 0 {
        snap.done as f32 / snap.total as f32
    } else if snap.file_total > 0 {
        snap.file_index as f32 / snap.file_total as f32
    } else {
        0.0
    };
    egui::Frame::none()
        .fill(p.panel)
        .stroke(egui::Stroke::new(1.0_f32, p.accent_dim))
        .rounding(14.0)
        .inner_margin(egui::Margin::same(18.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                if let Some(anim) = anim {
                    anim.show(ui, 44.0);
                }
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(pick("Разбираем логи", "Analyzing logs"))
                            .size(14.0)
                            .strong()
                            .color(p.text),
                    );
                    let stage = if snap.stage.is_empty() {
                        pick("Читаем файлы…", "Reading files…").to_string()
                    } else {
                        snap.stage.clone()
                    };
                    ui.label(RichText::new(stage).size(11.5).color(p.accent));
                    if !snap.detail.is_empty() {
                        ui.add(
                            egui::Label::new(
                                RichText::new(trim_to(&snap.detail, 90))
                                    .size(10.5)
                                    .color(p.muted),
                            )
                            .truncate(),
                        )
                        .on_hover_text(&snap.detail);
                    }
                });
            });
            ui.add_space(8.0);
            w::progress_bar(ui, fraction, p, 6.0);
            ui.add_space(4.0);
            let text = if snap.file_total > 0 {
                fill(
                    &pick(
                        "файл {done} из {total} · {percent}%",
                        "file {done} of {total} · {percent}%",
                    ),
                    &[
                        ("done", &snap.file_index.to_string()),
                        ("total", &snap.file_total.to_string()),
                        ("percent", &((fraction * 100.0).round() as i32).to_string()),
                    ],
                )
            } else {
                pick("Поиск файлов…", "Searching files…").to_string()
            };
            ui.horizontal(|ui| {
                ui.label(RichText::new(text).size(10.5).color(p.muted));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if w::soft(ui, pick("Отмена", "Cancel"), p, true).clicked() {
                        progress.cancel();
                    }
                });
            });
        });
}

/// Карточка отчёта. Возвращает (клик по заголовку, действие из кнопок).
fn report_card(
    ui: &mut egui::Ui,
    p: &Palette,
    report: &CrashReport,
    open: bool,
) -> (bool, Option<CrashAction>) {
    let mut clicked = false;
    let mut action = None;
    let color = confidence_color(report.confidence, p);

    egui::Frame::none()
        .fill(p.panel)
        .stroke(egui::Stroke::new(
            1.0_f32,
            if open { p.accent_dim } else { p.border },
        ))
        .rounding(14.0)
        .inner_margin(egui::Margin::same(16.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let arrow = if open { "▾" } else { "▸" };
                let title = format!("{arrow}  {}", report.title());
                let resp = ui.add(
                    egui::Label::new(RichText::new(title).size(14.0).strong().color(if open {
                        p.accent
                    } else {
                        p.text
                    }))
                    .sense(egui::Sense::click()),
                );
                if resp
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    clicked = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    w::tag(ui, &format!("{}%", report.confidence), color, p);
                });
            });
            ui.add_space(2.0);
            let mut parts: Vec<String> = Vec::new();
            if report.started > 0 {
                parts.push(format_ts_secs(report.started));
            }
            if report.duration_s > 0 {
                parts.push(fill(
                    &pick("игра шла {time}", "ran for {time}"),
                    &[("time", &human_eta(report.duration_s as f64))],
                ));
            }
            parts.push(report.game_name.clone());
            if let Some(code) = report.exit_code.filter(|code| *code != 0) {
                parts.push(format!("0x{:08X}", code as u32));
            }
            ui.add(
                egui::Label::new(RichText::new(parts.join("  ·  ")).size(10.5).color(p.muted))
                    .truncate(),
            );
            let cause = report.cause();
            ui.add_space(4.0);
            ui.label(
                RichText::new(if open {
                    cause.clone()
                } else {
                    trim_to(&cause, 220)
                })
                .size(12.0)
                .color(if open { p.text } else { p.muted }),
            );

            if !open {
                return;
            }

            ui.add_space(10.0);
            w::divider(ui, p);
            ui.add_space(10.0);

            w::section_title(ui, pick("УВЕРЕННОСТЬ", "CONFIDENCE"), p);
            w::progress_bar(ui, report.confidence as f32 / 100.0, p, 6.0);
            ui.add_space(3.0);
            ui.label(
                RichText::new(fill(
                    &pick(
                        "Оценка по правилам анализа логов: {percent}%. Это вероятность \
                         причины, а не гарантия.",
                        "Rule-based estimate from the logs: {percent}%. This is the likelihood \
                         of the cause, not a guarantee.",
                    ),
                    &[("percent", &report.confidence.to_string())],
                ))
                .size(10.5)
                .color(p.muted),
            );

            ui.add_space(10.0);
            w::section_title(ui, pick("ЧТО ДЕЛАТЬ", "WHAT TO DO"), p);
            for (index, line) in report.advice().iter().enumerate() {
                ui.label(
                    RichText::new(format!("{}. {line}", index + 1))
                        .size(12.0)
                        .color(p.text),
                );
                ui.add_space(2.0);
            }

            if !report.recent.is_empty() {
                ui.add_space(8.0);
                w::section_title(
                    ui,
                    pick(
                        "МОДЫ, УСТАНОВЛЕННЫЕ ПЕРЕД ЗАПУСКОМ",
                        "MODS INSTALLED BEFORE LAUNCH",
                    ),
                    p,
                );
                for item in &report.recent {
                    let when = format_ts_secs(item.at);
                    let download = if item.download_ms > 0 {
                        fill(
                            &pick(" · скачан за {time}", " · downloaded in {time}"),
                            &[("time", &human_ms(item.download_ms))],
                        )
                    } else {
                        String::new()
                    };
                    ui.label(
                        RichText::new(fill(
                            &pick("{name} — {when}{download}", "{name} — {when}{download}"),
                            &[
                                ("name", &item.name),
                                ("when", &when),
                                ("download", &download),
                            ],
                        ))
                        .size(11.5)
                        .color(p.text),
                    );
                }
            }

            if !report.evidence.is_empty() {
                ui.add_space(8.0);
                w::section_title(ui, pick("УЛИКИ ИЗ ЛОГОВ", "EVIDENCE FROM LOGS"), p);
                for item in &report.evidence {
                    let file = short_file(&item.file);
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            RichText::new(format!("{}:{}", file, item.line))
                                .size(10.0)
                                .color(p.accent_dim),
                        )
                        .on_hover_text(&item.file);
                        ui.label(
                            RichText::new(&item.text)
                                .monospace()
                                .size(10.5)
                                .color(p.text),
                        );
                    });
                    ui.add_space(1.0);
                }
            }

            if !report.logs.is_empty() {
                ui.add_space(6.0);
                ui.collapsing(
                    RichText::new(fill(
                        &pick("Просмотренные логи · {count}", "Logs scanned · {count}"),
                        &[("count", &report.scanned.to_string())],
                    ))
                    .size(11.5)
                    .color(p.muted),
                    |ui| {
                        for line in &report.logs {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(short_file(line))
                                        .monospace()
                                        .size(10.0)
                                        .color(p.muted),
                                )
                                .truncate(),
                            )
                            .on_hover_text(line);
                        }
                    },
                );
            }

            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                if w::soft(ui, pick("Открыть папку логов", "Open logs folder"), p, true).clicked()
                {
                    action = Some(CrashAction::OpenLogs);
                }
                if w::soft(ui, pick("Открыть сборку", "Open build"), p, true).clicked()
                {
                    action = Some(CrashAction::OpenBuild(report.game_id.clone()));
                }
                if w::soft(ui, pick("Скопировать отчёт", "Copy report"), p, true).clicked()
                {
                    action = Some(CrashAction::Copy(report_text(report)));
                }
            });
        });

    (clicked, action)
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

fn confidence_color(confidence: u8, p: &Palette) -> egui::Color32 {
    if confidence >= 70 {
        p.green
    } else if confidence >= 45 {
        p.accent
    } else {
        p.red
    }
}

fn short_file(path: &str) -> String {
    let parts: Vec<&str> = path.rsplit(['\\', '/']).take(2).collect();
    parts.into_iter().rev().collect::<Vec<_>>().join("\\")
}

fn report_text(report: &CrashReport) -> String {
    let mut out = format!(
        "YanLan — {}\n{}: {}%\n{}: {}\n",
        report.title(),
        pick("Уверенность", "Confidence"),
        report.confidence,
        pick("Сборка", "Build"),
        report.game_name,
    );
    if report.started > 0 {
        out.push_str(&format!(
            "{}: {}\n",
            pick("Запуск", "Started"),
            format_ts_secs(report.started)
        ));
    }
    if !report.detail.is_empty() {
        out.push_str(&format!(
            "{}: {}\n",
            pick("Подробности", "Details"),
            report.detail
        ));
    }
    out.push_str(&format!("\n{}\n", report.cause()));
    for (index, line) in report.advice().iter().enumerate() {
        out.push_str(&format!("{}. {line}\n", index + 1));
    }
    if !report.evidence.is_empty() {
        out.push_str(&format!("\n{}:\n", pick("Улики", "Evidence")));
        for item in &report.evidence {
            out.push_str(&format!("{}:{}: {}\n", item.file, item.line, item.text));
        }
    }
    out
}
