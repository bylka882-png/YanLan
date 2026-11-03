//! Запуск игры и системные действия.

use std::path::Path;
use std::process::Command;

use crate::i18n::{fill, pick};

pub fn launch(game_dir: &Path, exe: &Path, args: &[String]) -> Result<(), String> {
    Command::new(exe)
        .args(args)
        .current_dir(game_dir)
        .spawn()
        .map(|_| ())
        .map_err(|e| {
            fill(
                &pick(
                    "Не удалось запустить игру: {err}",
                    "Failed to launch the game: {err}",
                ),
                &[("err", &e.to_string())],
            )
        })
}

pub fn reveal(path: &Path) {
    if path.exists() {
        crate::util::open_path(path);
    }
}
