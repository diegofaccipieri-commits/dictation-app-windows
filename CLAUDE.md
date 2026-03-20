# DictationApp Windows

Offline speech-to-text app for Windows using Tauri 2 + whisper.cpp.

## Tech Stack
- **Backend:** Rust (Tauri 2)
- **Frontend:** HTML/CSS/JS
- **Transcription:** whisper-rs (whisper.cpp binding)
- **Audio:** cpal (WASAPI on Windows)
- **Hotkey:** Win32 WH_KEYBOARD_LL hook (double-tap Ctrl)
- **Paste:** arboard (clipboard) + Win32 SendInput (Ctrl+V)

## Project Status
All source code written. Needs compilation and testing on Windows.

## Build Instructions
```bash
# Prerequisites: Rust, Node.js, Visual Studio Build Tools (C++ workload)
npm install
npx tauri build
```

The .msi installer will be in `src-tauri/target/release/bundle/msi/`.

For debug builds: `npx tauri dev`

## Architecture
- `src-tauri/src/main.rs` — App entry, tray, Tauri commands, recording flow
- `src-tauri/src/audio.rs` — Mic capture via cpal, resample to 16kHz
- `src-tauri/src/transcriber.rs` — whisper-rs wrapper, Small + Turbo models
- `src-tauri/src/text_cleaner.rs` — Hallucination filter, punctuation commands
- `src-tauri/src/hotkey.rs` — Win32 keyboard hook (double-tap Ctrl)
- `src-tauri/src/paste.rs` — Clipboard + SendInput Ctrl+V
- `src-tauri/src/models.rs` — Model download from HuggingFace, cache in %APPDATA%
- `src-tauri/src/batch.rs` — File/folder transcription with timestamps
- `src/` — Frontend (index.html, app.js, style.css)

## Specs and Plans
- Design spec: `docs/superpowers/specs/2026-03-20-dictation-app-windows-design.md`
- Implementation plan: `docs/superpowers/plans/2026-03-20-dictation-app-windows.md`

## Current Task
**Task 12: Build, Test, and Release.** Code was written on macOS without compilation. Fix any compilation errors on Windows, test the full flow, create GitHub release.

## Key Notes
- whisper-rs requires cmake and a C compiler to build whisper.cpp from source
- The `windows` crate features are only compiled on Windows (`cfg(windows)`)
- Models are downloaded on first launch (~466MB Small + ~1.5GB Turbo)
- Auto-update pubkey in tauri.conf.json needs to be filled after generating signing keys
