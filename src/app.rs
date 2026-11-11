//! Состояние приложения и общий каркас интерфейса.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};

use egui::{RichText, Vec2};

use crate::builds::{self, Build};
use crate::config::{Config, ProxyKind};
use crate::i18n::{fill, pick};
use crate::jobs::{self, Ev};
use crate::library::{GameInstall, Library};
use crate::mods::{self, ModGroup, ModType, ModVariant};
use crate::progress::Progress;
use crate::runner;
use crate::theme::{self, Palette};
use crate::ui::widgets as w;
use crate::util::{self, human_size, human_speed, trim_to};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Builds,
    Catalog,
    Installed,
    Settings,
}

impl Tab {
    pub(crate) fn title(&self) -> &'static str {
        match self {
            Tab::Builds => pick("Сборки", "Builds"),
            Tab::Catalog => pick("Каталог", "Catalog"),
            Tab::Installed => pick("Установлено", "Installed"),
            Tab::Settings => pick("Настройки", "Settings"),
        }
    }

    fn subtitle(&self) -> &'static str {
        match self {
            Tab::Builds => pick(
                "Создавайте отдельные сборки — под моды, патчи и эксперименты",
                "Create separate builds — for mods, patches and experiments",
            ),
            Tab::Catalog => pick(
                "Моды, патчи и текстуры сообщества — с превью и совместимостью",
                "Community mods, patches and textures — with previews and compatibility",
            ),
            Tab::Installed => pick(
                "Моды во всех сборках — по секции на сборку",
                "Mods across all builds — one section per build",
            ),
            Tab::Settings => pick(
                "Папки, тема и обслуживание",
                "Folders, theme and maintenance",
            ),
        }
    }

    fn icon(&self) -> &'static str {
        match self {
            Tab::Builds => "▶",
            Tab::Catalog => "◆",
            Tab::Installed => "▣",
            Tab::Settings => "☰",
        }
    }
}

#[derive(Clone)]
pub enum ConfirmAction {
    DeleteGame(GameInstall),
    UninstallMod {
        key: String,
        name: String,
        game_id: String,
    },
}

/// Установка из каталога, ожидающая подтверждения: зависимости и конфликты.
#[derive(Clone)]
pub struct CatalogPending {
    pub variant: ModVariant,
    /// Зависимости, которые будут скачаны и установлены первыми.
    pub deps: Vec<ModVariant>,
    /// Зависимости, которых нет в каталоге или у них нет ссылки на архив.
    pub missing: Vec<String>,
    /// Уже установленные моды, чьи файлы пересекаются с `affected_files`.
    pub conflicts: Vec<String>,
}

pub struct Toast {
    pub text: String,
    pub ok: bool,
    pub at: Instant,
}

/// Готовая обложка каталога: кадры GIF (или один кадр) и их задержки.
#[derive(Clone)]
pub struct CoverAnim {
    pub frames: Vec<egui::TextureHandle>,
    pub delays: Vec<f32>,
    pub total: f32,
}

impl CoverAnim {
    /// Кадр на момент времени `time` и сколько секунд он ещё будет виден.
    pub fn frame(&self, time: f64) -> (usize, f32) {
        if self.frames.len() <= 1 || self.total <= 0.0 {
            return (0, 0.0);
        }
        let mut left = (time % self.total as f64) as f32;
        for (index, delay) in self.delays.iter().enumerate() {
            if left < *delay {
                return (index, *delay - left);
            }
            left -= *delay;
        }
        (self.frames.len() - 1, 0.0)
    }
}

pub struct App {
    pub cfg: Config,
    pub p: Palette,
    pub library: Library,
    pub games: Vec<GameInstall>,

    pub builds: Vec<Build>,
    pub builds_loaded: bool,
    pub builds_loading: bool,
    pub builds_error: Option<String>,
    pub builds_fetched_at: Option<i64>,

    pub catalog: Vec<ModGroup>,
    pub catalog_loaded: bool,
    pub catalog_loading: bool,
    pub catalog_error: Option<String>,
    pub catalog_fetched_at: Option<i64>,
    pub catalog_query: String,
    /// `None` — раздел «Все».
    pub catalog_section: Option<ModType>,
    pub catalog_sort: mods::CatalogSort,
    pub catalog_covers: HashMap<String, CoverAnim>,
    pub catalog_cover_pending: HashSet<String>,
    pub catalog_cover_failed: HashSet<String>,
    /// Открытая карточка (id мода) и выбранный в ней вариант.
    pub catalog_selected: Option<String>,
    pub catalog_variant: usize,
    pub catalog_shot: usize,
    /// Показывать в карточке мода `info.yaml` вместо описания.
    pub catalog_info: bool,
    pub catalog_pending: Option<CatalogPending>,
    /// Мод типа `Build`, для которого вводится название новой сборки.
    pub catalog_build: Option<ModVariant>,
    pub catalog_build_name: String,
    pub catalog_build_focus: bool,

    pub tab: Tab,
    pub selected_game: Option<String>,
    pub progress: Option<Progress>,
    /// Номер задачи, прогресс которой сейчас показывается в статус-баре.
    pub progress_id: Option<u64>,
    pub job_title: Option<String>,
    /// Задачи, ждущие свободного статус-бара.
    pub pending_jobs: Vec<(String, Progress)>,

    pub show_create: bool,
    pub new_name: String,
    pub new_version: Option<Build>,
    pub picker_query: String,
    /// Поставить курсор в поле названия при следующем кадре диалога.
    pub focus_name: bool,

    pub tx: Sender<Ev>,
    pub rx: Receiver<Ev>,
    pub ctx: egui::Context,

    pub log: Vec<String>,
    pub toasts: Vec<Toast>,

    pub confirm: Option<(String, ConfirmAction)>,
    /// Переименование сборки: (id сборки, новый текст в поле ввода).
    pub rename: Option<(String, String)>,
    /// Поставить курсор в поле переименования при следующем кадре.
    pub rename_focus: bool,
    /// Параметры запуска сборки: (id сборки, текст в поле ввода).
    pub launch_opts: Option<(String, String)>,
    pub launch_opts_focus: bool,
    pub speed: (Instant, u64, f64),
    pub root_input: String,
    /// Адрес прокси без схемы: «host:port» (логин и пароль — необязательно).
    pub proxy_input: String,
    /// Тип прокси, выбранный в настройках.
    pub proxy_kind: ProxyKind,
    /// Какая тема уже применена к системному заголовку окна.
    pub chrome_light: Option<bool>,
    /// Анимированный индикатор загрузки из `assets/loading.gif`.
    pub loading: Option<crate::loading::Anim>,
    /// Фильтр «Установлено» по id сборки; `None` — показать все.
    pub installed_filter: Option<String>,
    /// Ролик, который открыт во встроенном плеере.
    pub player_id: Option<String>,
    pub player: Option<crate::player::Player>,
    /// Открытый редактор конфига установленного мода.
    pub mod_cfg: Option<crate::cfg_edit::Editor>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let cfg = Config::load();
        crate::i18n::set(cfg.lang);
        cfg.ensure_dirs();
        let p = theme::apply(&cc.egui_ctx, cfg.light_theme);
        let root_input = cfg.root_dir.display().to_string();
        let (proxy_kind, proxy_input) = ProxyKind::split(&cfg.proxy);
        crate::net::set_proxy(if cfg.proxy.trim().is_empty() {
            None
        } else {
            Some(cfg.proxy.clone())
        });
        let (tx, rx) = channel();

        let mut app = App {
            cfg,
            p,
            library: Library::default(),
            games: Vec::new(),
            builds: Vec::new(),
            builds_loaded: false,
            builds_loading: false,
            builds_error: None,
            builds_fetched_at: None,
            catalog: Vec::new(),
            catalog_loaded: false,
            catalog_loading: false,
            catalog_error: None,
            catalog_fetched_at: None,
            catalog_query: String::new(),
            catalog_section: None,
            catalog_sort: mods::CatalogSort::default(),
            catalog_covers: HashMap::new(),
            catalog_cover_pending: HashSet::new(),
            catalog_cover_failed: HashSet::new(),
            catalog_selected: None,
            catalog_variant: 0,
            catalog_shot: 0,
            catalog_info: false,
            catalog_pending: None,
            catalog_build: None,
            catalog_build_name: String::new(),
            catalog_build_focus: false,
            tab: Tab::Builds,
            selected_game: None,
            progress: None,
            progress_id: None,
            job_title: None,
            pending_jobs: Vec::new(),
            show_create: false,
            new_name: String::new(),
            new_version: None,
            picker_query: String::new(),
            focus_name: false,
            tx,
            rx,
            ctx: cc.egui_ctx.clone(),
            log: Vec::new(),
            toasts: Vec::new(),
            confirm: None,
            rename: None,
            rename_focus: false,
            launch_opts: None,
            launch_opts_focus: false,
            speed: (Instant::now(), 0, 0.0),
            root_input,
            proxy_input,
            proxy_kind,
            chrome_light: None,
            loading: crate::loading::Anim::new(&cc.egui_ctx),
            installed_filter: None,
            player_id: None,
            player: None,
            mod_cfg: None,
        };

        app.reload_library();

        if let Some(cache) = builds::load_cache(&app.cfg) {
            app.set_builds(cache.builds);
            app.builds_fetched_at = Some(cache.fetched_at);
            app.builds_loaded = true;
        }

        if let Some(cache) = mods::load_cache(&app.cfg) {
            app.catalog = cache.mods;
            app.catalog_fetched_at = Some(cache.fetched_at);
            app.catalog_loaded = true;
        }

        app.log(fill(
            &pick("Каталог лаунчера: {path}", "Launcher folder: {path}"),
            &[("path", &app.cfg.root_dir.display().to_string())],
        ));

        let stale_builds = app
            .builds_fetched_at
            .map(|ts| util::now_ts() - ts > 12 * 3600)
            .unwrap_or(true);
        if stale_builds && app.cfg.auto_refresh {
            app.refresh_builds();
        }

        let stale_catalog = app
            .catalog_fetched_at
            .map(|ts| util::now_ts() - ts > mods::CACHE_TTL)
            .unwrap_or(true);
        if stale_catalog && app.cfg.auto_refresh {
            app.refresh_catalog();
        }

        app
    }

    pub fn set_builds(&mut self, builds: Vec<Build>) {
        let mut builds = builds;
        // Версии с датой — сверху (новые первыми), спецсборки без даты — в конце.
        builds.sort_by(|a, b| match (&b.date, &a.date) {
            (Some(x), Some(y)) => x.cmp(y),
            (Some(_), None) => std::cmp::Ordering::Greater,
            (None, Some(_)) => std::cmp::Ordering::Less,
            (None, None) => a.label.cmp(&b.label),
        });
        self.builds = builds;
    }

    pub fn reload_library(&mut self) {
        self.library = Library::load(&self.cfg.root_dir);
        self.games = crate::library::scan_games(&self.cfg.games_dir(), &self.library.mods);
        if self.selected_game.is_none() {
            self.selected_game = self.games.first().map(|g| g.meta.id.clone());
        }
        if let Some(selected) = &self.selected_game {
            if !self.games.iter().any(|g| &g.meta.id == selected) {
                self.selected_game = self.games.first().map(|g| g.meta.id.clone());
            }
        }
    }

    pub fn current_game(&self) -> Option<&GameInstall> {
        let id = self.selected_game.as_ref()?;
        self.games.iter().find(|g| &g.meta.id == id)
    }

    pub fn toast(&mut self, text: impl Into<String>, ok: bool) {
        let text = text.into();
        if text.is_empty() {
            return;
        }
        self.toasts.push(Toast {
            text,
            ok,
            at: Instant::now(),
        });
        if self.toasts.len() > 5 {
            self.toasts.remove(0);
        }
    }

    pub fn log(&mut self, text: impl Into<String>) {
        let stamp = util::civil_from_ts(util::now_ts());
        let line = format!(
            "{:02}:{:02}:{:02}  {}",
            stamp.3,
            stamp.4,
            (util::now_ts() % 60),
            text.into()
        );
        self.log.push(line);
        if self.log.len() > 400 {
            self.log.remove(0);
        }
    }

    pub fn begin_job(&mut self, title: impl Into<String>, progress: Progress) {
        self.progress_id = Some(progress.id());
        self.job_title = Some(title.into());
        self.progress = Some(progress);
        self.speed = (Instant::now(), 0, 0.0);
    }

    /// Запускает задачу, которая может идти параллельно с текущей.
    ///
    /// Если статус-бар занят, задача ждёт в очереди и получает его, когда
    /// предыдущая закончится.
    pub fn start_parallel_job(&mut self, title: impl Into<String>, progress: Progress) {
        let title = title.into();
        if self.progress.is_none() {
            self.begin_job(title, progress);
        } else {
            self.log(title.clone());
            self.pending_jobs.push((title, progress));
        }
    }

    /// Показывает следующую незавершённую задачу из очереди.
    fn promote_job(&mut self) {
        while !self.pending_jobs.is_empty() {
            let (title, progress) = self.pending_jobs.remove(0);
            if progress.snapshot().finished {
                continue;
            }
            self.begin_job(title, progress);
            return;
        }
    }

    pub fn refresh_builds(&mut self) {
        if self.builds_loading {
            return;
        }
        self.builds_loading = true;
        self.builds_error = None;
        // Старый список убираем сразу: во время обновления показывается
        // индикатор загрузки, а не устаревшие версии.
        self.builds.clear();
        self.builds_loaded = false;
        let c = jobs::Ctx2 {
            ctx: self.ctx.clone(),
            tx: self.tx.clone(),
        };
        let _ = jobs::fetch_builds(c, self.cfg.clone());
    }

    /// Обновляет каталог модов YanMod в фоне.
    pub fn refresh_catalog(&mut self) {
        if self.catalog_loading {
            return;
        }
        self.catalog_loading = true;
        self.catalog_error = None;
        self.catalog.clear();
        self.catalog_loaded = false;
        self.catalog_selected = None;
        self.catalog_pending = None;
        self.catalog_build = None;
        let c = jobs::Ctx2 {
            ctx: self.ctx.clone(),
            tx: self.tx.clone(),
        };
        let _ = jobs::fetch_catalog(c, self.cfg.clone());
    }

    /// Открывает диалог создания сборки, подставляя последнюю версию.
    pub fn open_create(&mut self) {
        self.show_create = true;
        self.rename = None;
        self.rename_focus = false;
        self.new_name.clear();
        self.picker_query.clear();
        self.focus_name = true;
        self.new_version = Some(builds::latest());
        if !self.builds_loaded && !self.builds_loading {
            self.refresh_builds();
        }
    }

    pub fn name_taken(&self, name: &str) -> bool {
        let id = util::sanitize_dir_name(name).to_lowercase();
        self.games.iter().any(|g| g.meta.id.to_lowercase() == id)
    }

    pub fn create_build(&mut self) {
        if self.progress.is_some() {
            self.toast(
                pick(
                    "Дождитесь завершения текущей задачи",
                    "Wait for the current task to finish",
                ),
                false,
            );
            return;
        }
        let name = self.new_name.trim().to_string();
        if name.is_empty() {
            self.toast(pick("Укажите название сборки", "Enter a build name"), false);
            return;
        }
        if self.name_taken(&name) {
            self.toast(
                pick(
                    "Сборка с таким названием уже существует",
                    "A build with this name already exists",
                ),
                false,
            );
            return;
        }
        let Some(build) = self.new_version.clone() else {
            self.toast(pick("Выберите версию игры", "Choose a game version"), false);
            return;
        };
        let c = jobs::Ctx2 {
            ctx: self.ctx.clone(),
            tx: self.tx.clone(),
        };
        let progress = jobs::install_build(c, self.cfg.clone(), build.clone(), name.clone());
        self.begin_job(
            fill(
                &pick("Создание сборки «{name}»", "Creating build “{name}”"),
                &[("name", &name)],
            ),
            progress,
        );
        self.log(fill(
            &pick(
                "Создание сборки «{name}» из версии {version}",
                "Creating build “{name}” from version {version}",
            ),
            &[("name", &name), ("version", &build.label)],
        ));
        self.show_create = false;
        self.tab = Tab::Builds;
    }

    /// Меняет отображаемое название сборки, не трогая папку на диске.
    pub fn apply_rename(&mut self, id: &str, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            self.toast(
                pick("Название не может быть пустым", "The name cannot be empty"),
                false,
            );
            return;
        }
        let Some(game) = self.games.iter().find(|g| g.meta.id == id).cloned() else {
            self.toast(pick("Сборка не найдена", "Build not found"), false);
            return;
        };
        let mut meta = game.meta.clone();
        meta.label = name.to_string();
        if let Err(err) = crate::library::write_meta(&game.path, &meta) {
            self.toast(err, false);
            return;
        }
        self.reload_library();
        let message = fill(
            &pick(
                "Сборка переименована в «{name}»",
                "Build renamed to “{name}”",
            ),
            &[("name", name)],
        );
        self.log(message.clone());
        self.toast(message, true);
    }

    /// Сохраняет дополнительные параметры запуска сборки в `.yanlan.json`.
    pub fn apply_launch_args(&mut self, id: &str, raw: &str) {
        let Some(game) = self.games.iter().find(|g| g.meta.id == id).cloned() else {
            self.toast(pick("Сборка не найдена", "Build not found"), false);
            return;
        };
        let mut meta = game.meta.clone();
        meta.args = raw.trim().to_string();
        if let Err(err) = crate::library::write_meta(&game.path, &meta) {
            self.toast(err, false);
            return;
        }
        self.reload_library();
        let message = if meta.args.is_empty() {
            pick("Параметры запуска очищены", "Launch options cleared").to_string()
        } else {
            fill(
                &pick(
                    "Параметры запуска сохранены: {args}",
                    "Launch options saved: {args}",
                ),
                &[("args", &meta.args)],
            )
        };
        self.log(message.clone());
        self.toast(message, true);
    }

    /// Применяет прокси из полей настроек ко всем сетевым запросам.
    ///
    /// Адрес можно вводить без схемы: `127.0.0.1:8080`. Если схема указана,
    /// она важнее выбранного типа — так удобно вставлять готовую ссылку.
    pub fn apply_proxy(&mut self) {
        let text = self.proxy_input.trim().to_string();
        if text.is_empty() {
            if self.cfg.proxy.trim().is_empty() {
                self.toast(
                    pick(
                        "Сначала укажите адрес прокси",
                        "Enter the proxy address first",
                    ),
                    false,
                );
            } else {
                self.disable_proxy(true);
            }
            return;
        }
        let (parsed, address) = ProxyKind::split(&text);
        let kind = if text.contains("://") {
            parsed
        } else {
            self.proxy_kind
        };
        let url = format!("{}{}", kind.scheme(), address.trim());
        if let Err(err) = ureq::Proxy::new(&url) {
            let message = fill(
                &pick(
                    "Не удалось разобрать адрес прокси: {err}",
                    "Could not parse the proxy address: {err}",
                ),
                &[("err", &err.to_string())],
            );
            self.toast(message, false);
            return;
        }
        self.proxy_kind = kind;
        self.proxy_input = address.trim().to_string();
        crate::net::set_proxy(Some(url.clone()));
        self.cfg.proxy = url.clone();
        self.cfg.save();
        let message = fill(
            &pick("Прокси включён: {proxy}", "Proxy enabled: {proxy}"),
            &[("proxy", &url)],
        );
        self.log(message.clone());
        self.toast(message, true);
    }

    /// Отключает прокси; введённый адрес остаётся в поле для быстрого включения.
    pub fn disable_proxy(&mut self, notify: bool) {
        crate::net::set_proxy(None);
        self.cfg.proxy.clear();
        self.cfg.save();
        if notify {
            self.log(pick("Прокси отключён", "Proxy disabled"));
            self.toast(pick("Прокси отключён", "Proxy disabled"), true);
        }
    }

    fn pump(&mut self) {
        while let Ok(event) = self.rx.try_recv() {
            match event {
                Ev::Builds {
                    builds,
                    fetched_at,
                    error,
                } => {
                    self.builds_loading = false;
                    if !builds.is_empty() {
                        self.set_builds(builds);
                        self.builds_loaded = true;
                        self.builds_error = None;
                        self.builds_fetched_at = Some(fetched_at);
                    } else {
                        self.builds_error = error;
                    }
                }
                Ev::Catalog {
                    mods,
                    fetched_at,
                    error,
                } => {
                    self.catalog_loading = false;
                    if !mods.is_empty() {
                        self.catalog = mods;
                        self.catalog_loaded = true;
                        self.catalog_error = None;
                        self.catalog_fetched_at = Some(fetched_at);
                        // Обложки могли измениться — старые текстуры больше не нужны.
                        self.catalog_covers.clear();
                        self.catalog_cover_pending.clear();
                        self.catalog_cover_failed.clear();
                    } else {
                        self.catalog_error = error;
                    }
                }
                Ev::Cover { key, result } => {
                    self.catalog_cover_pending.remove(&key);
                    match result {
                        Ok(cover) => {
                            if cover.frames.is_empty() {
                                self.catalog_cover_failed.insert(key);
                                continue;
                            }
                            let mut frames = Vec::with_capacity(cover.frames.len());
                            let mut delays = Vec::with_capacity(cover.frames.len());
                            let total = cover.delays.iter().sum::<f32>();
                            for (index, image) in cover.frames.into_iter().enumerate() {
                                frames.push(self.ctx.load_texture(
                                    format!("catalog-cover-{key}-{index}"),
                                    image,
                                    egui::TextureOptions::LINEAR,
                                ));
                                delays.push(cover.delays.get(index).copied().unwrap_or(0.0));
                            }
                            self.catalog_covers.insert(
                                key,
                                CoverAnim {
                                    frames,
                                    delays,
                                    total,
                                },
                            );
                        }
                        Err(_) => {
                            // Битая или недоступная картинка: не пытаемся
                            // скачивать её каждый кадр.
                            self.catalog_cover_failed.insert(key);
                        }
                    }
                }
                Ev::Log(text) => {
                    self.log(text);
                }
                Ev::DataDone { ok, message } => {
                    if !ok {
                        self.toast(message, false);
                    }
                }
                Ev::JobDone { id, ok, message } => {
                    if !ok {
                        self.toast(message, false);
                    } else if !message.is_empty() {
                        self.toast(message, true);
                    }
                    // Фоновая задача не должна сбрасывать прогресс скачивания,
                    // которое продолжается параллельно.
                    if self.progress_id == Some(id) {
                        self.progress = None;
                        self.progress_id = None;
                        self.job_title = None;
                        self.promote_job();
                    }
                    self.reload_library();
                }
            }
        }
        if !self.toasts.is_empty() {
            self.toasts
                .retain(|t| t.at.elapsed() < Duration::from_secs(7));
        }
    }

    pub fn play_game(&mut self) {
        let Some(game) = self.current_game().cloned() else {
            self.toast(
                pick(
                    "Сначала создайте или выберите сборку",
                    "Create or select a build first",
                ),
                false,
            );
            return;
        };
        let Some(exe) = game.exe.clone() else {
            self.toast(
                pick(
                    "В папке сборки нет исполняемого файла",
                    "There is no executable file in the build folder",
                ),
                false,
            );
            return;
        };
        match runner::launch(&game.path, &exe, &crate::util::split_args(&game.meta.args)) {
            Ok(()) => self.log(fill(
                &pick("Запуск {name} ({path})", "Launching {name} ({path})"),
                &[
                    ("name", &game.display_name()),
                    ("path", &exe.display().to_string()),
                ],
            )),
            Err(err) => self.toast(err, false),
        }
    }

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        let p = self.p;
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("♥").size(24.0).color(p.accent));
            ui.vertical(|ui| {
                ui.label(RichText::new("YANLAN").size(21.0).strong().color(p.text));
                ui.label(RichText::new("Yandere Simulator").size(10.5).color(p.muted));
            });
        });
        ui.add_space(22.0);
        w::section_title(ui, pick("РАЗДЕЛЫ", "SECTIONS"), &p);
        ui.add_space(4.0);

        let tabs = [Tab::Builds, Tab::Catalog, Tab::Installed, Tab::Settings];
        for tab in tabs {
            if nav_item(ui, &p, self.tab == tab, tab.icon(), tab.title()) {
                self.tab = tab;
                if tab == Tab::Installed || tab == Tab::Builds {
                    self.reload_library();
                }
            }
            ui.add_space(2.0);
        }

        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            ui.label(
                RichText::new(fill(
                    &pick(
                        "v{version} · неофициальный лаунчер",
                        "v{version} · unofficial launcher",
                    ),
                    &[("version", env!("CARGO_PKG_VERSION"))],
                ))
                .size(10.5)
                .color(p.muted),
            );
            ui.add_space(6.0);
            if ui
                .add(
                    egui::Label::new(
                        RichText::new("archive.org · yanderesimulator.com")
                            .size(10.5)
                            .color(p.muted),
                    )
                    .sense(egui::Sense::hover()),
                )
                .hovered()
            {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Default);
            }
        });
    }

    fn topbar(&mut self, ui: &mut egui::Ui) {
        let p = self.p;
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.add_space(2.0);
                ui.label(
                    RichText::new(self.tab.title())
                        .size(25.0)
                        .strong()
                        .color(p.text),
                );
                ui.label(RichText::new(self.tab.subtitle()).size(12.0).color(p.muted));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Играть можно и во время скачивания другой сборки.
                let has_game = self
                    .current_game()
                    .map(|g| g.exe.is_some())
                    .unwrap_or(false);
                if w::primary(ui, pick("▶  Играть", "▶  Play"), &p, has_game).clicked() {
                    self.play_game();
                }
                ui.add_space(6.0);

                let current_label = self
                    .current_game()
                    .map(|g| g.display_name())
                    .unwrap_or_else(|| pick("Сборка не выбрана", "No build selected").to_string());
                let mut chosen: Option<String> = None;
                // Название сборки может быть длинным — обрезаем его в кнопке,
                // иначе комбобокс растягивается и наезжает на соседние элементы.
                let shown = trim_to(&current_label, 26);
                let combo = egui::ComboBox::from_id_salt("selected_game")
                    .width(196.0)
                    .selected_text(RichText::new(shown).size(13.0))
                    .show_ui(ui, |ui| {
                        if self.games.is_empty() {
                            ui.label(RichText::new(pick("Нет сборок", "No builds")).color(p.muted));
                        }
                        let games = self.games.clone();
                        for game in games {
                            let selected = self.selected_game.as_deref() == Some(&game.meta.id);
                            let name = trim_to(&game.display_name(), 44);
                            if ui
                                .selectable_label(
                                    selected,
                                    format!("{name} · {}", human_size(game.size)),
                                )
                                .on_hover_text(game.display_name())
                                .clicked()
                            {
                                chosen = Some(game.meta.id.clone());
                            }
                        }
                    });
                combo.response.on_hover_text(&current_label);
                if let Some(id) = chosen {
                    self.selected_game = Some(id);
                }
            });
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let p = self.p;
        if let Some(progress) = self.progress.clone() {
            let snap = progress.snapshot();
            if snap.done != self.speed.1 {
                let now = Instant::now();
                let dt = (now - self.speed.0).as_secs_f64();
                if dt > 0.15 {
                    let instant = (snap.done.saturating_sub(self.speed.1)) as f64 / dt;
                    self.speed.2 = if self.speed.2 <= 1.0 {
                        instant
                    } else {
                        self.speed.2 * 0.65 + instant * 0.35
                    };
                    self.speed = (now, snap.done, self.speed.2);
                }
            }

            let fraction = if snap.total > 0 {
                snap.done as f32 / snap.total as f32
            } else if snap.file_total > 0 {
                snap.file_index as f32 / snap.file_total as f32
            } else {
                0.0
            };

            let eta = if snap.total > snap.done && self.speed.2 > 1024.0 {
                util::human_eta((snap.total - snap.done) as f64 / self.speed.2)
            } else {
                String::new()
            };
            let mut right = String::new();
            if snap.total > 0 {
                right.push_str(&format!(
                    "{} / {}  ·  {:.0}%",
                    human_size(snap.done),
                    human_size(snap.total),
                    fraction * 100.0
                ));
            } else if snap.file_total > 0 {
                right.push_str(&fill(
                    &pick("{index}/{total} файлов", "{index}/{total} files"),
                    &[
                        ("index", &snap.file_index.to_string()),
                        ("total", &snap.file_total.to_string()),
                    ],
                ));
            }
            if !eta.is_empty() {
                right.push_str(&fill(
                    &pick("  ·  осталось {eta}", "  ·  {eta} left"),
                    &[("eta", &eta)],
                ));
            }
            if self.speed.2 > 1024.0 {
                right.push_str(&format!("  ·  {}", human_speed(self.speed.2)));
            }
            if snap.elapsed > 2.0 {
                right.push_str(&fill(
                    &pick("  ·  прошло {time}", "  ·  {time} elapsed"),
                    &[("time", &util::human_eta(snap.elapsed))],
                ));
            }

            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(if snap.stage.is_empty() {
                        pick("Подготовка", "Preparing")
                    } else {
                        &snap.stage
                    })
                    .size(13.5)
                    .strong()
                    .color(p.text),
                );
                if !self.pending_jobs.is_empty() {
                    ui.label(
                        RichText::new(fill(
                            &pick("+{n} в фоне", "+{n} in background"),
                            &[("n", &self.pending_jobs.len().to_string())],
                        ))
                        .size(11.0)
                        .color(p.muted),
                    );
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if w::soft(ui, pick("Отмена", "Cancel"), &p, !snap.cancelled).clicked() {
                        progress.cancel();
                        self.log(pick("Запрошена отмена", "Cancellation requested"));
                    }
                    if snap.error.is_some() {
                        ui.label(
                            RichText::new(pick("ошибка", "error"))
                                .color(p.red)
                                .size(12.0),
                        );
                    }
                    if !right.is_empty() {
                        ui.label(RichText::new(right).size(11.5).color(p.muted));
                    }
                });
            });
            // Длинный путь к файлу — отдельной строкой: иначе он залезает под
            // счётчики и время, которые выровнены по правому краю.
            if !snap.detail.is_empty() {
                ui.add_space(2.0);
                ui.add(
                    egui::Label::new(RichText::new(&snap.detail).size(11.0).color(p.muted))
                        .truncate(),
                );
            }
            ui.add_space(6.0);
            w::progress_bar(ui, fraction, &p, 7.0);
        } else {
            ui.horizontal(|ui| {
                let (icon, text, color) = if let Some(err) = &self.builds_error {
                    ("!", err.as_str(), p.red)
                } else if self.games.is_empty() {
                    (
                        "♥",
                        pick(
                            "Создайте первую сборку — выберите версию игры и задайте название.",
                            "Create your first build — pick a game version and give it a name.",
                        ),
                        p.muted,
                    )
                } else {
                    (
                        "♥",
                        pick(
                            "Сборки и моды готовы. Выберите активную сборку сверху и нажмите «Играть».",
                            "Builds and mods are ready. Select a build above and press “Play”.",
                        ),
                        p.muted,
                    )
                };
                ui.label(RichText::new(icon).size(13.0).color(color));
                ui.label(
                    RichText::new(text)
                        .size(11.5)
                        .color(if self.builds_error.is_some() {
                            p.red
                        } else {
                            p.muted
                        }),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.builds_loading {
                        if let Some(loading) = &self.loading {
                            loading.show(ui, 24.0);
                        }
                        ui.label(
                            RichText::new(pick("обновление каталога…", "updating catalog…"))
                                .size(11.0)
                                .color(p.muted),
                        );
                    } else if let Some(ts) = self.builds_fetched_at {
                        ui.label(
                            RichText::new(fill(
                                &pick("каталог обновлён {time}", "catalog updated {time}"),
                                &[("time", &util::format_ts(ts))],
                            ))
                            .size(10.5)
                            .color(p.muted),
                        );
                    }
                });
            });
        }
    }

    fn draw_toasts(&mut self, ctx: &egui::Context) {
        if self.toasts.is_empty() {
            return;
        }
        let p = self.p;
        // Рисуем уведомления прямо на слое, без `Area` и виджетов: в egui 0.29
        // даже `interactable(false)` создаёт невидимый hover-виджет, который
        // выигрывает hit-test и не даёт нажать кнопку под уведомлением.
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Tooltip,
            egui::Id::new("toasts_paint"),
        ));
        let screen = ctx.screen_rect();
        let font = egui::FontId::proportional(12.5);
        // Уведомления должны висеть над статус-баром: во время задачи он выше,
        // иначе тост налезает на полосу прогресса.
        let status = if self.progress.is_some() { 102.0 } else { 74.0 };
        let mut bottom = screen.bottom() - status - 12.0;
        for toast in self.toasts.iter().rev() {
            let color = if toast.ok { p.green } else { p.red };
            let text = util::trim_to(&toast.text, 160);
            let galley = painter.layout(text, font.clone(), p.text, 360.0);
            let size = egui::vec2(galley.size().x + 56.0, galley.size().y + 20.0);
            let rect = egui::Rect::from_min_size(
                egui::pos2(screen.right() - 20.0 - size.x, bottom - size.y),
                size,
            );
            painter.rect(
                rect,
                10.0,
                p.panel,
                egui::Stroke::new(1.0_f32, color.gamma_multiply(0.6)),
            );
            painter.text(
                egui::pos2(rect.left() + 14.0, rect.center().y),
                egui::Align2::LEFT_CENTER,
                if toast.ok { "✓" } else { "!" },
                egui::FontId::proportional(13.0),
                color,
            );
            painter.galley(
                egui::pos2(rect.left() + 34.0, rect.top() + 10.0),
                galley,
                p.text,
            );
            bottom = rect.top() - 6.0;
            if bottom < 60.0 {
                break;
            }
        }
        ctx.request_repaint_after(Duration::from_millis(250));
    }

    fn draw_confirm(&mut self, ctx: &egui::Context) {
        let Some((message, action)) = self.confirm.clone() else {
            return;
        };
        let p = self.p;
        let mut confirmed = false;
        let mut cancel = false;

        let frame = egui::Frame::none()
            .fill(p.panel)
            .stroke(egui::Stroke::new(1.0_f32, p.border))
            .rounding(16.0)
            .inner_margin(egui::Margin::same(22.0))
            .shadow(egui::epaint::Shadow {
                offset: Vec2::new(0.0, 12.0),
                blur: 30.0,
                spread: 0.0,
                color: egui::Color32::from_black_alpha(if self.cfg.light_theme { 60 } else { 150 }),
            });

        egui::Window::new("confirm_dialog")
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::new(0.0, -8.0))
            .frame(frame)
            .show(ctx, |ui| {
                ui.set_width(420.0);
                ui.horizontal(|ui| {
                    egui::Frame::none()
                        .fill(p.red.gamma_multiply(0.14))
                        .rounding(9.0)
                        .inner_margin(egui::Margin::symmetric(9.0, 3.0))
                        .show(ui, |ui| {
                            ui.label(RichText::new("!").size(17.0).strong().color(p.red));
                        });
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new(pick("Подтверждение", "Confirm"))
                            .size(18.0)
                            .strong()
                            .color(p.text),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                        if w::close(ui, &p)
                            .on_hover_text(pick("Закрыть", "Close"))
                            .clicked()
                        {
                            cancel = true;
                        }
                    });
                });
                ui.add_space(14.0);
                ui.label(RichText::new(message).size(13.5).color(p.text));
                ui.add_space(18.0);
                ui.horizontal(|ui| {
                    if w::danger(ui, pick("Удалить", "Delete"), &p, true).clicked() {
                        confirmed = true;
                    }
                    if w::soft(ui, pick("Отмена", "Cancel"), &p, true).clicked() {
                        cancel = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(pick(
                                "Enter — подтвердить, Esc — отмена",
                                "Enter — confirm, Esc — cancel",
                            ))
                            .size(10.5)
                            .color(p.muted),
                        );
                    });
                });
                if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    confirmed = true;
                }
            });

        if confirmed {
            self.perform_confirm(action);
            self.confirm = None;
        } else if cancel || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.confirm = None;
        }
    }

    pub(crate) fn perform_confirm(&mut self, action: ConfirmAction) {
        let c = jobs::Ctx2 {
            ctx: self.ctx.clone(),
            tx: self.tx.clone(),
        };
        match action {
            ConfirmAction::DeleteGame(game) => {
                let name = game.display_name();
                let progress = jobs::delete_game(c, self.cfg.clone(), game.clone());
                self.start_parallel_job(
                    fill(
                        &pick("Удаление {name}", "Deleting {name}"),
                        &[("name", &name)],
                    ),
                    progress,
                );
            }
            ConfirmAction::UninstallMod { key, name, game_id } => {
                let Some(game) = self.games.iter().find(|g| g.meta.id == game_id).cloned() else {
                    self.toast(
                        pick(
                            "Сборка, в которую установлен мод, не найдена",
                            "The build this mod is installed into was not found",
                        ),
                        false,
                    );
                    return;
                };
                let progress = jobs::uninstall_mod(c, self.cfg.clone(), game, key);
                self.start_parallel_job(
                    fill(
                        &pick("Удаление мода {name}", "Removing mod {name}"),
                        &[("name", &name)],
                    ),
                    progress,
                );
            }
        }
    }

    /// Затемняет интерфейс под модальным окном и перехватывает клики мимо него.
    fn draw_modal_dim(&self, ctx: &egui::Context) {
        if !self.show_create
            && self.rename.is_none()
            && self.confirm.is_none()
            && self.launch_opts.is_none()
            && self.catalog_selected.is_none()
            && self.catalog_pending.is_none()
            && self.catalog_build.is_none()
            && self.mod_cfg.is_none()
        {
            return;
        }
        let alpha = if self.cfg.light_theme { 90 } else { 140 };
        egui::Area::new(egui::Id::new("modal_dim"))
            .order(egui::Order::Middle)
            .fixed_pos(egui::Pos2::ZERO)
            .show(ctx, |ui| {
                let screen = ctx.screen_rect();
                let (rect, _) =
                    ui.allocate_exact_size(screen.size(), egui::Sense::click_and_drag());
                ui.painter()
                    .rect_filled(rect, 0.0, egui::Color32::from_black_alpha(alpha));
            });
    }

    fn draw_create_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_create {
            return;
        }
        let p = self.p;
        let latest = builds::latest();
        let busy = self.progress.is_some();

        let query = self.picker_query.trim().to_lowercase();
        let mut filtered: Vec<usize> = Vec::new();
        // Последняя версия всегда закреплена сверху.
        filtered.push(usize::MAX);
        for (index, build) in self.builds.iter().enumerate() {
            if !query.is_empty() {
                let haystack = format!(
                    "{} {} {} {}",
                    build.label,
                    build.source,
                    build.date.clone().unwrap_or_default(),
                    build.id
                )
                .to_lowercase();
                if !haystack.contains(&query) {
                    continue;
                }
            }
            filtered.push(index);
        }

        let name = self.new_name.trim().to_string();
        let name_taken = !name.is_empty() && self.name_taken(&name);
        let can_create = !name.is_empty() && !name_taken && !busy;
        let mut create = false;
        let mut cancel = false;
        let mut refresh = false;

        let frame = egui::Frame::none()
            .fill(p.panel)
            .stroke(egui::Stroke::new(1.0_f32, p.border))
            .rounding(16.0)
            .inner_margin(egui::Margin::same(22.0))
            .shadow(egui::epaint::Shadow {
                offset: Vec2::new(0.0, 12.0),
                blur: 30.0,
                spread: 0.0,
                color: egui::Color32::from_black_alpha(if self.cfg.light_theme { 60 } else { 150 }),
            });

        egui::Window::new("new_build_dialog")
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::new(0.0, -8.0))
            .frame(frame)
            .show(ctx, |ui| {
                ui.set_width(600.0);
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.label(
                            RichText::new(pick("Новая сборка", "New build"))
                                .size(19.0)
                                .strong()
                                .color(p.text),
                        );
                        ui.label(
                            RichText::new(pick(
                                "Отдельная копия игры со своими модами",
                                "A separate copy of the game with its own mods",
                            ))
                            .size(11.5)
                            .color(p.muted),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                        if w::close(ui, &p)
                            .on_hover_text(pick("Закрыть", "Close"))
                            .clicked()
                        {
                            cancel = true;
                        }
                    });
                });
                ui.add_space(16.0);

                ui.label(
                    RichText::new(pick("НАЗВАНИЕ СБОРКИ", "BUILD NAME"))
                        .size(10.5)
                        .strong()
                        .color(p.muted),
                );
                ui.add_space(4.0);
                let response = w::text_input(
                    ui,
                    &mut self.new_name,
                    pick(
                        "Например: Для модов или Чистая 2025",
                        "For example: For mods or Clean 2025",
                    ),
                    34.0,
                );
                if self.focus_name {
                    response.request_focus();
                    self.focus_name = false;
                }
                ui.add_space(5.0);
                let (status, color) = if name_taken {
                    (
                        pick(
                            "Сборка с таким названием уже существует",
                            "A build with this name already exists",
                        ),
                        p.red,
                    )
                } else if name.is_empty() {
                    (
                        pick(
                            "Название нужно, чтобы различать сборки в списке",
                            "A name is needed to tell builds apart in the list",
                        ),
                        p.muted,
                    )
                } else {
                    (
                        pick(
                            "Сборка появится в папке games/ под этим названием",
                            "The build will appear in the games/ folder under this name",
                        ),
                        p.muted,
                    )
                };
                // Строка подсказки всегда занимает одно и то же место — окно
                // не «прыгает» при вводе названия.
                ui.allocate_ui_with_layout(
                    Vec2::new(ui.available_width(), 16.0),
                    egui::Layout::left_to_right(egui::Align::Min),
                    |ui| {
                        ui.label(RichText::new(status).size(11.0).color(color));
                    },
                );

                ui.add_space(10.0);
                w::divider(ui, &p);
                ui.add_space(10.0);

                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(pick("ВЕРСИЯ ИГРЫ", "GAME VERSION"))
                            .size(10.5)
                            .strong()
                            .color(p.muted),
                    );
                    if let Some(build) = &self.new_version {
                        let mut selected = format!("· {}", build.label);
                        if let Some(unity) = build.unity_label() {
                            selected.push_str(" · ");
                            selected.push_str(unity);
                        }
                        if build.size > 0 {
                            selected.push_str(&format!(" · {}", human_size(build.size)));
                        }
                        ui.label(RichText::new(selected).size(11.5).color(p.accent));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if w::soft(
                            ui,
                            pick("⟳  Обновить", "⟳  Refresh"),
                            &p,
                            !self.builds_loading,
                        )
                        .clicked()
                        {
                            refresh = true;
                        }
                    });
                });
                ui.add_space(6.0);
                w::text_input(
                    ui,
                    &mut self.picker_query,
                    pick(
                        "Поиск: 2024.10, prototype, Osana…",
                        "Search: 2024.10, prototype, Osana…",
                    ),
                    32.0,
                );
                ui.add_space(8.0);

                // Список версий фиксированной высоты: окно не меняет размер
                // от фильтра и от количества найденных версий.
                let list_height = 252.0;
                ui.allocate_ui_with_layout(
                    Vec2::new(ui.available_width(), list_height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        if self.builds_loading && self.builds.is_empty() {
                            ui.vertical_centered(|ui| {
                                ui.add_space(list_height / 2.0 - 28.0);
                                if let Some(loading) = &self.loading {
                                    loading.show(ui, 56.0);
                                }
                            });
                        } else if self.builds.is_empty() {
                            ui.add_space(16.0);
                            ui.label(
                                RichText::new(self.builds_error.clone().unwrap_or_else(|| {
                                    pick("Список версий пока пуст", "The version list is empty")
                                        .to_string()
                                }))
                                .size(11.5)
                                .color(p.red),
                            );
                        } else {
                            egui::ScrollArea::vertical()
                                .id_salt("create_picker_v2")
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    ui.spacing_mut().item_spacing.y = 6.0;
                                    for &index in &filtered {
                                        let build: &Build = if index == usize::MAX {
                                            &latest
                                        } else {
                                            &self.builds[index]
                                        };
                                        let selected = self
                                            .new_version
                                            .as_ref()
                                            .map(|v| v.id == build.id)
                                            .unwrap_or(false);
                                        if picker_row(ui, &p, build, selected, index == usize::MAX)
                                        {
                                            self.new_version = Some(build.clone());
                                        }
                                    }
                                    if filtered.is_empty() {
                                        ui.add_space(20.0);
                                        ui.vertical_centered(|ui| {
                                            w::muted(
                                                ui,
                                                pick("Ничего не найдено", "Nothing found"),
                                                &p,
                                            );
                                        });
                                        ui.add_space(20.0);
                                    }
                                });
                        }
                    },
                );

                ui.add_space(10.0);
                w::divider(ui, &p);
                ui.add_space(12.0);

                ui.horizontal(|ui| {
                    if w::primary(ui, pick("Создать сборку", "Create build"), &p, can_create)
                        .clicked()
                    {
                        create = true;
                    }
                    if w::soft(ui, pick("Отмена", "Cancel"), &p, true).clicked() {
                        cancel = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let (note, color) = if busy {
                            (
                                pick(
                                    "Дождитесь завершения текущей задачи",
                                    "Wait for the current task to finish",
                                ),
                                p.red,
                            )
                        } else if self.new_version.is_some() {
                            (
                                pick(
                                    "Скачивание и установка идут в фоне",
                                    "Downloading and installing run in the background",
                                ),
                                p.muted,
                            )
                        } else {
                            (
                                pick("Выберите версию в списке", "Choose a version from the list"),
                                p.muted,
                            )
                        };
                        ui.label(RichText::new(note).size(11.0).color(color));
                    });
                });

                // Enter в поле ввода подтверждает создание.
                if can_create && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    create = true;
                }
                ui.add_space(8.0);
                ui.label(
                    RichText::new(pick(
                        "Ctrl+N — открыть окно, Esc — закрыть",
                        "Ctrl+N — open the window, Esc — close",
                    ))
                    .size(10.5)
                    .color(p.muted),
                );
            });

        self.show_create = !create && !cancel;
        if refresh {
            self.refresh_builds();
        }
        if cancel {
            self.show_create = false;
        }
        if create {
            self.create_build();
        }
    }

    fn draw_rename_dialog(&mut self, ctx: &egui::Context) {
        let Some((id, current)) = self.rename.clone() else {
            return;
        };
        let p = self.p;
        let mut buffer = current;
        let mut save = false;
        let mut cancel = false;

        let frame = egui::Frame::none()
            .fill(p.panel)
            .stroke(egui::Stroke::new(1.0_f32, p.border))
            .rounding(16.0)
            .inner_margin(egui::Margin::same(22.0))
            .shadow(egui::epaint::Shadow {
                offset: Vec2::new(0.0, 12.0),
                blur: 30.0,
                spread: 0.0,
                color: egui::Color32::from_black_alpha(if self.cfg.light_theme { 60 } else { 150 }),
            });

        egui::Window::new("rename_build_dialog")
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .frame(frame)
            .show(ctx, |ui| {
                ui.set_width(420.0);
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.label(
                            RichText::new(pick("Переименовать сборку", "Rename build"))
                                .size(18.0)
                                .strong()
                                .color(p.text),
                        );
                        ui.label(
                            RichText::new(pick(
                                "Название видно в списке сборок и в меню сверху. \
                                 Папка на диске не переименовывается.",
                                "The name is shown in the build list and the menu above. \
                                 The folder on disk is not renamed.",
                            ))
                            .size(11.5)
                            .color(p.muted),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                        if w::close(ui, &p)
                            .on_hover_text(pick("Закрыть", "Close"))
                            .clicked()
                        {
                            cancel = true;
                        }
                    });
                });
                ui.add_space(14.0);
                ui.label(
                    RichText::new(pick("НОВОЕ НАЗВАНИЕ", "NEW NAME"))
                        .size(10.5)
                        .strong()
                        .color(p.muted),
                );
                ui.add_space(4.0);
                let response = w::text_input(
                    ui,
                    &mut buffer,
                    pick(
                        "Например: Чистая 2026 или Для модов",
                        "For example: Clean 2026 or For mods",
                    ),
                    32.0,
                );
                if self.rename_focus {
                    response.request_focus();
                    // Выделяем текущее название целиком: ввод сразу заменит его.
                    if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), response.id) {
                        let end = egui::text::CCursor::new(buffer.chars().count());
                        state
                            .cursor
                            .set_char_range(Some(egui::text::CCursorRange::two(
                                egui::text::CCursor::new(0),
                                end,
                            )));
                        state.store(ui.ctx(), response.id);
                    }
                    self.rename_focus = false;
                }
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if w::primary(ui, pick("Сохранить", "Save"), &p, !buffer.trim().is_empty())
                        .clicked()
                    {
                        save = true;
                    }
                    if w::soft(ui, pick("Отмена", "Cancel"), &p, true).clicked() {
                        cancel = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(pick(
                                "Enter — сохранить, Esc — отмена",
                                "Enter — save, Esc — cancel",
                            ))
                            .size(10.5)
                            .color(p.muted),
                        );
                    });
                });
                if !buffer.trim().is_empty() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    save = true;
                }
            });

        if save {
            self.rename = None;
            self.rename_focus = false;
            self.apply_rename(&id, &buffer);
        } else if cancel {
            self.rename = None;
            self.rename_focus = false;
        } else {
            self.rename = Some((id, buffer));
        }
    }

    fn draw_launch_opts_dialog(&mut self, ctx: &egui::Context) {
        let Some((id, current)) = self.launch_opts.clone() else {
            return;
        };
        let p = self.p;
        let mut buffer = current;
        let mut save = false;
        let mut cancel = false;

        let frame = egui::Frame::none()
            .fill(p.panel)
            .stroke(egui::Stroke::new(1.0_f32, p.border))
            .rounding(16.0)
            .inner_margin(egui::Margin::same(22.0))
            .shadow(egui::epaint::Shadow {
                offset: Vec2::new(0.0, 12.0),
                blur: 30.0,
                spread: 0.0,
                color: egui::Color32::from_black_alpha(if self.cfg.light_theme { 60 } else { 150 }),
            });

        egui::Window::new("launch_opts_dialog")
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .frame(frame)
            .show(ctx, |ui| {
                ui.set_width(460.0);
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.label(
                            RichText::new(pick("Параметры запуска", "Launch options"))
                                .size(18.0)
                                .strong()
                                .color(p.text),
                        );
                        ui.label(
                            RichText::new(pick(
                                "Флаги передаются игре при каждом запуске этой сборки \
                                 и хранятся в .yanlan.json.",
                                "The flags are passed to the game on every launch of this \
                                 build and stored in .yanlan.json.",
                            ))
                            .size(11.5)
                            .color(p.muted),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                        if w::close(ui, &p)
                            .on_hover_text(pick("Закрыть", "Close"))
                            .clicked()
                        {
                            cancel = true;
                        }
                    });
                });
                ui.add_space(14.0);
                ui.label(
                    RichText::new(pick("АРГУМЕНТЫ КОМАНДНОЙ СТРОКИ", "COMMAND LINE ARGUMENTS"))
                        .size(10.5)
                        .strong()
                        .color(p.muted),
                );
                ui.add_space(4.0);
                let response = w::text_input(
                    ui,
                    &mut buffer,
                    pick(
                        "Например: -screen-width 1280 -screen-height 720",
                        "For example: -screen-width 1280 -screen-height 720",
                    ),
                    32.0,
                );
                if self.launch_opts_focus {
                    response.request_focus();
                    self.launch_opts_focus = false;
                }
                ui.add_space(6.0);
                ui.label(
                    RichText::new(pick(
                        "Значения с пробелами берите в кавычки. Если в папке сборки есть \
                         Pose-мод, лаунчер запустит его вместо основного exe.",
                        "Wrap values with spaces in quotes. If the build folder contains a \
                         Pose mod, the launcher starts it instead of the main exe.",
                    ))
                    .size(10.5)
                    .color(p.muted),
                );
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if w::primary(ui, pick("Сохранить", "Save"), &p, true).clicked() {
                        save = true;
                    }
                    if w::soft(ui, pick("Отмена", "Cancel"), &p, true).clicked() {
                        cancel = true;
                    }
                    if !buffer.is_empty()
                        && w::soft(ui, pick("Очистить", "Clear"), &p, true).clicked()
                    {
                        buffer.clear();
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(pick(
                                "Enter — сохранить, Esc — отмена",
                                "Enter — save, Esc — cancel",
                            ))
                            .size(10.5)
                            .color(p.muted),
                        );
                    });
                });
                if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    save = true;
                }
            });

        if save {
            self.launch_opts = None;
            self.launch_opts_focus = false;
            self.apply_launch_args(&id, &buffer);
        } else if cancel {
            self.launch_opts = None;
            self.launch_opts_focus = false;
        } else {
            self.launch_opts = Some((id, buffer));
        }
    }
}

fn picker_row(ui: &mut egui::Ui, p: &Palette, build: &Build, selected: bool, pinned: bool) -> bool {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 54.0), egui::Sense::click());
    let bg = if selected {
        p.accent.gamma_multiply(0.16)
    } else if response.hovered() {
        p.card_hover
    } else {
        p.card
    };
    let border = if selected { p.accent } else { p.border };
    ui.painter()
        .rect(rect, 9.0, bg, egui::Stroke::new(1.0_f32, border));

    let inner = rect.shrink2(Vec2::new(12.0, 6.0));
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(util::trim_to(&build.label, 52))
                            .size(13.5)
                            .strong()
                            .color(p.text),
                    );
                    if pinned {
                        w::tag(ui, pick("с сайта", "official"), p.green, p);
                    }
                });
                let mut meta = String::new();
                if let Some(date) = &build.date {
                    meta.push_str(&util::format_date(date));
                    meta.push_str("  ·  ");
                }
                if let Some(unity) = build.unity_label() {
                    meta.push_str(unity);
                    meta.push_str("  ·  ");
                }
                meta.push_str(&builds::source_label(&build.source));
                if build.size > 0 {
                    meta.push_str(&format!("  ·  {}", human_size(build.size)));
                }
                if build.added_at > 0 {
                    meta.push_str(&fill(
                        &pick("  ·  добавлено {date}", "  ·  added {date}"),
                        &[("date", &util::format_ts(build.added_at))],
                    ));
                } else if let Some(note) = &build.note {
                    meta.push_str(&format!("  ·  {note}"));
                }
                ui.label(
                    RichText::new(util::trim_to(&meta, 76))
                        .size(10.5)
                        .color(p.muted),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if selected {
                    ui.label(RichText::new("✓").size(15.0).color(p.accent));
                }
            });
        },
    );

    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response.clicked()
}

fn nav_item(ui: &mut egui::Ui, p: &Palette, selected: bool, icon: &str, label: &str) -> bool {
    let height = 42.0;
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::click());
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, 10.0, p.accent.gamma_multiply(0.14));
        let bar = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 1.0, rect.top() + 9.0),
            Vec2::new(3.0, rect.height() - 18.0),
        );
        painter.rect_filled(bar, 2.0, p.accent);
    } else if response.hovered() {
        painter.rect_filled(rect, 10.0, p.card);
    }
    let color = if selected {
        p.accent
    } else if response.hovered() {
        p.text
    } else {
        p.muted
    };
    painter.text(
        egui::pos2(rect.left() + 18.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        icon,
        egui::FontId::proportional(14.0),
        color,
    );
    painter.text(
        egui::pos2(rect.left() + 46.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(14.5),
        color,
    );
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response.clicked()
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.ctx = ctx.clone();
        self.pump();

        // Горячие клавиши: Ctrl+N — создать сборку, Esc — закрыть диалог.
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::N)) {
            if !self.show_create {
                self.open_create();
            }
        }
        if self.show_create && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.show_create = false;
        }
        if self.rename.is_some() && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.rename = None;
            self.rename_focus = false;
        }
        if self.launch_opts.is_some() && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.launch_opts = None;
            self.launch_opts_focus = false;
        }
        let esc = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let capturing = self
            .mod_cfg
            .as_ref()
            .map_or(false, |editor| editor.capturing());
        let mod_esc = self.mod_cfg.is_some() && esc && !capturing;
        if mod_esc {
            self.mod_cfg = None;
        }
        let catalog_esc = esc && self.player_id.is_none() && !mod_esc;
        if catalog_esc {
            if self.catalog_pending.is_some() {
                self.catalog_pending = None;
            } else if self.catalog_build.is_some() {
                self.catalog_build = None;
                self.catalog_build_focus = false;
            } else if self.catalog_selected.is_some() {
                self.catalog_selected = None;
            }
        }

        // Каталог подгружается при первом открытии вкладки. Если загрузка
        // уже падала, сами не повторяем — ждём кнопку «Обновить».
        if self.tab == Tab::Catalog
            && !self.catalog_loaded
            && !self.catalog_loading
            && self.catalog_error.is_none()
        {
            self.refresh_catalog();
        }

        let busy = self.progress.is_some() || self.builds_loading;
        if busy || self.catalog_loading {
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        // Настройки могли измениться — синхронизируем тему и заголовок окна.
        let want_theme = if self.cfg.light_theme {
            egui::Theme::Light
        } else {
            egui::Theme::Dark
        };
        if ctx.theme() != want_theme {
            self.p = theme::apply(ctx, self.cfg.light_theme);
        } else {
            self.p = theme::palette(self.cfg.light_theme);
        }
        if self.chrome_light != Some(self.cfg.light_theme) {
            theme::apply_window_chrome(frame, self.cfg.light_theme);
            self.chrome_light = Some(self.cfg.light_theme);
        }

        egui::SidePanel::left("sidebar")
            .exact_width(232.0)
            .resizable(false)
            .frame(
                egui::Frame::none()
                    .fill(self.p.panel)
                    .inner_margin(egui::Margin::symmetric(14.0, 16.0)),
            )
            .show(ctx, |ui| self.sidebar(ui));

        // Во время задачи в статус-баре три строки (этап, длинный путь и полоса
        // прогресса) — фиксированной высоты не хватает, полоса обрезалась.
        let status_height = if self.progress.is_some() { 102.0 } else { 74.0 };
        egui::TopBottomPanel::bottom("statusbar")
            .exact_height(status_height)
            .frame(
                egui::Frame::none()
                    .fill(self.p.panel)
                    .inner_margin(egui::Margin::symmetric(24.0, 10.0)),
            )
            .show(ctx, |ui| self.status_bar(ui));

        egui::TopBottomPanel::top("topbar")
            .exact_height(86.0)
            .frame(
                egui::Frame::none()
                    .fill(self.p.bg)
                    .inner_margin(egui::Margin::symmetric(26.0, 14.0)),
            )
            .show(ctx, |ui| self.topbar(ui));

        egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(self.p.bg)
                    .inner_margin(egui::Margin::symmetric(26.0, 4.0)),
            )
            .show(ctx, |ui| match self.tab {
                Tab::Builds => self.builds_view(ui),
                Tab::Catalog => self.catalog_view(ui),
                Tab::Installed => self.installed_view(ui),
                Tab::Settings => self.settings_view(ui),
            });

        self.draw_toasts(ctx);
        self.draw_confirm(ctx);
        self.draw_modal_dim(ctx);
        self.draw_create_dialog(ctx);
        self.draw_rename_dialog(ctx);
        self.draw_launch_opts_dialog(ctx);
        self.draw_catalog_details(ctx);
        self.draw_catalog_pending(ctx);
        self.draw_catalog_build(ctx);
        self.draw_mod_settings(ctx);
        self.draw_player(ctx, frame);
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        // Панели не всегда закрывают самый край окна (дробный масштаб), и тогда
        // проступает стандартный тёмный цвет очистки — в светлой теме это
        // заметная чёрная полоска в 1 пиксель. Красим очистку в фон интерфейса.
        self.p.bg.to_normalized_gamma_f32()
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.cfg.save();
    }
}
