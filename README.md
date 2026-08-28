<div align="center">

# Dictate

**Voice dictation and voice-driven editing, anywhere on your desktop.**

Press a global hotkey, speak, and your words are transcribed and pasted at the cursor — in any app, on macOS or Windows. Select text first and dictate a command instead ("make this more formal") to edit it in place with AI, no separate "edit mode" required.

[![Tauri](https://img.shields.io/badge/Tauri-v2-24C8DB?logo=tauri&logoColor=white)](https://tauri.app)
[![Rust](https://img.shields.io/badge/Rust-stable-DEA584?logo=rust&logoColor=white)](https://www.rust-lang.org)
[![TypeScript](https://img.shields.io/badge/TypeScript-5-3178C6?logo=typescript&logoColor=white)](https://www.typescriptlang.org)
[![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Windows-lightgrey)](#getting-started)
[![AssemblyAI](https://img.shields.io/badge/STT-AssemblyAI-1E1E2E)](https://www.assemblyai.com)
[![Gemini](https://img.shields.io/badge/LLM-Gemini%203.7%20Flash-4285F4?logo=googlegemini&logoColor=white)](https://ai.google.dev)
[![Status](https://img.shields.io/badge/status-done-brightgreen)](#additional-features)

</div>

---


https://github.com/user-attachments/assets/3b5a7a7f-5417-46cb-94d3-a2c6fe2e218f


## Contents

- [How it works](#how-it-works)
- [Features](#features)
- [Tech stack](#tech-stack)
- [Getting started](#getting-started)
  - [Prerequisites](#prerequisites)
  - [macOS setup](#macos-setup)
  - [Windows setup](#windows-setup)
  - [Environment variables](#environment-variables)
- [Running](#running)
- [Testing](#testing)
- [Building for production](#building-for-production)
- [Project structure](#project-structure)
- [Documentation](#documentation)
- [Additional features](#additional-features)
- [License](#license)

## How it works

1. Press a global hotkey (default `Cmd/Ctrl+Shift+Space`) from inside any app.
2. Speak — audio streams live to [AssemblyAI](https://www.assemblyai.com) for real-time, multilingual transcription.
3. Press the hotkey again to stop.
4. **Nothing selected?** The transcript is pasted verbatim at your cursor.
5. **Text selected before you pressed the hotkey?** Your utterance is treated as an instruction — [Gemini](https://ai.google.dev) transforms the selected text accordingly and the result replaces it.

No separate "command mode," no keyword triggers — the OS's own text selection is the signal. See [`docs/intent-state-management.md`](docs/intent-state-management.md) for the full design rationale.

## Features

- 🌍 **Multilingual, auto-detected** — dictate in any of AssemblyAI's supported languages; no configuration needed.
- ✍️ **Select-to-edit** — highlight text anywhere, dictate a command, and it's transformed in place.
- 🌐 **Language-preserving edits** — commands transform selected text without translating it, even if you speak the command in a different language.
- ⚡ **Fast path for plain dictation** — no LLM call at all unless there's a selection to act on.
- 🖥️ **System-wide** — works in any app with a text cursor: editors, browsers, chat apps, terminals.
- 🔒 **Local-first secrets** — API keys are read from a local `.env`, never bundled or transmitted anywhere but the two APIs above.

## Tech stack

| Layer | Choice |
|---|---|
| App shell | [Tauri v2](https://tauri.app) (Rust backend + minimal TypeScript frontend) |
| Global hotkey | `tauri-plugin-global-shortcut` |
| Audio capture | [`cpal`](https://github.com/RustAudio/cpal) + [`rubato`](https://github.com/HEnquist/rubato) (resampling) |
| Speech-to-text | AssemblyAI Universal-Streaming (multilingual, real-time) |
| Command transform | Gemini 3.7 Flash |
| Paste / selection | [`enigo`](https://github.com/enigo-rs/enigo) + [`arboard`](https://github.com/1Password/arboard) |

Full architecture and design decisions: [`docs/implementation-plan.md`](docs/implementation-plan.md).

## Getting started

### Prerequisites

| Tool | Notes |
|---|---|
| [Rust](https://www.rust-lang.org/tools/install) (via `rustup`) | Stable toolchain |
| [Node.js](https://nodejs.org) 18+ | npm ships with it |
| API keys | [AssemblyAI](https://www.assemblyai.com/dashboard) + [Gemini](https://aistudio.google.com/apikey) — both have free tiers |

### macOS setup

```bash
# Xcode Command Line Tools (if you don't already have them)
xcode-select --install

# Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
# or: brew install rustup-init && rustup-init

# Node (if needed)
brew install node
```

Microphone and Accessibility permissions are requested by macOS the first time the app tries to use them — grant both (System Settings → Privacy & Security). Run the dev server from a real Terminal window, not an IDE's embedded terminal — some embedded terminals never surface the microphone permission prompt at all.

### Windows setup

```powershell
# Rust
winget install Rustlang.Rustup
# or download from https://www.rust-lang.org/tools/install

# Node (if needed)
winget install OpenJS.NodeJS.LTS

# Tauri's Windows prerequisites (WebView2 runtime + MSVC Build Tools)
# https://tauri.app/start/prerequisites/#windows
```

WebView2 ships with Windows 11 and most up-to-date Windows 10 installs; if missing, Tauri's prerequisites page above has the installer link.

### Environment variables

```bash
cp .env.example .env
# then edit .env and fill in your keys
```

## Running

Clone, install dependencies, and launch the dev build — same commands on both platforms once prerequisites are installed:

```bash
git clone <this-repo-url>
cd dictate
npm install
npm run tauri dev
```

<details>
<summary>Windows (PowerShell) equivalent</summary>

```powershell
git clone <this-repo-url>
cd dictate
npm install
npm run tauri dev
```

</details>

The app has no window — it lives in the system tray and the global hotkey. Watch the terminal for live logs (audio device, transcription turns, paste/command results).

## Testing

The AssemblyAI streaming pipeline can be exercised end-to-end without a live microphone, using synthesized speech:

```bash
# macOS (uses the built-in `say` command)
say -o /tmp/test_speech.aiff "This is a test of the dictation pipeline"
afconvert -f WAVE -d LEI16@16000 -c 1 /tmp/test_speech.aiff /tmp/test_speech_16k.wav
cd src-tauri
cargo run --example test_stt_from_file -- /tmp/test_speech_16k.wav
```

On Windows, generate an equivalent 16kHz mono 16-bit PCM WAV file by any means (e.g. `System.Speech` in PowerShell, or any TTS tool) and pass it to the same example.

## Building for production

```bash
npm run tauri build
```

Produces a native installer/bundle for the platform you run it on (`.app`/`.dmg` on macOS, `.msi`/`.exe` on Windows). Code signing and notarization are not yet configured — see [Additional features](#additional-features).

## Project structure

```
dictate/
├── docs/                        # Architecture and design docs
│   ├── implementation-plan.md
│   └── intent-state-management.md
├── src/                         # Frontend (minimal — this app is tray/hotkey-driven)
├── src-tauri/
│   ├── src/
│   │   ├── audio.rs             # Mic capture + resampling to 16kHz mono PCM16
│   │   ├── assemblyai.rs        # Real-time multilingual STT streaming client
│   │   ├── gemini.rs            # Selected-text transform via Gemini
│   │   ├── paste.rs             # Selection capture + paste-at-cursor
│   │   ├── dictation.rs         # Orchestration: hotkey → capture → route → act
│   │   ├── config.rs            # .env loading
│   │   └── lib.rs               # Tauri setup: tray, hotkey, app state
│   └── examples/
│       └── test_stt_from_file.rs  # STT pipeline test, no mic required
├── .env.example
└── README.md
```

## Documentation

- [`docs/implementation-plan.md`](docs/implementation-plan.md) — full architecture, tech stack rationale, external API details, milestones.
- [`docs/intent-state-management.md`](docs/intent-state-management.md) — why command detection is selection-driven rather than language-guessed.

## Additional features

The core app — system-wide dictation and select-to-edit — is complete and working. These are optional enhancements on top of it, not blockers:

- Customizable hotkey (currently a fixed default)
- Floating listening indicator with live partial-transcript preview
- macOS code signing + notarization, Windows code signing
- Auto-update channel
- Custom vocabulary / domain terms (`keyterms_prompt`)

## License

No license is currently granted — all rights reserved. This repository is shared for visibility, not for reuse or redistribution.
