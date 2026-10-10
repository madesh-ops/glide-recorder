use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid JSON in {path}: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("unsupported project version {found} (this build supports up to {supported})")]
    UnsupportedVersion { found: u32, supported: u32 },
    #[error("path escapes the project folder: {0}")]
    PathEscape(String),
    #[error("FFmpeg was not found. Install FFmpeg or place ffmpeg.exe next to the application.")]
    FfmpegMissing,
    #[error("no working H.264 encoder was found")]
    NoEncoder,
    #[error("FFmpeg failed: {0}")]
    Ffmpeg(String),
    #[error("invalid media: {0}")]
    InvalidMedia(String),
    #[error("cancelled")]
    Cancelled,
    #[error("{0}")]
    Invalid(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
