//! Локализация интерфейса: русский и английский языки.
//!
//! Строки не хранятся в словаре с ключами: рядом с местом использования
//! передаются оба варианта (`pick("Сборки", "Builds")`). Так перевод виден
//! сразу в коде и невозможно забыть добавить ключ в таблицу.
//!
//! Текущий язык лежит в глобальной переменной: его читают и фоновые задачи,
//! которые формируют тексты уведомлений и журнала.

use std::sync::atomic::{AtomicU8, Ordering};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    #[default]
    Ru,
    En,
}

impl Lang {
    pub const ALL: [Lang; 2] = [Lang::Ru, Lang::En];

    /// Название языка на нём самом.
    pub fn native(self) -> &'static str {
        match self {
            Lang::Ru => "Русский",
            Lang::En => "English",
        }
    }

    pub fn short(self) -> &'static str {
        match self {
            Lang::Ru => "RU",
            Lang::En => "EN",
        }
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(Lang::Ru as u8);

pub fn set(lang: Lang) {
    CURRENT.store(lang as u8, Ordering::Relaxed);
}

pub fn get() -> Lang {
    if CURRENT.load(Ordering::Relaxed) == Lang::En as u8 {
        Lang::En
    } else {
        Lang::Ru
    }
}

pub fn is_ru() -> bool {
    get() == Lang::Ru
}

/// Возвращает строку на текущем языке.
pub fn pick<T>(ru: T, en: T) -> T {
    if is_ru() {
        ru
    } else {
        en
    }
}

/// Подставляет значения в шаблон вида `{name}`.
pub fn fill(template: &str, args: &[(&str, &str)]) -> String {
    let mut out = template.to_string();
    for (key, value) in args {
        out = out.replace(&format!("{{{key}}}"), value);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_replaces_all_placeholders() {
        let text = fill(
            "{name}: {count} / {count}",
            &[("name", "Build"), ("count", "2")],
        );
        assert_eq!(text, "Build: 2 / 2");
    }
}
