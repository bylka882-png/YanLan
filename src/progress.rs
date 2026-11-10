//! Разделяемый между потоком-воркером и UI прогресс текущей задачи.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

static NEXT_JOB_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Default)]
struct Inner {
    stage: String,
    detail: String,
    done: u64,
    total: u64,
    file_index: usize,
    file_total: usize,
    finished: bool,
    error: Option<String>,
    started: Option<Instant>,
}

#[derive(Clone)]
pub struct Progress {
    inner: Arc<Mutex<Inner>>,
    cancel: Arc<AtomicBool>,
    /// Уникальный номер задачи: по нему интерфейс отличает свою задачу от фоновой.
    id: u64,
}

#[derive(Clone, Default)]
pub struct Snapshot {
    pub stage: String,
    pub detail: String,
    pub done: u64,
    pub total: u64,
    pub file_index: usize,
    pub file_total: usize,
    /// Оставлено для отладки: задача завершена (успешно или с ошибкой).
    #[allow(dead_code)]
    pub finished: bool,
    pub error: Option<String>,
    pub elapsed: f64,
    pub cancelled: bool,
}

impl Default for Progress {
    fn default() -> Self {
        Self::new()
    }
}

impl Progress {
    pub fn new() -> Self {
        let inner = Inner {
            started: Some(Instant::now()),
            ..Default::default()
        };
        Self {
            inner: Arc::new(Mutex::new(inner)),
            cancel: Arc::new(AtomicBool::new(false)),
            id: NEXT_JOB_ID.fetch_add(1, Ordering::SeqCst),
        }
    }

    /// Номер задачи, уникальный в пределах запуска лаунчера.
    pub fn id(&self) -> u64 {
        self.id
    }

    fn with<F: FnOnce(&mut Inner)>(&self, f: F) {
        if let Ok(mut guard) = self.inner.lock() {
            f(&mut guard);
        }
    }

    pub fn stage(&self, text: impl Into<String>) {
        let text = text.into();
        self.with(|i| {
            i.stage = text;
            i.detail.clear();
            i.file_index = 0;
            i.file_total = 0;
        });
    }

    pub fn detail(&self, text: impl Into<String>) {
        let text = text.into();
        self.with(|i| i.detail = text);
    }

    pub fn reset_bytes(&self, total: u64) {
        self.with(|i| {
            i.done = 0;
            i.total = total;
        });
    }

    pub fn total(&self, total: u64) {
        self.with(|i| i.total = total);
    }

    pub fn set_done(&self, done: u64) {
        self.with(|i| i.done = done);
    }

    pub fn add_done(&self, delta: u64) {
        self.with(|i| i.done = i.done.saturating_add(delta));
    }

    pub fn files(&self, index: usize, total: usize) {
        self.with(|i| {
            i.file_index = index;
            i.file_total = total;
        });
    }

    pub fn finish(&self) {
        self.with(|i| i.finished = true);
    }

    pub fn fail(&self, message: impl Into<String>) {
        self.with(|i| {
            i.error = Some(message.into());
            i.finished = true;
        });
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    pub fn snapshot(&self) -> Snapshot {
        let cancelled = self.is_cancelled();
        match self.inner.lock() {
            Ok(i) => Snapshot {
                stage: i.stage.clone(),
                detail: i.detail.clone(),
                done: i.done,
                total: i.total,
                file_index: i.file_index,
                file_total: i.file_total,
                finished: i.finished,
                error: i.error.clone(),
                elapsed: i.started.map(|s| s.elapsed().as_secs_f64()).unwrap_or(0.0),
                cancelled,
            },
            Err(_) => Snapshot {
                cancelled,
                ..Default::default()
            },
        }
    }
}
