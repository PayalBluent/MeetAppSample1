use serde::{Serialize, Serializer};

/// Application-wide error type. Command handlers return `Result<T, AppError>`;
/// Tauri serializes the error to the frontend as a string message.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("meeting not found: {0}")]
    MeetingNotFound(String),

    #[error("a recording is already in progress")]
    AlreadyRecording,

    #[error("no active recording to stop")]
    NotRecording,

    #[error("audio device error: {0}")]
    Audio(String),

    #[error("transcription error: {0}")]
    Transcription(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Other(String),
}

impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl From<anyhow::Error> for AppError {
    fn from(e: anyhow::Error) -> Self {
        AppError::Other(e.to_string())
    }
}

impl From<String> for AppError {
    fn from(e: String) -> Self {
        AppError::Other(e)
    }
}

pub type AppResult<T> = Result<T, AppError>;
