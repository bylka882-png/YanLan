//! Анализ логов после краха игры.
//!
//! Логи Unity и BepInEx не имеют общего формата, поэтому анализ построен на
//! признаках, а не на разборе структуры: имена исключений, нативные коды,
//! сигнатуры драйверов, строки стека с файлами модов. Результат — вероятная
//! причина и уверенность в процентах, а не «магическая» цифра.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use crate::i18n::pick;
use crate::journal;
use crate::library::{GameInstall, Library};
use crate::progress::Progress;
use crate::util::{now_ts, sanitize_name};

/// Сколько байт читать с конца файла. Краш обычно в хвосте, а логи бывают
/// на сотни мегабайт — читать целиком нельзя.
const TAIL_BYTES: u64 = 8 * 1024 * 1024;
/// Предел числа найденных файлов, чтобы обход папок не длился вечно.
const MAX_FILES: usize = 300;
const MAX_EVIDENCE: usize = 8;
const MAX_LINE: usize = 220;
/// Мод, установленный за час до запуска, — вероятный подозреваемый.
const RECENT_SECS: i64 = 3600;

#[derive(Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub file: String,
    pub line: usize,
    pub text: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct RecentMod {
    pub name: String,
    pub at: i64,
    #[serde(default)]
    pub download_ms: i64,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CrashReport {
    /// `<unix>-<сборка>`: имя файла отчёта и ключ в списке.
    pub id: String,
    /// Время, когда лаунчер заметил завершение игры.
    pub at: i64,
    pub started: i64,
    pub duration_s: i64,
    pub game_id: String,
    pub game_name: String,
    #[serde(default)]
    pub exit_code: Option<i32>,
    pub confidence: u8,
    /// Машинный вид причины: `mod_crash`, `gpu`, `out_of_memory` и т.д.
    /// Текст собирается при показе, поэтому смена языка не ломает историю.
    pub kind: String,
    /// Короткая выжимка: класс исключения, DLL, сигнатура драйвера, код выхода.
    pub detail: String,
    #[serde(default)]
    pub mods: Vec<String>,
    #[serde(default)]
    pub recent: Vec<RecentMod>,
    #[serde(default)]
    pub evidence: Vec<Evidence>,
    #[serde(default)]
    pub scanned: usize,
    #[serde(default)]
    pub logs: Vec<String>,
}

impl CrashReport {
    pub fn title(&self) -> String {
        let mods = self.mods_text();
        match self.kind.as_str() {
            "mod_incompatible" if !mods.is_empty() => pick(
                format!("Мод {mods} несовместим со сборкой"),
                format!("The mod {mods} is incompatible with the build"),
            ),
            "mod_incompatible" => pick(
                "Плагин несовместим со сборкой",
                "A plugin is incompatible with the build",
            )
            .to_string(),
            "mod_missing_file" if !mods.is_empty() => pick(
                format!("Моду {mods} не хватает файла"),
                format!("The mod {mods} is missing a file"),
            ),
            "mod_missing_file" => {
                pick("Плагину не хватает DLL", "A plugin is missing a DLL").to_string()
            }
            "mod_conflict" if !mods.is_empty() => pick(
                format!("Конфликт модов: {mods}"),
                format!("Mod conflict: {mods}"),
            ),
            "mod_conflict" => pick("Конфликт модов", "Mod conflict").to_string(),
            "mod_crash" if !mods.is_empty() => pick(
                format!("Краш в моде {mods}"),
                format!("Crash inside the mod {mods}"),
            ),
            "mod_crash" => pick("Краш в моде", "Crash inside a mod").to_string(),
            "discord" => pick("Сбой интеграции Discord", "Discord integration failure").to_string(),
            "plugin_incompatible" if !self.detail.is_empty() => pick(
                format!("Плагин «{}» несовместим со сборкой", self.detail),
                format!(
                    "The plugin “{}” is incompatible with the build",
                    self.detail
                ),
            ),
            "plugin_incompatible" => pick(
                "Плагин несовместим со сборкой",
                "A plugin is incompatible with the build",
            )
            .to_string(),
            "incompatible" => pick(
                "Плагин несовместим со сборкой",
                "A plugin is incompatible with the build",
            )
            .to_string(),
            "gpu" => pick("Сбой видеодрайвера", "GPU driver failure").to_string(),
            "out_of_memory" => {
                pick("Игре не хватило памяти", "The game ran out of memory").to_string()
            }
            "runtime" => pick("Сбой рантайма Unity", "Unity runtime failure").to_string(),
            "game_bug" => pick("Исключение в коде игры", "Exception in the game code").to_string(),
            "disk" => pick("Ошибка доступа к файлам", "File or disk access error").to_string(),
            "missing_file" => {
                pick("Не найден файл или DLL", "A file or DLL is missing").to_string()
            }
            "no_logs" => pick("Логи не найдены", "No logs found").to_string(),
            _ => pick("Причина не определена", "Cause not determined").to_string(),
        }
    }

    pub fn cause(&self) -> String {
        let mods = self.mods_text();
        let detail = if self.detail.is_empty() {
            pick("подробностей нет", "no details")
        } else {
            self.detail.as_str()
        };
        match self.kind.as_str() {
            "mod_incompatible" => pick(
                format!(
                    "Стек указывает на мод {mods}, а исключение {detail} означает, что код мода \
                     собран под другую версию игры или BepInEx."
                ),
                format!(
                    "The stack points to {mods} and the {detail} exception means the mod was \
                     built for another game or BepInEx version."
                ),
            ),
            "mod_missing_file" => pick(
                format!(
                    "Мод {mods} обращается к {detail}, которого нет в папке сборки. Обычно это \
                     забытая зависимость или неполный архив мода."
                ),
                format!(
                    "The mod {mods} asks for {detail}, which is not in the build folder. This is \
                     usually a missing dependency or an incomplete mod archive."
                ),
            ),
            "mod_conflict" => pick(
                format!(
                    "В одном стеке упоминаются несколько модов ({mods}) — они патчат один и тот \
                     же код. Побеждает тот, кто загрузился позже, остальные падают."
                ),
                format!(
                    "Several mods appear in the same stack ({mods}) and patch the same code. The \
                     one loaded last wins, the others fail."
                ),
            ),
            "mod_crash" => pick(
                format!(
                    "В стеке ошибки есть файлы мода {mods}, и игра упала внутри его кода. \
                     Исключение: {detail}."
                ),
                format!(
                    "The error stack contains files from {mods} and the game crashed inside that \
                     code. Exception: {detail}."
                ),
            ),
            "discord" => pick(
                format!(
                    "Мод Discord Rich Presence не смог запуститься: {detail}. Discord SDK \
                     вернул ошибку — обычно не хватает discord_game_sdk.dll, Discord не \
                     запущен или версия SDK не совпадает. После этого упал рантайм Unity."
                ),
                format!(
                    "The Discord Rich Presence mod could not start: {detail}. The Discord SDK \
                     returned an error — usually discord_game_sdk.dll is missing, Discord is not \
                     running or the SDK version does not match. The Unity runtime then crashed."
                ),
            ),
            "plugin_incompatible" => pick(
                format!(
                    "Сборка упала сразу после загрузки плагина «{detail}». Плагин собран под \
                     другую версию игры, Unity или BepInEx."
                ),
                format!(
                    "The build crashed right after loading the plugin “{detail}”. The plugin was \
                     built for another game, Unity or BepInEx version."
                ),
            ),
            "incompatible" => pick(
                format!(
                    "Плагин не смог загрузиться в эту версию игры: {detail}. Это несовместимость \
                     сборок, а не повреждённый файл."
                ),
                format!(
                    "A plugin failed to load into this game version: {detail}. This is a build \
                     incompatibility, not a damaged file."
                ),
            ),
            "gpu" => pick(
                format!(
                    "Драйвер видеокарты или сама карта перестали отвечать: {detail}. Это не \
                     ошибка модов, если только мод не меняет шейдеры или рендер."
                ),
                format!(
                    "The GPU driver or the card stopped responding: {detail}. This is not a mod \
                     error unless a mod replaces shaders or rendering."
                ),
            ),
            "out_of_memory" => pick(
                format!(
                    "Игра не смогла выделить память и была закрыта системой: {detail}. Моды, \
                     текстуры высокого разрешения и большие сборки увеличивают расход."
                ),
                format!(
                    "The game could not allocate memory and was closed by the system: {detail}. \
                     Mods, high-resolution textures and large builds increase usage."
                ),
            ),
            "runtime" => pick(
                format!(
                    "Упал сам рантайм Unity/Mono, без явной ошибки в коде мода: {detail}. Часто \
                     это последствие несовместимого нативного плагина или повреждённых файлов."
                ),
                format!(
                    "The Unity/Mono runtime itself failed without a clear mod exception: {detail}. \
                     This is often an incompatible native plugin or damaged game files."
                ),
            ),
            "game_bug" => pick(
                format!(
                    "Исключение возникло в коде самой игры: {detail}. Моды могут лишь ускорить \
                     его, если меняют те же объекты сцены."
                ),
                format!(
                    "The exception came from the game's own code: {detail}. Mods can only trigger \
                     it earlier if they touch the same scene objects."
                ),
            ),
            "disk" => pick(
                format!(
                    "Система не смогла прочитать или записать файл: {detail}. Проверьте место на \
                     диске и права на папку сборки."
                ),
                format!(
                    "The system could not read or write a file: {detail}. Check free disk space \
                     and permissions on the build folder."
                ),
            ),
            "missing_file" => pick(
                format!(
                    "Игра или плагин не нашли {detail}. Файл либо не установлен, либо удалён \
                     антивирусом."
                ),
                format!(
                    "The game or a plugin could not find {detail}. It is either not installed or \
                     was removed by antivirus."
                ),
            ),
            "no_logs" => pick(
                "Логи этой сессии не найдены: игра не успела их записать или они лежат в \
                 неожиданном месте. Анализировать нечего.",
                "No logs for this session were found: the game did not write them or they are in \
                 an unexpected place. There is nothing to analyze.",
            )
            .to_string(),
            _ => pick(
                format!(
                    "В логах есть следы падения ({detail}), но ни одно правило не совпало \
                     уверенно. Стек ниже — единственная зацепка."
                ),
                format!(
                    "The logs show a crash ({detail}) but no rule matched confidently. The stack \
                     below is the only lead."
                ),
            ),
        }
    }

    pub fn advice(&self) -> Vec<String> {
        let mods = self.mods_text();
        let mut out = Vec::new();
        match self.kind.as_str() {
            "mod_incompatible" => {
                out.push(
                    pick(
                        "Обновите мод до версии для этой сборки игры.",
                        "Update the mod to a version for this game build.",
                    )
                    .to_string(),
                );
                out.push(
                    pick(
                        "Проверьте версию BepInEx: мод мог требовать другую.",
                        "Check the BepInEx version: the mod may require another one.",
                    )
                    .to_string(),
                );
                out.push(pick(
                    "Если обновления нет — выключите мод переключателем и проверьте игру без него.",
                    "If there is no update, turn the mod off with the switch and test without it.",
                )
                .to_string());
            }
            "mod_missing_file" | "missing_file" => {
                out.push(
                    pick(
                        "Установите зависимость, которая содержит этот файл.",
                        "Install the dependency that provides this file.",
                    )
                    .to_string(),
                );
                out.push(pick(
                    "Проверьте карантин антивируса: DLL могла быть удалена после установки.",
                    "Check the antivirus quarantine: the DLL may have been removed after install.",
                )
                .to_string());
            }
            "mod_conflict" => {
                out.push(
                    pick(
                        "Оставьте включённым только один из конфликтующих модов.",
                        "Keep only one of the conflicting mods enabled.",
                    )
                    .to_string(),
                );
                out.push(
                    pick(
                        "Если нужны оба, проверьте, нет ли у них общей зависимости или патча.",
                        "If you need both, check whether they share a dependency or patch.",
                    )
                    .to_string(),
                );
            }
            "mod_crash" => {
                out.push(
                    pick(
                        "Выключите этот мод и запустите игру снова.",
                        "Turn this mod off and launch the game again.",
                    )
                    .to_string(),
                );
                out.push(pick(
                    "Если без мода игра работает — дело в нём; сообщите автору мода стек из отчёта.",
                    "If the game works without the mod, the mod is at fault; send its author the stack.",
                )
                .to_string());
            }
            "discord" => {
                out.push(
                    pick(
                        "Запустите Discord до игры и дождитесь входа в аккаунт.",
                        "Start Discord before the game and wait until you are signed in.",
                    )
                    .to_string(),
                );
                out.push(
                    pick(
                        "Переустановите мод Discord Rich Presence и проверьте, что discord_game_sdk.dll лежит в папке плагинов.",
                        "Reinstall the Discord Rich Presence mod and check that discord_game_sdk.dll is in the plugins folder.",
                    )
                    .to_string(),
                );
                out.push(
                    pick(
                        "Если интеграция не нужна — выключите мод, игра запустится без неё.",
                        "If you do not need the integration, turn the mod off and the game will start without it.",
                    )
                    .to_string(),
                );
            }
            "plugin_incompatible" | "incompatible" => {
                out.push(
                    pick(
                        "Обновите плагин до версии для этой сборки игры.",
                        "Update the plugin to a version for this game build.",
                    )
                    .to_string(),
                );
                out.push(
                    pick(
                        "Проверьте версии Unity и BepInEx: плагин должен быть собран под них.",
                        "Check the Unity and BepInEx versions: the plugin must be built for them.",
                    )
                    .to_string(),
                );
                out.push(
                    pick(
                        "Выключите плагин и запустите игру без него.",
                        "Turn the plugin off and launch the game without it.",
                    )
                    .to_string(),
                );
            }
            "gpu" => {
                out.push(
                    pick(
                        "Обновите драйвер видеокарты (чистая установка).",
                        "Update the GPU driver (clean install).",
                    )
                    .to_string(),
                );
                out.push(
                    pick(
                        "Отключите оверлеи: Discord, GeForce Experience, MSI Afterburner.",
                        "Disable overlays: Discord, GeForce Experience, MSI Afterburner.",
                    )
                    .to_string(),
                );
                out.push(
                    pick(
                        "Уберите разгон и проверьте температуру; попробуйте другую версию DirectX.",
                        "Remove overclocking and check temperatures; try another DirectX version.",
                    )
                    .to_string(),
                );
            }
            "out_of_memory" => {
                out.push(
                    pick(
                        "Закройте браузер и другие тяжёлые программы перед запуском.",
                        "Close the browser and other heavy apps before launching.",
                    )
                    .to_string(),
                );
                out.push(
                    pick(
                        "Увеличьте файл подкачки Windows.",
                        "Increase the Windows page file.",
                    )
                    .to_string(),
                );
                out.push(
                    pick(
                        "Выключите моды с текстурами высокого разрешения.",
                        "Turn off mods with high-resolution textures.",
                    )
                    .to_string(),
                );
            }
            "runtime" => {
                out.push(
                    pick(
                        "Проверьте целостность файлов сборки или создайте её заново.",
                        "Verify the build files or create the build again.",
                    )
                    .to_string(),
                );
                out.push(
                    pick(
                        "Выключайте моды по одному, начиная с установленных недавно.",
                        "Turn mods off one by one, starting with recently installed ones.",
                    )
                    .to_string(),
                );
                out.push(
                    pick(
                        "Переустановите BepInEx, если сборка его использует.",
                        "Reinstall BepInEx if the build uses it.",
                    )
                    .to_string(),
                );
            }
            "game_bug" => {
                out.push(
                    pick(
                        "Запустите игру без модов: если краш остался, это ошибка самой сборки.",
                        "Launch without mods: if it still crashes, the build itself is at fault.",
                    )
                    .to_string(),
                );
                out.push(
                    pick(
                        "Попробуйте другую версию игры.",
                        "Try another game version.",
                    )
                    .to_string(),
                );
            }
            "disk" => {
                out.push(
                    pick(
                        "Освободите место на диске и проверьте права на папку сборки.",
                        "Free disk space and check permissions on the build folder.",
                    )
                    .to_string(),
                );
                out.push(
                    pick(
                        "Исключите папку лаунчера из антивируса.",
                        "Exclude the launcher folder from antivirus.",
                    )
                    .to_string(),
                );
            }
            "no_logs" => {
                out.push(
                    pick(
                        "Запустите игру ещё раз и дайте ей хотя бы несколько секунд до падения.",
                        "Launch the game again and let it run for a few seconds before it crashes.",
                    )
                    .to_string(),
                );
            }
            _ => {
                out.push(
                    pick(
                        "Выключите моды, установленные перед этим запуском.",
                        "Turn off the mods installed before this launch.",
                    )
                    .to_string(),
                );
                out.push(
                    pick(
                        "Приложите этот отчёт, если будете спрашивать помощь.",
                        "Attach this report if you ask for help.",
                    )
                    .to_string(),
                );
            }
        }
        if !mods.is_empty() && !self.kind.starts_with("mod_") {
            out.push(pick(
                format!("В стеке упоминаются моды: {mods}. Начните проверку с них."),
                format!("The stack mentions these mods: {mods}. Start checking them."),
            ));
        }
        if !self.recent.is_empty() {
            let names: Vec<&str> = self.recent.iter().map(|m| m.name.as_str()).collect();
            out.push(pick(
                format!(
                    "Незадолго до запуска устанавливались: {}. Это первые подозреваемые.",
                    names.join(", ")
                ),
                format!(
                    "Installed shortly before launch: {}. These are the first suspects.",
                    names.join(", ")
                ),
            ));
        }
        out
    }

    fn mods_text(&self) -> String {
        self.mods
            .iter()
            .map(|m| format!("«{m}»"))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

pub fn dir(root: &Path) -> PathBuf {
    root.join("crashes")
}

pub fn save(root: &Path, report: &CrashReport) -> Result<(), String> {
    let dir = dir(root);
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let text = serde_json::to_string_pretty(report).map_err(|e| e.to_string())?;
    crate::util::write_atomic(&dir.join(format!("{}.json", report.id)), text)
        .map_err(|e| format!("{}: {e}", dir.display()))?;
    prune(root, 50);
    Ok(())
}

pub fn load(root: &Path) -> Vec<CrashReport> {
    let Ok(entries) = fs::read_dir(dir(root)) else {
        return Vec::new();
    };
    let mut reports: Vec<CrashReport> = entries
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .filter_map(|entry| fs::read_to_string(entry.path()).ok())
        .filter_map(|text| serde_json::from_str(crate::util::strip_bom(&text)).ok())
        .collect();
    reports.sort_by(|a, b| b.at.cmp(&a.at));
    reports
}

fn prune(root: &Path, keep: usize) {
    let Ok(entries) = fs::read_dir(dir(root)) else {
        return;
    };
    let mut files: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
    if files.len() <= keep {
        return;
    }
    files.sort();
    for path in &files[..files.len() - keep] {
        let _ = fs::remove_file(path);
    }
}

/// Признаки краша, по которым стоит запускать разбор, без полного сканирования.
pub fn crash_signal(game_dir: &Path, started: i64) -> bool {
    for path in signal_files(game_dir) {
        if file_ts(&path) < started - 60 {
            continue;
        }
        let Some(text) = read_tail(&path, 512 * 1024) else {
            continue;
        };
        let low = text.to_lowercase();
        if [
            "crash!!!",
            "crash has been intercepted",
            "the game crashed",
            "fatal error",
            "has_native_crash\": true",
            "has_native_crash\":true",
        ]
        .iter()
        .any(|mark| low.contains(mark))
        {
            return true;
        }
    }
    false
}

/// Коды выхода Windows, которые означают аварийное завершение.
pub fn is_crash_code(code: i32) -> bool {
    let code = code as u32;
    // NTSTATUS: 0xC0000000..=0xFFFFFFFF. 0xC000013A — обычное закрытие консоли.
    code & 0xC000_0000 == 0xC000_0000 && code != 0xC000_013A
}

/// Любое завершение игры, кроме нормального. Unity при падении возвращает
/// собственные коды (например, 47818), а не NTSTATUS, поэтому одних кодов
/// Windows недостаточно. 0xC000013A — обычное закрытие консоли.
pub fn is_abnormal_code(code: i32) -> bool {
    code as u32 != 0 && code as u32 != 0xC000_013A
}

/// Папка, где Unity хранит Player.log и отчёты CrashReporter.
pub fn logs_dir() -> PathBuf {
    if let Some(local) = dirs::data_local_dir() {
        if let Some(appdata) = local.parent() {
            let dir = appdata
                .join("LocalLow")
                .join("YandereDev")
                .join("YandereSimulator");
            if dir.exists() {
                return dir;
            }
        }
    }
    if let Some(tmp) = std::env::var_os("TEMP") {
        let dir = PathBuf::from(tmp)
            .join("YandereDev")
            .join("YandereSimulator")
            .join("Crashes");
        if dir.exists() {
            return dir;
        }
    }
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

fn signal_files(game_dir: &Path) -> Vec<PathBuf> {
    let mut files = vec![
        game_dir.join("BepInEx").join("LogOutput.log"),
        game_dir.join("Player.log"),
    ];
    if let Some(local) = dirs::data_local_dir() {
        if let Some(appdata) = local.parent() {
            let root = appdata
                .join("LocalLow")
                .join("YandereDev")
                .join("YandereSimulator");
            files.push(root.join("Player.log"));
            files.push(root.join("Player-prev.log"));
            // Unity хранит отчёт в Unity/<guid>/CrashReporter.
            if let Ok(entries) = fs::read_dir(root.join("Unity")) {
                for entry in entries.flatten() {
                    let dir = entry.path().join("CrashReporter");
                    files.extend(newest_json(&dir));
                }
            }
        }
    }
    files
}

fn newest_json(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<(i64, PathBuf)> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .map(|path| (file_ts(&path), path))
        .collect();
    out.sort_by_key(|(ts, _)| -ts);
    out.into_iter().take(1).map(|(_, path)| path).collect()
}

fn file_ts(path: &Path) -> i64 {
    fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Главная точка входа: разбирает логи и сохраняет отчёт.
pub fn analyze(
    root: &Path,
    game: &GameInstall,
    library: &Library,
    started: i64,
    exit_code: Option<i32>,
    duration_s: i64,
    progress: &Progress,
) -> Result<CrashReport, String> {
    let mut extra = Vec::new();
    if let Some(local) = dirs::data_local_dir() {
        if let Some(appdata) = local.parent() {
            let dir = appdata.join("LocalLow").join("YandereDev");
            if dir.exists() {
                extra.push(dir);
            }
        }
        let dir = local.join("Temp").join("YandereDev");
        if dir.exists() {
            extra.push(dir);
        }
    }
    if let Some(tmp) = std::env::var_os("TEMP") {
        let dir = PathBuf::from(tmp).join("YandereDev");
        if dir.exists() {
            extra.push(dir);
        }
    }
    analyze_in(
        root, game, library, &extra, started, exit_code, duration_s, progress,
    )
}

/// Та же логика, но с явным списком внешних папок — для тестов.
#[allow(clippy::too_many_arguments)]
pub fn analyze_in(
    root: &Path,
    game: &GameInstall,
    library: &Library,
    extra_roots: &[PathBuf],
    started: i64,
    exit_code: Option<i32>,
    duration_s: i64,
    progress: &Progress,
) -> Result<CrashReport, String> {
    progress.stage(pick("Поиск логов", "Searching logs"));
    let mut paths = Vec::new();
    let mut seen = HashSet::new();
    // Внешние папки маленькие и содержат самые важные логи, поэтому идём по
    // ним первыми: бюджет обхода не должен срезать их из-за большого билда.
    let mut budget = 120_000usize;
    for dir in extra_roots {
        collect(dir, started, 0, &mut paths, &mut seen, &mut budget);
    }
    collect(&game.path, started, 0, &mut paths, &mut seen, &mut budget);
    paths.sort_by_key(|path| rank(path));
    paths.truncate(MAX_FILES);

    let tokens = mod_tokens(library, &game.meta.id);
    let mut found = Found::default();
    progress.total(paths.len() as u64);

    for (index, path) in paths.iter().enumerate() {
        if progress.is_cancelled() {
            return Err(pick("Анализ отменён", "Analysis cancelled").to_string());
        }
        progress.files(index, paths.len());
        progress.detail(path.display().to_string());
        if let Some(text) = read_tail(path, TAIL_BYTES) {
            let name = path.display().to_string();
            scan(&mut found, &name, &text, &tokens);
        }
        progress.set_done(index as u64 + 1);
    }

    progress.stage(pick("Поиск причины", "Finding the cause"));
    let logs: Vec<String> = paths
        .iter()
        .map(|path| path.display().to_string())
        .take(30)
        .collect();
    let report = build(
        root,
        game,
        started,
        exit_code,
        duration_s,
        found,
        logs,
        paths.len(),
    );
    Ok(report)
}

fn collect(
    dir: &Path,
    since: i64,
    depth: usize,
    out: &mut Vec<PathBuf>,
    seen: &mut HashSet<String>,
    budget: &mut usize,
) {
    if depth > 9 || *budget == 0 || out.len() >= MAX_FILES {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if *budget == 0 {
            return;
        }
        *budget -= 1;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let lower = name.to_lowercase();
        if lower == ".yanlan" {
            continue;
        }
        if path.is_dir() {
            collect(&path, since, depth + 1, out, seen, budget);
            continue;
        }
        if !is_log_name(&lower, &path) || file_ts(&path) < since - 60 {
            continue;
        }
        let key = path.to_string_lossy().to_lowercase();
        if seen.insert(key) {
            out.push(path);
        }
    }
}

fn is_log_name(lower: &str, path: &Path) -> bool {
    let ext = path
        .extension()
        .map(|ext| ext.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if ext == "log" {
        return true;
    }
    if matches!(
        ext.as_str(),
        "dll"
            | "exe"
            | "dat"
            | "assets"
            | "bundle"
            | "ress"
            | "png"
            | "jpg"
            | "jpeg"
            | "webp"
            | "gif"
            | "mp4"
            | "zip"
            | "rar"
            | "7z"
            | "dmp"
    ) {
        return false;
    }
    if lower.contains("crash") || lower.contains("error") {
        return true;
    }
    lower.contains("log") && matches!(ext.as_str(), "txt" | "json" | "ini" | "out" | "")
}

fn rank(path: &Path) -> u8 {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if name == "player.log" {
        0
    } else if name == "player-prev.log" {
        1
    } else if name == "logoutput.log" {
        2
    } else if name.ends_with(".log") {
        3
    } else if name.contains("crash") || name.contains("error") {
        4
    } else {
        5
    }
}

fn read_tail(path: &Path, max: u64) -> Option<String> {
    let mut file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    if len == 0 {
        return None;
    }
    let from = len.saturating_sub(max);
    file.seek(SeekFrom::Start(from)).ok()?;
    let mut buffer = Vec::with_capacity((len - from) as usize);
    file.read_to_end(&mut buffer).ok()?;
    let text = String::from_utf8_lossy(&buffer).into_owned();
    // Начало может быть обрезано посреди строки — отбрасываем её.
    let text = if from > 0 {
        text.find('\n')
            .map(|pos| text[pos + 1..].to_string())
            .unwrap_or_default()
    } else {
        text
    };
    Some(crate::util::strip_bom(&text).to_string())
}

struct ModToken {
    token: String,
    name: String,
}

/// Имена DLL установленных модов. Именно они попадают в стек, поэтому по ним
/// можно назвать виновника, даже если в логе нет названия мода.
fn mod_tokens(library: &Library, game_id: &str) -> Vec<ModToken> {
    let mut tokens = Vec::new();
    let mut seen = HashSet::new();
    for item in library.mods.iter().filter(|item| item.game_id == game_id) {
        for file in &item.files {
            let Some(name) = file.path.rsplit('/').next() else {
                continue;
            };
            let lower = name.to_lowercase();
            let Some(stem) = lower.strip_suffix(".dll") else {
                continue;
            };
            if stem.len() < 3 || boring_dll(stem) || !seen.insert(stem.to_string()) {
                continue;
            }
            tokens.push(ModToken {
                token: stem.to_string(),
                name: item.name.clone(),
            });
        }
    }
    tokens.truncate(800);
    tokens
}

fn boring_dll(stem: &str) -> bool {
    [
        "unity",
        "system",
        "microsoft",
        "mono",
        "netstandard",
        "mscorlib",
        "0harmony",
        "harmony",
        "bepinex",
        "assembly-csharp",
        "il2cpp",
        "newtonsoft",
        "jetbrains",
    ]
    .iter()
    .any(|prefix| stem.starts_with(prefix))
}

#[derive(Default)]
struct Found {
    crash_marker: bool,
    native_flag: bool,
    managed_flag: bool,
    oom: i32,
    gpu: i32,
    runtime: i32,
    incompat: i32,
    missing_dll: i32,
    conflict: i32,
    disk: i32,
    game_bug: i32,
    generic: i32,
    mod_crash: i32,
    discord: i32,
    plugin: i32,
    plugin_name: String,
    plugin_load: Option<(usize, String)>,
    plugin_hit: bool,
    detail: String,
    mod_hits: HashMap<String, i32>,
    evidence: Vec<Evidence>,
}

impl Found {
    fn note(&mut self, file: &str, line: usize, text: &str) {
        if self.evidence.len() >= MAX_EVIDENCE {
            return;
        }
        let clean = short(text.trim(), MAX_LINE);
        if self
            .evidence
            .iter()
            .any(|item| item.file == file && item.line == line)
        {
            return;
        }
        self.evidence.push(Evidence {
            file: file.to_string(),
            line,
            text: clean,
        });
    }

    fn detail(&mut self, value: impl Into<String>) {
        if self.detail.is_empty() {
            self.detail = short(&value.into(), 120);
        }
    }
}

fn scan(found: &mut Found, file: &str, text: &str, tokens: &[ModToken]) {
    let lines: Vec<&str> = text.lines().collect();
    for (index, line) in lines.iter().enumerate() {
        let low = line.trim().to_lowercase();
        if low.is_empty() {
            continue;
        }
        let mut site = false;

        // Список плагинов BepInEx: падение сразу после строки «Loading […]»
        // указывает на конкретный плагин.
        if low.contains("bepinex]") && low.contains("loading [") {
            if let Some(start) = line.rfind('[') {
                if let Some(end) = line[start + 1..].find(']') {
                    let name = line[start + 1..start + 1 + end].trim();
                    found.plugin_load = Some((index, short(name, 80)));
                }
            }
        }

        if marker(&low) {
            found.crash_marker = true;
            site = true;
            found.note(file, index + 1, line);
        }
        if low.contains("\"has_native_crash\"") && low.contains("true") {
            found.native_flag = true;
        }
        if low.contains("\"has_managed_exception\"") && low.contains("true") {
            found.managed_flag = true;
        }

        if let Some(exception) = exception_name(&low) {
            site = true;
            found.note(file, index + 1, line);
            found.detail(exception);
            match exception {
                "OutOfMemoryException" => found.oom += 55,
                "MissingMethodException"
                | "MissingFieldException"
                | "TypeLoadException"
                | "TypeInitializationException"
                | "BadImageFormatException"
                | "EntryPointNotFoundException" => found.incompat += 42,
                "DllNotFoundException" | "FileNotFoundException" => found.missing_dll += 40,
                "StackOverflowException" => found.runtime += 55,
                "ExecutionEngineException" => found.runtime += 42,
                "NullReferenceException" => found.game_bug += 30,
                "IndexOutOfRangeException" | "InvalidOperationException" | "ArgumentException" => {
                    found.generic += 15
                }
                _ => found.generic += 12,
            }
        }

        if let Some(dll) = missing_dll_name(&low) {
            site = true;
            // Имя отсутствующей DLL важнее класса исключения: именно его
            // называют в причине и по нему ищут зависимость.
            found.detail = short(&dll, 120);
            found.note(file, index + 1, line);
        }

        // Discord SDK: ResultException приходит из Discord.Discord..ctor, а
        // DllNotFoundException — когда рядом нет discord_game_sdk.
        if low.contains("resultexception") {
            found.discord += 45;
            found.detail(short(line.trim(), 120));
            site = true;
            found.note(file, index + 1, line);
        }
        if low.contains("discordrpc") || low.contains("discord.discord..ctor") {
            found.discord += 30;
            site = true;
            found.note(file, index + 1, line);
        }
        if low.contains("dllnotfoundexception") && low.contains("discord") {
            found.discord += 45;
            site = true;
            found.note(file, index + 1, line);
        }

        if low.contains("exception_access_violation") {
            site = true;
            found.note(file, index + 1, line);
        }
        if low.contains("exception_stack_overflow") {
            found.runtime += 55;
            found.detail("EXCEPTION_STACK_OVERFLOW");
            site = true;
            found.note(file, index + 1, line);
        }
        if low.contains("exception_illegal_instruction")
            || low.contains("exception_invalid_handle")
            || low.contains("exception_in_page_error")
        {
            found.runtime += 25;
            site = true;
            found.note(file, index + 1, line);
        }
        if device_lost(&low) {
            found.gpu += 55;
            found.detail("DXGI_ERROR_DEVICE_REMOVED");
            site = true;
            found.note(file, index + 1, line);
        }
        if gpu_module(&low) && (low.contains("access violation") || low.contains("caused a crash"))
        {
            found.gpu += 50;
            found.detail(short(line.trim(), 120));
            site = true;
            found.note(file, index + 1, line);
        }
        if low.contains("unityplayer.dll caused an access violation") {
            found.runtime += 35;
            found.detail("UnityPlayer.dll access violation");
            site = true;
            found.note(file, index + 1, line);
        }
        if low.contains("failed to load plugin")
            || low.contains("could not load plugin")
            || low.contains("error loading plugin")
            || (low.contains("failed to load") && low.contains("assembly"))
        {
            found.incompat += 45;
            site = true;
            found.note(file, index + 1, line);
        }
        if low.contains("harmonyexception")
            || low.contains("failed to patch")
            || low.contains("conflict")
        {
            found.conflict += 30;
            site = true;
            found.note(file, index + 1, line);
        }
        if low.contains("access is denied")
            || low.contains("not enough space")
            || low.contains("os error 112")
            || low.contains("disk is full")
        {
            found.disk += 45;
            found.detail(short(line.trim(), 120));
            site = true;
            found.note(file, index + 1, line);
        }

        if !site {
            continue;
        }
        // Причину падения ищем в стеке под строкой: там имена DLL модов.
        let end = (index + 60).min(lines.len());
        let window = lines[index..end].join("\n").to_lowercase();
        let mut hits: HashMap<String, i32> = HashMap::new();
        for token in tokens {
            if window.contains(&token.token) {
                *hits.entry(token.name.clone()).or_insert(0) += 1;
            }
        }
        if !hits.is_empty() {
            found.mod_crash += 25 + 5 * hits.len().min(3) as i32;
            if hits.len() > 1 {
                found.conflict += 20;
            }
            for (name, count) in hits {
                *found.mod_hits.entry(name).or_insert(0) += count;
            }
            found.note(file, index + 1, line);
        }
        // Краш в первые строки после загрузки плагина BepInEx.
        if let Some((at, name)) = found.plugin_load.clone() {
            if index > at && index - at <= 15 && !found.plugin_hit {
                found.plugin_hit = true;
                found.plugin += 55;
                found.plugin_name = name;
                found.note(file, index + 1, line);
            }
        }
        if low.contains("exception_access_violation")
            && (window.contains("d3d11.dll")
                || window.contains("d3d12.dll")
                || window.contains("dxgi.dll")
                || window.contains("nvoglv64")
                || window.contains("atio6axx")
                || window.contains("igdgmm")
                || window.contains("vulkan-1.dll"))
        {
            found.gpu += 45;
            found.detail("EXCEPTION_ACCESS_VIOLATION");
        }
    }
}

fn build(
    root: &Path,
    game: &GameInstall,
    started: i64,
    exit_code: Option<i32>,
    duration_s: i64,
    mut found: Found,
    logs: Vec<String>,
    scanned: usize,
) -> CrashReport {
    let at = now_ts();
    let mut mods: Vec<(String, i32)> = found.mod_hits.drain().collect();
    mods.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let mods: Vec<String> = mods.into_iter().map(|(name, _)| name).take(3).collect();

    for score in [
        &mut found.oom,
        &mut found.gpu,
        &mut found.runtime,
        &mut found.incompat,
        &mut found.missing_dll,
        &mut found.conflict,
        &mut found.disk,
        &mut found.game_bug,
        &mut found.generic,
        &mut found.mod_crash,
        &mut found.discord,
        &mut found.plugin,
    ] {
        *score = (*score).min(100);
    }

    let kind = if scanned == 0 {
        "no_logs"
    } else if found.disk >= 45 {
        "disk"
    } else if found.oom >= 50 {
        "out_of_memory"
    } else if !mods.is_empty() && found.incompat >= 40 {
        "mod_incompatible"
    } else if !mods.is_empty() && found.missing_dll >= 35 {
        "mod_missing_file"
    } else if !mods.is_empty() && found.conflict >= 30 {
        "mod_conflict"
    } else if found.discord >= 50 {
        "discord"
    } else if found.missing_dll >= 35 {
        // Отсутствующая DLL — корень падения, даже если в нативном стеке
        // рядом оказались d3d11/dxgi: они там просто загружены.
        "missing_file"
    } else if found.incompat >= 40 {
        "incompatible"
    } else if found.gpu >= 45 {
        "gpu"
    } else if found.plugin >= 50 {
        "plugin_incompatible"
    } else if !mods.is_empty() && found.mod_crash >= 20 {
        "mod_crash"
    } else if found.runtime >= 30 {
        "runtime"
    } else if found.game_bug >= 25 {
        "game_bug"
    } else {
        "unknown"
    };

    if kind == "plugin_incompatible" && !found.plugin_name.is_empty() {
        found.detail = found.plugin_name.clone();
    }

    if found.detail.is_empty() {
        if let Some(code) = exit_code.filter(|code| is_crash_code(*code)) {
            found.detail = format!("0x{:08X}", code as u32);
        }
    }

    let mut confidence: i32 = match kind {
        "mod_incompatible" => 88,
        "mod_crash" => 82,
        "mod_missing_file" => 78,
        "mod_conflict" => 74,
        "disk" => 80,
        "out_of_memory" => 76,
        "gpu" => 72,
        "discord" => 74,
        "plugin_incompatible" => 64,
        "incompatible" => 62,
        "missing_file" => 66,
        "runtime" => 58,
        "game_bug" => 55,
        "no_logs" => 12,
        _ => 22,
    };
    if kind.starts_with("mod_") {
        confidence += if mods.is_empty() { -20 } else { 4 };
    } else if !mods.is_empty() {
        confidence += 3;
    }
    if kind == "unknown" && found.crash_marker {
        confidence += 8;
    }
    if found.native_flag {
        confidence += 4;
    }
    if found.managed_flag {
        confidence += 3;
    }
    if found.evidence.len() >= 3 {
        confidence += 3;
    }
    if scanned == 0 {
        confidence = confidence.min(15);
    }

    let recent = recent_mods(root, game, started);
    if kind.starts_with("mod_") && recent.iter().any(|item| mods.contains(&item.name)) {
        confidence += 4;
    }

    // Если две версии причины почти равны, честнее снизить уверенность.
    let mut scores = [
        found.oom,
        found.gpu,
        found.runtime,
        found.incompat,
        found.missing_dll,
        found.conflict,
        found.game_bug,
        found.discord,
        found.plugin,
    ];
    scores.sort_unstable();
    let second = scores[scores.len() - 2];
    if second > 0 && second * 5 >= scores[scores.len() - 1] * 4 {
        confidence -= 8;
    }

    CrashReport {
        id: format!("{at}-{}", sanitize_name(&game.meta.id)),
        at,
        started,
        duration_s,
        game_id: game.meta.id.clone(),
        game_name: game.display_name(),
        exit_code,
        confidence: confidence.clamp(5, 96) as u8,
        kind: kind.to_string(),
        detail: found.detail,
        mods,
        recent,
        evidence: found.evidence,
        scanned,
        logs,
    }
}

fn recent_mods(root: &Path, game: &GameInstall, started: i64) -> Vec<RecentMod> {
    let entries = journal::read(root);
    let mut out: Vec<RecentMod> = Vec::new();
    for item in Library::load(root)
        .mods
        .into_iter()
        .filter(|item| item.game_id == game.meta.id && item.enabled && item.installed_at > 0)
    {
        if item.installed_at > started + 5 || started - item.installed_at > RECENT_SECS {
            continue;
        }
        let ms = entries
            .iter()
            .rev()
            .find(|entry| {
                entry.event == "download"
                    && entry.name == item.name
                    && (entry.t - item.installed_at).abs() <= 600
            })
            .map(|entry| entry.ms)
            .unwrap_or(0);
        out.push(RecentMod {
            name: item.name,
            at: item.installed_at,
            download_ms: ms,
        });
    }
    out.sort_by_key(|item| -item.at);
    out.truncate(5);
    out
}

fn marker(low: &str) -> bool {
    low.contains("crash!!!")
        || low.contains("crash has been intercepted")
        || low.contains("the game crashed")
        || low.contains("fatal error")
        || low.contains("has_native_crash\": true")
        || low.contains("has_native_crash\":true")
}

fn exception_name(low: &str) -> Option<&'static str> {
    for name in [
        "OutOfMemoryException",
        "MissingMethodException",
        "MissingFieldException",
        "TypeLoadException",
        "TypeInitializationException",
        "BadImageFormatException",
        "EntryPointNotFoundException",
        "DllNotFoundException",
        "FileNotFoundException",
        "StackOverflowException",
        "ExecutionEngineException",
        "NullReferenceException",
        "IndexOutOfRangeException",
        "InvalidOperationException",
        "ArgumentException",
    ] {
        if low.contains(&name.to_lowercase()) {
            return Some(name);
        }
    }
    if low.contains("unhandled exception") {
        return Some("Unhandled exception");
    }
    None
}

fn missing_dll_name(low: &str) -> Option<String> {
    for marker in [
        "dllnotfoundexception:",
        "filenotfoundexception:",
        "could not load file or assembly '",
        "unable to load dll '",
    ] {
        let Some(pos) = low.find(marker) else {
            continue;
        };
        let rest = &low[pos + marker.len()..];
        let end = [
            rest.find(|c: char| c == '\'' || c == '\n' || c == ','),
            rest.find(" assembly"),
        ]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(rest.len());
        let value = short(rest[..end].trim(), 80);
        if !value.is_empty() {
            return Some(value);
        }
    }
    None
}

fn device_lost(low: &str) -> bool {
    low.contains("dxgi_error_device_removed")
        || low.contains("dxgi_error_device_hung")
        || low.contains("device removed")
        || low.contains("device lost")
}

fn gpu_module(low: &str) -> bool {
    low.contains("d3d11.dll")
        || low.contains("d3d12.dll")
        || low.contains("dxgi.dll")
        || low.contains("nvoglv64")
        || low.contains("nvwgf2umx")
        || low.contains("atio6axx")
        || low.contains("atidxx64")
        || low.contains("igdgmm64")
        || low.contains("vulkan-1.dll")
}

fn short(text: &str, max: usize) -> String {
    let mut out: String = text.chars().take(max).collect();
    if text.chars().count() > max {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::{GameInstall, GameMeta, InstalledMod, Library, ModFile};

    fn game(root: &Path) -> GameInstall {
        fs::create_dir_all(root.join("game")).unwrap();
        GameInstall {
            meta: GameMeta {
                id: "test".to_string(),
                label: "Test".to_string(),
                ..Default::default()
            },
            path: root.join("game"),
            size: 0,
            exe: None,
            mods: 0,
            unity: String::new(),
        }
    }

    fn library_with_mod() -> Library {
        let mut library = Library::default();
        library.mods.push(InstalledMod {
            key: "some-1".to_string(),
            mod_id: "some".to_string(),
            name: "Some Mod".to_string(),
            category: "mod".to_string(),
            author: String::new(),
            game_id: "test".to_string(),
            installed_at: now_ts() - 120,
            files: vec![ModFile {
                path: "BepInEx/plugins/SomeMod.dll".to_string(),
                replaced: false,
            }],
            cfg: String::new(),
            version: String::new(),
            bundled: false,
            enabled: true,
            pending: None,
        });
        library
    }

    fn analyze_text(root: &Path, text: &str) -> CrashReport {
        let game = game(root);
        let started = now_ts() - 60;
        fs::write(game.path.join("Player.log"), text).unwrap();
        analyze_in(
            root,
            &game,
            &library_with_mod(),
            &[],
            started,
            Some(-1073741819),
            60,
            &Progress::new(),
        )
        .unwrap()
    }

    #[test]
    fn names_the_mod_from_the_stack() {
        let root = std::env::temp_dir().join(format!("yanlan-diag-{}", now_ts()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let report = analyze_text(
            &root,
            "MissingMethodException: Method not found\n  at SomeMod.Plugin.Awake ()\n  at (wrapper)\n",
        );
        assert_eq!(report.kind, "mod_incompatible");
        assert_eq!(report.mods, vec!["Some Mod".to_string()]);
        assert!(report.confidence >= 80);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn recognises_gpu_failure() {
        let root = std::env::temp_dir().join(format!("yanlan-diag-gpu-{}", now_ts()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let report = analyze_text(
            &root,
            "Crash!!!\nd3d11.dll caused an Access Violation (0xc0000005) in module d3d11.dll\n",
        );
        assert_eq!(report.kind, "gpu");
        assert!(report.confidence >= 70);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn recognises_out_of_memory() {
        let root = std::env::temp_dir().join(format!("yanlan-diag-oom-{}", now_ts()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let report = analyze_text(&root, "OutOfMemoryException: Out of memory\n");
        assert_eq!(report.kind, "out_of_memory");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_logs_are_reported_honestly() {
        let root = std::env::temp_dir().join(format!("yanlan-diag-empty-{}", now_ts()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let game = game(&root);
        let report = analyze_in(
            &root,
            &game,
            &Library::default(),
            &[],
            now_ts() - 10,
            Some(-1073741819),
            0,
            &Progress::new(),
        )
        .unwrap();
        assert_eq!(report.kind, "no_logs");
        assert!(report.confidence <= 15);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn ignores_files_without_log_names() {
        assert!(is_log_name("player.log", Path::new("Player.log")));
        assert!(is_log_name("logoutput.log", Path::new("LogOutput.log")));
        assert!(is_log_name(
            "insights-crash-report.json",
            Path::new("insights-crash-report.json")
        ));
        assert!(!is_log_name("plugin.dll", Path::new("plugin.dll")));
        assert!(!is_log_name("logo.webp", Path::new("logo.webp")));
    }

    #[test]
    fn missing_dll_beats_gpu_modules_in_the_stack() {
        let root = std::env::temp_dir().join(format!("yanlan-diag-dll-{}", now_ts()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        // Реальный Player.log: отсутствует discord_game_sdk, после чего
        // рантайм падает, а в нативном стеке рядом лежат модули DirectX.
        let report = analyze_text(
            &root,
            "DllNotFoundException: discord_game_sdk\n\
             ExecutionEngineException: String conversion error\n\
             Crash!!!\n\
             EXCEPTION_ACCESS_VIOLATION (0xc0000005)\n\
             Loaded modules: d3d11.dll dxgi.dll discord_game_sdk.dll\n\
             A crash has been intercepted\n",
        );
        assert_eq!(report.kind, "missing_file");
        assert_eq!(report.detail, "discord_game_sdk");
        assert!(!report.evidence.is_empty());
        let _ = fs::remove_dir_all(&root);
    }
    #[test]
    fn recognises_discord_failure() {
        let root = std::env::temp_dir().join(format!("yanlan-diag-discord-{}", now_ts()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let report = analyze_text(
            &root,
            "ResultException: InternalError\n  at Discord.Discord..ctor ()\n  at DiscordRPC.Start ()\n\
             Crash!!!\n",
        );
        assert_eq!(report.kind, "discord");
        assert!(report.detail.contains("ResultException"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn blames_the_plugin_loaded_right_before_the_crash() {
        let root = std::env::temp_dir().join(format!("yanlan-diag-plugin-{}", now_ts()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let report = analyze_text(
            &root,
            "[Info   :   BepInEx] Loading [Configuration Manager 19.1]\n\
             Couldn't extract exception string from exception of type ExecutionEngineException\n\
             Crash!!!\n",
        );
        assert_eq!(report.kind, "plugin_incompatible");
        assert_eq!(report.detail, "Configuration Manager 19.1");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn does_not_blame_a_plugin_for_a_later_crash() {
        let root = std::env::temp_dir().join(format!("yanlan-diag-plugin-far-{}", now_ts()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let mut text = String::from("[Info   :   BepInEx] Loading [Configuration Manager 19.1]\n");
        for _ in 0..30 {
            text.push_str("[Info   :   BepInEx] ordinary log line\n");
        }
        text.push_str("ExecutionEngineException\nCrash!!!\n");
        let report = analyze_text(&root, &text);
        assert_eq!(report.kind, "runtime");
        let _ = fs::remove_dir_all(&root);
    }
    #[test]
    fn crash_codes_are_recognised() {
        assert!(is_crash_code(-1073741819));
        assert!(is_crash_code(-1073740791));
        assert!(!is_crash_code(0));
        assert!(!is_crash_code(1));
        assert!(!is_crash_code(-1073741510)); // 0xC000013A — обычное закрытие
    }

    #[test]
    fn unity_abnormal_exit_counts_as_crash() {
        assert!(is_abnormal_code(47818)); // 0xBACA — реальное падение Unity
        assert!(is_abnormal_code(1));
        assert!(is_abnormal_code(-1073741819));
        assert!(!is_abnormal_code(0));
        assert!(!is_abnormal_code(-1073741510)); // 0xC000013A — обычное закрытие
    }

    #[test]
    fn log_bom_is_not_shown_in_evidence() {
        let file = std::env::temp_dir().join(format!("yanlan-bom-{}.log", now_ts()));
        fs::write(&file, "\u{feff}NullReferenceException: boom\n").unwrap();
        assert_eq!(
            read_tail(&file, 1024).unwrap(),
            "NullReferenceException: boom\n"
        );
        let _ = fs::remove_file(&file);
    }
}
