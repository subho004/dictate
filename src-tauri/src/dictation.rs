//! Orchestrates one hotkey-toggled dictation session: mic capture -> AssemblyAI ->
//! act on the result. Two flows, distinguished with no language-guessing:
//!
//! 1. Nothing selected when the hotkey was pressed -> plain dictation, paste verbatim.
//! 2. Text was selected when the hotkey was pressed -> the utterance is a command;
//!    transform the selection via Gemini and paste the result (which overwrites the
//!    still-live selection, same as a normal OS paste).

use crate::{assemblyai, audio, config::Config, gemini, paste};
use tokio::sync::{mpsc, Mutex};

struct Session {
    capture: audio::CaptureHandle,
    handle: tokio::task::JoinHandle<anyhow::Result<assemblyai::SessionResult>>,
    /// Text that was selected in the focused app when the hotkey was pressed, if any
    /// — determines whether the finished utterance is dictation or a command.
    selected_text: Option<String>,
}

pub struct AppState {
    session: Mutex<Option<Session>>,
    pub config: Config,
}

impl AppState {
    pub fn new(config: Config) -> Self {
        Self {
            session: Mutex::new(None),
            config,
        }
    }
}

/// Toggles the current dictation session: starts listening if idle (capturing
/// whatever's selected first), or stops, transcribes, and acts if a session is
/// already running.
///
/// `app` is used to dispatch clipboard/keystroke steps onto the main thread:
/// `arboard`/`enigo` call into macOS AppKit/CoreGraphics APIs, which can hard-abort
/// the whole process (not just panic — a native crash Rust's panic machinery can't
/// catch) if invoked off the main thread, which is what a plain `spawn_blocking` would do.
pub async fn toggle(state: &AppState, app: &tauri::AppHandle) {
    let mut guard = state.session.lock().await;

    if guard.is_some() {
        let session = guard.take().expect("checked is_some above");
        drop(session.capture); // stops the mic stream, closing the PCM channel

        match session.handle.await {
            Ok(Ok(result)) if !result.text.trim().is_empty() => {
                println!(
                    "[dictation] final transcript ({:?}): {:?}",
                    result.language_code, result.text
                );
                handle_result(state, app, result.text, result.language_code, session.selected_text).await;
            }
            Ok(Ok(_)) => println!("[dictation] empty transcript, nothing to do"),
            Ok(Err(e)) => eprintln!("[dictation] transcription session error: {e}"),
            Err(e) => eprintln!("[dictation] transcription task panicked: {e}"),
        }
    } else {
        let selected_text = match run_on_main_thread_sync(app, paste::capture_current_selection).await {
            Ok(sel) => sel,
            Err(e) => {
                eprintln!("[dictation] failed to check for a text selection, assuming none: {e}");
                None
            }
        };
        if let Some(sel) = &selected_text {
            println!("[dictation] selection detected ({} chars) — this will be a command", sel.len());
        }

        match start_session(&state.config, selected_text).await {
            Ok(session) => {
                *guard = Some(session);
                println!("[dictation] listening...");
            }
            Err(e) => eprintln!("[dictation] failed to start: {e}"),
        }
    }
}

async fn handle_result(
    state: &AppState,
    app: &tauri::AppHandle,
    transcript: String,
    instruction_language: Option<String>,
    selected_text: Option<String>,
) {
    let text_to_paste = match selected_text {
        Some(selected) => {
            match gemini::transform_selection(
                &state.config.gemini_api_key,
                &selected,
                &transcript,
                instruction_language.as_deref(),
            )
            .await
            {
                Ok(transformed) => transformed,
                Err(e) => {
                    eprintln!("[dictation] Gemini transform failed, leaving selection untouched: {e}");
                    return;
                }
            }
        }
        None => transcript,
    };

    let run_result = app.run_on_main_thread(move || match paste::paste_at_cursor(&text_to_paste) {
        Ok(()) => println!("[dictation] pasted"),
        Err(e) => eprintln!("[dictation] paste failed: {e}"),
    });
    if let Err(e) = run_result {
        eprintln!("[dictation] failed to schedule paste on main thread: {e}");
    }
}

/// Runs a synchronous closure on the Tauri main thread and awaits its output via a
/// oneshot channel — for the pre-listen selection check, which must happen before we
/// can decide how to interpret the recording that follows.
async fn run_on_main_thread_sync<T: Send + 'static>(
    app: &tauri::AppHandle,
    f: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
) -> anyhow::Result<T> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        let _ = tx.send(f());
    })?;
    rx.await.map_err(|_| anyhow::anyhow!("main-thread task dropped"))?
}

async fn start_session(config: &Config, selected_text: Option<String>) -> anyhow::Result<Session> {
    let (capture, pcm_rx) = audio::start_capture()?;
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<assemblyai::TranscriptEvent>();

    tokio::spawn(async move {
        while let Some(evt) = event_rx.recv().await {
            match evt {
                assemblyai::TranscriptEvent::Partial(t) => println!("[partial] {t}"),
                assemblyai::TranscriptEvent::FinalTurn(t) => println!("[turn]    {t}"),
            }
        }
    });

    let api_key = config.assemblyai_api_key.clone();
    let handle = tokio::spawn(async move { assemblyai::run_session(&api_key, pcm_rx, event_tx).await });

    Ok(Session {
        capture,
        handle,
        selected_text,
    })
}
