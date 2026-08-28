//! Automated end-to-end test of the AssemblyAI streaming pipeline WITHOUT a live human
//! voice: reads a 16-bit PCM WAV file (e.g. synthesized via `say -o test.aiff "..."`,
//! then converted with `afconvert -f WAVE -d LEI16@16000 -c 1`), replays it as if it
//! were mic audio (paced in real time), and prints whatever AssemblyAI transcribes.
//!
//! Usage: cargo run --example test_stt_from_file -- /tmp/test_speech_16k.wav

use dictate_lib::{assemblyai, config};
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let path = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("usage: test_stt_from_file <path-to-16k-mono-pcm16-wav>"))?;

    let cfg = config::Config::load()?;

    let mut reader = hound::WavReader::open(&path)?;
    let spec = reader.spec();
    anyhow::ensure!(
        spec.sample_rate == 16_000 && spec.channels == 1 && spec.bits_per_sample == 16,
        "expected 16kHz mono 16-bit PCM WAV, got {spec:?}"
    );
    let samples: Vec<i16> = reader.samples::<i16>().collect::<Result<_, _>>()?;
    println!("[test] loaded {} samples ({:.2}s) from {path}", samples.len(), samples.len() as f64 / 16000.0);

    let (pcm_tx, pcm_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<assemblyai::TranscriptEvent>();

    tokio::spawn(async move {
        while let Some(evt) = event_rx.recv().await {
            match evt {
                assemblyai::TranscriptEvent::Partial(t) => println!("[partial] {t}"),
                assemblyai::TranscriptEvent::FinalTurn(t) => println!("[FINAL]   {t}"),
            }
        }
    });

    let api_key = cfg.assemblyai_api_key.clone();
    let ws_task = tokio::spawn(async move { assemblyai::run_session(&api_key, pcm_rx, event_tx).await });

    // Feed audio in ~100ms chunks, paced in real time, mimicking a live mic stream.
    let chunk_frames = 1600; // 100ms @ 16kHz
    for chunk in samples.chunks(chunk_frames) {
        let mut bytes = Vec::with_capacity(chunk.len() * 2);
        for s in chunk {
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        if pcm_tx.send(bytes).is_err() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    drop(pcm_tx); // signal end of audio

    let result = ws_task.await??;
    println!(
        "\n[test] session final transcript ({:?}): {:?}",
        result.language_code, result.text
    );
    anyhow::ensure!(!result.text.trim().is_empty(), "expected a non-empty transcript");
    println!("[test] PASS");
    Ok(())
}
