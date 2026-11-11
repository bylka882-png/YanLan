//! Фоновые задачи: сеть, установка, удаление.

use std::fs::File;
use std::io::copy;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::thread;

use crate::builds::{self, Build};
use crate::config::Config;
use crate::i18n::{fill, pick};
use crate::installer;
use crate::library::{update_library, write_meta, GameInstall, GameMeta, InstalledMod, Library};
use crate::mods::{self, ModVariant};
use crate::net;
use crate::progress::Progress;
use crate::util::{now_ts, sanitize_dir_name, sanitize_name};

pub enum Ev {
    Builds {
        builds: Vec<Build>,
        fetched_at: i64,
        error: Option<String>,
    },
    Catalog {
        mods: Vec<mods::ModGroup>,
        fetched_at: i64,
        error: Option<String>,
    },
    /// Готовый скриншот каталога (декодирование шло в фоне).
    Cover {
        key: String,
        result: Result<Cover, String>,
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
        Ok((list, warnings)) => {
            for warning in warnings.iter().take(3) {
                c.log(fill(
                    &pick("Зеркало пропущено: {err}", "Mirror skipped: {err}"),
                    &[("err", warning)],
                ));
            }
            if warnings.len() > 3 {
                c.log(fill(
                    &pick("Ещё {n} зеркал пропущено", "{n} more mirrors skipped"),
                    &[("n", &(warnings.len() - 3).to_string())],
                ));
            }
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

/// Загружает каталог модов YanMod и кладёт его в кэш.
pub fn fetch_catalog(c: Ctx2, cfg: Config) -> Progress {
    let progress = Progress::new();
    let p = progress.clone();
    spawn_data(c, progress, move |c, p| match mods::fetch(p) {
        Ok((list, warnings)) => {
            for warning in warnings.iter().take(3) {
                c.log(fill(
                    &pick("Манифест пропущен: {err}", "Manifest skipped: {err}"),
                    &[("err", warning)],
                ));
            }
            if warnings.len() > 3 {
                c.log(fill(
                    &pick("Ещё {n} манифестов пропущено", "{n} more manifests skipped"),
                    &[("n", &(warnings.len() - 3).to_string())],
                ));
            }
            mods::save_cache(&cfg, &list);
            c.emit(Ev::Catalog {
                mods: list,
                fetched_at: now_ts(),
                error: None,
            });
            Ok(pick("Каталог модов обновлён", "Mod catalog updated").to_string())
        }
        Err(err) => {
            c.emit(Ev::Catalog {
                mods: Vec::new(),
                fetched_at: now_ts(),
                error: Some(err.clone()),
            });
            Err(err)
        }
    });
    p
}

/// Декодированный скриншот: один кадр или анимация GIF.
pub struct Cover {
    pub frames: Vec<egui::ColorImage>,
    /// Задержка каждого кадра в секундах.
    pub delays: Vec<f32>,
}

/// Скачивает и декодирует один скриншот каталога в фоне.
pub fn fetch_cover(c: Ctx2, cache_dir: std::path::PathBuf, key: String, url: String) {
    thread::spawn(move || {
        let dir = cache_dir.join("covers");
        let file = dir.join(format!("{key}.img"));
        let broken = dir.join(format!("{key}.broken"));
        // Не декодировавшийся файл — это битые байты, сеть тут ни при чём:
        // помечаем его, чтобы не качать заново при каждом запуске.
        // `shortcut: метка живёт до очистки кэша; при обновлении картинки по
        // тому же URL её нужно удалить вручную.`
        let bytes = (|| -> Result<Vec<u8>, String> {
            if broken.exists() {
                return Err(format!("cover {key}: marked as broken"));
            }
            std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            if file.exists() {
                return std::fs::read(&file).map_err(|e| format!("{}: {e}", file.display()));
            }
            let bytes = net::get_bytes(&url)?;
            let _ = std::fs::write(&file, &bytes);
            Ok(bytes)
        })();
        let result = match bytes {
            Ok(bytes) => {
                let decoded = decode_cover(&bytes, &key);
                match &decoded {
                    Ok(_) => {
                        let _ = std::fs::remove_file(&broken);
                    }
                    Err(_) => {
                        let _ = std::fs::remove_file(&file);
                        let _ = std::fs::write(&broken, &url);
                    }
                }
                decoded
            }
            Err(err) => Err(err),
        };
        c.emit(Ev::Cover { key, result });
    });
}

fn decode_cover(bytes: &[u8], key: &str) -> Result<Cover, String> {
    let format = image::guess_format(bytes).map_err(|e| format!("cover {key}: {e}"))?;
    if format == image::ImageFormat::Gif {
        let decoder = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(bytes))
            .map_err(|e| format!("cover {key}: {e}"))?;
        let frames = image::AnimationDecoder::into_frames(decoder)
            .collect_frames()
            .map_err(|e| format!("cover {key}: {e}"))?;
        let mut out = Cover {
            frames: Vec::with_capacity(frames.len()),
            delays: Vec::with_capacity(frames.len()),
        };
        for frame in frames {
            let (num, den) = frame.delay().numer_denom_ms();
            let delay = if den == 0 {
                0.1
            } else {
                (num as f32 / den as f32 / 1000.0).clamp(0.02, 1.0)
            };
            let rgba = frame.into_buffer();
            let (width, height) = rgba.dimensions();
            out.delays.push(delay);
            out.frames.push(egui::ColorImage::from_rgba_unmultiplied(
                [width as usize, height as usize],
                rgba.as_raw(),
            ));
        }
        if out.frames.is_empty() {
            return Err(format!("cover {key}: empty gif"));
        }
        return Ok(out);
    }

    let image = image::load_from_memory(bytes).map_err(|e| format!("cover {key}: {e}"))?;
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    Ok(Cover {
        frames: vec![egui::ColorImage::from_rgba_unmultiplied(
            [width as usize, height as usize],
            rgba.as_raw(),
        )],
        delays: vec![0.0],
    })
}

fn download_catalog(
    c: &Ctx2,
    cfg: &Config,
    variant: &ModVariant,
    progress: &Progress,
) -> Result<PathBuf, String> {
    if variant.dl_url.trim().is_empty() {
        return Err(fill(
            &pick(
                "У мода «{name}» нет ссылки на скачивание",
                "The mod “{name}” has no download link",
            ),
            &[("name", &variant.name)],
        ));
    }
    let filename = variant.download_name();
    let dest = cfg.downloads_dir().join(&filename);
    if dest.exists() {
        progress.stage(pick("Архив уже скачан", "Archive already downloaded"));
        c.log(fill(
            &pick("Используем {path}", "Using {path}"),
            &[("path", &dest.display().to_string())],
        ));
        return Ok(dest);
    }
    progress.stage(pick("Скачивание", "Downloading"));
    progress.detail(&filename);
    let parts = variant.dl_parts();
    c.log(fill(
        &pick("Скачивание: {url}", "Downloading: {url}"),
        &[("url", &parts.join(", "))],
    ));
    if parts.len() <= 1 {
        let url = parts.first().map(String::as_str).unwrap_or(&variant.dl_url);
        net::download(url, &dest, progress, cfg.retry_forever)?;
        return Ok(dest);
    }

    // Многотомный архив: тома — куски одного потока, их надо скачать
    // по порядку и склеить, иначе распаковщик увидит только первый кусок.
    let stem = dest
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "archive".to_string());
    let files: Vec<PathBuf> = (1..=parts.len())
        .map(|index| cfg.downloads_dir().join(format!("{stem}.part{index}")))
        .collect();
    let joined = (|| -> Result<(), String> {
        for (index, (url, part)) in parts.iter().zip(&files).enumerate() {
            progress.detail(fill(
                &pick(
                    "{name} — том {index} из {total}",
                    "{name} — part {index} of {total}",
                ),
                &[
                    ("name", &filename),
                    ("index", &(index + 1).to_string()),
                    ("total", &parts.len().to_string()),
                ],
            ));
            net::download(url, part, progress, cfg.retry_forever)?;
        }

        progress.stage(pick("Склейка томов", "Joining parts"));
        progress.detail(&filename);
        let mut out = File::create(&dest).map_err(|e| format!("{}: {e}", dest.display()))?;
        for part in &files {
            let mut src = File::open(part).map_err(|e| format!("{}: {e}", part.display()))?;
            copy(&mut src, &mut out).map_err(|e| format!("{}: {e}", dest.display()))?;
        }
        Ok(())
    })();
    if let Err(err) = joined {
        let _ = std::fs::remove_file(&dest);
        // Отмена — не ошибка в данных: оставляем тома для докачки.
        if !progress.is_cancelled() {
            for part in &files {
                let _ = std::fs::remove_file(part);
                // Незавершённый том лежит как `<том>.part` (см. net::download).
                let name = part.file_name().unwrap_or_default().to_string_lossy();
                let _ = std::fs::remove_file(part.with_file_name(format!("{name}.part")));
            }
        }
        return Err(err);
    }
    if !cfg.keep_archives {
        for part in &files {
            let _ = std::fs::remove_file(part);
        }
    }
    Ok(dest)
}

/// Скачивает архив мода в `downloads/`, не устанавливая его.
pub fn download_catalog_mod(c: Ctx2, cfg: Config, variant: ModVariant) -> Progress {
    let progress = Progress::new();
    let p = progress.clone();
    spawn(c, progress, move |c, p| {
        cfg.ensure_dirs();
        download_catalog(c, &cfg, &variant, p)?;
        Ok(fill(
            &pick("«{name}» скачан", "“{name}” downloaded"),
            &[("name", &variant.name)],
        ))
    });
    p
}

/// Устанавливает моды из каталога в выбранную сборку.
///
/// Список уже упорядочен: зависимости идут раньше основного мода.
pub fn install_catalog_mods(
    c: Ctx2,
    cfg: Config,
    game: GameInstall,
    items: Vec<ModVariant>,
) -> Progress {
    let progress = Progress::new();
    let p = progress.clone();
    spawn(c, progress, move |c, p| {
        if items.is_empty() {
            return Err(pick("Нечего устанавливать", "Nothing to install").to_string());
        }
        let total = items.len();
        let mut installed = Vec::new();

        let result = (|| -> Result<(), String> {
            for (index, variant) in items.iter().enumerate() {
                if p.is_cancelled() {
                    return Err(pick("Установка отменена", "Installation cancelled").to_string());
                }
                p.stage(fill(
                    &pick(
                        "Установка {name} ({index}/{total})",
                        "Installing {name} ({index}/{total})",
                    ),
                    &[
                        ("name", &variant.name),
                        ("index", &(index + 1).to_string()),
                        ("total", &total.to_string()),
                    ],
                ));
                if !variant.version.is_empty() {
                    p.detail(fill(
                        &pick("версия {v}", "version {v}"),
                        &[("v", &variant.version)],
                    ));
                }
                let archive = download_catalog(c, &cfg, variant, p)?;

                // Ключ включает номер в очереди: установка нескольких модов
                // за одну секунду не должна перетирать чужие резервные копии.
                let key = format!("{}-{}-{}", sanitize_name(&variant.id), now_ts(), index);
                let backup_dir = game.backups_dir().join(&key);
                // У каждой задачи свой staging: параллельные установки в одну
                // сборку не должны удалять каталоги друг друга.
                let staging = game.work_dir().join(format!("tmp-{}", p.id()));
                let files = installer::install_mod(&archive, &game.path, &backup_dir, &staging, p)?;

                let mut item = InstalledMod::new(
                    &variant.id,
                    &variant.name,
                    variant.ty.code(),
                    &variant.author,
                    &game.meta.id,
                    files,
                );
                item.key = key;
                item.cfg = variant.cfg.clone();
                item.version = variant.version.clone();
                if let Err(err) = update_library(&cfg.root_dir, |library| {
                    library.add(item.clone());
                    Ok(())
                }) {
                    // Файлы уже легли в игру, а запись не сохранилась — откат.
                    let backup_dir = game.backups_dir().join(&item.key);
                    let _ = installer::uninstall_mod(&game.path, &backup_dir, &item.files);
                    return Err(err);
                }

                if !cfg.keep_archives {
                    let _ = std::fs::remove_file(&archive);
                }
                installed.push(variant.name.clone());
            }
            Ok(())
        })();

        if let Err(err) = result {
            if installed.is_empty() {
                return Err(err);
            }
            c.log(fill(
                &pick(
                    "Установка прервана. Уже установлено: {list}",
                    "Installation interrupted. Already installed: {list}",
                ),
                &[("list", &installed.join(", "))],
            ));
            return Err(fill(
                &pick(
                    "Установка прервана. Уже установлено: {list}. {err}",
                    "Installation interrupted. Already installed: {list}. {err}",
                ),
                &[("list", &installed.join(", ")), ("err", &err)],
            ));
        }

        let names = installed.join(", ");
        c.log(fill(
            &pick(
                "Установлено в {build}: {list}",
                "Installed into {build}: {list}",
            ),
            &[("build", &game.display_name()), ("list", &names)],
        ));
        Ok(fill(
            &pick(
                "«{list}» установлены в {build}",
                "“{list}” installed into {build}",
            ),
            &[("list", &names), ("build", &game.display_name())],
        ))
    });
    p
}

/// Устанавливает мод типа `Build` как отдельную сборку игры.
pub fn install_catalog_build(c: Ctx2, cfg: Config, variant: ModVariant, name: String) -> Progress {
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

        let archive = download_catalog(c, &cfg, &variant, p)?;
        p.stage(pick("Установка сборки", "Installing build"));
        p.detail(label.clone());
        let staging = cfg.games_dir().join(format!(".staging-{dir_name}"));
        installer::install_game(&archive, &target, &staging, p)?;

        let meta = GameMeta {
            id: dir_name,
            label: label.clone(),
            version: variant.version.clone(),
            date: None,
            source: format!("YanMod · {}", variant.name),
            installed_at: now_ts(),
            args: String::new(),
            ioa: variant.ioa.clone(),
        };
        write_meta(&target, &meta)?;

        if !cfg.keep_archives {
            let _ = std::fs::remove_file(&archive);
        }
        Ok(fill(
            &pick("Сборка «{name}» создана", "Build “{name}” created"),
            &[("name", &label)],
        ))
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
            ioa: Vec::new(),
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
        let staging = game.work_dir().join(format!("tmp-{}", p.id()));
        p.stage(pick("Установка мода", "Installing mod"));
        let files = installer::install_mod(&archive, &game.path, &backup_dir, &staging, p)?;
        let mut item = InstalledMod::new(&name, &name, "mod", "", &game.meta.id, files);
        item.key = key;
        if let Err(err) = update_library(&cfg.root_dir, |library| {
            library.add(item.clone());
            Ok(())
        }) {
            let backup_dir = game.backups_dir().join(&item.key);
            let _ = installer::uninstall_mod(&game.path, &backup_dir, &item.files);
            return Err(err);
        }
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
        // Сначала убираем файлы: если это не удалось, запись в библиотеке
        // должна остаться, иначе потеряются пути к резервным копиям.
        let item = Library::load(&cfg.root_dir)
            .mods
            .into_iter()
            .find(|m| m.key == key)
            .ok_or_else(|| pick("Запись о моде не найдена", "Mod record not found").to_string())?;
        p.stage(pick("Удаление мода", "Uninstalling mod"));
        let backup_dir = game.backups_dir().join(&item.key);
        installer::uninstall_mod(&game.path, &backup_dir, &item.files)?;
        update_library(&cfg.root_dir, |library| {
            library.remove(&key);
            Ok(())
        })?;
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
        // Папки уже нет — теперь убираем её записи из библиотеки.
        update_library(&cfg.root_dir, |library| {
            let keys: Vec<String> = library
                .for_game(&game.meta.id)
                .map(|m| m.key.clone())
                .collect();
            for key in keys {
                library.remove(&key);
            }
            Ok(())
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
