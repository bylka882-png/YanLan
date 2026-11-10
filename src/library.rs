//! Библиотека: установленные сборки игры и моды.
//!
//! Сборки находятся простым сканированием `games/`, а метаданные каждой сборки
//! лежат рядом с игрой в файле `.yanlan.json` — библиотеку можно переносить
//! вместе с папкой.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::i18n::{fill, pick};
use crate::util::{dir_size, now_ts};

pub const META_FILE: &str = ".yanlan.json";
pub const WORK_DIR: &str = ".yanlan";

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct GameMeta {
    #[serde(default)]
    pub id: String,
    /// Название сборки, которое задал пользователь.
    #[serde(default)]
    pub label: String,
    /// Версия игры, из которой собрана эта сборка.
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub date: Option<String>,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub installed_at: i64,
    /// Дополнительные параметры командной строки при запуске игры.
    #[serde(default)]
    pub args: String,
    /// Моды, уже входящие в сборку (`ioa: "PoseMod; 5"` в её манифесте).
    #[serde(default)]
    pub ioa: Vec<String>,
}

#[derive(Clone)]
pub struct GameInstall {
    pub meta: GameMeta,
    pub path: PathBuf,
    pub size: u64,
    pub exe: Option<PathBuf>,
    pub mods: usize,
    /// Версия Unity из ресурсов `YandereSimulator.exe` (может быть пустой).
    pub unity: String,
}

impl GameInstall {
    pub fn work_dir(&self) -> PathBuf {
        self.path.join(WORK_DIR)
    }

    pub fn backups_dir(&self) -> PathBuf {
        self.work_dir().join("backups")
    }

    /// Название сборки для интерфейса.
    ///
    /// Сборки, созданные старыми версиями лаунчера, не имеют пользовательского
    /// названия: в поле `label` у них лежит либо id папки, либо версия. Показывать
    /// такое напрямую нельзя — собираем понятную подпись.
    pub fn display_name(&self) -> String {
        let label = self.meta.label.trim();
        let id = self.meta.id.trim();
        if !label.is_empty() && label != id && !is_placeholder_label(label) {
            return label.to_string();
        }
        let version = self.meta.version.trim();
        if !version.is_empty() && version != id {
            return fill(
                &pick("Сборка {version}", "Build {version}"),
                &[("version", version)],
            );
        }
        if let Some(date) = &self.meta.date {
            return fill(
                &pick("Сборка от {date}", "Build from {date}"),
                &[("date", &crate::util::format_date(date))],
            );
        }
        if !label.is_empty() {
            return fill(
                &pick("Сборка {label}", "Build {label}"),
                &[("label", label)],
            );
        }
        pick("Сборка без названия", "Unnamed build").to_string()
    }

    /// Похоже ли, что у сборки нет настоящего названия (можно предложить переименовать).
    #[allow(dead_code)]
    pub fn needs_name(&self) -> bool {
        let label = self.meta.label.trim();
        label.is_empty()
            || label == self.meta.id.trim()
            || label == self.meta.version.trim()
            || is_placeholder_label(label)
    }

    /// Версия Unity для карточки сборки.
    ///
    /// Предпочитаем настоящую версию из свойств `YandereSimulator.exe`, а если
    /// её нет — определяем поколение Unity по дате сборки.
    pub fn unity_label(&self) -> Option<String> {
        if let Some(label) = unity_from_version(&self.unity) {
            return Some(label);
        }
        if self.meta.date.as_deref() == Some("2020-04-10")
            && self.meta.version.to_lowercase().contains("v2")
        {
            return Some("Unity 2019".to_string());
        }
        crate::builds::unity_for_date(self.meta.date.as_deref()).map(str::to_string)
    }
}

/// Старые версии лаунчера писали в `label` общую подпись вместо названия.
/// Встречается и «Cборка» с латинской `C` — сравниваем без учёта регистра
/// и обоих вариантов первой буквы.
fn is_placeholder_label(label: &str) -> bool {
    matches!(
        label.to_lowercase().as_str(),
        "сборка"
            | "cборка"
            | "build"
            | "сборка без названия"
            | "cборка без названия"
            | "unnamed build"
    )
}

fn read_meta(dir: &Path) -> Option<GameMeta> {
    let text = std::fs::read_to_string(dir.join(META_FILE)).ok()?;
    serde_json::from_str(crate::util::strip_bom(&text)).ok()
}

pub fn write_meta(dir: &Path, meta: &GameMeta) -> Result<(), String> {
    let text = serde_json::to_string_pretty(meta).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(META_FILE), text).map_err(|e| {
        fill(
            &pick(
                "Не удалось записать метаданные сборки: {err}",
                "Failed to write build metadata: {err}",
            ),
            &[("err", &e.to_string())],
        )
    })
}

/// Ищет исполняемый файл для запуска сборки.
///
/// Если в папке игры лежит Pose-мод (`PoseMod.exe`, `Pose32.exe`, `Pose64.exe`),
/// запускать нужно именно его — иначе мод не загрузится.
pub fn find_exe(dir: &Path) -> Option<PathBuf> {
    let game = find_game_exe(dir);
    let bits = game.as_deref().and_then(pe_bits);
    let pose = find_pose_exe(dir, bits);
    pose.or(game)
}

fn find_game_exe(dir: &Path) -> Option<PathBuf> {
    let mut best: Option<PathBuf> = None;
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if name == "yanderesimulator.exe" {
            return Some(path);
        }
        if name.ends_with(".exe") && best.is_none() {
            best = Some(path);
        }
    }
    best
}

fn find_pose_exe(dir: &Path, bits: Option<u8>) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if name.ends_with(".exe") && name.contains("pose") {
            candidates.push(path);
        }
    }
    candidates.sort();
    pick_pose(candidates, bits)
}

/// Из нескольких Pose-запускателей выбирает подходящий по разрядности сборки.
fn pick_pose(mut candidates: Vec<PathBuf>, bits: Option<u8>) -> Option<PathBuf> {
    match candidates.len() {
        0 => None,
        1 => candidates.pop(),
        _ => {
            if let Some(bits) = bits {
                let want = bits.to_string();
                let other = if bits == 64 { "32" } else { "64" };
                if let Some(found) = candidates.iter().find(|path| {
                    let name = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_lowercase();
                    name.contains(&want) && !name.contains(other)
                }) {
                    return Some(found.clone());
                }
            }
            candidates.into_iter().next()
        }
    }
}

/// Разрядность PE-файла: 32 или 64. `None` — файл не PE или не читается.
fn pe_bits(path: &Path) -> Option<u8> {
    use std::io::{Read, Seek, SeekFrom};

    let mut file = std::fs::File::open(path).ok()?;
    let mut head = [0u8; 0x40];
    file.read_exact(&mut head).ok()?;
    if &head[0..2] != b"MZ" {
        return None;
    }
    let offset = u32::from_le_bytes([head[0x3C], head[0x3D], head[0x3E], head[0x3F]]) as u64;
    file.seek(SeekFrom::Start(offset)).ok()?;
    let mut signature = [0u8; 6];
    file.read_exact(&mut signature).ok()?;
    if &signature[0..4] != b"PE\0\0" {
        return None;
    }
    match u16::from_le_bytes([signature[4], signature[5]]) {
        0x8664 => Some(64),
        0x014C => Some(32),
        _ => None,
    }
}

/// Похоже ли значение из свойств exe на версию Unity (`2019.4.40`, `5.6.7`,
/// `6.0.23`). Иначе это версия самой игры вроде `1.0.0.0`.
fn looks_like_unity_version(version: &str) -> bool {
    let first: String = version
        .trim()
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    match first.parse::<u32>() {
        Ok(4 | 5 | 6) => true,
        Ok(year) => (2017..=2035).contains(&year),
        Err(_) => false,
    }
}

fn unity_from_version(version: &str) -> Option<String> {
    let version = version.trim();
    if version.is_empty() || !looks_like_unity_version(version) {
        return None;
    }
    // У Unity 4/5 версия трёхсоставная, у годовых — `2019.4.40f1`.
    // Четвёртый компонент (номер сборки) отбрасываем — он ничего не говорит.
    let parts: Vec<&str> = version.split('.').collect();
    let core = if parts.len() > 3 {
        parts[..3].join(".")
    } else {
        version.to_string()
    };
    Some(format!("Unity {core}"))
}

/// Читает «Версия файла» / «Версия продукта» из ресурсов Windows PE.
#[cfg(windows)]
fn exe_version(path: &Path) -> Option<String> {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "version")]
    extern "system" {
        fn GetFileVersionInfoSizeW(filename: *const u16, handle: *mut u32) -> u32;
        fn GetFileVersionInfoW(
            filename: *const u16,
            handle: u32,
            len: u32,
            data: *mut c_void,
        ) -> i32;
        fn VerQueryValueW(
            block: *const c_void,
            sub_block: *const u16,
            buffer: *mut *mut c_void,
            len: *mut u32,
        ) -> i32;
    }

    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        let size = GetFileVersionInfoSizeW(wide.as_ptr(), std::ptr::null_mut());
        if size == 0 {
            return None;
        }
        let mut data = vec![0u8; size as usize];
        if GetFileVersionInfoW(wide.as_ptr(), 0, size, data.as_mut_ptr() as *mut c_void) == 0 {
            return None;
        }

        // Язык и кодовая страница ресурса; чаще всего 0409 (английский).
        let mut lang = 0x0409u16;
        let mut codepage = 0x04B0u16;
        let translation: Vec<u16> = "\\VarFileInfo\\Translation\0".encode_utf16().collect();
        let mut ptr: *mut c_void = std::ptr::null_mut();
        let mut len = 0u32;
        if VerQueryValueW(
            data.as_ptr() as *const c_void,
            translation.as_ptr(),
            &mut ptr,
            &mut len,
        ) != 0
            && len >= 4
            && !ptr.is_null()
        {
            let pair = std::slice::from_raw_parts(ptr as *const u16, 2);
            lang = pair[0];
            codepage = pair[1];
        }

        let mut found: Vec<String> = Vec::new();
        for key in ["FileVersion", "ProductVersion"] {
            let sub = format!("\\StringFileInfo\\{lang:04x}{codepage:04x}\\{key}\0");
            let sub: Vec<u16> = sub.encode_utf16().collect();
            let mut buffer: *mut c_void = std::ptr::null_mut();
            let mut length = 0u32;
            if VerQueryValueW(
                data.as_ptr() as *const c_void,
                sub.as_ptr(),
                &mut buffer,
                &mut length,
            ) != 0
                && !buffer.is_null()
                && length > 0
            {
                let text = String::from_utf16_lossy(std::slice::from_raw_parts(
                    buffer as *const u16,
                    length as usize,
                ));
                let text = text.trim_matches('\0').trim();
                if !text.is_empty() {
                    found.push(text.to_string());
                }
            }
        }
        found
            .iter()
            .find(|v| looks_like_unity_version(v))
            .cloned()
            .or_else(|| found.into_iter().next())
    }
}

#[cfg(not(windows))]
fn exe_version(_path: &Path) -> Option<String> {
    None
}

pub fn scan_games(games_dir: &Path, mods: &[InstalledMod]) -> Vec<GameInstall> {
    let mut result = Vec::new();
    let entries = match std::fs::read_dir(games_dir) {
        Ok(e) => e,
        Err(_) => return result,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let dir_name = entry.file_name().to_string_lossy().to_string();
        if dir_name.starts_with('.') {
            continue;
        }
        let mut meta = read_meta(&path).unwrap_or_else(|| GameMeta {
            id: dir_name.clone(),
            label: dir_name.clone(),
            version: String::new(),
            date: None,
            source: pick("локальная папка", "local folder").to_string(),
            installed_at: std::fs::metadata(&path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0),
            args: String::new(),
            ioa: Vec::new(),
        });
        if meta.id.is_empty() {
            meta.id = dir_name;
        }
        if meta.label.is_empty() {
            meta.label = meta.id.clone();
        }
        let exe = find_exe(&path);
        if exe.is_none() {
            // Папка без исполняемого файла — вероятно, остаток от неудачной установки.
            continue;
        }
        let unity = find_game_exe(&path)
            .as_deref()
            .and_then(exe_version)
            .unwrap_or_default();
        result.push(GameInstall {
            meta: meta.clone(),
            path: path.clone(),
            size: dir_size(&path),
            exe,
            mods: mods.iter().filter(|m| m.game_id == meta.id).count(),
            unity,
        });
    }
    result.sort_by(|a, b| {
        b.meta
            .date
            .cmp(&a.meta.date)
            .then_with(|| b.meta.installed_at.cmp(&a.meta.installed_at))
    });
    result
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ModFile {
    /// Путь относительно корня игры, всегда с прямыми слэшами.
    pub path: String,
    /// true — файл был заменён (есть резервная копия), false — добавлен.
    pub replaced: bool,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct InstalledMod {
    pub key: String,
    pub mod_id: String,
    pub name: String,
    pub category: String,
    pub author: String,
    pub game_id: String,
    pub installed_at: i64,
    pub files: Vec<ModFile>,
    /// Путь к конфигу мода относительно папки сборки (поле `cfg`).
    #[serde(default)]
    pub cfg: String,
    /// Версия из каталога (для модов, входящих в сборку).
    #[serde(default)]
    pub version: String,
    /// true — мод уже был в сборке (`ioa`), удалять его нельзя.
    #[serde(default)]
    pub bundled: bool,
}

impl InstalledMod {
    pub fn new(
        mod_id: &str,
        name: &str,
        category: &str,
        author: &str,
        game_id: &str,
        files: Vec<ModFile>,
    ) -> Self {
        let ts = now_ts();
        Self {
            key: format!("{}-{}", crate::util::sanitize_name(mod_id), ts),
            mod_id: mod_id.to_string(),
            name: name.to_string(),
            category: category.to_string(),
            author: author.to_string(),
            game_id: game_id.to_string(),
            installed_at: ts,
            files,
            cfg: String::new(),
            version: String::new(),
            bundled: false,
        }
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Library {
    #[serde(default)]
    pub mods: Vec<InstalledMod>,
}

impl Library {
    pub fn path(root: &Path) -> PathBuf {
        root.join("library.json")
    }

    pub fn load(root: &Path) -> Self {
        std::fs::read_to_string(Self::path(root))
            .ok()
            .and_then(|s| serde_json::from_str(crate::util::strip_bom(&s)).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, root: &Path) -> Result<(), String> {
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(Self::path(root), text).map_err(|e| format!("library.json: {e}"))
    }

    pub fn for_game(&self, game_id: &str) -> impl Iterator<Item = &InstalledMod> {
        let id = game_id.to_string();
        self.mods.iter().filter(move |m| m.game_id == id)
    }

    #[allow(dead_code)]
    pub fn is_installed(&self, game_id: &str, mod_id: &str) -> bool {
        self.mods
            .iter()
            .any(|m| m.game_id == game_id && m.mod_id == mod_id)
    }

    pub fn add(&mut self, item: InstalledMod) {
        // Одна и та же запись не должна дублироваться.
        self.mods.retain(|m| m.key != item.key);
        self.mods.push(item);
    }

    pub fn remove(&mut self, key: &str) -> Option<InstalledMod> {
        let pos = self.mods.iter().position(|m| m.key == key)?;
        Some(self.mods.remove(pos))
    }
}

pub fn kind_label(kind: &str) -> &'static str {
    match kind.trim().to_ascii_lowercase().as_str() {
        "mod" => pick("Мод", "Mod"),
        "patch" => pick("Патч", "Patch"),
        "texture" | "textures" | "tr" => pick("Текстуры", "Textures"),
        "pm" | "posemod" | "pose" => pick("Pose-мод", "Pose mod"),
        "ml" | "loader" => pick("Загрузчик", "Loader"),
        "build" => pick("Сборка", "Build"),
        "plugin" => pick("Плагин", "Plugin"),
        "translate" | "translation" => pick("Перевод", "Translation"),
        _ => pick("Другое", "Other"),
    }
}

pub fn kind_color(kind: &str) -> egui::Color32 {
    match kind.trim().to_ascii_lowercase().as_str() {
        "patch" => egui::Color32::from_rgb(0xF5, 0xB5, 0x5C),
        "texture" | "textures" | "tr" => egui::Color32::from_rgb(0xC7, 0x8A, 0xFC),
        "mod" => egui::Color32::from_rgb(0xFF, 0x7A, 0xA8),
        "pm" | "posemod" | "pose" => egui::Color32::from_rgb(0x5A, 0xD6, 0x8E),
        "ml" | "loader" => egui::Color32::from_rgb(0x62, 0xA8, 0xFF),
        "build" => egui::Color32::from_rgb(0xFF, 0xB8, 0x4D),
        "plugin" => egui::Color32::from_rgb(0xFF, 0x8A, 0x65),
        "translate" | "translation" => egui::Color32::from_rgb(0x5A, 0xD6, 0xD6),
        _ => egui::Color32::from_rgb(0xB0, 0xA8, 0xB4),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(label: &str, id: &str, version: &str) -> GameInstall {
        GameInstall {
            meta: GameMeta {
                id: id.to_string(),
                label: label.to_string(),
                version: version.to_string(),
                ..Default::default()
            },
            path: PathBuf::new(),
            size: 0,
            exe: None,
            mods: 0,
            unity: String::new(),
        }
    }

    #[test]
    fn generic_labels_are_not_names() {
        let build = game("Сборка", "091923", "2023.09.19");
        assert!(build.needs_name());
        assert_eq!(build.display_name(), "Сборка 2023.09.19");

        // Старый лаунчер писал «Cборка» с латинской C.
        let latin = game("Cборка", "091923", "2023.09.19");
        assert!(latin.needs_name());
        assert_eq!(latin.display_name(), "Сборка 2023.09.19");

        let named = game("Для модов", "091923", "2023.09.19");
        assert!(!named.needs_name());
        assert_eq!(named.display_name(), "Для модов");
    }

    #[test]
    fn poses_are_chosen_by_build_bits() {
        let both = vec![PathBuf::from("Pose32.exe"), PathBuf::from("Pose64.exe")];
        let name = |p: Option<PathBuf>| {
            p.and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
                .unwrap_or_default()
        };
        assert_eq!(name(pick_pose(both.clone(), Some(32))), "Pose32.exe");
        assert_eq!(name(pick_pose(both.clone(), Some(64))), "Pose64.exe");
        // Разрядность неизвестна — берём первый по алфавиту.
        assert_eq!(name(pick_pose(both, None)), "Pose32.exe");

        let single = vec![PathBuf::from("PoseMod.exe")];
        assert_eq!(name(pick_pose(single, Some(64))), "PoseMod.exe");
        assert!(pick_pose(Vec::new(), Some(64)).is_none());
    }

    fn write_pe(dir: &Path, name: &str, machine: u16) -> PathBuf {
        let mut data = vec![0u8; 0x100];
        data[0] = b'M';
        data[1] = b'Z';
        data[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        data[0x80..0x84].copy_from_slice(b"PE\0\0");
        data[0x84..0x86].copy_from_slice(&machine.to_le_bytes());
        let path = dir.join(name);
        std::fs::write(&path, &data).unwrap();
        path
    }

    #[test]
    fn reads_pe_bitness() {
        let dir = std::env::temp_dir();
        let pid = std::process::id();
        let x64 = write_pe(&dir, &format!("yanlan_pe64_{pid}.exe"), 0x8664);
        let x86 = write_pe(&dir, &format!("yanlan_pe32_{pid}.exe"), 0x014C);
        assert_eq!(pe_bits(&x64), Some(64));
        assert_eq!(pe_bits(&x86), Some(32));
        let _ = std::fs::remove_file(&x64);
        let _ = std::fs::remove_file(&x86);
        assert_eq!(pe_bits(Path::new("no-such-file.exe")), None);
    }

    #[test]
    fn find_exe_prefers_pose_of_matching_bits() {
        let dir = std::env::temp_dir().join(format!("yanlan_find_exe_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        write_pe(&dir, "YandereSimulator.exe", 0x8664);
        write_pe(&dir, "PoseMod32.exe", 0x014C);
        write_pe(&dir, "PoseMod64.exe", 0x8664);

        let found = find_exe(&dir).unwrap();
        assert_eq!(
            found.file_name().unwrap().to_string_lossy().to_lowercase(),
            "posemod64.exe"
        );

        // Без Pose-модов остаётся основной exe игры.
        std::fs::remove_file(dir.join("PoseMod32.exe")).unwrap();
        std::fs::remove_file(dir.join("PoseMod64.exe")).unwrap();
        let plain = find_exe(&dir).unwrap();
        assert_eq!(
            plain.file_name().unwrap().to_string_lossy().to_lowercase(),
            "yanderesimulator.exe"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
