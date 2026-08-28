# Command Detection: Selection-Driven, Not Language-Guessed

Companion to [implementation-plan.md](implementation-plan.md) §4. Earlier drafts of this doc tried to distinguish "plain dictation" from "a command referring to something already pasted" by guessing from phrasing (regex pre-filter + an LLM classify call) and, for edits, replaying keystrokes to relocate text the app itself had pasted in a *previous* hotkey session. That approach was replaced — simpler and far more reliable — by using the OS's own text-selection state as the signal instead of inferring intent from language at all.

## The rule

- **Nothing selected when the hotkey is pressed** → plain dictation. The utterance is pasted verbatim at the cursor. No LLM call.
- **Text is selected when the hotkey is pressed** → the utterance is a command about that selection (e.g. select a paragraph, hotkey, say "make this more formal"). Gemini transforms the selected text per the spoken instruction, and the result is pasted — which overwrites the still-live selection, exactly like a normal OS paste.

There is no separate "edit something I pasted three minutes ago" flow. If the user wants to revise something, they select it (mouse or keyboard) and speak a command about it — the same mechanism whether that text came from the app's own last dictation or was already sitting in the document. This also sidesteps the entire cross-session state problem the original design wrestled with: no `LastInsertion` record, no validity window, no re-locating text in a foreign app after the fact.

## How selection is detected

At the moment the hotkey is pressed to *start* listening (before any audio capture begins), the app:

1. Reads the current clipboard contents.
2. Simulates Cmd+C / Ctrl+C.
3. Reads the clipboard again. If it changed to something non-empty, that's the selection; if unchanged, there was nothing selected.
4. Restores the original clipboard shortly after (this is only a read — the app doesn't want to leave the user's clipboard polluted with whatever it just captured).

This runs on the Tauri main thread (see below) and takes well under 100ms, before the mic starts capturing.

Known limitation: if the clipboard already happened to contain exactly the text that's selected, the app can't tell the difference and treats it as no selection. Harmless in practice — it just means that specific case falls back to plain dictation.

## Why this must run on the main thread

`arboard` (clipboard) and `enigo` (simulated keystrokes) call into macOS AppKit/CoreGraphics APIs. Calling them off the main thread doesn't just risk a catchable Rust panic — it can hard-abort the whole process (a native crash outside Rust's panic machinery), which is exactly what happened during implementation when the paste step ran inside a `tokio::spawn_blocking` closure. The fix was routing every clipboard/keystroke operation through Tauri's `AppHandle::run_on_main_thread`, dispatched from the async orchestration code via a oneshot channel to get the result back.

## What got dropped from the original design

- **Tier-0 regex pre-filter for edit language** ("make", "change", "that", "it", etc.) — no longer needed since selection state is the signal, not phrasing.
- **`LastInsertion` tracking** (text, char count, frontmost app, timestamp) and its validity window — replaced by the OS's live selection.
- **Accessibility-API / key-simulation text relocation** ("select back exactly what I inserted last time, verify via read-back, then overwrite") — no longer needed; if there's something to edit, it's already selected.
- **"Remember that I prefer..." persistent preferences** — dropped for now as unnecessary complexity; may return later as a deliberate feature, not as part of intent detection.
- **The four-way Gemini classify call** (`dictate` / `mixed` / `edit_previous` / `set_preference` with structured output) — replaced by a single-purpose `transform_selection` call that only runs when there's actually a selection to act on.

## Trailing-silence note

Unrelated to intent detection, but discovered and fixed alongside it: AssemblyAI's turn detection (`end_of_turn`) is driven by silence in the audio stream. If the app stops sending audio the instant the user presses the hotkey to stop — with no trailing pause — the server never gets a chance to observe the speech ending and `end_of_turn` never fires, so the transcript comes back empty. The fix: before closing the WebSocket, the app feeds ~1.2s of synthetic silence so the server's VAD has something to detect the pause against.
