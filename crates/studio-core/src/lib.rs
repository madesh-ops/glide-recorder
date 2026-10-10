//! Platform-independent core of Glide, a Windows screen recorder and editor.
//!
//! Everything in this crate is deterministic and testable without Windows APIs:
//! the camera / auto-zoom engine, cursor smoothing, the non-destructive timeline,
//! layout geometry shared by preview and export, the project format, settings,
//! the recording state machine, the CPU compositor and the FFmpeg-based export.

pub mod camera;
pub mod cursor;
pub mod easing;
pub mod error;
pub mod export;
pub mod geometry;
pub mod layout;
pub mod media;
pub mod preview;
pub mod project;
pub mod recording;
pub mod render;
pub mod settings;
pub mod timeline;

pub use error::{Error, Result};
