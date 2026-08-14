# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build Commands

```powershell
# CPU build
cargo build --release

# GPU build (recommended — requires CUDA Toolkit + MSVC)
cargo build --release --features cuda

# Vulkan build (alternative GPU, no CUDA required)
cargo build --release --features vulkan
```

Binary output: `target/release/t-whisper.exe`

Set `CARGO_TARGET_DIR` to a local path if the repo is on a network drive (X:) for faster builds.

### Tests

```powershell
cargo test                       # all unit tests
cargo test speech                # single test by name substring (e.g. SpeechGate cases)
```

Unit tests are pure-logic and need no GPU/model: `SpeechGate` state machine (`vad.rs`), version comparison (`update.rs`), Swedish number parsing (`numbers.rs`).

### Installer

Built from `installer/t-whisper.iss` using Inno Setup. See the script header for full instructions.

### CI

GitHub Actions builds on push — see `.github/workflows/`.

## Versioning

After every change: bump version in `Cargo.toml`, update the README heading (`# T-Whisper vX.Y.Z`), and add a row to the versionshistorik table. See memory file `readme-versionshantering.md`.

## Architecture

Single-binary Windows tray app, no webview. All modules live under `src/`:

| Module | Role |
|---|---|
| `main.rs` | Event loop (`tao`), tray icon/menu (`tray-icon`), global hotkey (`global-hotkey`), `Cmd`/`AppState`/`UserEvent` types |
| `audio.rs` | WASAPI microphone via `cpal`; resamples to 16 kHz mono f32; lock-free ring buffer (`rtrb`) |
| `transcribe.rs` | whisper.cpp via `whisper-rs`; keeps the model loaded persistently in VRAM |
| `model.rs` | Downloads GGML model from Hugging Face on first use to `%APPDATA%\T-Whisper\models\` |
| `vad.rs` | Silero-VAD for continuous mode; downloaded automatically (<1 MB) |
| `config.rs` | `serde`+TOML config in `%APPDATA%\T-Whisper\config.toml` |
| `numbers.rs` | Converts Swedish number words to digits ("tjugofem" → 25) |
| `sound.rs` | Audio feedback (click/pling) via Windows audio |
| `update.rs` | Checks GitHub Releases for newer versions via `reqwest` |
| `winutil.rs` | Single-instance lock, `message_box`, logging to `%APPDATA%\T-Whisper\log.txt` |

### Data flow

```
[Global hotkey / VAD] → audio.rs (16 kHz ring buffer) → transcribe.rs (whisper.cpp GPU/CPU)
    → numbers.rs (optional) → enigo (clipboard+Ctrl+V paste at cursor)
```

Push-to-talk: keydown starts recording, keyup stops and transcribes.  
Continuous mode: VAD detects speech onset/offset; triggers transcription on silence.

The tray icon renders mic level (8 steps) and app state (Idle/Recording/Working) as pre-rendered icon frames. State transitions flow through a `tao` `EventLoopProxy<UserEvent>` so background threads never touch UI directly.

## Runtime Files

- Config: `%APPDATA%\T-Whisper\config.toml` (created on first start, self-documenting)
- Models: `%APPDATA%\T-Whisper\models\` (downloaded automatically)
- Log: `%APPDATA%\T-Whisper\log.txt` (rotated at ~1 MB)

## Known Constraints

- Apps running elevated won't receive simulated keyboard input unless T-Whisper is also elevated.
- The hotkey key is swallowed globally while the app runs.
- CUDA DLLs (`cudart64_13.dll`, `cublas64_13.dll`, `cublasLt64_13.dll`) must be beside the exe when running outside the installer on machines without CUDA Toolkit in PATH.
