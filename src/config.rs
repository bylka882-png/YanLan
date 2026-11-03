//! Конфигурация приложения (хранится в %APPDATA%\YanLan\config.json).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::i18n::Lang;

/// Порядок сборок в списке.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SortBy {
    /// Сначала новые версии игры.
    #[default]
    Version,
    /// Сначала самые большие сборки.
    Size,
    /// По названию (А → Я).
    Name,
}

impl SortBy {
    pub fn label(self) -> &'static str {
        match self {
            SortBy::Version => crate::i18n::pick("по версии", "by version"),
            SortBy::Size => crate::i18n::pick("по размеру", "by size"),
            SortBy::Name => crate::i18n::pick("по имени", "by name"),
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            SortBy::Version => {
                crate::i18n::pick("Сначала новые версии игры", "Newest game versions first")
            }
            SortBy::Size => {
                crate::i18n::pick("Сначала самые большие сборки", "Largest builds first")
            }
            SortBy::Name => crate::i18n::pick("По названию: А → Я", "By name: A → Z"),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Корневой каталог лаунчера: в нём лежат games/, downloads/, cache/.
    pub root_dir: PathBuf,
    /// Оставлять скачанные архивы после установки.
    pub keep_archives: bool,
    /// Спрашивать подтверждение перед удалением.
    pub confirm_delete: bool,
    /// Светлая тема.
    pub light_theme: bool,
    /// Автоматически проверять каталог сборок при запуске (не чаще раза в 12 часов).
    pub auto_refresh: bool,
    /// Сортировка списка собранных сборок.
    pub builds_sort: SortBy,
    /// Язык интерфейса.
    pub lang: Lang,
    /// Повторять оборвавшуюся загрузку без ограничения числа попыток.
    pub retry_forever: bool,
    /// Свой прокси для сети: `http://host:port` или `socks5://host:port`.
    pub proxy: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            root_dir: default_root(),
            keep_archives: false,
            confirm_delete: true,
            light_theme: false,
            auto_refresh: true,
            builds_sort: SortBy::Version,
            lang: Lang::Ru,
            retry_forever: true,
            proxy: String::new(),
        }
    }
}

pub fn default_root() -> PathBuf {
    dirs::data_local_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("YanLan")
}

pub fn config_path() -> PathBuf {
    dirs::config_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("YanLan")
        .join("config.json")
}

impl Config {
    pub fn load() -> Self {
        let path = config_path();
        let mut cfg: Config = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(crate::util::strip_bom(&s)).ok())
            .unwrap_or_default();
        if cfg.root_dir.as_os_str().is_empty() {
            cfg.root_dir = default_root();
        }
        cfg
    }

    pub fn save(&self) {
        let path = config_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(text) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, text);
        }
    }

    pub fn games_dir(&self) -> PathBuf {
        self.root_dir.join("games")
    }

    pub fn downloads_dir(&self) -> PathBuf {
        self.root_dir.join("downloads")
    }

    pub fn cache_dir(&self) -> PathBuf {
        self.root_dir.join("cache")
    }

    pub fn ensure_dirs(&self) {
        let _ = std::fs::create_dir_all(self.root_dir.clone());
        let _ = std::fs::create_dir_all(self.games_dir());
        let _ = std::fs::create_dir_all(self.downloads_dir());
        let _ = std::fs::create_dir_all(self.cache_dir());
    }
}
