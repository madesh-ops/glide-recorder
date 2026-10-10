# Third-party notices

Glide is licensed under MIT OR Apache-2.0. It depends on the open-source
components below. All are permissively licensed, apart from the MPL-2.0
crates noted under "Rust crates".

## Direct dependencies

| Component | Licence | Use |
|---|---|---|
| [Tauri 2](https://tauri.app) (`tauri`, `tauri-build`, `@tauri-apps/api`, `tauri-plugin-dialog`, `tauri-plugin-opener`) | MIT OR Apache-2.0 | Desktop shell, IPC, dialogs |
| [windows-rs](https://github.com/microsoft/windows-rs) (`windows`, `windows-core`) | MIT OR Apache-2.0 | Windows Graphics Capture, D3D11, Win32 |
| [cpal](https://github.com/RustAudio/cpal) | Apache-2.0 | WASAPI microphone and loopback capture |
| [hound](https://github.com/ruuda/hound) | Apache-2.0 | WAV writing |
| [serde](https://serde.rs), serde_json | MIT OR Apache-2.0 | Serialization |
| [rayon](https://github.com/rayon-rs/rayon) | MIT OR Apache-2.0 | Parallel compositor |
| [png](https://github.com/image-rs/image-png) | MIT OR Apache-2.0 | Cursor and background images |
| crossbeam-channel, parking_lot, thiserror, anyhow, log, env_logger, uuid, dirs, base64 | MIT OR Apache-2.0 | Utilities |
| [React](https://react.dev), react-dom | MIT | UI |
| [zustand](https://github.com/pmndrs/zustand) | MIT | UI state |
| [Lucide](https://lucide.dev) (`lucide-react`) | ISC | Icons |
| Vite, TypeScript, Vitest (development only) | MIT / Apache-2.0 | Build and test tooling |

## Rust crates

The full transitive dependency tree has about 495 crates (`cargo metadata`).
Their licences are MIT, Apache-2.0, MIT OR Apache-2.0, Zlib, BSD-2/3-Clause,
ISC, Unicode-3.0, Unlicense, CC0, 0BSD, and MPL-2.0. The MPL-2.0 crates
(5, mostly from the WebView/HTML tooling) are used unmodified. To regenerate
a complete list with licence texts:

```bash
cargo install cargo-about
```

```bash
cargo about generate about.hbs > licenses.html
```

## FFmpeg

Glide does **not** bundle or link FFmpeg. It runs a user-installed
`ffmpeg.exe` and `ffprobe.exe` as separate processes. FFmpeg is licensed
under LGPL-2.1 or later, or GPL-2.0 or later depending on the build (the
common "full" Windows builds are GPL). If you redistribute Glide together
with an FFmpeg binary, you must meet that binary's licence terms.

## Trademarks

"Screen Studio" is a trademark of its owners. Glide is not affiliated with or
endorsed by them. The name is used only to describe the workflow that
inspired this project. Glide includes no Screen Studio code, artwork,
wallpapers, or icons. The app icon and background presets are original.
