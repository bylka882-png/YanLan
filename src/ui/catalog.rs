//! Вкладка «Каталог» — заглушка под будущий каталог модов, патчей и текстур.

use egui::RichText;

use crate::app::App;
use crate::i18n::pick;
use crate::ui::widgets as w;

impl App {
    pub fn catalog_view(&mut self, ui: &mut egui::Ui) {
        let p = self.p;
        egui::ScrollArea::vertical()
            .id_salt("catalog_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add_space(10.0);
                egui::Frame::none()
                    .fill(p.card)
                    .stroke(egui::Stroke::new(1.0_f32, p.border))
                    .rounding(16.0)
                    .inner_margin(egui::Margin::same(30.0))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.vertical_centered(|ui| {
                            ui.label(
                                RichText::new("◆")
                                    .size(46.0)
                                    .color(p.accent.gamma_multiply(0.7)),
                            );
                            ui.add_space(6.0);
                            ui.label(
                                RichText::new(pick("Каталог скоро появится", "The catalog is coming soon"))
                                    .size(20.0)
                                    .strong()
                                    .color(p.text),
                            );
                            ui.add_space(6.0);
                            ui.label(
                                RichText::new(pick(
                                    "Здесь будет удобный каталог модов, патчей и текстур — \
                                     с превью, описаниями и установкой в выбранную сборку.",
                                    "A convenient catalog of mods, patches and textures will live here — \
                                     with previews, descriptions and installation into the selected build.",
                                ))
                                .size(12.5)
                                .color(p.muted),
                            );
                        });

                        ui.add_space(18.0);
                        w::divider(ui, &p);
                        ui.add_space(14.0);

                        let items = [
                            (
                                pick("Моды", "Mods"),
                                pick(
                                    "Готовые моды с фильтрами по категориям и популярности.",
                                    "Ready-made mods with filters by category and popularity.",
                                ),
                            ),
                            (
                                pick("Патчи", "Patches"),
                                pick(
                                    "Русификаторы, исправления и улучшения интерфейса.",
                                    "Translations, fixes and interface improvements.",
                                ),
                            ),
                            (
                                pick("Текстуры", "Textures"),
                                pick(
                                    "Скины персонажей, замены моделей и оформления.",
                                    "Character skins, model replacements and visual overhauls.",
                                ),
                            ),
                        ];
                        for (title, text) in items {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("•").size(15.0).color(p.accent));
                                ui.vertical(|ui| {
                                    ui.label(
                                        RichText::new(title).size(13.5).strong().color(p.text),
                                    );
                                    ui.label(RichText::new(text).size(11.5).color(p.muted));
                                });
                            });
                            ui.add_space(8.0);
                        }

                        ui.add_space(6.0);
                        w::divider(ui, &p);
                        ui.add_space(12.0);
                        ui.vertical_centered(|ui| {
                            ui.label(
                                RichText::new(pick(
                                    "Пока каталог в разработке. Устанавливать архивы вручную \
                                     можно на вкладке «Установлено».",
                                    "The catalog is still in development. You can install archives \
                                     manually on the “Installed” tab.",
                                ))
                                .size(11.5)
                                .color(p.muted),
                            );
                        });
                    });

                ui.add_space(14.0);
                ui.label(
                    RichText::new(pick(
                        "Идеи и пожелания по каталогу приветствуются — раздел спроектирован \
                         так, чтобы добавлять источники без изменения сборок.",
                        "Ideas and suggestions for the catalog are welcome — the section is designed \
                         so sources can be added without touching builds.",
                    ))
                    .size(11.0)
                    .color(p.muted),
                );
            });
    }
}
