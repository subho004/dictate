# Voice Dictation App — Implementation Plan

**Working name:** Dictate
**Platforms:** macOS + Windows (single Tauri v2 codebase)
**Core idea:** Press a user-defined global hotkey anywhere on the OS → speak → text is transcribed and pasted at the cursor. Select text first, then dictate, and the spoken utterance is instead treated as a command that transforms the selection via Gemini — no separate "command mode" or language guessing needed.

---

## 1. Product spec

### 1.1 Core flow (dictation)
1. User presses a global hotkey (toggle or push-to-talk, user's choice) from inside any app — editor, browser, Slack, terminal.
2. App shows a small floating indicator (or tray icon change) that it's listening.
3. Mic audio streams live to AssemblyAI; partial transcript can optionally show in the floating overlay.
4. User stops talking (or presses hotkey again to stop) → AssemblyAI signals end of turn → final transcript is captured.
5. Final transcript is written to the OS clipboard and auto-pasted at the current cursor position.
6. Clipboard is restored to its prior contents afterward (don't clobber the user's existing clipboard permanently).

### 1.2 Command flow (select, then dictate)
If the user selects text (mouse or keyboard) *before* pressing the hotkey, the app detects that selection at listen-start and treats the following utterance as an instruction about it instead of content to insert verbatim — e.g. select a paragraph, hotkey, say "make this more formal," hotkey. Gemini transforms the selected text per the instruction and the result is pasted, overwriting the still-live selection like a normal OS paste. See §4 and [intent-state-management.md](intent-state-management.md) for why this selection-driven approach replaced an earlier design that tried to guess intent from phrasing.

### 1.3 Non-goals (v1)
- No persistent cross-session memory or preference system (considered, deliberately dropped as unneeded complexity — see §4).
- No cloud sync of settings (local-only to start).
- No multi-user support.
- No offline/local transcription fallback (AssemblyAI-only for v1; local Whisper could be a v2 add-on).

---

## 2. Architecture overview

```
┌─────────────────────────────────────────────────────────────────┐
│                         Tauri v2 App                             │
│                                                                    │
│  Frontend (WebView: React/Svelte)     Rust Backend (core logic)  │
│  ─────────────────────────────       ───────────────────────────│
│  • Settings window                    • Global hotkey listener   │
│  • Floating "listening" overlay       • Audio capture (cpal)     │
│  • Onboarding / permission UX         • Resampler (rubato)       │
│  • Memory/profile editor              • AssemblyAI WS client     │
│                                        • Gemini REST client       │
│                                        • Command router           │
│                                        • Memory store (SQLite)    │
│                                        • Clipboard + paste (enigo)│
│                                        • OS keychain (API keys)   │
│                                        • System tray              │
└─────────────────────────────────────────────────────────────────┘
        │                          │                        │
        ▼                          ▼                        ▼
  Microphone (cpal)      AssemblyAI Universal-Streaming   Gemini API
  CoreAudio / WASAPI      wss://streaming.assemblyai.com   generativelanguage
                                  /v3/ws                    .googleapis.com
```

All orchestration (hotkey → capture → stream → interpret → paste) lives in the **Rust backend**, not the webview, so the app keeps working even with the window closed/hidden — the webview is only for settings/onboarding/UI, not in the critical path of dictation.

---

## 3. Tech stack

| Concern | Choice | Why |
|---|---|---|
| App shell | **Tauri v2** | Small binary, native perf, single Rust core across macOS/Windows |
| Global hotkey | `tauri-plugin-global-shortcut` | Official plugin; runtime register/unregister supports user rebinding |
| Mic capture | `cpal` | Cross-platform (CoreAudio/WASAPI), the de facto Rust standard |
| Resampling | `rubato` (`FftFixedIn`) or `fixed-resample` | Device native rate → 16kHz mono PCM16 required by AssemblyAI, off the audio thread |
| STT | **AssemblyAI Universal-Streaming (v3)**, multilingual Pro model | Sub-300ms partials, built-in end-of-turn detection, automatic language detection + mid-sentence code-switching across 18 languages, $0.45/hr |
| WS client | `tokio-tungstenite` | Standard async Rust WebSocket |
| LLM (selection commands) | **Gemini 3.7-flash** via REST (`reqwest`) | Confirmed current model; `thinking_level: low` for fast selection transforms |
| Secrets (API keys) | OS keychain via `keyring` crate (+ `tauri-plugin-store` for non-secret settings) | Never store AssemblyAI/Gemini keys in plaintext JSON |
| Paste + selection capture | `arboard` (clipboard) + `enigo` (simulate Cmd+C/Cmd+V), dispatched via Tauri's `run_on_main_thread` | Must run on the main thread — off-thread AppKit/CoreGraphics calls can hard-abort the process, discovered during implementation |
| macOS permissions | `tauri-plugin-macos-permissions` + Info.plist/entitlements | Wraps mic + Accessibility + Input Monitoring checks/requests |
| Tray | `tauri::tray::TrayIconBuilder` | Listening/idle icon states, quick menu |
| Frontend framework | React or Svelte (either fine — Svelte for a lighter bundle) | Settings UI, overlay, onboarding only — not on the critical audio path |

---

## 4. Command detection: selection-driven, not language-guessed

The app distinguishes plain dictation from a voice command using the OS's own text-selection state, not by guessing from phrasing. Full rationale and the (larger, since-simplified) design this replaced live in **[intent-state-management.md](intent-state-management.md)**. Summary:

```
Hotkey pressed (listen start)
        │
        ▼
  Capture current selection (simulate copy, diff clipboard before/after)
        │
        ├─ nothing selected ──────────────────────────► plain dictation mode
        └─ text selected ─────────────────────────────► command mode (selection is the target)
        │
        ▼
  ... record audio, transcribe via AssemblyAI ...
        │
        ▼
  plain dictation mode → paste the transcript verbatim, no LLM call
  command mode         → Gemini 3.7-flash (thinking_level=low) transforms the selected
                          text per the spoken instruction → paste the result, which
                          overwrites the still-live selection like a normal OS paste
```

No regex pre-filter, no cross-session "what did I last paste" tracking, no accessibility-API text relocation — selecting the text *is* how the user tells the app what to act on, so there's nothing to infer. This also means editing something works whether the user is revising the app's own last dictation or any other text already in the document.

A persistent "remember that I prefer..." preference system was considered and deliberately dropped for now as unnecessary complexity — the two-mode design above covers the actual use case cleanly.

---

## 5. Permissions & platform specifics

### macOS
- `NSMicrophoneUsageDescription` in Info.plist.
- Accessibility permission required for `enigo`-simulated paste; check via `AXIsProcessTrusted()`, guide user to System Settings → Privacy & Security → Accessibility if not granted.
- `NSAccessibilityUsageDescription` string + `Entitlements.plist` referenced from `tauri.conf.json` (hardened runtime, needed for notarization).
- **Gotcha (confirmed via cpal issues):** mic permission prompts can behave inconsistently once the app is code-signed vs. dev build — test the signed/notarized build specifically before release, not just `tauri dev`.
- Re-signing the app invalidates a previously granted Accessibility permission — keep signing identity stable across builds during testing.

### Windows
- No OS permission prompt typically required for `SendInput`-based paste simulation or mic capture.
- `SendInput` may be blocked by UAC-elevated target windows (e.g. an admin-elevated terminal) — document this as a known limitation, not a bug to chase in v1.

### Both
- First-run onboarding flow should: request mic permission, request Accessibility (macOS only), let the user set the hotkey, and do a one-shot test dictation before declaring "ready."

---

## 6. External APIs — key facts

### AssemblyAI Universal-Streaming (v3)
- `wss://streaming.assemblyai.com/v3/ws?sample_rate=16000&format_turns=true&speech_model=universal-3-5-pro&language_detection=true` (EU region: `streaming.eu.assemblyai.com`). The `speech_model`/`language_detection` params are what enable multilingual auto-detection — omitting them falls back to the cheaper English-only default model.
- Auth: raw API key in `Authorization` header (use short-lived temporary tokens if the key ever needs to touch a less-trusted context).
- Audio: mono 16-bit PCM, little-endian, 16kHz, sent as small binary frames — AssemblyAI rejects frames outside a 50–1000ms duration window (error 3007), so frames must be batched to that range regardless of how small/large the chunks are internally (see [audio.rs](../src-tauri/src/audio.rs) `SEND_CHUNK_BYTES`).
- Key message types:
  - `Begin` — session started, includes `expires_at`.
  - `Turn` — `{ transcript, end_of_turn, turn_order, turn_is_formatted, language_code, language_confidence, words: [...] }`. `end_of_turn: true` is the built-in VAD/turn-detection signal — this is what triggers "user stopped talking" without any custom silence-detection logic. `language_code`/`language_confidence` are only present with `language_detection=true` (e.g. `"es"`, `0.998`), confirmed working end-to-end against real synthesized Spanish audio.
  - `Termination` — session end summary.
- `format_turns=true` gets you a punctuated/cased second pass per turn (`turn_is_formatted: true`).
- **Trailing silence matters**: if the client stops sending audio the instant the user finishes speaking (no natural pause), the server's VAD never observes speech ending and `end_of_turn` never fires — the app feeds ~1.2s of synthetic silence before closing to force finalization (see `assemblyai.rs` `send_trailing_silence`).
- `keyterms_prompt` param biases recognition toward domain-specific vocabulary (e.g. product names, jargon) — worth exposing as a user setting.
- Sessions auto-close after 3 hours; billed by connection duration.
- **Pricing:** the multilingual Pro model used here (`universal-3-5-pro`) is **$0.45/hr**; the plain English-only `Universal-Streaming` model is **$0.15/hr** (~$0.0025/min) if multilingual support isn't needed. Close the WebSocket promptly when not listening — idle open connections still bill.
- **Multilingual coverage**: 18 languages with mid-sentence code-switching (e.g. Hinglish) at flagship accuracy on the Pro model.

### Gemini 3.7-flash
- Model ID: `gemini-3.7-flash` (confirmed current, part of the Gemini 3 family; 1,048,576 input / 65,536 output token context).
- Thinking control: `thinking_level` = `"low" | "medium" | "high"` (default `medium`). **Does not** support `"minimal"` — that's `gemini-3.6-flash`/`3.5-flash-lite` only. Do not also pass a token-based `thinking_budget` — the two are mutually exclusive and combining them 400s.
- Use `low` for reformat/cleanup calls (latency-sensitive), `medium` for command/intent parsing (needs more reasoning to disambiguate).
- Structured output (JSON mode) and function calling are both supported — use these for command parsing rather than parsing free text.
- Thinking tokens are billed as output tokens (`total_thought_tokens` exposed in the usage object) — factor this into cost, especially at `medium`.
- **Pricing (intro, through 2026-12-31):** $0.75 / 1M input tokens, $3.75 / 1M output tokens (rising to $1.50/$7.50 from 2027-01-01).
- No official Rust SDK — call the REST API directly with `reqwest`; unofficial crates exist (`google-generative-ai-rs`) but a fast-moving API surface makes direct REST calls the safer bet.
- Note for later: **Gemini Live API** (currently `gemini-3.1-flash-live`) offers a single bidirectional real-time audio socket with built-in transcription — a possible v2 consolidation of the AssemblyAI + Gemini split into one connection, once the command/memory layer is proven out on the simpler two-service architecture.

---

## 7. Security & secrets
- AssemblyAI key and Gemini key entered once in Settings, stored via the OS keychain (`keyring` crate — Keychain on macOS, Credential Manager on Windows). Never written to a plaintext config file.
- Non-secret settings (hotkey binding, active profile, reformat-on/off) can use `tauri-plugin-store` (plain JSON) since there's nothing sensitive there.
- Clipboard: always save the user's prior clipboard contents before writing the transcript, and restore it a short delay after paste — avoid silently discarding whatever they had copied.

---

## 8. Milestones

1. **Skeleton app** — Tauri v2 project, system tray, settings window shell, hotkey registration (no-op action), macOS/Windows permission onboarding flow.
2. **Audio pipeline** — cpal capture → resample to 16kHz mono PCM16 → confirm correct format with a local WAV dump before touching the network.
3. **STT integration** — WebSocket to AssemblyAI, stream audio, log partial/final turns to console; get `end_of_turn` driving the "stop listening" transition.
4. **Paste pipeline** — clipboard write + enigo paste simulation, working reliably across a test matrix of apps (VS Code, Notes/Notepad, Slack, browser address bar, terminal) on both OSes.
5. **End-to-end dictation** — hotkey → capture → transcribe → paste. Done and verified live: mic capture, AssemblyAI streaming (including a fix for the audio-chunk-duration minimum and a trailing-silence pad so `end_of_turn` actually fires), and paste-at-cursor (moved onto the main thread after an off-thread AppKit call was found to hard-crash the process).
6. **Selection-driven commands** — done: capture the OS selection at listen-start (simulate copy, diff clipboard), and if present, route the utterance through `gemini::transform_selection` instead of pasting verbatim. See [intent-state-management.md](intent-state-management.md) for why this replaced an earlier language-guessing design.
7. **Polish** — floating listening indicator with live partial-transcript preview, error states (no mic, permission revoked mid-session, network drop), cost/usage display, custom vocabulary (`keyterms_prompt`) setting, customizable hotkey (currently a fixed default).
8. **Packaging** — macOS notarization + signing, Windows code signing, auto-update channel.

---

## 9. Open questions / risks
- **Selection false-negative**: if the clipboard already happened to hold exactly the selected text, the app can't detect the selection changed and falls back to plain dictation — rare, harmless, but worth knowing about.
- **macOS signed-build mic permission flakiness** (per cpal issue reports) — needs explicit test pass on a notarized build, not just dev builds. Also confirmed during dev: an IDE's embedded terminal (e.g. Cursor) may never surface the mic permission prompt at all since it lacks its own usage-description declaration — run from a real Terminal.app if that happens.
- **Windows elevated-window paste failures** — document as known limitation; revisit with a lower-level driver approach only if it proves to be a frequent real-world complaint.
- **Cost at scale**: both APIs are usage-billed (AssemblyAI per connection-hour, Gemini per token including thinking tokens) — worth a simple local usage counter in Settings so the user isn't surprised by their bill.

---

## 10. Sources
- AssemblyAI Universal-Streaming docs (`streaming.assemblyai.com`, `/docs/speech-to-text/universal-streaming`)
- Tauri v2 docs: [global-shortcut](https://v2.tauri.app/plugin/global-shortcut/), [clipboard-manager](https://v2.tauri.app/plugin/clipboard/), [system tray](https://v2.tauri.app/learn/system-tray/), [macOS bundle/entitlements](https://v2.tauri.app/distribute/macos-application-bundle/)
- [enigo](https://docs.rs/enigo/latest/enigo/), [tauri-plugin-macos-permissions](https://github.com/ayangweb/tauri-plugin-macos-permissions), [cpal](https://github.com/RustAudio/cpal)
- [voicetypr](https://github.com/moinulmoin/voicetypr) — comparable open-source Tauri dictation app, used as an architecture reference
- Gemini API docs: [models](https://ai.google.dev/gemini-api/docs/models), [thinking](https://ai.google.dev/gemini-api/docs/thinking), [pricing](https://ai.google.dev/gemini-api/docs/pricing), [Live API](https://ai.google.dev/gemini-api/docs/live-api)
