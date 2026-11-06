//! YanLan — неофициальный лаунчер Yandere Simulator.
//!
//! Возможности: архив всех публичных сборок игры (2014–2026), создание
//! отдельных сборок под моды, безопасная установка архивов с резервными
//! копиями файлов и откатом изменений.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod builds;
mod cfg_edit;
mod config;
mod i18n;
mod installer;
mod jobs;
mod library;
mod loading;
mod mods;
mod net;
mod player;
mod progress;
mod runner;
mod theme;
mod ui;
mod util;

use app::App;

fn main() -> eframe::Result<()> {
    let viewport = egui::ViewportBuilder::default()
        .with_title("YanLan — Yandere Simulator")
        .with_inner_size([1200.0, 800.0])
        .with_min_inner_size([980.0, 640.0])
        .with_icon(theme::app_icon());

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native("YanLan", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}
