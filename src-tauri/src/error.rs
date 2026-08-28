//! Unified error type. Every Tauri command returns `AppResult<T>`, so the
//! frontend always receives `{ kind, message }` and can branch on `kind`
//! instead of pattern-matching localized strings.

use serde::{Serialize, Serializer, ser::SerializeStruct};

pub type AppResult<T> = std::result::Result<T, AppError>;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    Other(String),

    #[error("Требуется вход на трекер")]
    NotAuthenticated,

    #[error("Неверный логин или пароль")]
    BadCredentials,

    /// The tracker's public JSON API answered with an error object — it gets
    /// switched off from time to time. Update checking treats this as a signal
    /// to fall back to reading topic pages, not as a failure.
    #[error("API трекера недоступен: {0}")]
    ApiUnavailable(String),

    #[error("Трекер недоступен: {0}. Проверьте подключение или настройте прокси")]
    TrackerUnreachable(String),

    #[error("Не удалось разобрать ответ трекера: {0}")]
    Parse(String),

    #[error("Торрент не найден")]
    TorrentNotFound,

    #[error("Ошибка сети: {0}")]
    Network(#[from] reqwest::Error),

    #[error("Ошибка базы данных: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("Ошибка ввода-вывода: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Engine(#[from] anyhow::Error),
}

impl AppError {
    /// Stable machine-readable discriminant for the frontend.
    pub fn kind(&self) -> &'static str {
        match self {
            AppError::Other(_) => "other",
            AppError::NotAuthenticated => "not_authenticated",
            AppError::BadCredentials => "bad_credentials",
            AppError::ApiUnavailable(_) => "api_unavailable",
            AppError::TrackerUnreachable(_) => "tracker_unreachable",
            AppError::Parse(_) => "parse",
            AppError::TorrentNotFound => "torrent_not_found",
            AppError::Network(_) => "network",
            AppError::Db(_) => "db",
            AppError::Io(_) => "io",
            AppError::Engine(_) => "engine",
        }
    }

    pub fn msg(s: impl Into<String>) -> Self {
        AppError::Other(s.into())
    }
}

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut s = serializer.serialize_struct("AppError", 2)?;
        s.serialize_field("kind", self.kind())?;
        s.serialize_field("message", &self.to_string())?;
        s.end()
    }
}

impl From<librqbit::Error> for AppError {
    fn from(e: librqbit::Error) -> Self {
        AppError::Other(e.to_string())
    }
}
