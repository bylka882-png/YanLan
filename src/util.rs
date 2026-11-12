//! Мелкие утилиты: форматирование, работа с путями, запуск внешних программ.

use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::i18n::{fill, is_ru, pick};

pub const MONTHS_RU: [&str; 12] = [
    "января",
    "февраля",
    "марта",
    "апреля",
    "мая",
    "июня",
    "июля",
    "августа",
    "сентября",
    "октября",
    "ноября",
    "декабря",
];

pub const MONTHS_EN: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

const MONTHS_EN_SHORT: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

pub fn human_size(bytes: u64) -> String {
    let units: [&str; 5] = pick(["Б", "КБ", "МБ", "ГБ", "ТБ"], ["B", "KB", "MB", "GB", "TB"]);
    let mut value = bytes as f64;
    let mut unit = 0usize;
    while value >= 1024.0 && unit + 1 < units.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", units[0])
    } else {
        format!("{value:.1} {}", units[unit])
    }
}

pub fn human_speed(bytes_per_sec: f64) -> String {
    format!(
        "{}{}",
        human_size(bytes_per_sec.max(0.0) as u64),
        pick("/с", "/s")
    )
}

pub fn human_eta(seconds: f64) -> String {
    if !seconds.is_finite() || seconds < 0.0 {
        return String::new();
    }
    let total = seconds.round() as u64;
    if total >= 3600 {
        pick(
            format!("{} ч {} мин", total / 3600, (total % 3600) / 60),
            format!("{} h {} min", total / 3600, (total % 3600) / 60),
        )
    } else if total >= 60 {
        pick(
            format!("{} мин {} с", total / 60, total % 60),
            format!("{} min {} s", total / 60, total % 60),
        )
    } else {
        pick(format!("{total} с"), format!("{total} s"))
    }
}

pub fn now_ts() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Разложение unix-времени в гражданскую дату (алгоритм Ховарда Хиннанта).
/// Время трактуется как UTC — для локального используйте [`local_civil`].
pub fn civil_from_ts(ts: i64) -> (i32, u32, u32, u32, u32) {
    let days = ts.div_euclid(86_400);
    let secs_of_day = ts.rem_euclid(86_400) as u32;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = if m <= 2 { y + 1 } else { y } as i32;
    (year, m, d, secs_of_day / 3600, (secs_of_day % 3600) / 60)
}

#[repr(C)]
#[derive(Default)]
struct LocalTm {
    sec: i32,
    min: i32,
    hour: i32,
    mday: i32,
    mon: i32,
    year: i32,
    wday: i32,
    yday: i32,
    isdst: i32,
}

/// Раскладывает unix-время в поля локального времени компьютера.
///
/// `civil_from_ts` работает в UTC, поэтому показывать его напрямую нельзя:
/// в UTC+3 время отставало бы на три часа, а в отрицательных поясах — убегало
/// вперёд. Здесь используем системный `localtime`, чтобы дата и часы совпадали
/// с часами Windows при любом часовом поясе и переходе на летнее время.
pub fn local_civil(ts: i64) -> (i32, u32, u32, u32, u32) {
    if ts <= 0 {
        return (1970, 1, 1, 0, 0);
    }

    #[cfg(windows)]
    {
        // UCRT: errno_t _localtime64_s(struct tm *tmDest, const __time64_t *sourceTime).
        extern "C" {
            fn _localtime64_s(out: *mut LocalTm, time: *const i64) -> i32;
        }
        let mut tm = LocalTm::default();
        // SAFETY: `tm` — валидная выделенная структура, `ts` — корректный
        // указатель на время жизни вызова. Функция только заполняет `tm`.
        if unsafe { _localtime64_s(&mut tm, &ts) } == 0 {
            return (
                tm.year + 1900,
                (tm.mon + 1).clamp(1, 12) as u32,
                tm.mday.clamp(1, 31) as u32,
                tm.hour.clamp(0, 23) as u32,
                tm.min.clamp(0, 59) as u32,
            );
        }
    }

    #[cfg(not(windows))]
    {
        // POSIX: struct tm *localtime_r(const time_t *timep, struct tm *result).
        extern "C" {
            fn localtime_r(time: *const i64, out: *mut LocalTm) -> *mut LocalTm;
        }
        let mut tm = LocalTm::default();
        // SAFETY: см. ветку Windows — структура и указатель валидны.
        if !unsafe { localtime_r(&ts, &mut tm) }.is_null() {
            return (
                tm.year + 1900,
                (tm.mon + 1).clamp(1, 12) as u32,
                tm.mday.clamp(1, 31) as u32,
                tm.hour.clamp(0, 23) as u32,
                tm.min.clamp(0, 59) as u32,
            );
        }
    }

    civil_from_ts(ts)
}

pub fn format_ts(ts: i64) -> String {
    if ts <= 0 {
        return String::new();
    }
    let (y, m, d, hh, mm) = local_civil(ts);
    if is_ru() {
        format!("{d:02}.{m:02}.{y} {hh:02}:{mm:02}")
    } else {
        let month = MONTHS_EN_SHORT[(m as usize).clamp(1, 12) - 1];
        format!("{month} {d}, {y} {hh:02}:{mm:02}")
    }
}

/// Форматирует дату `YYYY-MM-DD` на текущем языке интерфейса.
/// Длительность скачивания: миллисекунды до секунды, дальше — секунды.
pub fn human_ms(ms: i64) -> String {
    if ms >= 1000 {
        format!("{:.1} {}", ms as f64 / 1000.0, pick("с", "s"))
    } else {
        format!("{ms} {}", pick("мс", "ms"))
    }
}

/// То же, что [`format_ts`], но с секундами: для журнала запусков и крашей
/// важна точность до секунды.
pub fn format_ts_secs(ts: i64) -> String {
    if ts <= 0 {
        return String::new();
    }
    let (y, m, d, hh, mm) = local_civil(ts);
    let ss = ts.rem_euclid(60);
    if is_ru() {
        format!("{d:02}.{m:02}.{y} {hh:02}:{mm:02}:{ss:02}")
    } else {
        let month = MONTHS_EN_SHORT[(m as usize).clamp(1, 12) - 1];
        format!("{month} {d}, {y} {hh:02}:{mm:02}:{ss:02}")
    }
}

pub fn format_date(date: &str) -> String {
    let parts: Vec<&str> = date.split('-').collect();
    if parts.len() == 3 {
        if let (Ok(y), Ok(m), Ok(d)) = (
            parts[0].parse::<i32>(),
            parts[1].parse::<usize>(),
            parts[2].parse::<u32>(),
        ) {
            if (1..=12).contains(&m) {
                return if is_ru() {
                    format!("{d} {} {y}", MONTHS_RU[m - 1])
                } else {
                    format!("{} {d}, {y}", MONTHS_EN[m - 1])
                };
            }
        }
    }
    date.to_string()
}

/// Безопасное имя каталога/файла из произвольной строки.
pub fn sanitize_name(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '-' | '_' => out.push(ch),
            ' ' => out.push('_'),
            '(' | '[' | '{' => out.push('-'),
            ')' | ']' | '}' => {}
            _ => {}
        }
        if out.len() >= 80 {
            break;
        }
    }
    let trimmed = out.trim_matches(|c| c == '.' || c == '_' || c == '-');
    if trimmed.is_empty() {
        "build".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Имя каталога сборки: сохраняет кириллицу и пробелы, убирает символы,
/// запрещённые в Windows, и служебные имена вроде `CON` или `NUL`.
pub fn sanitize_dir_name(input: &str) -> String {
    const FORBIDDEN: [char; 9] = ['<', '>', ':', '"', '/', '\\', '|', '?', '*'];
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        if ch.is_control() {
            continue;
        }
        if FORBIDDEN.contains(&ch) {
            out.push('_');
        } else {
            out.push(ch);
        }
    }
    let cleaned = out
        .trim()
        .trim_matches(|c: char| c == '.')
        .trim_end_matches(|c: char| c == ' ' || c == '.');
    let limited: String = cleaned.chars().take(60).collect();
    let limited = limited
        .trim_end_matches(|c: char| c == ' ' || c == '.')
        .trim();
    if limited.is_empty() {
        return "build".to_string();
    }
    let upper = limited.to_ascii_uppercase();
    let reserved = upper == "CON"
        || upper == "PRN"
        || upper == "AUX"
        || upper == "NUL"
        || (upper.len() == 4
            && (upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper.as_bytes()[3].is_ascii_digit());
    if reserved {
        format!("{limited}_")
    } else {
        limited.to_string()
    }
}

/// Убирает BOM, который некоторые редакторы добавляют в начало UTF-8 файла.
pub fn strip_bom(text: &str) -> &str {
    text.trim_start_matches('\u{feff}')
}

static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// Записывает файл атомарно: сначала во временный `<имя>.tmp`, затем rename.
///
/// Обрыв процесса посреди записи оставит мусорный tmp, но не обрежет сам файл.
pub fn write_atomic(path: &Path, data: impl AsRef<[u8]>) -> std::io::Result<()> {
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let tmp = path.with_file_name(format!("{name}.tmp{}-{seq}", std::process::id()));
    let result = std::fs::write(&tmp, data).and_then(|()| std::fs::rename(&tmp, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Проверяет, что относительный путь не выходит за пределы базового каталога.
pub fn safe_relative(rel: &str) -> Option<PathBuf> {
    let normalized = rel.replace('\\', "/");
    let mut out = PathBuf::new();
    for component in Path::new(&normalized).components() {
        match component {
            Component::Normal(part) => out.push(part),
            _ => return None,
        }
    }
    if out.as_os_str().is_empty() {
        None
    } else {
        Some(out)
    }
}

pub fn join_safe(base: &Path, rel: &str) -> Option<PathBuf> {
    safe_relative(rel).map(|r| base.join(r))
}

pub fn open_path(path: &Path) {
    #[cfg(windows)]
    {
        let _ = Command::new("explorer").arg(path).spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("xdg-open").arg(path).spawn();
    }
}

pub fn open_url(url: &str) {
    #[cfg(windows)]
    {
        let _ = Command::new("cmd").args(["/C", "start", "", url]).spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("xdg-open").arg(url).spawn();
    }
}

pub fn dir_size(path: &Path) -> u64 {
    let mut total = 0u64;
    let entries = match std::fs::read_dir(path) {
        Ok(e) => e,
        Err(_) => return 0,
    };
    for entry in entries.flatten() {
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.is_dir() {
            total = total.saturating_add(dir_size(&entry.path()));
        } else {
            total = total.saturating_add(meta.len());
        }
    }
    total
}

pub fn remove_dir_quiet(path: &Path) {
    if path.exists() {
        let _ = std::fs::remove_dir_all(path);
    }
}

pub fn move_contents(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| {
        fill(
            &pick(
                "Не удалось создать {path}: {err}",
                "Failed to create {path}: {err}",
            ),
            &[("path", &to.display().to_string()), ("err", &e.to_string())],
        )
    })?;
    let entries = std::fs::read_dir(from).map_err(|e| {
        fill(
            &pick(
                "Не удалось прочитать {path}: {err}",
                "Failed to read {path}: {err}",
            ),
            &[
                ("path", &from.display().to_string()),
                ("err", &e.to_string()),
            ],
        )
    })?;
    for entry in entries.flatten() {
        let src = entry.path();
        let name = entry.file_name();
        let dst = to.join(&name);
        if dst.exists() {
            if dst.is_dir() {
                remove_dir_quiet(&dst);
            } else {
                let _ = std::fs::remove_file(&dst);
            }
        }
        if std::fs::rename(&src, &dst).is_err() {
            copy_recursive(&src, &dst)?;
            if src.is_dir() {
                remove_dir_quiet(&src);
            } else {
                let _ = std::fs::remove_file(&src);
            }
        }
    }
    Ok(())
}

pub fn copy_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    let meta = std::fs::metadata(src).map_err(|e| format!("{}: {e}", src.display()))?;
    if meta.is_dir() {
        std::fs::create_dir_all(dst).map_err(|e| format!("{}: {e}", dst.display()))?;
        for entry in std::fs::read_dir(src)
            .map_err(|e| format!("{}: {e}", src.display()))?
            .flatten()
        {
            copy_recursive(&entry.path(), &dst.join(entry.file_name()))?;
        }
    } else {
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::copy(src, dst).map_err(|e| format!("{}: {e}", src.display()))?;
    }
    Ok(())
}

/// Разбирает строку параметров запуска на аргументы.
///
/// Пробелы разделяют аргументы, кавычки позволяют передать значение с пробелами:
/// `-screen "1280 720"` → `["-screen", "1280 720"]`.
pub fn split_args(input: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut started = false;
    for ch in input.chars() {
        match ch {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            c if c.is_whitespace() && !quoted => {
                if started || !current.is_empty() {
                    args.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            c => {
                current.push(c);
                started = true;
            }
        }
    }
    if started || !current.is_empty() {
        args.push(current);
    }
    args
}

pub fn trim_to(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        text.to_string()
    } else {
        let mut s: String = text.chars().take(max_chars.saturating_sub(1)).collect();
        s.push('…');
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_relative_rejects_escapes() {
        assert!(safe_relative("../evil").is_none());
        assert!(safe_relative("a/../../b").is_none());
        assert!(safe_relative("C:/Windows/system32").is_none());
        assert_eq!(
            safe_relative("a\\b/c.txt"),
            Some(PathBuf::from("a/b/c.txt"))
        );
    }

    #[test]
    fn atomic_write_replaces_and_leaves_no_tmp() {
        let dir = std::env::temp_dir().join(format!("yanlan_atomic_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("data.json");
        write_atomic(&path, "one").unwrap();
        write_atomic(&path, "two").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "two");
        let leftovers = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp"))
            .count();
        assert_eq!(leftovers, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sizes_are_human() {
        // Тест не зависит от выбранного в системе языка, поэтому сверяемся
        // с тем же вариантом, который вернёт `pick`.
        assert_eq!(human_size(512), format!("512 {}", pick("Б", "B")));
        assert_eq!(human_size(1536), format!("1.5 {}", pick("КБ", "KB")));
    }

    #[test]
    fn dir_names_keep_cyrillic_and_strip_forbidden() {
        assert_eq!(sanitize_dir_name("Сборка для модов"), "Сборка для модов");
        assert_eq!(sanitize_dir_name("a/b:c*d?"), "a_b_c_d_");
        assert_eq!(sanitize_dir_name("  ...  "), "build");
        assert_eq!(sanitize_dir_name("CON"), "CON_");
        assert_eq!(sanitize_dir_name("Игра."), "Игра");
        assert!(sanitize_dir_name(&"я".repeat(100)).chars().count() <= 60);
    }

    #[test]
    fn strips_utf8_bom() {
        assert_eq!(strip_bom("\u{feff}{}"), "{}");
        assert_eq!(strip_bom("{}"), "{}");
    }

    #[test]
    fn dates_are_civil() {
        assert_eq!(civil_from_ts(0), (1970, 1, 1, 0, 0));
        assert_eq!(civil_from_ts(1_700_000_000), (2023, 11, 14, 22, 13));
        // Нулевая метка означает «время неизвестно» — формат пустой.
        assert_eq!(format_ts(0), "");
    }

    #[test]
    fn args_split_respects_quotes() {
        assert_eq!(
            split_args("-screen-width 1280 -screen-height 720"),
            vec!["-screen-width", "1280", "-screen-height", "720"]
        );
        assert_eq!(
            split_args("  -screen \"1280 720\"  'x' "),
            vec!["-screen", "1280 720", "'x'"]
        );
        assert_eq!(split_args("\"a b\" c"), vec!["a b", "c"]);
        assert!(split_args("   ").is_empty());
        assert!(split_args("").is_empty());
    }
}
