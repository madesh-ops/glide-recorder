//! Windows-specific engine: screen capture, input tracking, audio capture and
//! recording sessions. Platform-independent logic lives in `studio-core`.

#![cfg(windows)]

pub mod audio;
pub mod capture;
pub mod clock;
pub mod input;
pub mod session;
pub mod sources;

pub use studio_core as core;
