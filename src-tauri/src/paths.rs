//! Validation of paths received from the webview.

use std::path::{Path, PathBuf};
use studio_core::project::PROJECT_FILE;
use tauri::{AppHandle, Manager};

/// Canonicalize a project folder and require that it contains `project.json`.
pub fn project_dir(dir: &str) -> Result<PathBuf, String> {
    let p = Path::new(dir);
    let canon = dunce(p.canonicalize().map_err(|e| format!("project folder not found: {e}"))?);
    if !canon.join(PROJECT_FILE).is_file() {
        return Err("this folder is not a Glide project".into());
    }
    Ok(canon)
}

/// `canonicalize` on Windows returns `\\?\C:\...`; strip the prefix for display
/// and for FFmpeg, keeping UNC paths intact.
pub fn dunce(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        if !rest.starts_with("UNC\\") {
            return PathBuf::from(rest);
        }
    }
    p
}

/// True if `child` is inside `root` (both canonicalized).
pub fn is_within(child: &Path, root: &Path) -> bool {
    match (child.canonicalize(), root.canonicalize()) {
        (Ok(c), Ok(r)) => c.starts_with(r),
        _ => false,
    }
}

/// Let the webview load files from `dir` through the asset protocol.
pub fn allow_asset_dir(app: &AppHandle, dir: &Path) {
    if let Err(e) = app.asset_protocol_scope().allow_directory(dir, true) {
        log::warn!("asset scope for {}: {e}", dir.display());
    }
}

/// Export targets must be .mp4 files in an existing folder.
pub fn export_target(path: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(path);
    if !p.is_absolute() {
        return Err("choose an absolute output path".into());
    }
    let ext_ok = p.extension().is_some_and(|e| e.eq_ignore_ascii_case("mp4"));
    if !ext_ok {
        return Err("exports must use the .mp4 extension".into());
    }
    let parent = p.parent().ok_or("invalid output path")?;
    if !parent.is_dir() {
        return Err("the output folder does not exist".into());
    }
    Ok(p)
}
