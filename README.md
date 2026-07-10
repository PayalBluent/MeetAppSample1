# MeetApp — AI Meeting Assistant

A premium, cross-platform desktop meeting assistant (in the spirit of Granola,
Fireflies.ai, Notta and Read.ai) built with **Tauri v2 + Rust** and
**React + TypeScript + Tailwind v4 + shadcn-style UI + Framer Motion + Zustand +
TanStack Query**.

It auto-detects meetings on Google Meet, Zoom, Microsoft Teams, Discord and Slack
Huddles, captures them in one of three modes, and produces a transcript, an AI
summary, and action items — all in memory (no database, no cloud sync yet).

> **First release targets Windows.** The core is platform-independent and all
> OS-specific code is isolated in `src-tauri/src/platform/`, so macOS can be added
> by implementing two facades (`platform::detect` and `platform::screen`).

---

## Highlights

- **Tray control panel** — the compact “MeetApp” popover: mode selector
  (Transcribe / Record / Record Video / Off), Record Live, Send Bot, and noise
  cancellation toggles.
- **Meetings list** — searchable, sortable, filterable; multi-select with bulk
  actions; star / bookmark / lock; rename & delete.
- **Meeting detail** — AI summary, key points & decisions, action items,
  full transcript with speaker diarization + click-to-seek, timeline, participants
  with talk-time, and an audio/video player with a scrubbable waveform.
- **Live pipeline** — detect → capture → streaming transcript → stop → summarize,
  streamed to the UI over Tauri events.
- **Native tray + background operation** — status-aware tray menu, quick actions,
  close-to-tray, single-instance, optional launch-on-startup.

---

## Tech decisions

| Concern | Choice | Why |
| --- | --- | --- |
| Microphone + system audio (Windows) | [`wasapi`](https://crates.io/crates/wasapi) + [`hound`](https://crates.io/crates/hound) | Safe, maintained WASAPI bindings. One code path captures the mic (default capture device) and system output (default render device → loopback), with shared-mode format auto-conversion to a fixed 48 kHz f32 stereo stream. Always on for Windows. |
| Microphone (other platforms) | [`cpal`](https://crates.io/crates/cpal) | Reliable cross-platform capture; used off-Windows and as a Windows last-resort fallback. |
| Screen + video | [`windows-capture`](https://crates.io/crates/windows-capture) | Fastest Windows capture, HW-accelerated encoder, Graphics.Capture API. Opt-in. |
| Transcription | [`whisper-rs`](https://crates.io/crates/whisper-rs) behind a `Transcriber` trait | Most-used whisper.cpp binding. Trait lets the app run without the heavy ML build and swap backends. Opt-in. |
| Meeting detection | [`sysinfo`](https://crates.io/crates/sysinfo) + Win32 window titles | Process scan + window titles to catch browser meetings (Google Meet). |
| Shell / tray | Tauri v2 `tray-icon` + `single-instance` + `autostart` + `opener`/`dialog`/`fs`/`notification` | Native, v2-first. |

Everything heavy is behind a **trait + Cargo feature**, so the default build uses
only reliable crates and is fully functional (real detection, real mic recording,
built-in transcription/summary). See [Feature flags](#feature-flags).

---

## Prerequisites

- **Node.js ≥ 18** and npm (verified with Node 24 / npm 11).
- **Rust toolchain** (stable) via [rustup](https://rustup.rs).
- **Microsoft C++ Build Tools** (the MSVC toolchain — Tauri needs `link.exe`).
- **WebView2 runtime** (preinstalled on Windows 11).

### Install Rust + Build Tools on Windows

```powershell
# 1) Rust (installs to %USERPROFILE%\.cargo)
winget install --id Rustlang.Rustup -e
# then restart the shell so cargo is on PATH, or run: rustup default stable

# 2) Microsoft C++ Build Tools (Desktop development with C++ workload)
winget install --id Microsoft.VisualStudio.2022.BuildTools -e `
  --override "--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```

Verify:

```powershell
cargo --version
rustc --version
```

---

## Running

Install JS dependencies once:

```bash
npm install
```

### A) Web preview (no Rust required)

The UI runs fully in a browser against an in-memory **mock backend** that mirrors
the exact Tauri command + event surface — great for UI work.

```bash
npm run dev        # http://localhost:1420
```

### B) Full desktop app (requires Rust + Build Tools)

```bash
npm run app:dev    # tauri dev — launches the native window + tray
npm run app:build  # tauri build — produces an installer in src-tauri/target/release/bundle
```

---

## Feature flags

The default build (`mic-capture`) is lean and fully functional. Enable heavier
native backends explicitly:

```bash
# System (loopback) audio capture — Windows/WASAPI (reserved; see note)
npm run app:dev -- --features system-audio

# Screen + video recording — Windows.Graphics.Capture (experimental)
npm run app:dev -- --features screen-capture

# On-device transcription with whisper.cpp (needs CMake + a model, see below)
npm run app:dev -- --features whisper
```

> **Note on capture completeness.** On Windows the default build captures **both**
> the microphone and system output and mixes them into a 48 kHz stereo WAV, using
> the [`wasapi`](https://crates.io/crates/wasapi) crate (the mic is the default
> capture device; system audio is the default render device via loopback). The
> `system-audio` Cargo feature is therefore now a no-op kept only for compatibility.
> `screen-capture` and `whisper` are functional but experimental and may need minor
> version alignment on first build.

### On-device transcription (whisper)

1. Install CMake and a C/C++ compiler (the MSVC Build Tools above suffice).
2. Download a GGML/GGUF model, e.g. `ggml-base.en.bin` from
   [whisper.cpp models](https://huggingface.co/ggerganov/whisper.cpp).
3. Point the app at it:

   ```powershell
   $env:MEETAPP_WHISPER_MODEL = "C:\models\ggml-base.en.bin"
   npm run app:dev -- --features whisper
   ```

Without the `whisper` feature, a high-quality **simulated transcriber** drives the
pipeline so the whole experience works end-to-end offline.

---

## Architecture

```
meetapp/
├── src/                         # React frontend (platform-independent)
│   ├── App.tsx                  # providers, router, window-label routing
│   ├── lib/
│   │   ├── tauri.ts             # bridge: real invoke/listen OR mock backend
│   │   ├── api.ts               # typed command façade
│   │   ├── query.ts             # TanStack Query client + keys
│   │   ├── media.ts             # asset-protocol media URLs
│   │   └── meta.tsx             # platform + mode metadata
│   ├── mock/                    # in-memory backend for browser/dev
│   ├── hooks/                   # query hooks + event → cache bridge
│   ├── stores/                  # Zustand UI state
│   ├── components/ui/           # shadcn-style Radix primitives
│   ├── layouts/                 # MainLayout (nav rail + titlebar)
│   └── features/                # panel · meetings · meeting-detail · settings
│
└── src-tauri/                   # Rust backend
    ├── src/
    │   ├── lib.rs               # app setup: plugins, state, tray, detection
    │   ├── main.rs              # thin launcher
    │   ├── error.rs / events.rs # typed errors + typed event emitters
    │   ├── state.rs             # in-memory AppState (RwLock/Mutex)
    │   ├── models/              # Meeting/Settings/Recorder (serde ⇄ TS types)
    │   ├── commands/            # thin Tauri commands
    │   ├── core/                # platform-INDEPENDENT domain logic
    │   │   ├── detection/       # detection loop (auto-record)
    │   │   ├── recorder/        # capture orchestration + finalize
    │   │   ├── transcription/   # Transcriber trait + simulated + whisper
    │   │   └── ai/              # Summarizer trait + heuristic
    │   ├── audio/               # cpal mic capture → WAV
    │   ├── platform/            # platform-SPECIFIC, isolated
    │   │   ├── detect.rs        # facade
    │   │   ├── screen.rs        # facade
    │   │   ├── windows/         # window titles, detect, screen capture
    │   │   └── macos/           # stubs for the next release
    │   └── tray.rs              # system tray
    ├── capabilities/            # Tauri v2 permissions
    └── tauri.conf.json          # two windows (main + panel), CSP, bundle
```

**Frontend ⇄ backend contract.** TS types in `src/types/index.ts` mirror the Rust
`models` (serde `rename_all = "camelCase"`). The UI only ever calls the typed
`api` façade and subscribes to typed events; the bridge routes to real Tauri IPC
or the mock, so the same UI runs in a browser and in the native shell.

**Adding macOS later.** Implement `platform::macos::detect_scan` (via
`CGWindowListCopyWindowInfo`) and a ScreenCaptureKit-based `screen::start`. Nothing
in `core/`, `commands/`, or the UI changes.

---

## Current scope & status

- ✅ **Frontend** — complete and verified live in a browser (all three screens,
  navigation, animations, and the full live-record → transcript → summarize flow
  via the mock backend).
- ✅ **Rust backend** — complete, modular, strongly typed: in-memory state,
  commands, events, detection loop, recorder orchestration, mic capture,
  simulated/whisper transcription, heuristic summarizer, and system tray.
- ✅ **Compilation** — builds and runs on Windows with Rust 1.96 + VS 2022 Build
  Tools. `npm run app:dev` launches the native window + system-tray icon.
- ⛔ **Out of scope (by request):** persistent database, cloud sync. State is
  in memory and resets on quit.

### Roadmap

- macOS support (detection + ScreenCaptureKit capture).
- Persistence (SQLite via `tauri-plugin-sql`) and optional cloud sync.
- Real diarization + speaker identification.
- Meeting-bot dispatch for “Send Bot”.
```
