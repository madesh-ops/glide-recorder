//! Headless command-line front end for smoke testing and scripting.
//!
//! ```text
//! studio-cli probe
//! studio-cli record [--display N | --window HWND] [--seconds S] [--fps F] [--mic] [--no-system-audio]
//!                   [--out DIR] [--name NAME] [--pause-at S --resume-at S]
//! studio-cli export PROJECT_DIR [--out FILE] [--height H] [--fps F] [--encoder NAME]
//! studio-cli info PROJECT_DIR
//! ```

#[cfg(not(windows))]
fn main() {
    eprintln!("studio-cli only runs on Windows");
}

#[cfg(windows)]
fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    if let Err(e) = cli::run(std::env::args().skip(1).collect()) {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

#[cfg(windows)]
mod cli {
    use anyhow::{anyhow, bail, Context, Result};
    use std::path::PathBuf;
    use std::sync::atomic::AtomicBool;
    use std::time::{Duration, Instant};
    use studio_core::export::{export, ExportRequest};
    use studio_core::media::{locate_ffmpeg, probe_encoders, probe_media};
    use studio_core::project::{resolve_media, Project};
    use studio_core::settings::AppSettings;
    use studio_engine::audio::list_input_devices;
    use studio_engine::session::{RecordOptions, RecordingSession, SourceSelection};
    use studio_engine::sources::{ensure_dpi_awareness, list_displays, list_windows};

    fn flag(args: &[String], name: &str) -> bool {
        args.iter().any(|a| a == name)
    }

    fn opt<T: std::str::FromStr>(args: &[String], name: &str) -> Result<Option<T>> {
        match args.iter().position(|a| a == name) {
            Some(i) => {
                let v = args.get(i + 1).ok_or_else(|| anyhow!("{name} needs a value"))?;
                Ok(Some(v.parse().map_err(|_| anyhow!("invalid value for {name}: {v}"))?))
            }
            None => Ok(None),
        }
    }

    pub fn run(args: Vec<String>) -> Result<()> {
        ensure_dpi_awareness();
        let cmd = args.first().map(String::as_str).unwrap_or("help");
        let rest = &args[1.min(args.len())..];
        match cmd {
            "probe" => probe(),
            "record" => record(rest),
            "export" => export_cmd(rest),
            "info" => info(rest),
            "preview-json" => preview_json(rest),
            "unfinished" => {
                let root: PathBuf = rest.first().map(PathBuf::from).unwrap_or_else(|| AppSettings::load_from(&AppSettings::config_path().unwrap_or_default()).output_dir());
                for u in studio_engine::recovery::find_unfinished(&root, None) {
                    println!("{}  ({}, {:.1} MB)", u.dir.display(), u.name, u.bytes as f64 / 1e6);
                }
                Ok(())
            }
            "recover" => {
                let dir = PathBuf::from(rest.first().ok_or_else(|| anyhow!("missing DIR"))?);
                let out = studio_engine::recovery::recover(&locate_ffmpeg()?, &dir)?;
                info(&[out.to_string_lossy().into_owned()])
            }
            _ => {
                println!("usage: studio-cli probe | record [...] | export PROJECT_DIR [...] | info PROJECT_DIR");
                Ok(())
            }
        }
    }

    fn probe() -> Result<()> {
        let ff = locate_ffmpeg()?;
        println!("ffmpeg:  {}", ff.ffmpeg.display());
        println!("ffprobe: {}", ff.ffprobe.display());
        let t = Instant::now();
        let enc = probe_encoders(&ff);
        println!("working H.264 encoders ({:.1}s):", t.elapsed().as_secs_f64());
        for e in &enc {
            println!("  {:<12} {}", e.name, e.label);
        }
        println!("displays:");
        for (i, d) in list_displays().iter().enumerate() {
            println!("  [{i}] {} {} {}x{} at ({},{}) scale {:.2}{}", d.id, d.name, d.width, d.height, d.x, d.y, d.scale, if d.primary { " primary" } else { "" });
        }
        println!("windows:");
        for w in list_windows().iter().take(25) {
            println!("  {:>10} {:<18} {}x{}  {}", w.id, w.app, w.width, w.height, w.title);
        }
        println!("microphones:");
        for d in list_input_devices() {
            println!("  {}{} [{}]", d.name, if d.is_default { " (default)" } else { "" }, d.id);
        }
        Ok(())
    }

    fn record(args: &[String]) -> Result<()> {
        let ff = locate_ffmpeg()?;
        let encoders = probe_encoders(&ff);
        let settings = AppSettings::load_from(&AppSettings::config_path().unwrap_or_default());
        let source = if let Some(hwnd) = opt::<u64>(args, "--window")? {
            SourceSelection::Window { id: hwnd }
        } else if let Some(area) = opt::<String>(args, "--area")? {
            let v: Vec<u32> = area.split(',').map(|n| n.trim().parse()).collect::<Result<_, _>>().map_err(|_| anyhow!("--area expects x,y,w,h"))?;
            let [x, y, w, h] = v[..] else { bail!("--area expects x,y,w,h") };
            let idx: usize = opt(args, "--display")?.unwrap_or(0);
            let d = list_displays().into_iter().nth(idx).ok_or_else(|| anyhow!("no display #{idx}"))?;
            SourceSelection::Area { id: d.id, x, y, w, h }
        } else {
            let idx: usize = opt(args, "--display")?.unwrap_or(0);
            let d = list_displays().into_iter().nth(idx).ok_or_else(|| anyhow!("no display #{idx}"))?;
            SourceSelection::Display { id: d.id }
        };
        let seconds: f64 = opt(args, "--seconds")?.unwrap_or(5.0);
        let pause_at: Option<f64> = opt(args, "--pause-at")?;
        let resume_at: Option<f64> = opt(args, "--resume-at")?;
        let out: Option<PathBuf> = opt(args, "--out")?;
        let opts = RecordOptions {
            source,
            fps: opt(args, "--fps")?.unwrap_or(settings.fps),
            encoder: opt(args, "--encoder")?.unwrap_or_else(|| settings.encoder.clone()),
            mic: flag(args, "--mic").then_some(None),
            system_audio: !flag(args, "--no-system-audio"),
            output_root: out.unwrap_or_else(|| settings.output_dir()),
            name: opt(args, "--name")?,
            zoom: settings.default_zoom.clone(),
        };
        let session = RecordingSession::start(&ff, &encoders, opts)?;
        println!("recording to {}", session.dir.display());
        let start = Instant::now();
        let mut paused = false;
        let mut resumed = false;
        loop {
            let wall = start.elapsed().as_secs_f64();
            if let (Some(p), false) = (pause_at, paused) {
                if wall >= p {
                    session.pause()?;
                    paused = true;
                    println!("paused at wall {wall:.2}s");
                }
            }
            if let (Some(r), true, false) = (resume_at, paused, resumed) {
                if wall >= r {
                    session.resume()?;
                    resumed = true;
                    println!("resumed at wall {wall:.2}s");
                }
            }
            let st = session.status();
            if st.needs_stop {
                println!("source ended: {:?}", st.warnings);
                break;
            }
            if st.elapsed >= seconds {
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
            print!(
                "\r  t={:.1}s frames={} captured={} lag={:.3}s size={:.1}MB mic={:?} sys={:?}   ",
                st.elapsed, st.frames_written, st.frames_captured, st.lag, st.bytes as f64 / 1e6, st.mic_level, st.system_level
            );
        }
        println!();
        let status = session.status();
        for w in &status.warnings {
            println!("warning: {w}");
        }
        let dir = session.stop()?;
        println!("project: {}", dir.display());
        info(&[dir.to_string_lossy().into_owned()])
    }

    fn export_cmd(args: &[String]) -> Result<()> {
        let dir = PathBuf::from(args.first().ok_or_else(|| anyhow!("missing PROJECT_DIR"))?);
        let ff = locate_ffmpeg()?;
        let encoders = probe_encoders(&ff);
        let project = Project::load(&dir).context("loading project")?;
        let mut settings = project.export.clone();
        if let Some(h) = opt(args, "--height")? {
            settings.height = h;
        }
        if let Some(f) = opt(args, "--fps")? {
            settings.fps = f;
        }
        if let Some(e) = opt::<String>(args, "--encoder")? {
            settings.encoder = e;
        }
        let out: PathBuf = opt(args, "--out")?.unwrap_or_else(|| dir.join("export.mp4"));
        let cancel = AtomicBool::new(false);
        let res = export(
            &ff,
            &encoders,
            ExportRequest { project_dir: &dir, output: &out, settings: Some(settings) },
            &cancel,
            &mut |p| {
                use std::io::Write;
                print!("\r  frame {}/{}  {:.1} fps  eta {:.0}s   ", p.frame, p.total_frames, p.fps, p.eta_secs);
                let _ = std::io::stdout().flush();
            },
        )?;
        println!();
        println!(
            "exported {} ({}x{}, {:.2}s, {} frames, {}, {:.1} MB) in {:.1}s",
            res.output.display(), res.width, res.height, res.duration, res.frames, res.encoder,
            res.size_bytes as f64 / 1e6, res.elapsed_secs
        );
        Ok(())
    }

    /// Print what the editor's `open_project` command returns (for UI fixtures).
    fn preview_json(args: &[String]) -> Result<()> {
        let dir = PathBuf::from(args.first().ok_or_else(|| anyhow!("missing PROJECT_DIR"))?);
        let p = Project::load(&dir)?;
        let track = p.load_cursor(&dir)?;
        let preview = studio_core::preview::build_preview(&p, track.as_ref());
        let out = serde_json::json!({
            "dir": dir, "videoPath": resolve_media(&dir, &p.media.video)?, "hasCursor": track.is_some(),
            "project": p, "preview": preview,
        });
        println!("{}", serde_json::to_string(&out)?);
        Ok(())
    }

    fn info(args: &[String]) -> Result<()> {
        let dir = PathBuf::from(args.first().ok_or_else(|| anyhow!("missing PROJECT_DIR"))?);
        let ff = locate_ffmpeg()?;
        let p = Project::load(&dir)?;
        println!("name: {}  source: {} {}x{} @{}fps", p.name, p.source.label, p.source.width, p.source.height, p.source.fps);
        println!("recorded: {:.3}s, {} frames, encoder {}, pauses {}", p.recording.duration, p.recording.frames_written, p.recording.encoder, p.recording.pause_count);
        let v = probe_media(&ff.ffprobe, &resolve_media(&dir, &p.media.video)?)?;
        println!("video file: {}x{} {:.3}s {:.2}fps {:?}", v.width, v.height, v.duration, v.fps, v.video_codec);
        for (label, rel) in [("mic", &p.media.mic), ("system", &p.media.system_audio)] {
            if let Some(r) = rel {
                let path = resolve_media(&dir, r)?;
                let reader = hound::WavReader::open(&path)?;
                let s = reader.spec();
                let secs = reader.duration() as f64 / s.sample_rate as f64;
                println!("{label} audio: {secs:.3}s @ {} Hz x{}  (Δ vs video {:+.3}s)", s.sample_rate, s.channels, secs - p.recording.duration);
            }
        }
        if let Some(track) = p.load_cursor(&dir)? {
            println!("cursor: {} samples, {} clicks, {} shapes", track.samples.len(), track.clicks.len(), track.shapes.len());
        }
        println!("zoom segments: {}", p.edit.zoom.segments.len());
        for s in &p.edit.zoom.segments {
            println!("  {:.2}s – {:.2}s  {:.2}x", s.start, s.end, s.level);
        }
        if p.recording.duration <= 0.0 {
            bail!("empty recording");
        }
        Ok(())
    }
}
