//! HTTP-клиент: JSON, байты, скачивание файлов с докачкой и прогрессом.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{OnceLock, RwLock};
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;

use crate::i18n::{fill, pick};
use crate::progress::Progress;

pub const UA: &str = "YanLan/1.6 (Windows; Yandere Simulator launcher)";

static PROXY: OnceLock<RwLock<Option<String>>> = OnceLock::new();
static AGENT: OnceLock<RwLock<Option<ureq::Agent>>> = OnceLock::new();

fn proxy_cell() -> &'static RwLock<Option<String>> {
    PROXY.get_or_init(|| RwLock::new(None))
}

fn agent_cell() -> &'static RwLock<Option<ureq::Agent>> {
    AGENT.get_or_init(|| RwLock::new(None))
}

/// Задаёт прокси для всех сетевых запросов лаунчера. `None` — подключаться напрямую.
pub fn set_proxy(proxy: Option<String>) {
    if let Ok(mut guard) = proxy_cell().write() {
        *guard = proxy.filter(|p| !p.trim().is_empty());
    }
    // Соединения старого агента идут через прежний прокси — начинаем заново.
    if let Ok(mut guard) = agent_cell().write() {
        *guard = None;
    }
}

fn agent() -> ureq::Agent {
    if let Ok(guard) = agent_cell().read() {
        if let Some(agent) = guard.as_ref() {
            return agent.clone();
        }
    }
    let agent = build_agent();
    if let Ok(mut guard) = agent_cell().write() {
        *guard = Some(agent.clone());
    }
    agent
}

fn build_agent() -> ureq::Agent {
    let mut builder = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(30))
        .timeout_read(Duration::from_secs(90))
        .timeout_write(Duration::from_secs(90))
        .user_agent(UA);
    if let Ok(guard) = proxy_cell().read() {
        if let Some(proxy) = guard.as_deref() {
            if let Ok(proxy) = ureq::Proxy::new(proxy) {
                builder = builder.proxy(proxy);
            }
        }
    }
    builder.build()
}

fn describe(err: ureq::Error) -> String {
    match err {
        ureq::Error::Status(code, response) => {
            let text = response.into_string().unwrap_or_default();
            let hint = match code {
                403 => pick(" (доступ запрещён)", " (access denied)"),
                404 => pick(" (не найдено)", " (not found)"),
                429 => pick(" (слишком много запросов)", " (too many requests)"),
                _ => "",
            };
            let snippet: String = text.chars().take(200).collect();
            format!("HTTP {code}{hint}. {snippet}")
        }
        ureq::Error::Transport(t) => fill(
            &pick("Сеть недоступна: {err}", "Network unavailable: {err}"),
            &[("err", &t.to_string())],
        ),
    }
}

pub fn get_json<T: DeserializeOwned>(url: &str) -> Result<T, String> {
    let response = agent()
        .get(url)
        .set("Accept", "application/json")
        .call()
        .map_err(describe)?;
    response.into_json::<T>().map_err(|e| {
        fill(
            &pick("Некорректный JSON: {err}", "Invalid JSON: {err}"),
            &[("err", &e.to_string())],
        )
    })
}

/// Скачивает текстовый файл (манифест, index.yaml).
pub fn get_text(url: &str) -> Result<String, String> {
    let response = agent().get(url).call().map_err(describe)?;
    response.into_string().map_err(|e| {
        fill(
            &pick(
                "Не удалось прочитать ответ: {err}",
                "Failed to read the response: {err}",
            ),
            &[("err", &e.to_string())],
        )
    })
}

/// То же, но 404 считается нормальным отсутствием файла.
pub fn get_text_opt(url: &str) -> Result<Option<String>, String> {
    match agent().get(url).call() {
        Ok(response) => response.into_string().map(Some).map_err(|e| {
            fill(
                &pick(
                    "Не удалось прочитать ответ: {err}",
                    "Failed to read the response: {err}",
                ),
                &[("err", &e.to_string())],
            )
        }),
        Err(ureq::Error::Status(404, _)) => Ok(None),
        Err(err) => Err(describe(err)),
    }
}

/// Скачивает файл в память — для скриншотов каталога.
pub fn get_bytes(url: &str) -> Result<Vec<u8>, String> {
    let response = agent().get(url).call().map_err(describe)?;
    let mut data = Vec::new();
    response.into_reader().read_to_end(&mut data).map_err(|e| {
        fill(
            &pick("Ошибка сети: {err}", "Network error: {err}"),
            &[("err", &e.to_string())],
        )
    })?;
    Ok(data)
}

/// Скачивает файл, поддерживая докачку. Возвращает размер файла.
///
/// `retry_forever` включает бесконечные повторы: загрузка продолжается с места
/// обрыва, пока пользователь не нажмёт «Отмена».
pub fn download(
    url: &str,
    dest: &Path,
    progress: &Progress,
    retry_forever: bool,
) -> Result<u64, String> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            fill(
                &pick(
                    "Не удалось создать {path}: {err}",
                    "Failed to create {path}: {err}",
                ),
                &[
                    ("path", &parent.display().to_string()),
                    ("err", &e.to_string()),
                ],
            )
        })?;
    }

    let mut attempt = 0u32;
    loop {
        if progress.is_cancelled() {
            return Err(pick("Загрузка отменена", "Download cancelled").to_string());
        }
        attempt += 1;
        match download_once(url, dest, progress) {
            Ok(size) => return Ok(size),
            Err(err) => {
                if progress.is_cancelled() {
                    return Err(pick("Загрузка отменена", "Download cancelled").to_string());
                }
                if !retry_forever && attempt >= 3 {
                    return Err(err);
                }
                let trimmed = crate::util::trim_to(&err, 90);
                if retry_forever {
                    progress.detail(fill(
                        &pick(
                            "Ошибка: {err}. Повтор {attempt}…",
                            "Error: {err}. Retry {attempt}…",
                        ),
                        &[("err", &trimmed), ("attempt", &attempt.to_string())],
                    ));
                } else {
                    progress.detail(fill(
                        &pick(
                            "Ошибка: {err}. Повтор {attempt}/3…",
                            "Error: {err}. Retry {attempt}/3…",
                        ),
                        &[("err", &trimmed), ("attempt", &attempt.to_string())],
                    ));
                }
                // При бесконечных повторах ждём дольше, но не больше 15 секунд,
                // чтобы не заснуть навсегда при долгом обрыве.
                let wait = (attempt * 2).min(if retry_forever { 15 } else { 6 });
                // Спим короткими порциями, чтобы «Отмена» срабатывала сразу.
                let until = Instant::now() + Duration::from_secs(wait as u64);
                while Instant::now() < until {
                    if progress.is_cancelled() {
                        return Err(pick("Загрузка отменена", "Download cancelled").to_string());
                    }
                    std::thread::sleep(Duration::from_millis(200));
                }
            }
        }
    }
}

fn part_path(dest: &Path) -> std::path::PathBuf {
    let mut name = dest
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "download.bin".to_string());
    name.push_str(".part");
    dest.with_file_name(name)
}

fn download_once(url: &str, dest: &Path, progress: &Progress) -> Result<u64, String> {
    let part = part_path(dest);
    let existing = fs::metadata(&part).map(|m| m.len()).unwrap_or(0);

    let mut request = agent().get(url);
    if existing > 0 {
        request = request.set("Range", &format!("bytes={existing}-"));
    }
    let response = match request.call() {
        Ok(response) => response,
        // Сервер считает, что файл уже скачан целиком.
        Err(ureq::Error::Status(416, _)) if existing > 0 => {
            progress.reset_bytes(existing);
            progress.set_done(existing);
            if dest.exists() {
                let _ = fs::remove_file(dest);
            }
            fs::rename(&part, dest).map_err(|e| {
                fill(
                    &pick(
                        "Не удалось переименовать файл: {err}",
                        "Failed to rename the file: {err}",
                    ),
                    &[("err", &e.to_string())],
                )
            })?;
            return Ok(existing);
        }
        Err(err) => return Err(describe(err)),
    };

    let status = response.status();
    let length = response
        .header("Content-Length")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);

    let resumed = status == 206 && existing > 0;
    let expected = if resumed { existing + length } else { length };
    if resumed {
        progress.detail(pick("Продолжаем загрузку", "Resuming download"));
    } else {
        let _ = fs::remove_file(&part);
    }
    progress.reset_bytes(expected);

    let mut file = if resumed {
        OpenOptions::new()
            .append(true)
            .open(&part)
            .map_err(|e| format!("{}: {e}", part.display()))?
    } else {
        File::create(&part).map_err(|e| format!("{}: {e}", part.display()))?
    };

    let mut reader = response.into_reader();
    let mut buffer = vec![0u8; 256 * 1024];
    let mut done = if resumed { existing } else { 0 };
    loop {
        if progress.is_cancelled() {
            return Err(pick("Загрузка отменена", "Download cancelled").to_string());
        }
        let read = reader.read(&mut buffer).map_err(|e| {
            fill(
                &pick("Ошибка сети: {err}", "Network error: {err}"),
                &[("err", &e.to_string())],
            )
        })?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read]).map_err(|e| {
            fill(
                &pick("Ошибка записи: {err}", "Write error: {err}"),
                &[("err", &e.to_string())],
            )
        })?;
        done += read as u64;
        progress.set_done(done);
    }
    file.flush().map_err(|e| {
        fill(
            &pick("Ошибка записи: {err}", "Write error: {err}"),
            &[("err", &e.to_string())],
        )
    })?;
    drop(file);

    if expected > 0 && done < expected {
        return Err(fill(
            &pick(
                "Файл получен не полностью ({done} из {total})",
                "File incomplete ({done} of {total})",
            ),
            &[
                ("done", &crate::util::human_size(done)),
                ("total", &crate::util::human_size(expected)),
            ],
        ));
    }

    if dest.exists() {
        let _ = fs::remove_file(dest);
    }
    fs::rename(&part, dest).map_err(|e| {
        fill(
            &pick(
                "Не удалось переименовать файл: {err}",
                "Failed to rename the file: {err}",
            ),
            &[("err", &e.to_string())],
        )
    })?;
    Ok(done)
}
