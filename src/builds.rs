//! Каталог сборок Yandere Simulator.
//!
//! Официальный сайт отдаёт только последнюю версию, поэтому полный список
//! старых сборок берётся из зеркал на archive.org. Каждое зеркало — отдельный
//! item, метаданные которого содержат имена и размеры файлов.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::i18n::{fill, pick};
use crate::net;
use crate::progress::Progress;
use crate::util::{now_ts, sanitize_name};

/// id зеркала и подпись источника.
const ARCHIVE_ITEMS: &[(&str, &str, &str)] = &[
    (
        "yandere-simulator-2026-archive",
        "2026",
        "archive.org · 2026",
    ),
    (
        "yandere-simulator-2025-archive",
        "2025",
        "archive.org · 2025",
    ),
    (
        "yandere-simulator-2024-archive",
        "2024",
        "archive.org · 2024",
    ),
    (
        "yandere-simulator-2023-archive",
        "2023",
        "archive.org · 2023",
    ),
    (
        "dc778803-73c7-406b-8739-61fdf82294b5",
        "2014–2022",
        "archive.org · 2014–2022",
    ),
    (
        "yandere-simulator-versions",
        "разное",
        "archive.org · разное",
    ),
];

#[derive(Clone, Serialize, Deserialize)]
pub struct Build {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub date: Option<String>,
    pub url: String,
    #[serde(default)]
    pub size: u64,
    pub source: String,
    #[serde(default)]
    pub group: String,
    #[serde(default)]
    pub format: String,
    #[serde(default)]
    pub page: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    /// Время появления файла в зеркале (unix), если известно.
    #[serde(default)]
    pub added_at: i64,
    #[serde(default)]
    pub filename: String,
}

impl Build {
    #[allow(dead_code)]
    pub fn is_latest(&self) -> bool {
        self.id == "latest"
    }

    /// Поколение Unity, на котором собрана эта версия игры.
    pub fn unity_label(&self) -> Option<&'static str> {
        if self.id == "latest" {
            return Some("Unity 6");
        }
        if self.date.as_deref() == Some("2020-04-10") && self.id.to_lowercase().contains("v2") {
            return Some("Unity 2019");
        }
        unity_for_date(self.date.as_deref())
    }
}

/// Определяет поколение Unity по дате сборки Yandere Simulator.
///
/// Границы взяты по сменам движка: Unity 4 → 5 (2017.06.28), 5 → 2017
/// (2018.07.04), 2017 → 2019 (2020.04.10 V2), 2019 → 2020 (2021.11.12),
/// 2020 → Unity 6 (2025.02.15).
pub fn unity_for_date(date: Option<&str>) -> Option<&'static str> {
    let date = date?;
    Some(if date <= "2017-06-28" {
        "Unity 4"
    } else if date <= "2018-07-04" {
        "Unity 5"
    } else if date <= "2020-04-10" {
        "Unity 2017"
    } else if date <= "2021-11-12" {
        "Unity 2019"
    } else if date <= "2025-02-15" {
        "Unity 2020"
    } else {
        "Unity 6"
    })
}

pub fn latest() -> Build {
    Build {
        id: "latest".to_string(),
        label: pick("Последняя версия", "Latest version").to_string(),
        date: None,
        url: "https://yanderesimulator.com/dl/latest.zip".to_string(),
        size: 0,
        source: "yanderesimulator.com".to_string(),
        group: "official".to_string(),
        format: "zip".to_string(),
        page: Some("https://yanderesimulator.com/download/".to_string()),
        note: Some(
            pick(
                "Официальная сборка с сайта разработчика",
                "Official build from the developer's website",
            )
            .to_string(),
        ),
        added_at: 0,
        filename: "YandereSimulator-latest.zip".to_string(),
    }
}

/// Подпись источника сборки на текущем языке.
pub fn source_label(source: &str) -> String {
    match source {
        "archive.org · разное" => {
            pick("archive.org · разное", "archive.org · misc").to_string()
        }
        "локальная папка" => pick("локальная папка", "local folder").to_string(),
        "официальный сайт" => {
            pick("официальный сайт", "official website").to_string()
        }
        other => other.to_string(),
    }
}

#[derive(Serialize, Deserialize)]
pub struct CachedBuilds {
    pub fetched_at: i64,
    pub builds: Vec<Build>,
}

pub fn cache_path(cfg: &Config) -> std::path::PathBuf {
    cfg.cache_dir().join("builds.json")
}

pub fn load_cache(cfg: &Config) -> Option<CachedBuilds> {
    let text = std::fs::read_to_string(cache_path(cfg)).ok()?;
    let mut cached: CachedBuilds = serde_json::from_str(crate::util::strip_bom(&text)).ok()?;
    // Старые версии сохраняли в note готовую русскую подпись — она не зависит
    // от выбранного языка, поэтому такие заметки просто отбрасываем.
    for build in &mut cached.builds {
        if let Some(note) = &build.note {
            if note.starts_with("добавлено ") || note == "специальная сборка"
            {
                build.note = None;
            }
        }
    }
    Some(cached)
}

pub fn save_cache(cfg: &Config, builds: &[Build]) {
    let cached = CachedBuilds {
        fetched_at: now_ts(),
        builds: builds.to_vec(),
    };
    if let Ok(text) = serde_json::to_string(&cached) {
        let _ = crate::util::write_atomic(&cache_path(cfg), text);
    }
}

#[derive(Deserialize)]
struct IaResponse {
    #[serde(default)]
    files: Vec<IaFile>,
}

#[derive(Deserialize)]
struct IaFile {
    name: String,
    #[serde(default)]
    size: Option<serde_json::Value>,
    #[serde(default)]
    mtime: Option<serde_json::Value>,
}

fn value_to_u64(value: &Option<serde_json::Value>) -> u64 {
    match value {
        Some(serde_json::Value::String(s)) => s.parse::<u64>().unwrap_or(0),
        Some(serde_json::Value::Number(n)) => n.as_u64().unwrap_or(0),
        _ => 0,
    }
}

fn archive_url(item: &str, name: &str) -> String {
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
        .add(b'>')
        .add(b'/');
    let encoded: Vec<String> = name
        .split('/')
        .map(|part| utf8_percent_encode(part, SEG).to_string())
        .collect();
    format!("https://archive.org/download/{item}/{}", encoded.join("/"))
}

fn month_from_name(name: &str) -> Option<u32> {
    let lower = name.to_lowercase();
    let start = lower.find(|c: char| c.is_ascii_alphabetic())?;
    let key: String = lower.chars().skip(start).take(3).collect();
    let months = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    months.iter().position(|m| *m == key).map(|i| i as u32 + 1)
}

fn valid_date(y: i32, m: u32, d: u32) -> bool {
    (2013..=2030).contains(&y) && (1..=12).contains(&m) && (1..=31).contains(&d)
}

fn variant_of(stem: &str) -> String {
    let lower = stem.to_lowercase();
    if let Some(caps) = VERSION_RE
        .get_or_init(|| {
            regex::Regex::new(r"(?i)(?:^|[^a-z])v(\d{1,2})(?:[^0-9]|$)").expect("regex")
        })
        .captures(&lower)
    {
        if let Some(v) = caps.get(1) {
            return format!("v{}", v.as_str());
        }
    }
    if let Some(caps) = B_SUFFIX_RE
        .get_or_init(|| regex::Regex::new(r"(?i)(?:^|[\s_\-])[\(\[]?b[\)\]]?$").expect("regex"))
        .captures(&lower)
    {
        if caps.get(0).is_some() {
            return "b".to_string();
        }
    }
    String::new()
}

static VERSION_RE: OnceLock<regex::Regex> = OnceLock::new();
static B_SUFFIX_RE: OnceLock<regex::Regex> = OnceLock::new();
static MONTH_RE: OnceLock<regex::Regex> = OnceLock::new();
static YMD_RE: OnceLock<regex::Regex> = OnceLock::new();
static US_DATE_RE: OnceLock<regex::Regex> = OnceLock::new();

/// Разбирает имя файла сборки в дату `YYYY-MM-DD` и вариант (`v2`, `b`, ...).
pub fn parse_name(name: &str) -> Option<(String, String)> {
    let stem = name
        .rsplit('/')
        .next()
        .unwrap_or(name)
        .rsplit_once('.')
        .map(|(s, _)| s)
        .unwrap_or(name);

    if let Some(caps) = MONTH_RE.get_or_init(|| {
        regex::Regex::new(
            r"(?i)(jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec)[a-z]*\s+(\d{1,2})(?:st|nd|rd|th)?\s*,?\s*(\d{4})",
        )
        .expect("regex")
    }).captures(stem)
    {
        let month = month_from_name(caps.get(1)?.as_str())?;
        let day: u32 = caps.get(2)?.as_str().parse().ok()?;
        let year: i32 = caps.get(3)?.as_str().parse().ok()?;
        if valid_date(year, month, day) {
            return Some((
                format!("{year:04}-{month:02}-{day:02}"),
                variant_of(stem),
            ));
        }
    }

    if let Some(caps) = YMD_RE
        .get_or_init(|| {
            regex::Regex::new(r"(?i)\bys_(\d{4})_(\d{2})_(\d{2})(v\d+)?").expect("regex")
        })
        .captures(stem)
    {
        let year: i32 = caps.get(1)?.as_str().parse().ok()?;
        let month: u32 = caps.get(2)?.as_str().parse().ok()?;
        let day: u32 = caps.get(3)?.as_str().parse().ok()?;
        if valid_date(year, month, day) {
            let variant = caps
                .get(4)
                .map(|m| m.as_str().to_lowercase())
                .unwrap_or_else(|| variant_of(stem));
            return Some((format!("{year:04}-{month:02}-{day:02}"), variant));
        }
    }

    if let Some(caps) = US_DATE_RE
        .get_or_init(|| regex::Regex::new(r"(?i)(\d{4})[-_.](\d{2})[-_.](\d{2})").expect("regex"))
        .captures(stem)
    {
        let year: i32 = caps.get(1)?.as_str().parse().ok()?;
        let month: u32 = caps.get(2)?.as_str().parse().ok()?;
        let day: u32 = caps.get(3)?.as_str().parse().ok()?;
        if valid_date(year, month, day) {
            return Some((format!("{year:04}-{month:02}-{day:02}"), variant_of(stem)));
        }
    }
    None
}

fn ext_of(name: &str) -> String {
    name.rsplit('.').next().unwrap_or("").to_ascii_lowercase()
}

fn build_from_date(
    date: &str,
    variant: &str,
    file: &IaFile,
    item: &str,
    group: &str,
    source: &str,
) -> Build {
    let mut label = date.replace('-', ".");
    let mut id = label.clone();
    if !variant.is_empty() {
        label.push_str(&format!(" ({})", variant.to_uppercase()));
        id.push('-');
        id.push_str(variant);
    }
    let filename = file
        .name
        .rsplit('/')
        .next()
        .unwrap_or(&file.name)
        .to_string();
    Build {
        id,
        label,
        date: Some(date.to_string()),
        url: archive_url(item, &file.name),
        size: value_to_u64(&file.size),
        source: source.to_string(),
        group: group.to_string(),
        format: ext_of(&file.name),
        page: Some(format!("https://archive.org/details/{item}")),
        note: None,
        added_at: 0,
        filename,
    }
}

fn fetch_item(item: &str, group: &str, source: &str) -> Result<Vec<Build>, String> {
    let url = format!("https://archive.org/metadata/{item}");
    let response: IaResponse = net::get_json(&url)?;
    let mut result = Vec::new();
    for file in response.files {
        let size = value_to_u64(&file.size);
        if size == 0 {
            continue;
        }
        let ext = ext_of(&file.name);
        if !matches!(ext.as_str(), "zip" | "rar" | "7z" | "car") {
            continue;
        }
        if file.name.contains("_meta.") || file.name.ends_with("_files.xml") {
            continue;
        }
        if let Some((date, variant)) = parse_name(&file.name) {
            result.push(build_from_date(&date, &variant, &file, item, group, source));
        } else {
            // Специальные сборки (Prototype, OsanaRelease, ...) — тоже часть истории.
            let basename = file.name.rsplit('/').next().unwrap_or(&file.name);
            let stem = basename
                .rsplit_once('.')
                .map(|(s, _)| s)
                .unwrap_or(basename);
            let label = stem.replace('_', " ");
            let mtime = value_to_u64(&file.mtime);
            result.push(Build {
                id: format!("special-{}", sanitize_name(stem)),
                label,
                date: None,
                url: archive_url(item, &file.name),
                size,
                source: source.to_string(),
                group: group.to_string(),
                format: ext,
                page: Some(format!("https://archive.org/details/{item}")),
                note: None,
                added_at: mtime as i64,
                filename: basename.to_string(),
            });
        }
    }
    Ok(result)
}

/// Загружает полный каталог сборок со всех зеркал.
pub fn fetch_all(progress: &Progress) -> Result<(Vec<Build>, Vec<String>), String> {
    let total = ARCHIVE_ITEMS.len();
    progress.stage(pick(
        "Загрузка каталога сборок",
        "Loading the build catalog",
    ));
    progress.total(total as u64);
    let done = AtomicUsize::new(0);

    let mut collected: Vec<Result<Vec<Build>, String>> = Vec::new();
    std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for (item, group, source) in ARCHIVE_ITEMS {
            let progress_ref = progress.clone();
            let done_ref = &done;
            handles.push(scope.spawn(move || {
                let result = fetch_item(item, group, source);
                let finished = done_ref.fetch_add(1, Ordering::SeqCst) + 1;
                progress_ref.set_done(finished as u64);
                let count = result.as_ref().map(|v| v.len()).unwrap_or(0);
                progress_ref.detail(fill(
                    &pick("{source}: {count} сборок", "{source}: {count} builds"),
                    &[("source", source), ("count", &count.to_string())],
                ));
                result
            }));
        }
        for handle in handles {
            if let Ok(result) = handle.join() {
                collected.push(result);
            }
        }
    });

    let mut builds: Vec<Build> = Vec::new();
    let mut errors = Vec::new();
    let mut order: HashMap<String, usize> = HashMap::new();
    for (index, (_, _, source)) in ARCHIVE_ITEMS.iter().enumerate() {
        order.insert(source.to_string(), index);
    }

    for result in collected {
        match result {
            Ok(list) => builds.extend(list),
            Err(err) => errors.push(err),
        }
    }

    if builds.is_empty() {
        return Err(fill(
            &pick(
                "Не удалось получить список сборок. {details}",
                "Failed to fetch the build list. {details}",
            ),
            &[("details", &errors.first().cloned().unwrap_or_default())],
        ));
    }

    // Убираем дубликаты: одна и та же дата может встречаться в нескольких зеркалах.
    // Приоритет — порядок в ARCHIVE_ITEMS (годовые архивы важнее «сборных» item'ов).
    let mut merged: HashMap<String, (usize, Build)> = HashMap::new();
    for build in builds {
        let priority = *order.get(&build.source).unwrap_or(&99);
        let should_replace = merged
            .get(&build.id)
            .map(|(existing, _)| priority < *existing)
            .unwrap_or(true);
        if should_replace {
            merged.insert(build.id.clone(), (priority, build));
        }
    }
    let mut final_list: Vec<Build> = merged.into_values().map(|(_, b)| b).collect();
    final_list.sort_by(|a, b| match (&b.date, &a.date) {
        (Some(x), Some(y)) => x.cmp(y),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.label.cmp(&b.label),
    });
    Ok((final_list, errors))
}

/// Возвращает кэш и признак того, что его стоит обновить.
#[allow(dead_code)]
pub fn load_or_refresh(cfg: &Config) -> (Option<CachedBuilds>, bool) {
    let cached = load_cache(cfg);
    let stale = cached
        .as_ref()
        .map(|c| now_ts() - c.fetched_at > 12 * 3600)
        .unwrap_or(true);
    (cached, stale)
}

#[allow(dead_code)]
pub fn total_size(builds: &[Build]) -> u64 {
    builds.iter().map(|b| b.size).sum()
}

#[allow(dead_code)]
pub fn games_dir(cfg: &Config) -> std::path::PathBuf {
    cfg.games_dir()
}

#[allow(dead_code)]
pub fn exists(builds: &[Build], id: &str) -> bool {
    builds.iter().any(|b| b.id == id)
}

#[allow(dead_code)]
pub fn build_path(root: &Path, id: &str) -> std::path::PathBuf {
    root.join("games").join(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_month_names() {
        assert_eq!(
            parse_name("Yandere Simulator October 17th 2026.zip"),
            Some(("2026-10-17".into(), "".into()))
        );
        assert_eq!(
            parse_name("2026/2 February/Yandere Simulator Februrary 1st 2026.zip"),
            Some(("2026-02-01".into(), "".into()))
        );
        assert_eq!(
            parse_name("Yandere Simulator October 1st 2026 - B.zip"),
            Some(("2026-10-01".into(), "b".into()))
        );
    }

    #[test]
    fn parses_underscore_and_dash_dates() {
        assert_eq!(
            parse_name("ys_2018_05_01.zip"),
            Some(("2018-05-01".into(), "".into()))
        );
        assert_eq!(
            parse_name("ys_2015_05_20v2.rar"),
            Some(("2015-05-20".into(), "v2".into()))
        );
        assert_eq!(
            parse_name("YandereSimulator-2023-08-15-v2.zip"),
            Some(("2023-08-15".into(), "v2".into()))
        );
    }

    #[test]
    fn ignores_non_builds() {
        assert_eq!(parse_name("YandereSimulatorPrototype.zip"), None);
        assert_eq!(parse_name("Test.zip"), None);
    }

    #[test]
    fn maps_dates_to_unity_generations() {
        assert_eq!(unity_for_date(Some("2014-01-01")), Some("Unity 4"));
        assert_eq!(unity_for_date(Some("2017-06-28")), Some("Unity 4"));
        assert_eq!(unity_for_date(Some("2017-07-09")), Some("Unity 5"));
        assert_eq!(unity_for_date(Some("2018-07-04")), Some("Unity 5"));
        assert_eq!(unity_for_date(Some("2018-07-13")), Some("Unity 2017"));
        assert_eq!(unity_for_date(Some("2020-04-10")), Some("Unity 2017"));
        assert_eq!(unity_for_date(Some("2020-05-01")), Some("Unity 2019"));
        assert_eq!(unity_for_date(Some("2021-11-12")), Some("Unity 2019"));
        assert_eq!(unity_for_date(Some("2021-11-13")), Some("Unity 2020"));
        assert_eq!(unity_for_date(Some("2025-02-15")), Some("Unity 2020"));
        assert_eq!(unity_for_date(Some("2025-03-15")), Some("Unity 6"));
        assert_eq!(unity_for_date(None), None);
    }
}
