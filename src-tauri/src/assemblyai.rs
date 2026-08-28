//! AssemblyAI Universal-Streaming (v3) client, multilingual mode.
//! wss://streaming.assemblyai.com/v3/ws — mono 16-bit PCM LE @ 16kHz, `Turn` messages
//! carry `end_of_turn` as the built-in "user stopped talking" signal. `speech_model=
//! universal-3-5-pro` + `language_detection=true` enables automatic language
//! detection and mid-sentence code-switching across AssemblyAI's supported languages
//! (see docs/implementation-plan.md §6) — note this is the Pro tier, priced at
//! $0.45/hr rather than the $0.15/hr English-only model.

use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message;

const WS_URL: &str = "wss://streaming.assemblyai.com/v3/ws?sample_rate=16000&format_turns=true&speech_model=universal-3-5-pro&language_detection=true";

#[derive(Deserialize, Debug)]
#[serde(tag = "type")]
enum ServerMessage {
    Begin {
        #[allow(dead_code)]
        id: Option<String>,
    },
    Turn {
        transcript: String,
        end_of_turn: bool,
        #[serde(default)]
        turn_is_formatted: bool,
        #[serde(default)]
        language_code: Option<String>,
        #[serde(default)]
        language_confidence: Option<f64>,
    },
    Termination {
        #[allow(dead_code)]
        audio_duration_seconds: Option<f64>,
    },
    #[serde(other)]
    Unknown,
}

/// Events surfaced to the caller while a session is running.
pub enum TranscriptEvent {
    Partial(String),
    /// A formatted final turn (punctuated/cased) — the best text to act on.
    FinalTurn(String),
}

/// What a session produced once the caller stops recording: the last formatted final
/// turn, and the language AssemblyAI detected it was spoken in (e.g. "en", "es") —
/// used downstream so Gemini can preserve that language in its output rather than
/// defaulting to English. `None` if detection never returned a result (e.g. no
/// speech, or the connection ended before a formatted turn arrived).
#[derive(Default)]
pub struct SessionResult {
    pub text: String,
    pub language_code: Option<String>,
}

/// Streams `pcm_rx` (raw PCM16 LE mono 16kHz frames) to AssemblyAI and forwards
/// transcript events to `event_tx` until `pcm_rx` closes (caller stops sending audio)
/// or the connection ends.
pub async fn run_session(
    api_key: &str,
    mut pcm_rx: mpsc::UnboundedReceiver<Vec<u8>>,
    event_tx: mpsc::UnboundedSender<TranscriptEvent>,
) -> anyhow::Result<SessionResult> {
    let mut request = WS_URL.into_client_request()?;
    request
        .headers_mut()
        .insert("Authorization", HeaderValue::from_str(api_key)?);

    let (ws_stream, _resp) = tokio_tungstenite::connect_async(request).await?;
    eprintln!("[assemblyai] websocket connected");
    let (mut write, mut read) = ws_stream.split();

    let mut result = SessionResult::default();
    let mut audio_done = false;
    let mut bytes_sent: u64 = 0;
    let mut chunks_sent: u64 = 0;

    // Phase 1: forward audio while also draining incoming transcript events.
    while !audio_done {
        tokio::select! {
            audio = pcm_rx.recv() => {
                match audio {
                    Some(bytes) => {
                        bytes_sent += bytes.len() as u64;
                        chunks_sent += 1;
                        if chunks_sent % 20 == 0 {
                            eprintln!("[assemblyai] sent {chunks_sent} chunks, {bytes_sent} bytes total");
                        }
                        if let Err(e) = write.send(Message::Binary(bytes.into())).await {
                            eprintln!("[assemblyai] failed to send audio frame: {e}");
                            audio_done = true;
                        }
                    }
                    None => {
                        eprintln!("[assemblyai] audio ended: sent {chunks_sent} chunks, {bytes_sent} bytes total");
                        // The caller stopped the instant the user finished speaking. Relying
                        // on AssemblyAI's silence-based VAD to notice speech ended (via
                        // trailing silence padding) turned out unreliable — its threshold
                        // varies enough by language/confidence that some turns (observed
                        // with Hindi) never finalized even after 1.2s of injected silence.
                        // ForceEndpoint is the explicit, authoritative alternative: it tells
                        // the server directly to finalize the current turn now. Crucially,
                        // do NOT send Close in the same breath — that raced the server in
                        // testing, tearing the connection down before it could respond to
                        // ForceEndpoint at all. Close is sent only after Phase 2 below has
                        // had a chance to actually read that response.
                        let _ = write.send(Message::Text(r#"{"type":"ForceEndpoint"}"#.into())).await;
                        audio_done = true;
                    }
                }
            }
            msg = read.next() => {
                if !handle_message(msg, &mut result, &event_tx) {
                    return Ok(result);
                }
            }
        }
    }

    // Phase 2: audio is done, but the server is still finalizing the last turn in
    // response to ForceEndpoint — keep reading until it closes/terminates or times out.
    let drain = async {
        loop {
            let msg = read.next().await;
            if !handle_message(msg, &mut result, &event_tx) {
                break;
            }
        }
    };
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), drain).await;

    let _ = write.send(Message::Close(None)).await;
    Ok(result)
}

/// Returns false when the connection is finished (closed/errored/terminated).
fn handle_message(
    msg: Option<Result<Message, tokio_tungstenite::tungstenite::Error>>,
    result: &mut SessionResult,
    event_tx: &mpsc::UnboundedSender<TranscriptEvent>,
) -> bool {
    match msg {
        Some(Ok(Message::Text(text))) => {
            if let Ok(parsed) = serde_json::from_str::<ServerMessage>(&text) {
                match parsed {
                    ServerMessage::Turn {
                        transcript,
                        end_of_turn,
                        turn_is_formatted,
                        language_code,
                        language_confidence,
                    } => {
                        eprintln!(
                            "[assemblyai] turn: end_of_turn={end_of_turn} formatted={turn_is_formatted} \
                             lang={language_code:?} confidence={language_confidence:?} text={transcript:?}"
                        );
                        if end_of_turn && turn_is_formatted {
                            result.text = transcript.clone();
                            result.language_code = language_code;
                            let _ = event_tx.send(TranscriptEvent::FinalTurn(transcript));
                        } else if !end_of_turn {
                            let _ = event_tx.send(TranscriptEvent::Partial(transcript));
                        }
                    }
                    ServerMessage::Termination { .. } => {
                        eprintln!("[assemblyai] termination received");
                        return false;
                    }
                    ServerMessage::Begin { id } => {
                        eprintln!("[assemblyai] session begin: {id:?}");
                    }
                    ServerMessage::Unknown => {
                        eprintln!("[assemblyai] unrecognized message: {text}");
                    }
                }
            }
            true
        }
        Some(Ok(Message::Close(_))) | None => false,
        Some(Err(e)) => {
            eprintln!("[assemblyai] websocket error: {e}");
            false
        }
        _ => true,
    }
}
