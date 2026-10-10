//! Microphone and system-audio (WASAPI loopback) capture to clock-aligned WAV.
//!
//! The cpal stream lives on its own thread (streams are not `Send` on every
//! backend). Callbacks only convert to f32 and push into a bounded channel; a
//! writer thread aligns samples to the recording clock:
//!
//! * gaps (loopback delivers nothing while the system is silent, or the device
//!   hiccups) are filled with silence;
//! * if the device runs ahead of the clock (drift), excess samples are dropped;
//! * samples arriving while paused are discarded;
//! * on stop the file is padded to the exact recording duration.

use crate::clock::SessionClock;
use anyhow::{anyhow, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, StreamConfig};
use crossbeam_channel::{bounded, Receiver, RecvTimeoutError};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Allowed misalignment before correcting (seconds).
const TOLERANCE: f64 = 0.04;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioDeviceInfo {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

fn device_id(d: &cpal::Device) -> String {
    d.id().map(|i| i.id().to_string()).unwrap_or_default()
}

fn device_name(d: &cpal::Device) -> String {
    d.description().map(|x| x.name().to_string()).unwrap_or_else(|_| "Unknown device".into())
}

pub fn list_input_devices() -> Vec<AudioDeviceInfo> {
    let host = cpal::default_host();
    let default_id = host.default_input_device().map(|d| device_id(&d));
    let Ok(devs) = host.input_devices() else { return Vec::new() };
    devs.map(|d| {
        let id = device_id(&d);
        AudioDeviceInfo { is_default: Some(&id) == default_id.as_ref(), name: device_name(&d), id }
    })
    .collect()
}

#[derive(Debug, Clone)]
pub enum AudioSource {
    /// Microphone by cpal device id (`None` = default input).
    Microphone(Option<String>),
    /// What the default output device is playing.
    SystemLoopback,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioStats {
    pub path: PathBuf,
    pub sample_rate: u32,
    pub channels: u16,
    pub frames: u64,
    pub silence_inserted_frames: u64,
    pub dropped_frames: u64,
}

struct Chunk {
    /// When the chunk's *first* sample was captured by the device.
    first_sample_at: Instant,
    data: Vec<f32>,
}

pub struct AudioRecorder {
    stop: Arc<AtomicBool>,
    stream_thread: Option<JoinHandle<()>>,
    writer: Option<JoinHandle<Result<AudioStats>>>,
    final_duration: Arc<AtomicU64>,
    level: Arc<AtomicU32>,
    pub device_name: String,
}

fn find_device(source: &AudioSource) -> Result<cpal::Device> {
    let host = cpal::default_host();
    match source {
        AudioSource::SystemLoopback => {
            host.default_output_device().ok_or_else(|| anyhow!("no audio output device for system audio"))
        }
        AudioSource::Microphone(None) => host.default_input_device().ok_or_else(|| anyhow!("no microphone found")),
        AudioSource::Microphone(Some(id)) => host
            .input_devices()?
            .find(|d| &device_id(d) == id)
            .or_else(|| host.default_input_device())
            .ok_or_else(|| anyhow!("microphone not found")),
    }
}

fn to_f32(data: &cpal::Data) -> Option<Vec<f32>> {
    Some(match data.sample_format() {
        SampleFormat::F32 => data.as_slice::<f32>()?.to_vec(),
        SampleFormat::F64 => data.as_slice::<f64>()?.iter().map(|&s| s as f32).collect(),
        SampleFormat::I16 => data.as_slice::<i16>()?.iter().map(|&s| s as f32 / 32768.0).collect(),
        SampleFormat::I32 => data.as_slice::<i32>()?.iter().map(|&s| s as f32 / 2_147_483_648.0).collect(),
        SampleFormat::U16 => data.as_slice::<u16>()?.iter().map(|&s| (s as f32 - 32768.0) / 32768.0).collect(),
        SampleFormat::U8 => data.as_slice::<u8>()?.iter().map(|&s| (s as f32 - 128.0) / 128.0).collect(),
        SampleFormat::I8 => data.as_slice::<i8>()?.iter().map(|&s| s as f32 / 128.0).collect(),
        _ => return None,
    })
}

impl AudioRecorder {
    pub fn start(source: AudioSource, clock: Arc<SessionClock>, path: &Path) -> Result<Self> {
        let device = find_device(&source)?;
        let device_name = device_name(&device);
        let supported = match source {
            AudioSource::SystemLoopback => device.default_output_config(),
            AudioSource::Microphone(_) => device.default_input_config(),
        }
        .with_context(|| format!("audio device '{device_name}' has no usable format"))?;
        let config: StreamConfig = supported.config();
        let (rate, channels) = (config.sample_rate, config.channels);

        let stop = Arc::new(AtomicBool::new(false));
        let level = Arc::new(AtomicU32::new(0));
        let (tx, rx) = bounded::<Chunk>(512);
        let (ready_tx, ready_rx) = bounded::<Result<(), String>>(1);

        let stream_stop = stop.clone();
        let cb_level = level.clone();
        let stream_thread = std::thread::Builder::new().name("studio-audio-stream".into()).spawn(move || {
            let err_fn = |e: cpal::Error| log::warn!("audio stream error: {e}");
            let stream = device.build_input_stream_raw(
                config,
                supported.sample_format(),
                move |data: &cpal::Data, info: &cpal::InputCallbackInfo| {
                    // WASAPI reports the QPC time of the first captured sample; use it
                    // instead of the (later) callback time so audio is not placed late.
                    let ts = info.timestamp();
                    let latency = ts.callback.duration_since(ts.capture).min(Duration::from_millis(500));
                    let first_sample_at = Instant::now().checked_sub(latency).unwrap_or_else(Instant::now);
                    if let Some(samples) = to_f32(data) {
                        let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
                        cb_level.store(peak.to_bits(), Ordering::Relaxed);
                        let _ = tx.try_send(Chunk { first_sample_at, data: samples });
                    }
                },
                err_fn,
                None,
            );
            let stream = match stream.and_then(|s| s.play().map(|_| s)) {
                Ok(s) => {
                    let _ = ready_tx.send(Ok(()));
                    s
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e.to_string()));
                    return;
                }
            };
            while !stream_stop.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(20));
            }
            drop(stream);
        })?;

        match ready_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                stop.store(true, Ordering::SeqCst);
                let _ = stream_thread.join();
                return Err(anyhow!("could not open '{device_name}': {e}"));
            }
            Err(_) => {
                stop.store(true, Ordering::SeqCst);
                return Err(anyhow!("timed out opening '{device_name}'"));
            }
        }

        let final_duration = Arc::new(AtomicU64::new(u64::MAX));
        let fd = final_duration.clone();
        let wstop = stop.clone();
        let path = path.to_path_buf();
        let writer = std::thread::Builder::new()
            .name("studio-audio-writer".into())
            .spawn(move || write_loop(rx, clock, path, rate, channels, wstop, fd))?;

        Ok(Self {
            stop,
            stream_thread: Some(stream_thread),
            writer: Some(writer),
            final_duration,
            level,
            device_name,
        })
    }

    /// Most recent peak level (0..1).
    pub fn level(&self) -> f32 {
        f32::from_bits(self.level.load(Ordering::Relaxed))
    }

    /// Stop and pad the file to `duration` seconds.
    pub fn finish(mut self, duration: f64) -> Result<AudioStats> {
        self.final_duration.store(duration.max(0.0).to_bits(), Ordering::SeqCst);
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.stream_thread.take() {
            let _ = h.join();
        }
        self.writer.take().unwrap().join().map_err(|_| anyhow!("audio writer panicked"))?
    }
}

impl Drop for AudioRecorder {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

fn write_loop(
    rx: Receiver<Chunk>,
    clock: Arc<SessionClock>,
    path: PathBuf,
    rate: u32,
    channels: u16,
    stop: Arc<AtomicBool>,
    final_duration: Arc<AtomicU64>,
) -> Result<AudioStats> {
    let spec = hound::WavSpec { channels, sample_rate: rate, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut w = hound::WavWriter::create(&path, spec)?;
    let ch = channels as usize;
    let mut written: u64 = 0; // frames
    let mut stats = AudioStats { path: path.clone(), sample_rate: rate, channels, ..Default::default() };
    let tol = (TOLERANCE * rate as f64) as i64;
    let silence = |w: &mut hound::WavWriter<_>, frames: u64| -> Result<()> {
        for _ in 0..frames * ch as u64 {
            w.write_sample(0i16)?;
        }
        Ok(())
    };

    loop {
        let chunk = match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(c) => c,
            Err(RecvTimeoutError::Timeout) => {
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => break,
        };
        let frames = (chunk.data.len() / ch) as i64;
        let Some(t_start) = clock.at(chunk.first_sample_at) else { continue }; // paused
        let expected_start = (t_start * rate as f64) as i64;
        let mut skip = 0usize;
        let diff = expected_start - written as i64;
        if diff > tol {
            silence(&mut w, diff as u64)?;
            written += diff as u64;
            stats.silence_inserted_frames += diff as u64;
        } else if diff < -tol {
            let excess = (-diff).min(frames) as usize;
            skip = excess;
            stats.dropped_frames += excess as u64;
        }
        for s in &chunk.data[skip * ch..] {
            w.write_sample((s.clamp(-1.0, 1.0) * 32767.0) as i16)?;
        }
        written += (frames as usize - skip) as u64;
    }
    let dur = f64::from_bits(final_duration.load(Ordering::SeqCst));
    if dur.is_finite() {
        let target = (dur * rate as f64).round() as u64;
        if target > written {
            silence(&mut w, target - written)?;
            stats.silence_inserted_frames += target - written;
            written = target;
        }
    }
    w.finalize()?;
    stats.frames = written;
    Ok(stats)
}
