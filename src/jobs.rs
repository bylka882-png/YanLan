//! Фоновые задачи: сеть, установка, удаление.

use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::thread;

use crate::builds::{self, Build};
use crate::config::Config;
use crate::i18n::{fill, pick};
use crate::installer;
use crate::library::{write_meta, GameInstall, GameMeta, InstalledMod, Library};
use crate::net;
use crate::progress::Progress;
use crate::util::{now_ts, sanitize_dir_name, sanitize_name};

pub enum Ev {
    Builds {
        builds: Vec<Build>,
        fetched_at: i64,
        error: Option<String>,
    },
    JobDone {
        /// Номер задачи (`Progress::id`), чтобы интерфейс не сбрасывал чужой прогресс.
        id: u64,
        ok: bool,
        message: String,
    },
    DataDone {
        ok: bool,
        message: String,
    },
    Log(String),
}

#[derive(Clone)]
pub struct Ctx2 {
    pub ctx: egui::Context,
    pub tx: Sender<Ev>,
}

impl Ctx2 {
    pub fn emit(&self, event: Ev) {
        let _ = self.tx.send(event);
        self.ctx.request_repaint();
    }

    pub fn log(&self, text: impl Into<String>) {
        self.emit(Ev::Log(text.into()));
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Notify {
    /// Действие пользователя: показываем тост и сбрасываем индикатор задачи.
    Job,
    /// Фоновое обновление данных: не трогаем индикатор установки.
    Data,
}

fn finish(c: &Ctx2, progress: &Progress, notify: Notify, result: Result<String, String>) {
    match result {
        Ok(message) => {
            progress.finish();
            if !message.is_empty() {
                c.log(message.clone());
            }
            c.emit(if notify == Notify::Job {
                Ev::JobDone {
                    id: progress.id(),
                    ok: true,
                    message,
                }
            } else {
                Ev::DataDone { ok: true, message }
            });
        }
        Err(message) => {
            progress.fail(message.clone());
            c.log(fill(
                &pick("Ошибка: {msg}", "Error: {msg}"),
                &[("msg", &message)],
            ));
            c.emit(if notify == Notify::Job {
                Ev::JobDone {
                    id: progress.id(),
                    ok: false,
                    message,
                }
            } else {
                Ev::DataDone { ok: false, message }
            });
        }
    }
}

fn spawn<F>(c: Ctx2, progress: Progress, job: F)
where
    F: FnOnce(&Ctx2, &Progress) -> Result<String, String> + Send + 'static,
{
    spawn_impl(c, progress, Notify::Job, job);
}

fn spawn_data<F>(c: Ctx2, progress: Progress, job: F)
where
    F: FnOnce(&Ctx2, &Progress) -> Result<String, String> + Send + 'static,
{
    spawn_impl(c, progress, Notify::Data, job);
}

fn spawn_impl<F>(c: Ctx2, progress: Progress, notify: Notify, job: F)
where
    F: FnOnce(&Ctx2, &Progress) -> Result<String, String> + Send + 'static,
{
    thread::spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job(&c, &progress)))
            .unwrap_or_else(|_| {
                Err(pick(
                    "Внутренняя ошибка (паника в фоновой задаче)",
                    "Internal error (panic in a background task)",
                )
                .to_string())
            });
        finish(&c, &progress, notify, result);
    });
}

pub fn fetch_builds(c: Ctx2, cfg: Config) -> Progress {
    let progress = Progress::new();
    let p = progress.clone();
    spawn_data(c, progress, move |c, p| match builds::fetch_all(p) {
        Ok(list) => {
            builds::save_cache(&cfg, &list);
            c.emit(Ev::Builds {
                builds: list,
                fetched_at: now_ts(),
                error: None,
            });
            Ok(pick("Каталог сборок обновлён", "Build catalog updated").to_string())
        }
        Err(err) => {
            c.emit(Ev::Builds {
                builds: Vec::new(),
                fetched_at: now_ts(),
                error: Some(err.clone()),
            });
            Err(err)
        }
    });
    p
}

fn download(c: &Ctx2, cfg: &Config, build: &Build, progress: &Progress) -> Result<PathBuf, String> {
    // Имя файла в downloads/ должно быть уникальным для версии: у разных
    // зеркал архивы могут называться одинаково (и у официальной сборки имя
    // всегда YandereSimulator-latest.zip). Иначе при включённых архивах
    // вместо новой версии распаковывался бы старый файл.
    let stem = build
        .filename
        .rsplit_once('.')
        .map(|(s, _)| s)
        .unwrap_or(&build.filename);
    let ext = build
        .filename
        .rsplit_once('.')
        .map(|(_, e)| e)
        .unwrap_or("zip");
    let key = sanitize_name(&build.id);
    let mut name = sanitize_name(&format!("{key}-{stem}"));
    name.push('.');
    name.push_str(&sanitize_name(ext));
    let dest = cfg.downloads_dir().join(name);

    // Официальная «последняя версия» меняется по тому же URL, поэтому старый
    // архив нельзя переиспользовать — скачиваем заново, сохраняя копию на
    // случай, если сеть откажет.
    let is_latest = build.id == "latest";
    if dest.exists() && !is_latest {
        progress.stage(pick("Архив уже скачан", "Archive already downloaded"));
        c.log(fill(
            &pick("Используем {path}", "Using {path}"),
            &[("path", &dest.display().to_string())],
        ));
        return Ok(dest);
    }

    let backup = dest.with_extension("old");
    if dest.exists() {
        let _ = std::fs::remove_file(&backup);
        let _ = std::fs::rename(&dest, &backup);
    }

    progress.stage(pick("Скачивание", "Downloading"));
    progress.detail(build.filename.clone());
    c.log(fill(
        &pick("Скачивание: {url}", "Downloading: {url}"),
        &[("url", &build.url)],
    ));
    match net::download(&build.url, &dest, progress, cfg.retry_forever) {
        Ok(_) => {
            let _ = std::fs::remove_file(&backup);
            Ok(dest)
        }
        Err(err) => {
            // Не удалось обновить — возвращаем ранее скачанный архив.
            if backup.exists() && !dest.exists() {
                let _ = std::fs::rename(&backup, &dest);
            }
            Err(err)
        }
    }
}

/// Создаёт новую сборку: скачивает выбранную версию и раскладывает её
/// в `games/<название пользователя>`.
pub fn install_build(c: Ctx2, cfg: Config, build: Build, name: String) -> Progress {
    let progress = Progress::new();
    let p = progress.clone();
    spawn(c, progress, move |c, p| {
        cfg.ensure_dirs();
        let label = name.trim().to_string();
        if label.is_empty() {
            return Err(pick("Укажите название сборки", "Enter a build name").to_string());
        }
        let dir_name = sanitize_dir_name(&label);
        let target = cfg.games_dir().join(&dir_name);
        if target.exists() {
            return Err(fill(
                &pick(
                    "Сборка «{name}» уже существует. Выберите другое название.",
                    "Build “{name}” already exists. Choose another name.",
                ),
                &[("name", &label)],
            ));
        }
        let archive = download(c, &cfg, &build, p)?;
        p.stage(pick("Установка сборки", "Installing build"));
        p.detail(label.clone());
        let staging = cfg.games_dir().join(format!(".staging-{dir_name}"));
        installer::install_game(&archive, &target, &staging, p)?;

        let meta = GameMeta {
            id: dir_name,
            label,
            version: build.label.clone(),
            date: build.date.clone(),
            source: build.source.clone(),
            installed_at: now_ts(),
            args: String::new(),
        };
        write_meta(&target, &meta)?;

        if !cfg.keep_archives {
            let _ = std::fs::remove_file(&archive);
        }
        Ok(fill(
            &pick("Сборка «{name}» создана", "Build “{name}” created"),
            &[("name", &meta.label)],
        ))
    });
    p
}

pub fn install_local_archive(
    c: Ctx2,
    cfg: Config,
    game: GameInstall,
    archive: PathBuf,
) -> Progress {
    let progress = Progress::new();
    let p = progress.clone();
    spawn(c, progress, move |c, p| {
        if !archive.exists() {
            return Err(pick("Файл не найден", "File not found").to_string());
        }
        let name = archive
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| pick("Локальный мод", "Local mod").to_string());
        let key = format!("{}-{}", sanitize_name(&name), now_ts());
        let backup_dir = game.backups_dir().join(&key);
        let staging = game.work_dir().join("tmp");
        p.stage(pick("Установка мода", "Installing mod"));
        let files = installer::install_mod(&archive, &game.path, &backup_dir, &staging, p)?;
        let mut item = InstalledMod::new(&name, &name, "mod", "", &game.meta.id, files);
        item.key = key;
        let mut library = Library::load(&cfg.root_dir);
        library.add(item);
        library.save(&cfg.root_dir)?;
        c.log(fill(
            &pick(
                "Установлен локальный архив {path}",
                "Installed local archive {path}",
            ),
            &[("path", &archive.display().to_string())],
        ));
        Ok(fill(
            &pick(
                "«{name}» установлен в {build}",
                "“{name}” installed into {build}",
            ),
            &[("name", &name), ("build", &game.display_name())],
        ))
    });
    p
}

pub fn uninstall_mod(c: Ctx2, cfg: Config, game: GameInstall, key: String) -> Progress {
    let progress = Progress::new();
    let p = progress.clone();
    spawn(c, progress, move |c, p| {
        let mut library = Library::load(&cfg.root_dir);
        let item = library
            .remove(&key)
            .ok_or_else(|| pick("Запись о моде не найдена", "Mod record not found").to_string())?;
        p.stage(pick("Удаление мода", "Uninstalling mod"));
        let backup_dir = game.backups_dir().join(&item.key);
        installer::uninstall_mod(&game.path, &backup_dir, &item.files)?;
        library.save(&cfg.root_dir)?;
        c.log(fill(
            &pick("Удалён мод «{name}»", "Mod “{name}” removed"),
            &[("name", &item.name)],
        ));
        Ok(fill(
            &pick("«{name}» удалён", "“{name}” removed"),
            &[("name", &item.name)],
        ))
    });
    p
}

pub fn delete_game(c: Ctx2, cfg: Config, game: GameInstall) -> Progress {
    let progress = Progress::new();
    let p = progress.clone();
    spawn(c, progress, move |c, p| {
        p.stage(pick("Удаление сборки", "Deleting build"));
        let mut library = Library::load(&cfg.root_dir);
        let keys: Vec<String> = library
            .for_game(&game.meta.id)
            .map(|m| m.key.clone())
            .collect();
        for key in keys {
            library.remove(&key);
        }
        library.save(&cfg.root_dir)?;
        std::fs::remove_dir_all(&game.path).map_err(|e| {
            fill(
                &pick(
                    "Не удалось удалить {path}: {err}",
                    "Failed to delete {path}: {err}",
                ),
                &[
                    ("path", &game.path.display().to_string()),
                    ("err", &e.to_string()),
                ],
            )
        })?;
        let name = game.display_name();
        c.log(fill(
            &pick("Удалена сборка {name}", "Build {name} deleted"),
            &[("name", &name)],
        ));
        Ok(fill(
            &pick("Сборка {name} удалена", "Build {name} deleted"),
            &[("name", &name)],
        ))
    });
    p
}
