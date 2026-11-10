//! Распаковка архивов и установка сборок/модов.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::i18n::{fill, pick};
use crate::library::ModFile;
use crate::progress::Progress;
use crate::util::{join_safe, move_contents, remove_dir_quiet, safe_relative};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ArchiveKind {
    Zip,
    Rar,
    SevenZip,
    Unknown,
}

pub fn detect_kind(path: &Path) -> ArchiveKind {
    let mut buffer = [0u8; 8];
    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return ArchiveKind::Unknown,
    };
    let read = file.read(&mut buffer).unwrap_or(0);
    if read >= 4 && (&buffer[..4] == b"PK\x03\x04" || &buffer[..4] == b"PK\x05\x06") {
        ArchiveKind::Zip
    } else if read >= 4 && &buffer[..4] == b"Rar!" {
        ArchiveKind::Rar
    } else if read >= 6 && &buffer[..6] == b"7z\xBC\xAF\x27\x1C" {
        ArchiveKind::SevenZip
    } else {
        ArchiveKind::Unknown
    }
}

pub fn extract_archive(archive: &Path, dest: &Path, progress: &Progress) -> Result<(), String> {
    fs::create_dir_all(dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    match detect_kind(archive) {
        ArchiveKind::Zip => extract_zip(archive, dest, progress),
        ArchiveKind::Rar => extract_rar(archive, dest, progress),
        ArchiveKind::SevenZip => extract_7z(archive, dest, progress),
        ArchiveKind::Unknown => Err(fill(
            &pick(
                "Не удалось определить формат архива: {path}",
                "Could not detect the archive format: {path}",
            ),
            &[("path", &archive.display().to_string())],
        )),
    }
}

// sevenz-rust2 декодирует solid-блок одним последовательным проходом (включая
// BCJ2), поэтому большие сборки распаковываются за минуты, а не часы.
fn extract_7z(archive: &Path, dest: &Path, progress: &Progress) -> Result<(), String> {
    use sevenz_rust2::{ArchiveReader, EntryDecision, Password};

    let mut reader = ArchiveReader::open(archive, Password::empty()).map_err(|e| {
        fill(
            &pick(
                "Не удалось открыть 7z-архив: {err}",
                "Failed to open the 7z archive: {err}",
            ),
            &[("err", &e.to_string())],
        )
    })?;
    let count = reader.archive().files.len();
    let total: u64 = reader.archive().files.iter().map(|f| f.size).sum();
    progress.reset_bytes(total);

    let mut buffer = vec![0u8; 1024 * 1024];
    let mut index = 0usize;
    let result = reader.for_each_entries_with_decision(|entry, src| {
        index += 1;
        progress.files(index, count);
        let name = entry.name.as_str();
        let Some(rel) = safe_relative(name) else {
            return Ok(EntryDecision::Continue); // защита от выхода за пределы каталога
        };
        let out_path = dest.join(&rel);
        if entry.is_directory {
            fs::create_dir_all(&out_path)?;
            return Ok(EntryDecision::Continue);
        }
        progress.detail(crate::util::trim_to(name, 70));
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut out = File::create(&out_path)?;
        loop {
            if progress.is_cancelled() {
                return Ok(EntryDecision::Stop);
            }
            let read = src.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            out.write_all(&buffer[..read])?;
            progress.add_done(read as u64);
        }
        Ok(EntryDecision::Continue)
    });

    if progress.is_cancelled() {
        return Err(pick("Распаковка отменена", "Extraction cancelled").to_string());
    }
    result.map_err(|e| {
        fill(
            &pick("Ошибка распаковки 7z: {err}", "7z extraction error: {err}"),
            &[("err", &e.to_string())],
        )
    })
}

fn extract_zip(archive: &Path, dest: &Path, progress: &Progress) -> Result<(), String> {
    let file = File::open(archive).map_err(|e| format!("{}: {e}", archive.display()))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| {
        fill(
            &pick(
                "Повреждённый ZIP-архив: {err}",
                "Corrupted ZIP archive: {err}",
            ),
            &[("err", &e.to_string())],
        )
    })?;
    let count = zip.len();

    let mut total = 0u64;
    for i in 0..count {
        if let Ok(entry) = zip.by_index(i) {
            total = total.saturating_add(entry.size());
        }
    }
    progress.reset_bytes(total);

    let mut buffer = vec![0u8; 512 * 1024];
    for i in 0..count {
        if progress.is_cancelled() {
            return Err(pick("Распаковка отменена", "Extraction cancelled").to_string());
        }
        let mut entry = zip.by_index(i).map_err(|e| {
            fill(
                &pick("Ошибка чтения архива: {err}", "Archive read error: {err}"),
                &[("err", &e.to_string())],
            )
        })?;
        let raw_name = entry.name().to_string();
        let Some(rel) = safe_relative(&raw_name) else {
            continue; // защита от выхода за пределы каталога
        };
        let out_path = dest.join(&rel);
        if entry.is_dir() || raw_name.ends_with('/') {
            fs::create_dir_all(&out_path).map_err(|e| format!("{}: {e}", out_path.display()))?;
            continue;
        }
        progress.detail(crate::util::trim_to(&raw_name, 70));
        progress.files(i + 1, count);
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        let mut out =
            File::create(&out_path).map_err(|e| format!("{}: {e}", out_path.display()))?;
        loop {
            let read = entry.read(&mut buffer).map_err(|e| {
                fill(
                    &pick(
                        "Ошибка распаковки {name}: {err}",
                        "Extraction error for {name}: {err}",
                    ),
                    &[("name", &raw_name), ("err", &e.to_string())],
                )
            })?;
            if read == 0 {
                break;
            }
            out.write_all(&buffer[..read]).map_err(|e| {
                fill(
                    &pick(
                        "Ошибка записи {path}: {err}",
                        "Write error for {path}: {err}",
                    ),
                    &[
                        ("path", &out_path.display().to_string()),
                        ("err", &e.to_string()),
                    ],
                )
            })?;
            progress.add_done(read as u64);
        }
    }
    Ok(())
}

fn extract_rar(archive: &Path, dest: &Path, progress: &Progress) -> Result<(), String> {
    let mut archive = unrar::Archive::new(archive)
        .open_for_processing()
        .map_err(|e| {
            fill(
                &pick(
                    "Не удалось открыть RAR: {err}",
                    "Failed to open the RAR: {err}",
                ),
                &[("err", &e.to_string())],
            )
        })?;
    let mut index = 0usize;
    loop {
        if progress.is_cancelled() {
            return Err(pick("Распаковка отменена", "Extraction cancelled").to_string());
        }
        let header = match archive.read_header() {
            Ok(Some(h)) => h,
            Ok(None) => break,
            Err(e) => {
                return Err(fill(
                    &pick("Ошибка RAR-архива: {err}", "RAR archive error: {err}"),
                    &[("err", &e.to_string())],
                ))
            }
        };
        let entry = header.entry();
        let name = entry.filename.to_string_lossy().replace('\\', "/");
        let is_dir = entry.is_directory();
        index += 1;
        progress.files(index, 0);
        progress.detail(crate::util::trim_to(&name, 70));

        let target = match safe_relative(&name) {
            Some(rel) => dest.join(rel),
            None => {
                archive = header.skip().map_err(|e| {
                    fill(
                        &pick("Ошибка RAR: {err}", "RAR error: {err}"),
                        &[("err", &e.to_string())],
                    )
                })?;
                continue;
            }
        };

        if is_dir {
            fs::create_dir_all(&target).map_err(|e| format!("{}: {e}", target.display()))?;
            archive = header.skip().map_err(|e| {
                fill(
                    &pick("Ошибка RAR: {err}", "RAR error: {err}"),
                    &[("err", &e.to_string())],
                )
            })?;
        } else {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
            }
            archive = header.extract_to(&target).map_err(|e| {
                fill(
                    &pick(
                        "Ошибка распаковки {name}: {err}",
                        "Extraction error for {name}: {err}",
                    ),
                    &[("name", &name), ("err", &e.to_string())],
                )
            })?;
        }
    }
    Ok(())
}

fn has_game_marker(dir: &Path) -> bool {
    const MARKERS: [&str; 4] = [
        "yanderesimulator.exe",
        "unityplayer.dll",
        "yanderesimulator_data",
        "yanderesimulator_Data",
    ];
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let lower = name.to_ascii_lowercase();
            if MARKERS.iter().any(|m| lower == m.to_ascii_lowercase()) {
                return true;
            }
        }
    }
    false
}

fn subdirs(dir: &Path) -> Vec<PathBuf> {
    let mut result = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                result.push(path);
            }
        }
    }
    result.sort();
    result
}

/// Находит настоящий корень игры внутри распакованного каталога.
pub fn find_game_root(dir: &Path) -> PathBuf {
    if has_game_marker(dir) {
        return dir.to_path_buf();
    }
    // Глубина 1: иногда сборка лежит в единственной вложенной папке.
    for child in subdirs(dir) {
        if has_game_marker(&child) {
            return child;
        }
    }
    // Глубина 2: папка вида "YandereSimulator/YandereSimulator_Data".
    for child in subdirs(dir) {
        for grandchild in subdirs(&child) {
            if has_game_marker(&grandchild) {
                return grandchild;
            }
        }
    }
    // Ничего не нашли — считаем корнем единственную папку (или сам каталог).
    let dirs = subdirs(dir);
    let files = fs::read_dir(dir)
        .map(|e| e.flatten().filter(|x| x.path().is_file()).count())
        .unwrap_or(0);
    if dirs.len() == 1 && files == 0 {
        dirs.into_iter().next().unwrap()
    } else {
        dir.to_path_buf()
    }
}

/// Распаковывает архив сборки во временный каталог и переносит игру в `target`.
pub fn install_game(
    archive: &Path,
    target: &Path,
    staging: &Path,
    progress: &Progress,
) -> Result<PathBuf, String> {
    remove_dir_quiet(staging);
    progress.stage(pick("Распаковка архива", "Extracting the archive"));
    extract_archive(archive, staging, progress)?;

    progress.stage(pick("Поиск файлов игры", "Looking for game files"));
    let root = find_game_root(staging);
    progress.stage(pick("Перенос файлов игры", "Moving game files"));
    fs::create_dir_all(target).map_err(|e| format!("{}: {e}", target.display()))?;
    move_contents(&root, target)?;
    remove_dir_quiet(staging);
    Ok(target.to_path_buf())
}

fn collect_files(dir: &Path, base: &Path, out: &mut Vec<(PathBuf, String)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, base, out);
        } else if let Ok(rel) = path.strip_prefix(base) {
            let rel_str = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().to_string())
                .collect::<Vec<_>>()
                .join("/");
            out.push((path.clone(), rel_str));
        }
    }
}

/// Корень установки мода: ищем папку, содержимое которой нужно положить в игру.
fn mod_root(dir: &Path) -> PathBuf {
    if has_game_marker(dir) {
        return dir.to_path_buf();
    }
    let dirs = subdirs(dir);
    let files = fs::read_dir(dir)
        .map(|e| e.flatten().filter(|x| x.path().is_file()).count())
        .unwrap_or(0);
    // Типичные корни модов.
    for child in &dirs {
        let name = child
            .file_name()
            .map(|n| n.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        if matches!(
            name.as_str(),
            "bepinex" | "yanderesimulator_data" | "mods" | "plugins"
        ) {
            return dir.to_path_buf();
        }
    }
    if dirs.len() == 1 && files == 0 {
        let candidate = dirs.into_iter().next().unwrap();
        if has_game_marker(&candidate) || candidate.join("BepInEx").exists() {
            return candidate;
        }
        return mod_root(&candidate);
    }
    dir.to_path_buf()
}

/// Устанавливает мод в каталог игры, сохраняя резервные копии заменённых файлов.
pub fn install_mod(
    archive: &Path,
    game_dir: &Path,
    backup_dir: &Path,
    staging: &Path,
    progress: &Progress,
) -> Result<Vec<ModFile>, String> {
    remove_dir_quiet(staging);
    progress.stage(pick("Распаковка мода", "Extracting the mod"));
    extract_archive(archive, staging, progress)?;

    progress.stage(pick("Поиск файлов мода", "Looking for mod files"));
    let root = mod_root(staging);
    let mut files = Vec::new();
    collect_files(&root, &root, &mut files);
    if files.is_empty() {
        remove_dir_quiet(staging);
        return Err(pick(
            "В архиве не найдено файлов",
            "No files found in the archive",
        )
        .to_string());
    }

    progress.stage(pick("Установка файлов мода", "Installing mod files"));
    progress.total(files.len() as u64);
    progress.set_done(0);
    let mut result = Vec::new();
    for (index, (source, rel)) in files.iter().enumerate() {
        if progress.is_cancelled() {
            remove_dir_quiet(staging);
            return Err(pick("Установка отменена", "Installation cancelled").to_string());
        }
        let Some(target) = join_safe(game_dir, rel) else {
            continue;
        };
        progress.detail(crate::util::trim_to(rel, 70));
        if target.exists() {
            let backup = join_safe(backup_dir, rel).ok_or_else(|| {
                fill(
                    &pick(
                        "Некорректный путь резервной копии: {path}",
                        "Invalid backup path: {path}",
                    ),
                    &[("path", rel)],
                )
            })?;
            if let Some(parent) = backup.parent() {
                fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
            }
            fs::copy(&target, &backup).map_err(|e| {
                fill(
                    &pick(
                        "Не удалось создать резервную копию {path}: {err}",
                        "Failed to back up {path}: {err}",
                    ),
                    &[("path", rel), ("err", &e.to_string())],
                )
            })?;
            result.push(ModFile {
                path: rel.clone(),
                replaced: true,
            });
        } else {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
            }
            result.push(ModFile {
                path: rel.clone(),
                replaced: false,
            });
        }
        fs::copy(source, &target).map_err(|e| {
            fill(
                &pick(
                    "Не удалось записать {path}: {err}",
                    "Failed to write {path}: {err}",
                ),
                &[("path", rel), ("err", &e.to_string())],
            )
        })?;
        progress.set_done(index as u64 + 1);
    }
    remove_dir_quiet(staging);
    Ok(result)
}

/// Откатывает установку мода: добавленные файлы удаляются, заменённые восстанавливаются.
pub fn uninstall_mod(game_dir: &Path, backup_dir: &Path, files: &[ModFile]) -> Result<(), String> {
    for file in files.iter().rev() {
        let Some(target) = join_safe(game_dir, &file.path) else {
            continue;
        };
        if file.replaced {
            let backup = join_safe(backup_dir, &file.path);
            if let Some(backup) = backup {
                if backup.exists() {
                    if let Some(parent) = target.parent() {
                        let _ = fs::create_dir_all(parent);
                    }
                    let _ = fs::copy(&backup, &target);
                }
            }
        } else if target.exists() {
            fs::remove_file(&target).map_err(|e| {
                fill(
                    &pick(
                        "Не удалось удалить {path}: {err}",
                        "Failed to delete {path}: {err}",
                    ),
                    &[
                        ("path", &target.display().to_string()),
                        ("err", &e.to_string()),
                    ],
                )
            })?;
        }
    }
    remove_dir_quiet(backup_dir);
    remove_empty_dirs(game_dir, 0);
    Ok(())
}

fn remove_empty_dirs(dir: &Path, depth: usize) {
    if depth > 12 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name == ".yanlan" {
                continue;
            }
            remove_empty_dirs(&path, depth + 1);
            let is_empty = fs::read_dir(&path)
                .map(|mut e| e.next().is_none())
                .unwrap_or(false);
            if is_empty {
                let _ = fs::remove_dir(&path);
            }
        }
    }
}

#[allow(dead_code)]
pub fn count_files(dir: &Path) -> usize {
    let mut files = Vec::new();
    collect_files(dir, dir, &mut files);
    files.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yanlan_test_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn detects_unknown_format() {
        let dir = temp_dir("detect");
        let path = dir.join("file.bin");
        let _ = fs::write(&path, b"hello");
        assert_eq!(detect_kind(&path), ArchiveKind::Unknown);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn installs_and_restores_mod_files() {
        let root = temp_dir("mod");
        let game = root.join("game");
        fs::create_dir_all(game.join("YandereSimulator_Data")).unwrap();
        fs::write(game.join("YandereSimulator.exe"), b"exe").unwrap();

        let replaced = "YandereSimulator_Data/original.txt";
        let added = "YandereSimulator_Data/added.txt";
        fs::write(game.join(replaced), b"old").unwrap();

        let archive = root.join("mod.zip");
        {
            let file = File::create(&archive).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default();
            zip.start_file(replaced, options).unwrap();
            zip.write_all(b"new").unwrap();
            zip.start_file(added, options).unwrap();
            zip.write_all(b"added").unwrap();
            zip.finish().unwrap();
        }

        let progress = Progress::new();
        let backup = root.join("backups");
        let staging = root.join("staging");
        let files = install_mod(&archive, &game, &backup, &staging, &progress).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(fs::read_to_string(game.join(replaced)).unwrap(), "new");
        assert_eq!(fs::read_to_string(game.join(added)).unwrap(), "added");
        assert!(backup.join(replaced).exists());

        uninstall_mod(&game, &backup, &files).unwrap();
        assert_eq!(fs::read_to_string(game.join(replaced)).unwrap(), "old");
        assert!(!game.join(added).exists());
        assert!(!backup.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn extracts_seven_zip_archive() {
        let root = temp_dir("sevenzip");
        let archive = root.join("mod.7z");
        {
            let options = zesven::WriteOptions::new()
                .method(zesven::codec::CodecMethod::Lzma2)
                .level(1)
                .unwrap()
                .solid();
            let mut writer = zesven::Writer::create_path(&archive)
                .unwrap()
                .options(options);
            writer
                .add_bytes(
                    zesven::ArchivePath::new("YandereSimulator_Data/added.txt").unwrap(),
                    b"seven",
                )
                .unwrap();
            writer
                .add_bytes(
                    zesven::ArchivePath::new("YandereSimulator_Data/second.txt").unwrap(),
                    b"second",
                )
                .unwrap();
            writer
                .add_bytes(
                    zesven::ArchivePath::new("YandereSimulator_Data/empty.txt").unwrap(),
                    b"",
                )
                .unwrap();
            let _ = writer.finish().unwrap();
        }
        assert_eq!(detect_kind(&archive), ArchiveKind::SevenZip);

        let dest = root.join("out");
        let progress = Progress::new();
        extract_archive(&archive, &dest, &progress).unwrap();
        assert_eq!(
            fs::read_to_string(dest.join("YandereSimulator_Data/added.txt")).unwrap(),
            "seven"
        );
        assert_eq!(
            fs::read_to_string(dest.join("YandereSimulator_Data/second.txt")).unwrap(),
            "second"
        );
        assert!(dest.join("YandereSimulator_Data/empty.txt").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn finds_nested_game_root() {
        let root = temp_dir("root");
        let nested = root.join("YandereSimulator");
        fs::create_dir_all(nested.join("YandereSimulator_Data")).unwrap();
        fs::write(nested.join("YandereSimulator.exe"), b"exe").unwrap();
        assert_eq!(find_game_root(&root), nested);
        let _ = fs::remove_dir_all(root);
    }
}
