//! Каталог модов YanMod: загрузка манифестов, совместимость со сборкой.
//!
//! Источник — репозиторий `bylka882-png/YanMod`. В нём лежат только
//! `info.yaml` и скриншоты `.webp`, а архивы — в GitHub Releases. Список
//! файлов берётся из `index.yaml`, если он есть, иначе из GitHub Tree API,
//! а при исчерпании лимита — из jsDelivr.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::sync::Mutex;

use serde::{Deserialize, Deserializer, Serialize};

use crate::config::Config;
use crate::i18n::{is_ru, pick};
use crate::library::GameInstall;
use crate::net;
use crate::progress::Progress;
use crate::util::sanitize_name;

pub const OWNER: &str = "bylka882-png";
pub const REPO: &str = "YanMod";
pub const BRANCH: &str = "main";
/// Кэш каталога считается свежим шесть часов.
pub const CACHE_TTL: i64 = 6 * 3600;
/// Версия формата кэша. Старый кэш без `types`/`cfg`/`icon` приходится
/// сбрасывать: иначе моды годами остаются в неправильных категориях.
const CACHE_VERSION: u32 = 4;

/// Тип мода из манифеста.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModType {
    Mod,
    Pm,
    Textures,
    Loader,
    Build,
    Plugin,
    Translate,
}

impl ModType {
    /// Порядок разделов в каталоге — как в README YanMod.
    pub const ALL: [ModType; 7] = [
        ModType::Pm,
        ModType::Build,
        ModType::Textures,
        ModType::Loader,
        ModType::Mod,
        ModType::Plugin,
        ModType::Translate,
    ];

    #[allow(dead_code)]
    pub fn parse(value: &str) -> Option<Self> {
        Self::parse_all(value).into_iter().next()
    }

    /// Разбирает составной тип: `TR; Plugin; Translate`, `Mod, Patch`.
    /// Неизвестные слова пропускаются, порядок сохраняется.
    pub fn parse_all(value: &str) -> Vec<Self> {
        let mut out = Vec::new();
        for part in value.split(|c| c == ';' || c == ',' || c == '|' || c == '/') {
            let Some(ty) = Self::parse_one(part) else {
                continue;
            };
            if !out.contains(&ty) {
                out.push(ty);
            }
        }
        out
    }

    fn parse_one(value: &str) -> Option<Self> {
        Some(match value.trim().to_ascii_lowercase().as_str() {
            "mod" | "patch" => ModType::Mod,
            "pm" | "posemod" | "pose-mod" | "pose" => ModType::Pm,
            "tr" | "texture" | "textures" => ModType::Textures,
            "ml" | "loader" => ModType::Loader,
            "build" | "builds" => ModType::Build,
            "plugin" | "plugins" => ModType::Plugin,
            "translate" | "translation" | "translations" => ModType::Translate,
            _ => return None,
        })
    }

    /// Тип по верхней папке — для пустых шаблонов, где поле `type` не заполнено.
    pub fn from_folder(path: &str) -> Self {
        match path
            .split('/')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str()
        {
            "pm" | "posemod" | "pose" => ModType::Pm,
            "ml" | "loader" => ModType::Loader,
            "build" | "builds" => ModType::Build,
            "tr" | "textures" | "texture" => ModType::Textures,
            "plugin" | "plugins" => ModType::Plugin,
            "translate" | "translation" | "translations" => ModType::Translate,
            _ => ModType::Mod,
        }
    }

    /// Код для `library.json` и подписей.
    pub fn code(self) -> &'static str {
        match self {
            ModType::Mod => "Mod",
            ModType::Pm => "PM",
            ModType::Textures => "TR",
            ModType::Loader => "ML",
            ModType::Build => "Build",
            ModType::Plugin => "Plugin",
            ModType::Translate => "Translate",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ModType::Mod => pick("Мод", "Mod"),
            ModType::Pm => pick("Pose-мод", "Pose mod"),
            ModType::Textures => pick("Текстуры", "Textures"),
            ModType::Loader => pick("Загрузчик", "Loader"),
            ModType::Build => pick("Сборка", "Build"),
            ModType::Plugin => pick("Плагин", "Plugin"),
            ModType::Translate => pick("Перевод", "Translation"),
        }
    }

    pub fn color(self) -> egui::Color32 {
        match self {
            ModType::Mod => egui::Color32::from_rgb(0xFF, 0x7A, 0xA8),
            ModType::Pm => egui::Color32::from_rgb(0x5A, 0xD6, 0x8E),
            ModType::Textures => egui::Color32::from_rgb(0xC7, 0x8A, 0xFC),
            ModType::Loader => egui::Color32::from_rgb(0x62, 0xA8, 0xFF),
            ModType::Build => egui::Color32::from_rgb(0xFF, 0xB8, 0x4D),
            ModType::Plugin => egui::Color32::from_rgb(0xFF, 0x8A, 0x65),
            ModType::Translate => egui::Color32::from_rgb(0x5A, 0xD6, 0xD6),
        }
    }
}

/// Порядок карточек в каталоге.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum CatalogSort {
    /// По названию (А → Я).
    #[default]
    Name,
    /// Сначала подходящие выбранной сборке.
    Compat,
}

impl CatalogSort {
    pub const ALL: [CatalogSort; 2] = [CatalogSort::Name, CatalogSort::Compat];

    pub fn label(self) -> &'static str {
        match self {
            CatalogSort::Name => pick("по названию", "by name"),
            CatalogSort::Compat => pick("по совместимости", "by compatibility"),
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            CatalogSort::Name => pick("Карточки по алфавиту", "Cards in alphabetical order"),
            CatalogSort::Compat => pick(
                "Сначала моды, подходящие выбранной сборке",
                "Mods compatible with the selected build first",
            ),
        }
    }
}

/// Совместимость мода с выбранной сборкой.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Compat {
    Yes,
    No,
    Unknown,
}

impl Compat {
    pub fn label(self) -> &'static str {
        match self {
            Compat::Yes => pick("подходит", "compatible"),
            Compat::No => pick("не подходит", "incompatible"),
            Compat::Unknown => pick("неизвестно", "unknown"),
        }
    }
}

/// Один вариант мода (конкретная версия под конкретную версию игры).
#[derive(Clone, Serialize, Deserialize)]
pub struct ModVariant {
    pub id: String,
    pub name: String,
    pub author: String,
    pub credits: String,
    pub version: String,
    pub game_v: String,
    pub description: String,
    pub dl_url: String,
    pub ty: ModType,
    /// Путь к конфигу мода относительно папки сборки (поле `cfg` в манифесте).
    #[serde(default)]
    pub cfg: String,
    /// Квадратная иконка `ico.webp` рядом со скриншотами.
    #[serde(default)]
    pub icon: String,
    /// Моды, уже входящие в сборку (`ioa: "PoseMod; 5"`).
    #[serde(default)]
    pub ioa: Vec<String>,
    /// Все категории из поля `type`. В манифестах встречается
    /// `TR; Plugin; Translate`, и мод должен попадать в каждый раздел.
    #[serde(default)]
    pub types: Vec<ModType>,
    pub affected_files: Vec<String>,
    pub dependencies: Vec<String>,
    /// Путь к `info.yaml` относительно корня репозитория.
    pub manifest_path: String,
    /// Исходный текст `info.yaml` — показывается вместо описания по кнопке.
    #[serde(default)]
    pub manifest_text: String,
    /// Папка мода относительно корня репозитория.
    pub folder: String,
    /// Скриншоты `.webp`/`.gif` из папки мода (отсортированы по имени).
    pub screenshots: Vec<String>,
    /// true — ссылки на архив нет, карточка показывает «скоро».
    pub soon: bool,
}

impl ModVariant {
    /// Является ли вариант сборкой целиком.
    pub fn is_build(&self) -> bool {
        self.types.iter().any(|ty| *ty == ModType::Build) || self.ty == ModType::Build
    }

    /// Все категории варианта; для старых кэшей без `types` — основная.
    pub fn type_list(&self) -> Vec<ModType> {
        if self.types.is_empty() {
            vec![self.ty]
        } else {
            self.types.clone()
        }
    }

    /// Описание на текущем языке. Формат манифеста — `RU|EN`.
    /// Ссылки на YouTube из текста убираются: вместо них в карточке
    /// показывается встроенный плеер.
    pub fn description_text(&self) -> String {
        strip_youtube_links(&self.description_localized())
    }

    /// Описание на текущем языке без вырезания ссылок.
    pub fn description_localized(&self) -> String {
        let mut parts = self.description.splitn(2, '|');
        let ru = parts.next().unwrap_or("").trim();
        let en = parts.next().map(str::trim).unwrap_or(ru);
        let text = if is_ru() {
            if ru.is_empty() {
                en
            } else {
                ru
            }
        } else if en.is_empty() {
            ru
        } else {
            en
        };
        text.to_string()
    }

    /// Совместимость с конкретной сборкой.
    pub fn compat(&self, game: Option<&GameInstall>) -> Compat {
        compat_of(&self.game_v, game)
    }

    /// Имя файла в `downloads/`: уникальное для мода и версии.
    pub fn download_name(&self) -> String {
        let parts = self.dl_parts();
        let base = parts.first().map(String::as_str).unwrap_or("");
        let base = base.split('?').next().unwrap_or(base);
        let base = base.split('#').next().unwrap_or(base);
        let ext: String = base
            .rsplit('.')
            .next()
            .unwrap_or("7z")
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        // У многотомного архива расширение последнего тома — `.001`, а не `.7z`.
        let ext = if ext.len() == 3 && ext.chars().all(|c| c.is_ascii_digit()) {
            base.rsplitn(3, '.').nth(1).unwrap_or("7z").to_string()
        } else if ext.is_empty() {
            "7z".to_string()
        } else {
            ext
        };
        let mut stem = if self.version.trim().is_empty() {
            sanitize_name(&self.id)
        } else {
            sanitize_name(&format!("{}-{}", self.id, self.version))
        };
        if stem == "build" {
            stem = "mod".to_string();
        }
        format!("{stem}.{ext}")
    }

    /// Ссылки на все тома архива.
    ///
    /// `DL_url` может перечислять части через `;`: первая — полный адрес,
    /// остальные — имена файлов в той же папке релиза.
    pub fn dl_parts(&self) -> Vec<String> {
        let raw = self.dl_url.trim();
        if raw.is_empty() {
            return Vec::new();
        }
        let mut segments = raw.split(';').map(str::trim).filter(|s| !s.is_empty());
        let Some(first) = segments.next() else {
            return Vec::new();
        };
        let base = first.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("");
        let mut out = vec![first.to_string()];
        for name in segments {
            if name.starts_with("http://") || name.starts_with("https://") {
                out.push(name.to_string());
            } else {
                out.push(format!("{base}/{}", name.trim_start_matches('/')));
            }
        }
        out
    }
}

/// `"PoseMod; 5"` → (`PoseMod`, `5`). Без второй части версия пустая.
pub fn dep_spec(dep: &str) -> (String, String) {
    let mut parts = dep.splitn(2, ';');
    let id = parts.next().unwrap_or("").trim().to_string();
    let version = parts.next().unwrap_or("").trim().to_string();
    (id, version)
}

/// Мод целиком: несколько вариантов с одним `id`.
#[derive(Clone, Serialize, Deserialize)]
pub struct ModGroup {
    pub id: String,
    pub name: String,
    pub author: String,
    pub ty: ModType,
    /// Все категории группы (объединение категорий её вариантов).
    #[serde(default)]
    pub types: Vec<ModType>,
    pub variants: Vec<ModVariant>,
}

impl ModGroup {
    /// Все категории группы; для старых кэшей — основная.
    pub fn type_list(&self) -> Vec<ModType> {
        if self.types.is_empty() {
            let mut out = vec![self.ty];
            for variant in &self.variants {
                for ty in variant.type_list() {
                    if !out.contains(&ty) {
                        out.push(ty);
                    }
                }
            }
            out
        } else {
            self.types.clone()
        }
    }

    pub fn has_type(&self, ty: ModType) -> bool {
        self.types.contains(&ty)
            || self.ty == ty
            || self
                .variants
                .iter()
                .any(|variant| variant.type_list().contains(&ty))
    }

    pub fn is_build(&self) -> bool {
        self.has_type(ModType::Build)
    }

    /// Индекс варианта, который лучше всего подходит сборке.
    ///
    /// Варианты уже отсортированы от новых к старым, поэтому берём первый
    /// совместимый, иначе первый с неизвестной совместимостью.
    pub fn best_variant(&self, game: Option<&GameInstall>) -> usize {
        if game.is_none() {
            return 0;
        }
        let mut unknown = None;
        for (index, variant) in self.variants.iter().enumerate() {
            match variant.compat(game) {
                Compat::Yes => return index,
                Compat::Unknown if unknown.is_none() => unknown = Some(index),
                _ => {}
            }
        }
        unknown.unwrap_or(0)
    }

    pub fn description(&self) -> String {
        self.variants
            .iter()
            .map(ModVariant::description_text)
            .find(|s| !s.is_empty())
            .unwrap_or_default()
    }
}

#[derive(Serialize, Deserialize)]
pub struct CatalogCache {
    #[serde(default)]
    pub version: u32,
    pub fetched_at: i64,
    pub mods: Vec<ModGroup>,
}

pub fn cache_path(cfg: &Config) -> std::path::PathBuf {
    cfg.cache_dir().join("mods.json")
}

pub fn load_cache(cfg: &Config) -> Option<CatalogCache> {
    let text = std::fs::read_to_string(cache_path(cfg)).ok()?;
    let cached: CatalogCache = serde_json::from_str(crate::util::strip_bom(&text)).ok()?;
    if cached.version != CACHE_VERSION || cached.mods.is_empty() {
        let _ = std::fs::remove_file(cache_path(cfg));
        None
    } else {
        Some(cached)
    }
}

pub fn save_cache(cfg: &Config, mods: &[ModGroup]) {
    let cached = CatalogCache {
        version: CACHE_VERSION,
        fetched_at: crate::util::now_ts(),
        mods: mods.to_vec(),
    };
    if let Ok(text) = serde_json::to_string(&cached) {
        let _ = crate::util::write_atomic(&cache_path(cfg), text);
    }
}

/// Короткий стабильный ключ файла: по нему кэшируются обложки.
pub fn cover_key(path: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in path.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

pub fn raw_url(path: &str) -> String {
    use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};
    const SEG: &AsciiSet = &CONTROLS
        .add(b' ')
        .add(b'"')
        .add(b'#')
        .add(b'%')
        .add(b'?')
        .add(b'[')
        .add(b']')
        .add(b'{')
        .add(b'}')
        .add(b'|')
        .add(b'\\')
        .add(b'^')
        .add(b'`')
        .add(b'<')
        .add(b'>');
    let encoded: Vec<String> = path
        .trim_start_matches('/')
        .split('/')
        .map(|part| utf8_percent_encode(part, SEG).to_string())
        .collect();
    format!(
        "https://raw.githubusercontent.com/{OWNER}/{REPO}/{BRANCH}/{}",
        encoded.join("/")
    )
}

fn tree_url() -> String {
    format!("https://api.github.com/repos/{OWNER}/{REPO}/git/trees/{BRANCH}?recursive=1")
}

fn jsdelivr_url() -> String {
    format!("https://data.jsdelivr.com/v1/packages/gh/{OWNER}/{REPO}@{BRANCH}?structure=flat")
}

/// Загружает каталог: список файлов, манифесты и скриншоты.
pub fn fetch(progress: &Progress) -> Result<(Vec<ModGroup>, Vec<String>), String> {
    progress.stage(pick("Загрузка каталога модов", "Loading the mod catalog"));
    progress.detail(pick("Поиск index.yaml", "Looking for index.yaml"));

    let files = load_file_list(progress)?;
    let manifests: Vec<String> = files
        .iter()
        .filter(|p| p.to_ascii_lowercase().ends_with("/info.yaml"))
        .cloned()
        .collect();
    if manifests.is_empty() {
        return Err(pick(
            "В репозитории не найдено ни одного info.yaml",
            "No info.yaml files found in the repository",
        )
        .to_string());
    }

    progress.stage(pick("Чтение манифестов", "Reading manifests"));
    progress.files(0, manifests.len());
    let (loaded, warnings) = fetch_manifests(&manifests, progress)?;

    progress.stage(pick("Сборка каталога", "Building the catalog"));
    let mods = group_variants(&files, loaded);
    if mods.is_empty() {
        return Err(pick(
            "Не удалось прочитать ни одного мода",
            "Could not read any mods",
        )
        .to_string());
    }
    Ok((mods, warnings))
}

fn load_file_list(progress: &Progress) -> Result<Vec<String>, String> {
    // 1. index.yaml — если владелец репозитория его добавил.
    match net::get_text_opt(&raw_url("index.yaml")) {
        Ok(Some(text)) => {
            if let Ok(files) = parse_index(&text) {
                if files.iter().any(|p| p.ends_with("/info.yaml")) {
                    progress.detail("index.yaml");
                    return Ok(files);
                }
            }
            progress.detail(pick(
                "index.yaml не разобран, берём список с GitHub",
                "index.yaml could not be parsed, using the GitHub listing",
            ));
        }
        Ok(None) => {}
        Err(_) => {}
    }

    // 2. GitHub Tree API — один запрос на весь репозиторий.
    match list_tree() {
        Ok(files) => {
            progress.detail("GitHub API");
            return Ok(files);
        }
        Err(err) => {
            // 3. jsDelivr — резервный источник без лимита запросов.
            progress.detail(pick(
                "GitHub недоступен, пробуем jsDelivr",
                "GitHub is unavailable, trying jsDelivr",
            ));
            return list_jsdelivr().map_err(|second| {
                format!(
                    "{}; {}",
                    crate::util::trim_to(&err, 90),
                    crate::util::trim_to(&second, 90)
                )
            });
        }
    }
}

#[derive(Deserialize)]
struct TreeResponse {
    #[serde(default)]
    tree: Vec<TreeEntry>,
    #[serde(default)]
    truncated: bool,
}

#[derive(Deserialize)]
struct TreeEntry {
    path: String,
    #[serde(default, rename = "type")]
    kind: String,
}

fn list_tree() -> Result<Vec<String>, String> {
    let response: TreeResponse = net::get_json(&tree_url())?;
    if response.truncated {
        return Err(pick(
            "Список файлов GitHub обрезан",
            "The GitHub file listing was truncated",
        )
        .to_string());
    }
    Ok(response
        .tree
        .into_iter()
        .filter(|entry| entry.kind == "blob")
        .map(|entry| entry.path)
        .collect())
}

#[derive(Deserialize)]
struct JsResponse {
    #[serde(default)]
    files: Vec<JsFile>,
}

#[derive(Deserialize)]
struct JsFile {
    name: String,
}

fn list_jsdelivr() -> Result<Vec<String>, String> {
    let response: JsResponse = net::get_json(&jsdelivr_url())?;
    Ok(response
        .files
        .into_iter()
        .map(|file| file.name.trim_start_matches('/').to_string())
        .collect())
}

#[derive(Deserialize, Default)]
struct IndexDoc {
    #[serde(default)]
    files: Vec<String>,
    #[serde(default)]
    manifests: Vec<String>,
    #[serde(default)]
    screenshots: Vec<String>,
    #[serde(default)]
    mods: Vec<IndexMod>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum IndexMod {
    Path(String),
    Full(IndexModFull),
}

#[derive(Deserialize, Default)]
struct IndexModFull {
    #[serde(default)]
    manifest: String,
    #[serde(default)]
    screenshots: Vec<String>,
}

/// Разбирает `index.yaml`. Поддерживаются три формы:
/// `files: [...]`, `manifests: [...]` и `mods: [{ manifest, screenshots }]`.
fn parse_index(text: &str) -> Result<Vec<String>, String> {
    let value: serde_yaml::Value = serde_yaml::from_str(crate::util::strip_bom(text))
        .map_err(|e| format!("index.yaml: {e}"))?;
    let mut out = Vec::new();
    match value {
        serde_yaml::Value::Sequence(seq) => {
            for item in seq {
                if let serde_yaml::Value::String(path) = item {
                    out.push(path);
                }
            }
        }
        serde_yaml::Value::Mapping(_) => {
            let doc: IndexDoc =
                serde_yaml::from_value(value).map_err(|e| format!("index.yaml: {e}"))?;
            out.extend(doc.files);
            out.extend(doc.manifests);
            out.extend(doc.screenshots);
            for item in doc.mods {
                match item {
                    IndexMod::Path(path) => out.push(path),
                    IndexMod::Full(full) => {
                        if !full.manifest.trim().is_empty() {
                            out.push(full.manifest);
                        }
                        out.extend(full.screenshots);
                    }
                }
            }
        }
        _ => {
            return Err(pick(
                "index.yaml должен содержать список файлов",
                "index.yaml must contain a file list",
            )
            .to_string())
        }
    }
    let mut result: Vec<String> = out
        .into_iter()
        .map(|p| {
            p.trim()
                .trim_start_matches("./")
                .trim_start_matches('/')
                .to_string()
        })
        .filter(|p| !p.is_empty())
        .collect();
    result.sort();
    result.dedup();
    Ok(result)
}

#[derive(Deserialize, Default)]
struct RawManifest {
    #[serde(default, deserialize_with = "de_text")]
    id: String,
    #[serde(default, deserialize_with = "de_text")]
    name: String,
    #[serde(default, deserialize_with = "de_text")]
    by: String,
    #[serde(default, deserialize_with = "de_text")]
    credits: String,
    #[serde(default, deserialize_with = "de_text")]
    v: String,
    #[serde(default, rename = "gameV", deserialize_with = "de_text")]
    game_v: String,
    #[serde(default, deserialize_with = "de_text")]
    description: String,
    #[serde(default, rename = "DL_url", deserialize_with = "de_text")]
    dl_url: String,
    #[serde(default, alias = "type", deserialize_with = "de_text")]
    ty: String,
    #[serde(default, deserialize_with = "de_text")]
    cfg: String,
    /// Только для сборок: моды, которые уже входят в неё (`"PoseMod; 5"`).
    #[serde(default, deserialize_with = "de_list")]
    ioa: Vec<String>,
    #[serde(default, deserialize_with = "de_list")]
    affected_files: Vec<String>,
    #[serde(default, deserialize_with = "de_list")]
    dependencies: Vec<String>,
}

fn value_text(value: serde_yaml::Value) -> String {
    match value {
        serde_yaml::Value::String(s) => s,
        serde_yaml::Value::Number(n) => n.to_string(),
        serde_yaml::Value::Bool(b) => b.to_string(),
        _ => String::new(),
    }
}

/// В `cfg` пути пишут с обратными слэшами. В двойных кавычках YAML такой
/// слэш — escape-символ, и `\c` ломает разбор всего манифеста. Удваиваем
/// одиночные слэши только в значении `cfg`, не трогая остальной файл.
fn sanitize_cfg_backslashes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (index, line) in text.split('\n').enumerate() {
        if index > 0 {
            out.push('\n');
        }
        let trimmed = line.trim_start();
        let Some(rest) = trimmed.strip_prefix("cfg:") else {
            out.push_str(line);
            continue;
        };
        let value = rest.trim();
        if value.len() < 2 || !value.starts_with('"') || !value.ends_with('"') {
            out.push_str(line);
            continue;
        }
        let indent = &line[..line.len() - trimmed.len()];
        let inner = &value[1..value.len() - 1];
        let mut fixed = String::with_capacity(inner.len() + 8);
        let mut chars = inner.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '\\' {
                fixed.push('\\');
                if chars.peek() == Some(&'\\') {
                    chars.next();
                }
                fixed.push('\\');
            } else {
                fixed.push(ch);
            }
        }
        out.push_str(indent);
        out.push_str("cfg: \"");
        out.push_str(&fixed);
        out.push('"');
    }
    out
}

fn de_text<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_yaml::Value::deserialize(deserializer)?;
    Ok(value_text(value).trim().to_string())
}

fn de_list<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_yaml::Value::deserialize(deserializer)?;
    let mut out = Vec::new();
    match value {
        serde_yaml::Value::Sequence(seq) => {
            for item in seq {
                let text = value_text(item).trim().to_string();
                if !text.is_empty() {
                    out.push(text);
                }
            }
        }
        serde_yaml::Value::Null => {}
        other => {
            let text = value_text(other).trim().to_string();
            if !text.is_empty() {
                out.push(text);
            }
        }
    }
    Ok(out)
}

fn fetch_manifests(
    manifests: &[String],
    progress: &Progress,
) -> Result<(Vec<(String, RawManifest, String)>, Vec<String>), String> {
    let count = manifests.len();
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let results: Mutex<Vec<Option<Result<(String, RawManifest, String), String>>>> =
        Mutex::new((0..count).map(|_| None).collect());

    std::thread::scope(|scope| {
        let workers = count.min(6).max(1);
        for _ in 0..workers {
            scope.spawn(|| loop {
                let index = next.fetch_add(1, AtomicOrdering::Relaxed);
                if index >= count {
                    break;
                }
                let path = &manifests[index];
                let result = net::get_text(&raw_url(path)).and_then(|text| {
                    let original = crate::util::strip_bom(&text).to_string();
                    let raw: RawManifest =
                        serde_yaml::from_str(&sanitize_cfg_backslashes(&original))
                            .map_err(|e| format!("{path}: {e}"))?;
                    Ok((path.clone(), raw, original))
                });
                if let Ok(mut guard) = results.lock() {
                    guard[index] = Some(result);
                }
                let finished = done.fetch_add(1, AtomicOrdering::Relaxed) + 1;
                progress.files(finished, count);
            });
        }
    });

    let guard = results
        .into_inner()
        .map_err(|_| pick("Ошибка чтения манифестов", "Failed to read manifests").to_string())?;
    let mut loaded = Vec::new();
    let mut errors = Vec::new();
    for item in guard.into_iter().flatten() {
        match item {
            Ok(value) => loaded.push(value),
            Err(err) => errors.push(err),
        }
    }
    if loaded.is_empty() {
        return Err(errors.into_iter().next().unwrap_or_else(|| {
            pick("Манифесты не прочитаны", "Manifests were not read").to_string()
        }));
    }
    Ok((loaded, errors))
}

fn group_variants(files: &[String], loaded: Vec<(String, RawManifest, String)>) -> Vec<ModGroup> {
    let mut shots: HashMap<String, Vec<String>> = HashMap::new();
    let mut icons: HashMap<String, String> = HashMap::new();
    for file in files {
        let lower = file.to_ascii_lowercase();
        if !(lower.ends_with(".webp") || lower.ends_with(".gif") || lower.ends_with(".png")) {
            continue;
        }
        let Some((folder, name)) = file.rsplit_once('/') else {
            continue;
        };
        // Иконка — отдельный файл рядом с `1.webp`, а не скриншот.
        if name.eq_ignore_ascii_case("ico.webp")
            || name.eq_ignore_ascii_case("icon.webp")
            || name.eq_ignore_ascii_case("ico.png")
        {
            icons
                .entry(folder.to_string())
                .or_insert_with(|| file.clone());
            continue;
        }
        if !lower.ends_with(".png") {
            shots
                .entry(folder.to_string())
                .or_default()
                .push(file.clone());
        }
    }
    for list in shots.values_mut() {
        list.sort_by(|a, b| natural_cmp(a, b));
    }

    let mut groups: Vec<ModGroup> = Vec::new();
    let mut index_by_id: HashMap<String, usize> = HashMap::new();

    for (path, raw, manifest_text) in loaded {
        let folder = path.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("");
        let mut types = ModType::parse_all(&raw.ty);
        if types.is_empty() {
            types.push(ModType::from_folder(&path));
        }
        let ty = types[0];

        let mut id = raw.id.trim().to_string();
        if id.is_empty() {
            id = derive_id(&path, ty);
        }
        let mut version = raw.v.trim().to_string();
        if version.is_empty() {
            let name = folder.rsplit('/').next().unwrap_or("");
            if looks_like_version(name) {
                version = name.to_string();
            }
        }
        let name = if raw.name.trim().is_empty() {
            id.clone()
        } else {
            raw.name.trim().to_string()
        };
        let screenshots = shots.get(folder).cloned().unwrap_or_default();
        let variant = ModVariant {
            id: id.clone(),
            name: name.clone(),
            author: raw.by.trim().to_string(),
            credits: raw.credits.trim().to_string(),
            version,
            game_v: raw.game_v.trim().to_string(),
            description: raw.description.trim().to_string(),
            dl_url: raw.dl_url.trim().to_string(),
            ty,
            cfg: raw.cfg.trim().to_string(),
            icon: icons.get(folder).cloned().unwrap_or_default(),
            ioa: raw.ioa,
            types: types.clone(),
            affected_files: raw.affected_files,
            dependencies: raw.dependencies,
            folder: folder.to_string(),
            manifest_path: path,
            manifest_text,
            screenshots,
            soon: raw.dl_url.trim().is_empty(),
        };

        if let Some(&position) = index_by_id.get(&id) {
            let group: &mut ModGroup = &mut groups[position];
            if group.author.is_empty() && !variant.author.is_empty() {
                group.author = variant.author.clone();
            }
            for extra in &variant.types {
                if !group.types.contains(extra) {
                    group.types.push(*extra);
                }
            }
            group.variants.push(variant);
        } else {
            index_by_id.insert(id.clone(), groups.len());
            groups.push(ModGroup {
                id,
                name,
                author: variant.author.clone(),
                ty,
                types,
                variants: vec![variant],
            });
        }
    }

    for group in &mut groups {
        group
            .variants
            .sort_by(|a, b| compare_versions(&b.version, &a.version));
        if group.name.trim().is_empty() {
            group.name = group.id.clone();
        }
        if let Some(first) = group.variants.first() {
            if group.name == group.id && !first.name.is_empty() {
                group.name = first.name.clone();
            }
        }
    }
    groups
}

fn derive_id(path: &str, ty: ModType) -> String {
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() < 2 {
        return "mod".to_string();
    }
    let folder = parts[parts.len() - 2];
    if ty == ModType::Build || !looks_like_version(folder) {
        return folder.to_string();
    }
    if parts.len() >= 4 {
        parts[parts.len() - 3].to_string()
    } else {
        folder.to_string()
    }
}

fn looks_like_version(text: &str) -> bool {
    !text.is_empty()
        && text.chars().any(|c| c.is_ascii_digit())
        && text
            .chars()
            .all(|c| c.is_ascii_digit() || c == '.' || c == '-' || c == '_')
}

/// Сравнение версий по числовым частям: `3.10` новее `3.2`.
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    let pa = number_parts(a);
    let pb = number_parts(b);
    pa.cmp(&pb)
        .then_with(|| a.to_ascii_lowercase().cmp(&b.to_ascii_lowercase()))
}

fn number_parts(text: &str) -> Vec<u64> {
    let mut parts = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch.is_ascii_digit() {
            current.push(ch);
        } else if !current.is_empty() {
            parts.push(current.parse().unwrap_or(0));
            current.clear();
        }
    }
    if !current.is_empty() {
        parts.push(current.parse().unwrap_or(0));
    }
    parts
}

fn natural_cmp(a: &str, b: &str) -> Ordering {
    number_parts(a).cmp(&number_parts(b)).then_with(|| a.cmp(b))
}

/// Куда ведёт ссылка из поля `credits`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LinkKind {
    Discord,
    YouTube,
    GitHub,
    Web,
}

impl LinkKind {
    pub fn label(self) -> &'static str {
        match self {
            LinkKind::Discord => "Discord",
            LinkKind::YouTube => "YouTube",
            LinkKind::GitHub => "GitHub",
            LinkKind::Web => pick("Ссылка", "Link"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct CreditLink {
    pub kind: LinkKind,
    pub url: String,
}

/// Разбирает `credits` на отдельные ссылки. Поле может содержать несколько
/// адресов через `;`, а также обычный текст, который просто пропускается.
pub fn credit_links(credits: &str) -> Vec<CreditLink> {
    static LINKS: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = LINKS.get_or_init(|| {
        regex::Regex::new(
            r#"(?i)(?:https?://[^\s;,"'<>]+|(?:www\.)?(?:discord\.gg|discord\.com|youtube\.com|youtu\.be|github\.com|twitch\.tv|twitter\.com|x\.com|t\.me|boosty\.to|patreon\.com)/[^\s;,"'<>]+)"#,
        )
        .expect("valid credit link regex")
    });

    let mut out: Vec<CreditLink> = Vec::new();
    for found in re.find_iter(credits) {
        let raw = found.as_str().trim().trim_end_matches(|c: char| {
            matches!(c, '.' | ',' | ';' | ')' | '(' | '!' | '"' | '\'')
        });
        if raw.is_empty() {
            continue;
        }
        let lower = raw.to_ascii_lowercase();
        let url = if lower.starts_with("http://") || lower.starts_with("https://") {
            raw.to_string()
        } else {
            format!("https://{raw}")
        };
        let kind = if lower.contains("discord.") {
            LinkKind::Discord
        } else if lower.contains("youtu") {
            LinkKind::YouTube
        } else if lower.contains("github.com") {
            LinkKind::GitHub
        } else {
            LinkKind::Web
        };
        if out.iter().any(|item| item.url == url) {
            continue;
        }
        out.push(CreditLink { kind, url });
    }
    out
}

/// Идентификаторы YouTube-видео из произвольного текста (описание, credits).
pub fn youtube_ids(text: &str) -> Vec<String> {
    static YT: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = YT.get_or_init(|| {
        regex::Regex::new(
            r"(?i)(?:youtube\.com/(?:watch\?(?:[^#\s]*?&)?v=|shorts/|embed/|live/)|youtu\.be/)([A-Za-z0-9_-]{11})",
        )
        .expect("valid youtube regex")
    });
    let mut out: Vec<String> = Vec::new();
    for capture in re.captures_iter(text) {
        if let Some(id) = capture.get(1) {
            let id = id.as_str().to_string();
            if !out.contains(&id) {
                out.push(id);
            }
        }
    }
    out
}

pub fn youtube_watch_url(id: &str) -> String {
    format!("https://www.youtube.com/watch?v={id}")
}

/// Убирает YouTube-ссылки из описания: вместо них показывается плеер.
///
/// Ссылка может стоять отдельной строкой или внутри текста, поэтому после
/// удаления подчищаются повисшие двоеточия, лишние пробелы и пустые строки.
pub fn strip_youtube_links(text: &str) -> String {
    static YT: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = YT.get_or_init(|| {
        regex::Regex::new(
            r"(?i)https?://(?:www\.)?(?:youtube\.com/(?:watch\?(?:[^#\s]*?&)?v=|shorts/|embed/|live/)|youtu\.be/)[A-Za-z0-9_-]{11}(?:[?&][^\s]*)?",
        )
        .expect("valid youtube url regex")
    });
    if !re.is_match(text) {
        return text.to_string();
    }

    let mut lines = Vec::new();
    for line in re.replace_all(text, "").lines() {
        let mut line = line.trim_end().to_string();
        while line.ends_with(':')
            || line.ends_with('-')
            || line.ends_with('—')
            || line.ends_with(',')
        {
            line.pop();
            line = line.trim_end().to_string();
        }
        // Схлопываем двойные пробелы, оставшиеся на месте ссылки.
        let mut clean = String::with_capacity(line.len());
        let mut space = false;
        for ch in line.chars() {
            if ch == ' ' {
                if !space {
                    clean.push(ch);
                }
                space = true;
            } else {
                clean.push(ch);
                space = false;
            }
        }
        lines.push(clean);
    }

    // Не больше одной пустой строки между абзацами.
    let mut out = String::new();
    let mut blanks = 0;
    for line in lines {
        if line.trim().is_empty() {
            blanks += 1;
            if blanks > 1 {
                continue;
            }
        } else {
            blanks = 0;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&line);
    }
    out.trim().to_string()
}

pub fn youtube_thumb_url(id: &str) -> String {
    format!("https://i.ytimg.com/vi/{id}/mqdefault.jpg")
}

/// Совместимость значения `gameV` со сборкой.
pub fn compat_of(spec: &str, game: Option<&GameInstall>) -> Compat {
    let Some(game) = game else {
        return Compat::Unknown;
    };
    let spec = spec.trim();
    if spec.is_empty() || spec.eq_ignore_ascii_case("own") {
        return Compat::Unknown;
    }

    let (op, rest) = if let Some(rest) = spec.strip_prefix(">=") {
        (Some(true), rest.trim())
    } else if let Some(rest) = spec.strip_prefix("<=") {
        (Some(false), rest.trim())
    } else {
        (None, spec)
    };
    if rest.is_empty() {
        return Compat::Unknown;
    }
    let rest = rest.replace(['–', '—'], "-");
    let rest = rest.trim();

    // Период версий Unity: U5-U6, U4 - U6, U5..U6.
    if op.is_none() {
        if let Some((left, right)) = unity_period(rest) {
            let Some(have) = game_unity_rank(game) else {
                return Compat::Unknown;
            };
            return if have >= left && have <= right {
                Compat::Yes
            } else {
                Compat::No
            };
        }
    }

    // Версия Unity: U4/U5/U17/U19/U20/U6.
    if let Some(want) = unity_rank_code(rest) {
        let Some(have) = game_unity_rank(game) else {
            return Compat::Unknown;
        };
        return match op {
            Some(true) if have >= want => Compat::Yes,
            Some(true) => Compat::No,
            Some(false) if have <= want => Compat::Yes,
            Some(false) => Compat::No,
            None if have == want => Compat::Yes,
            None => Compat::No,
        };
    }

    // Период дат: 2017.10.04-2019.07.01.
    if let Some((start, end)) = parse_period(rest) {
        let Some(date) = build_date(game) else {
            return Compat::Unknown;
        };
        return if date >= start && date <= end {
            Compat::Yes
        } else {
            Compat::No
        };
    }

    // Одна дата: 2019.07.01 или 2019.07.
    if let Some((date, full)) = normalize_date(rest) {
        let Some(have) = build_date(game) else {
            return Compat::Unknown;
        };
        return match op {
            Some(true) if have >= date => Compat::Yes,
            Some(true) => Compat::No,
            Some(false) if have <= date => Compat::Yes,
            Some(false) => Compat::No,
            None if full && have == date => Compat::Yes,
            None if !full && have.starts_with(&date[..7]) => Compat::Yes,
            None => Compat::No,
        };
    }

    // Обычная версия игры — сравниваем с подписью версии сборки.
    let have = game.meta.version.trim();
    if have.is_empty() {
        return Compat::Unknown;
    }
    let ordering = compare_versions(have, rest);
    match op {
        Some(true) if ordering != Ordering::Less => Compat::Yes,
        Some(true) => Compat::No,
        Some(false) if ordering != Ordering::Greater => Compat::Yes,
        Some(false) => Compat::No,
        None if ordering == Ordering::Equal => Compat::Yes,
        None => Compat::No,
    }
}

fn unity_period(text: &str) -> Option<(u8, u8)> {
    let (left, right) = text.split_once('-')?;
    let left = unity_rank_code(left)?;
    let right = unity_rank_code(right)?;
    Some(if left <= right {
        (left, right)
    } else {
        (right, left)
    })
}

fn unity_rank_code(text: &str) -> Option<u8> {
    let text = text.trim().to_ascii_uppercase();
    let number: u32 = text.strip_prefix('U')?.parse().ok()?;
    Some(match number {
        4 => 4,
        5 => 5,
        6 => 60,
        17 | 19 | 20 => number as u8,
        year if (2017..=2035).contains(&year) => (year - 2000) as u8,
        _ => return None,
    })
}

fn unity_rank_label(label: &str) -> Option<u8> {
    let rest = label.trim().strip_prefix("Unity ")?;
    let first: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    let number: u32 = first.parse().ok()?;
    Some(match number {
        4 => 4,
        5 => 5,
        6 => 60,
        year if (2017..=2035).contains(&year) => (year - 2000) as u8,
        _ => return None,
    })
}

/// Дата сборки: поле `date`, а если его нет — версия вроде `2020.01.01`.
pub fn build_date(game: &GameInstall) -> Option<String> {
    normalize_date(game.meta.date.as_deref().unwrap_or(""))
        .map(|(date, _)| date)
        .or_else(|| normalize_date(&game.meta.version).map(|(date, _)| date))
}

/// Поколение Unity сборки: из exe, а если там пусто — по дате версии.
pub fn unity_of(game: &GameInstall) -> Option<String> {
    game.unity_label().or_else(|| {
        build_date(game)
            .and_then(|date| crate::builds::unity_for_date(Some(&date)).map(str::to_string))
    })
}

fn game_unity_rank(game: &GameInstall) -> Option<u8> {
    unity_of(game).and_then(|label| unity_rank_label(&label))
}

/// `YYYY.MM.DD` / `YYYY-MM-DD` / `YYYY.MM` → `YYYY-MM-DD`.
fn normalize_date(text: &str) -> Option<(String, bool)> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let parts: Vec<&str> = text
        .split(|c| c == '.' || c == '-' || c == '/')
        .filter(|p| !p.is_empty())
        .collect();
    if parts.len() < 2 || parts.len() > 3 {
        return None;
    }
    let year: i32 = parts[0].parse().ok()?;
    if !(2000..=2100).contains(&year) {
        return None;
    }
    let month: u32 = parts[1].parse().ok()?;
    if !(1..=12).contains(&month) {
        return None;
    }
    if parts.len() == 3 {
        let day: u32 = parts[2].parse().ok()?;
        if !(1..=31).contains(&day) {
            return None;
        }
        Some((format!("{year:04}-{month:02}-{day:02}"), true))
    } else {
        Some((format!("{year:04}-{month:02}-01"), false))
    }
}

fn parse_period(text: &str) -> Option<(String, String)> {
    let (left, right) = text.split_once('-')?;
    let (start, _) = normalize_date(left)?;
    let (end, end_full) = if right.len() <= 2 && right.chars().all(|c| c.is_ascii_digit()) {
        let month: u32 = right.parse().ok()?;
        if !(1..=12).contains(&month) {
            return None;
        }
        (format!("{}-{month:02}-01", &start[..4]), false)
    } else {
        normalize_date(right)?
    };
    let end = if end_full {
        end
    } else {
        format!("{}-31", &end[..7])
    };
    Some((start, end))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::{GameInstall, GameMeta};
    use std::path::PathBuf;

    fn game(version: &str, unity: &str) -> GameInstall {
        GameInstall {
            meta: GameMeta {
                id: "test".to_string(),
                label: "Test".to_string(),
                version: version.to_string(),
                date: None,
                ..Default::default()
            },
            path: PathBuf::new(),
            size: 0,
            exe: None,
            mods: 0,
            unity: unity.to_string(),
        }
    }

    #[test]
    fn dependency_versions_and_volumes_are_split() {
        let mut variant = ModVariant {
            id: "ChristmasParty".to_string(),
            name: "ChristmasParty".to_string(),
            author: String::new(),
            credits: String::new(),
            version: "1".to_string(),
            game_v: String::new(),
            description: String::new(),
            dl_url: "https://example.com/rel/ChristmasParty.7z.001; ChristmasParty.7z.002; ChristmasParty.7z.003"
                .to_string(),
            ty: ModType::Loader,
            cfg: String::new(),
            icon: String::new(),
            ioa: Vec::new(),
            types: vec![ModType::Loader],
            affected_files: Vec::new(),
            dependencies: Vec::new(),
            folder: String::new(),
            manifest_path: String::new(),
            manifest_text: String::new(),
            screenshots: Vec::new(),
            soon: false,
        };
        let parts = variant.dl_parts();
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[1], "https://example.com/rel/ChristmasParty.7z.002");
        assert_eq!(variant.download_name(), "ChristmasParty-1.7z");
        variant.dl_url = "https://example.com/a.zip".to_string();
        assert_eq!(variant.download_name(), "ChristmasParty-1.zip");
        assert_eq!(
            dep_spec("PoseMod; 6"),
            ("PoseMod".to_string(), "6".to_string())
        );
        assert_eq!(dep_spec("PoseMod"), ("PoseMod".to_string(), String::new()));
    }

    #[test]
    fn version_order_is_numeric() {
        assert_eq!(compare_versions("3.10", "3.2"), Ordering::Greater);
        assert_eq!(compare_versions("5.4.23.6", "5.4.22"), Ordering::Greater);
        assert_eq!(compare_versions("4", "4"), Ordering::Equal);
    }

    #[test]
    fn unity_versions_match() {
        let build = game("2020.01.01", "");
        assert_eq!(compat_of("U17", Some(&build)), Compat::Yes);
        assert_eq!(compat_of("U19", Some(&build)), Compat::No);
        assert_eq!(compat_of(">=U17", Some(&build)), Compat::Yes);
        assert_eq!(compat_of("<=U17", Some(&build)), Compat::Yes);
        assert_eq!(compat_of("U6", Some(&build)), Compat::No);
        // Дата — резервный источник поколения Unity.
        assert_eq!(compat_of("U17", Some(&build)), Compat::Yes);
    }

    #[test]
    fn dates_and_periods_match() {
        let build = game("2019.01.15", "");
        assert_eq!(compat_of("2019.01.15", Some(&build)), Compat::Yes);
        assert_eq!(
            compat_of("2017.10.04-2019.07.01", Some(&build)),
            Compat::Yes
        );
        assert_eq!(compat_of(">=2019.01.01", Some(&build)), Compat::Yes);
        assert_eq!(compat_of("<=2018.12.31", Some(&build)), Compat::No);
    }

    #[test]
    fn missing_build_is_unknown() {
        assert_eq!(compat_of("U19", None), Compat::Unknown);
        assert_eq!(
            compat_of("", Some(&game("2020.01.01", ""))),
            Compat::Unknown
        );
        assert_eq!(
            compat_of("own", Some(&game("2020.01.01", ""))),
            Compat::Unknown
        );
    }

    #[test]
    fn parses_flat_and_structured_index() {
        let flat = parse_index("files:\n  - ML/A/1.0/info.yaml\n  - ML/A/1.0/1.webp\n").unwrap();
        assert_eq!(flat.len(), 2);
        let structured = parse_index(
            "mods:\n  - manifest: PM/Bee/1.3/info.yaml\n    screenshots:\n      - PM/Bee/1.3/1.webp\n",
        )
        .unwrap();
        assert!(structured.contains(&"PM/Bee/1.3/info.yaml".to_string()));
        assert!(structured.contains(&"PM/Bee/1.3/1.webp".to_string()));
    }

    #[test]
    fn raw_manifest_accepts_type_alias() {
        let with_type: RawManifest = serde_yaml::from_str("type: Plugin\n").unwrap();
        let with_ty: RawManifest = serde_yaml::from_str("ty: Plugin\n").unwrap();
        assert_eq!(with_type.ty, "Plugin");
        assert_eq!(with_ty.ty, "Plugin");
    }

    #[test]
    fn raw_manifest_accepts_numbers_and_empty_lists() {
        let raw: RawManifest = serde_yaml::from_str(
            "id: Test\nname: Test\nv: '3.10'\ngameV: U19\naffected_files:\n  - ''\ndependencies: []\n",
        )
        .unwrap();
        assert_eq!(raw.v, "3.10");
        assert_eq!(raw.game_v, "U19");
        assert!(raw.affected_files.is_empty());

        // Числа в YAML тоже допустимы: автор может написать `v: 5`.
        let numeric: RawManifest = serde_yaml::from_str("v: 5\n").unwrap();
        assert_eq!(numeric.v, "5");
    }

    #[test]
    fn cfg_path_with_backslashes_is_parsed() {
        let yaml = "id: Test\ncfg: \"BepInEx\\config\\BepInEx.cfg\"\n";
        let raw: RawManifest = serde_yaml::from_str(&sanitize_cfg_backslashes(yaml)).unwrap();
        assert_eq!(raw.cfg, r"BepInEx\config\BepInEx.cfg");
    }

    #[test]
    fn composite_types_are_split() {
        assert_eq!(
            ModType::parse_all("TR; Plugin; Translate"),
            vec![ModType::Textures, ModType::Plugin, ModType::Translate]
        );
        // «Patch» и «Mod» — одна и та же категория, дубликатов быть не должно.
        assert_eq!(ModType::parse_all("mod, patch"), vec![ModType::Mod]);
        assert_eq!(ModType::parse("TR; Plugin"), Some(ModType::Textures));
        assert!(ModType::parse_all("unknown").is_empty());
    }

    #[test]
    fn types_fall_back_to_folder_name() {
        assert_eq!(ModType::from_folder("TR/LOL/2.gif"), ModType::Textures);
        assert_eq!(
            ModType::from_folder("Plugins/ModMenu/info.yaml"),
            ModType::Plugin
        );
        assert_eq!(
            ModType::from_folder("Translate/Ru17.09.2023/info.yaml"),
            ModType::Translate
        );
        assert_eq!(ModType::from_folder("Other/Thing/info.yaml"), ModType::Mod);
    }

    #[test]
    fn youtube_links_are_removed_from_description() {
        let text = "Крутой мод\nВидео: https://youtu.be/dQw4w9WgXcQ\nСмотрите!";
        let clean = strip_youtube_links(text);
        assert_eq!(clean, "Крутой мод\nВидео\nСмотрите!");
    }

    #[test]
    fn unity_periods_match() {
        let u17 = game("2017.10.04", "2017.4.0f1");
        assert_eq!(compat_of("U5-U6", Some(&u17)), Compat::Yes);
        assert_eq!(compat_of("U4 - U6", Some(&u17)), Compat::Yes);
        assert_eq!(compat_of("U5–U6", Some(&u17)), Compat::Yes);
        assert_eq!(compat_of("U20-U6", Some(&u17)), Compat::No);

        let u20 = game("2020.06.01", "2020.3.0f1");
        assert_eq!(compat_of("U17-U20", Some(&u20)), Compat::Yes);
        assert_eq!(compat_of("U5-U17", Some(&u20)), Compat::No);
    }

    #[test]
    fn credits_are_split_into_links() {
        let links = credit_links(
            "Kenny; https://github.com/Kenny14653; https://www.youtube.com/@polluxe5517",
        );
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].kind, LinkKind::GitHub);
        assert_eq!(links[1].kind, LinkKind::YouTube);
        assert!(links[1].url.starts_with("https://"));

        let discord = credit_links("discord.gg/abcd");
        assert_eq!(discord.len(), 1);
        assert_eq!(discord[0].kind, LinkKind::Discord);
        assert_eq!(discord[0].url, "https://discord.gg/abcd");
    }

    #[test]
    fn youtube_ids_are_extracted() {
        let ids = youtube_ids(
            "Трейлер: https://www.youtube.com/watch?v=dQw4w9WgXcQ и https://youtu.be/aqz-KE-bpKQ",
        );
        assert_eq!(ids, vec!["dQw4w9WgXcQ", "aqz-KE-bpKQ"]);
        // Ссылка на канал — не видео.
        assert!(youtube_ids("https://www.youtube.com/@polluxe5517").is_empty());
    }

    /// Проверка живого репозитория YanMod: запускается вручную.
    #[test]
    #[ignore = "требует доступ в интернет"]
    fn real_catalog_loads() {
        let progress = Progress::new();
        let (list, _) = fetch(&progress).expect("каталог должен загрузиться");
        assert!(!list.is_empty());
        for group in &list {
            assert!(!group.variants.is_empty());
        }
        let group = |id: &str| {
            list.iter().find(|group| group.id == id).unwrap_or_else(|| {
                panic!(
                    "нет группы {id}: {:?}",
                    list.iter().map(|g| &g.id).collect::<Vec<_>>()
                )
            })
        };
        let pose = group("PoseMod");
        assert_eq!(pose.ty, ModType::Loader);
        assert_eq!(pose.variants.len(), 8);
        assert!(pose.variants.iter().all(|variant| !variant.soon));

        let beedrops = group("Beedrops");
        assert_eq!(beedrops.ty, ModType::Pm);
        assert_eq!(beedrops.variants.len(), 1);
        assert_eq!(beedrops.variants[0].version, "1.3");
        assert!(beedrops.variants[0].soon);

        let build = group("80sStudentCouncil");
        assert_eq!(build.ty, ModType::Build);
        assert!(build.variants[0].soon);

        let bepin = group("BepinX");
        assert_eq!(bepin.ty, ModType::Loader);
        assert_eq!(bepin.variants.len(), 5);
    }
}
