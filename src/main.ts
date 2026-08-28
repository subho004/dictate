// Dictate is tray + global-hotkey driven — all orchestration (capture, transcription,
// command routing, paste) lives in the Rust backend (src-tauri/src). This window is a
// placeholder; it's hidden on startup (see src-tauri/src/lib.rs) and reserved for a
// future settings UI (hotkey rebinding, permission onboarding — see docs/implementation-plan.md).
