//! Журнал событий лаунчера: запуски игры и скачивания модов.
//!
//! Append-only JSONL. Нужен анализатору крашей, чтобы связать падение с тем,
//! что происходило за секунды и минуты до него. `library.json` для этого не
//! годится: там только текущее состояние, без истории и времени скачивания.

use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

static JOURNAL_LOCK: Mutex<()> = Mutex::new(());

/// Сколько последних событий оставлять при разрастании файла.
const KEEP_LINES: usize = 4000;

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Entry {
    /// Unix-время начала события (для скачивания — первого байта).
    pub t: i64,
    /// `launch`, `exit` или `download`.
    pub event: String,
    #[serde(default)]
    pub game: String,
    #[serde(default)]
    pub name: String,
    /// Код выхода процесса.
    #[serde(default)]
    pub code: i32,
    #[serde(default)]
    pub seconds: i64,
    #[serde(default)]
    pub crash: bool,
    /// Длительность скачивания в миллисекундах.
    #[serde(default)]
    pub ms: i64,
    #[serde(default)]
    pub bytes: u64,
    /// Включённые моды на момент запуска.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mods: Vec<String>,
}

pub fn path(root: &Path) -> PathBuf {
    root.join("journal.jsonl")
}

pub fn append(root: &Path, entry: Entry) {
    let _guard = JOURNAL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _ = fs::create_dir_all(root);
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path(root));
    let Ok(mut file) = file else {
        return;
    };
    if let Ok(line) = serde_json::to_string(&entry) {
        let _ = writeln!(file, "{line}");
    }
    drop(file);
    prune(root);
}

pub fn read(root: &Path) -> Vec<Entry> {
    let Ok(file) = fs::File::open(path(root)) else {
        return Vec::new();
    };
    BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter_map(|line| serde_json::from_str(crate::util::strip_bom(&line)).ok())
        .collect()
}

/// Обрезает журнал, когда он вырос: старые события для диагностики не нужны.
fn prune(root: &Path) {
    let path = path(root);
    let Ok(text) = fs::read_to_string(&path) else {
        return;
    };
    if text.len() < 512 * 1024 {
        return;
    }
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= KEEP_LINES {
        return;
    }
    let tail = lines[lines.len() - KEEP_LINES..].join("\n");
    let _ = crate::util::write_atomic(&path, format!("{tail}\n"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::now_ts;

    #[test]
    fn appended_entries_are_read_back() {
        let root = std::env::temp_dir().join(format!("yanlan-journal-{}", now_ts()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        append(
            &root,
            Entry {
                t: 100,
                event: "launch".to_string(),
                game: "ioa".to_string(),
                mods: vec!["Pose Mod".to_string()],
                ..Default::default()
            },
        );
        append(
            &root,
            Entry {
                t: 101,
                event: "download".to_string(),
                name: "Pose Mod".to_string(),
                ms: 1500,
                ..Default::default()
            },
        );
        // Битая строка не должна ломать чтение.
        let mut file = OpenOptions::new().append(true).open(path(&root)).unwrap();
        writeln!(file, "not json").unwrap();
        drop(file);
        let entries = read(&root);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].event, "launch");
        assert_eq!(entries[1].ms, 1500);
        let _ = fs::remove_dir_all(&root);
    }
}
