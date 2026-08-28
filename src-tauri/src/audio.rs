//! Microphone capture -> mono 16kHz PCM16 little-endian bytes, ready for AssemblyAI.
//!
//! Real-time-safety note: the cpal callback only pushes raw f32 samples into an
//! unbounded channel. All downmixing/resampling happens on a separate task so the
//! audio thread never blocks or allocates heavily.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, StreamConfig};
use rubato::{Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction};
use tokio::sync::mpsc;

pub const TARGET_SAMPLE_RATE: u32 = 16_000;
const RESAMPLE_CHUNK: usize = 1024;

/// A live capture session. Dropping (or calling `stop`) tears down the input stream.
pub struct CaptureHandle {
    _stream: cpal::Stream, // kept alive for the duration of the recording
}

// cpal::Stream is not Send on some platforms' backends; we only ever touch it from the
// thread that created it and drop it there, so this is safe for our usage pattern.
unsafe impl Send for CaptureHandle {}

/// Starts capturing from the default input device and returns:
/// - a handle that must be kept alive (and dropped/stopped when done)
/// - a channel of raw little-endian PCM16 mono 16kHz frames, ready to send to AssemblyAI
pub fn start_capture() -> anyhow::Result<(CaptureHandle, mpsc::UnboundedReceiver<Vec<u8>>)> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| anyhow::anyhow!("no default input (microphone) device found"))?;

    let supported = device.default_input_config()?;
    let sample_format = supported.sample_format();
    let device_channels = supported.channels() as usize;
    let device_rate: u32 = supported.sample_rate().into();

    eprintln!(
        "[audio] device={device} format={sample_format:?} channels={device_channels} rate={device_rate}"
    );

    let config: StreamConfig = supported.into();

    // Raw f32 mono samples at device_rate, pushed straight from the audio callback.
    let (raw_tx, raw_rx) = mpsc::unbounded_channel::<Vec<f32>>();
    // Final PCM16 16kHz mono byte frames, consumed by the AssemblyAI writer task.
    let (pcm_tx, pcm_rx) = mpsc::unbounded_channel::<Vec<u8>>();

    let err_fn = |err| eprintln!("[audio] stream error: {err}");

    // Debug instrumentation: prove the callback is firing at all and show real signal
    // level, so "empty transcript" can be diagnosed (no callback vs. silent input).
    let debug_calls = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let debug_peak_bits = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    {
        let debug_calls = debug_calls.clone();
        let debug_peak_bits = debug_peak_bits.clone();
        tokio::spawn(async move {
            use std::sync::atomic::Ordering;
            for _ in 0..20 {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                let calls = debug_calls.load(Ordering::Relaxed);
                let peak = f32::from_bits(debug_peak_bits.swap(0, Ordering::Relaxed));
                eprintln!("[audio] callback_calls={calls} peak_level={peak:.4}");
            }
        });
    }

    let stream = match sample_format {
        SampleFormat::F32 => device.build_input_stream(
            config.clone(),
            move |data: &[f32], _| {
                use std::sync::atomic::Ordering;
                debug_calls.fetch_add(1, Ordering::Relaxed);
                let peak = data.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
                debug_peak_bits.fetch_max(peak.to_bits(), Ordering::Relaxed);
                let mono = downmix_to_mono(data, device_channels);
                let _ = raw_tx.send(mono);
            },
            err_fn,
            None,
        )?,
        SampleFormat::I16 => device.build_input_stream(
            config.clone(),
            move |data: &[i16], _| {
                use std::sync::atomic::Ordering;
                debug_calls.fetch_add(1, Ordering::Relaxed);
                let peak = data.iter().fold(0i16, |a, &b| a.max(b.abs()));
                debug_peak_bits.fetch_max((peak as f32 / i16::MAX as f32).to_bits(), Ordering::Relaxed);
                let f32_data: Vec<f32> = data.iter().map(|s| *s as f32 / i16::MAX as f32).collect();
                let mono = downmix_to_mono(&f32_data, device_channels);
                let _ = raw_tx.send(mono);
            },
            err_fn,
            None,
        )?,
        SampleFormat::U16 => device.build_input_stream(
            config.clone(),
            move |data: &[u16], _| {
                use std::sync::atomic::Ordering;
                debug_calls.fetch_add(1, Ordering::Relaxed);
                let f32_data: Vec<f32> = data
                    .iter()
                    .map(|s| (*s as f32 - u16::MAX as f32 / 2.0) / (u16::MAX as f32 / 2.0))
                    .collect();
                let peak = f32_data.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
                debug_peak_bits.fetch_max(peak.to_bits(), Ordering::Relaxed);
                let mono = downmix_to_mono(&f32_data, device_channels);
                let _ = raw_tx.send(mono);
            },
            err_fn,
            None,
        )?,
        other => return Err(anyhow::anyhow!("unsupported sample format: {other:?}")),
    };

    stream.play()?;

    // Resampling task: device_rate mono f32 -> 16kHz mono PCM16 bytes.
    tokio::spawn(resample_task(raw_rx, pcm_tx, device_rate));

    Ok((CaptureHandle { _stream: stream }, pcm_rx))
}

fn downmix_to_mono(data: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return data.to_vec();
    }
    data.chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

// AssemblyAI rejects binary audio frames outside this duration range (error 3007,
// "Input Duration Violation") — each frame we send over the WebSocket must represent
// between MIN_SEND_MS and MAX_SEND_MS of 16kHz mono 16-bit audio, regardless of how
// small/large the chunks cpal/rubato happen to hand us internally.
const MIN_SEND_MS: usize = 100;
const BYTES_PER_MS: usize = (TARGET_SAMPLE_RATE as usize / 1000) * 2; // mono, 16-bit
const SEND_CHUNK_BYTES: usize = MIN_SEND_MS * BYTES_PER_MS;

async fn resample_task(
    mut raw_rx: mpsc::UnboundedReceiver<Vec<f32>>,
    pcm_tx: mpsc::UnboundedSender<Vec<u8>>,
    device_rate: u32,
) {
    let mut out_buffer: Vec<u8> = Vec::with_capacity(SEND_CHUNK_BYTES * 2);

    macro_rules! flush_ready {
        () => {
            while out_buffer.len() >= SEND_CHUNK_BYTES {
                let send_bytes: Vec<u8> = out_buffer.drain(..SEND_CHUNK_BYTES).collect();
                if pcm_tx.send(send_bytes).is_err() {
                    return;
                }
            }
        };
    }

    if device_rate == TARGET_SAMPLE_RATE {
        // Fast path: device already gives us 16kHz, just convert to i16 bytes and batch.
        while let Some(chunk) = raw_rx.recv().await {
            out_buffer.extend_from_slice(&f32_to_pcm16_bytes(&chunk));
            flush_ready!();
        }
    } else {
        let params = SincInterpolationParameters {
            sinc_len: 256,
            f_cutoff: 0.95,
            interpolation: SincInterpolationType::Linear,
            oversampling_factor: 256,
            window: WindowFunction::BlackmanHarris2,
        };
        let ratio = TARGET_SAMPLE_RATE as f64 / device_rate as f64;
        let mut resampler = match SincFixedIn::<f32>::new(ratio, 2.0, params, RESAMPLE_CHUNK, 1) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("[audio] failed to build resampler: {e}");
                return;
            }
        };

        let mut in_buffer: Vec<f32> = Vec::with_capacity(RESAMPLE_CHUNK * 2);

        while let Some(chunk) = raw_rx.recv().await {
            in_buffer.extend_from_slice(&chunk);

            while in_buffer.len() >= RESAMPLE_CHUNK {
                let input_chunk: Vec<f32> = in_buffer.drain(..RESAMPLE_CHUNK).collect();
                match resampler.process(&[input_chunk], None) {
                    Ok(output) => {
                        out_buffer.extend_from_slice(&f32_to_pcm16_bytes(&output[0]));
                        flush_ready!();
                    }
                    Err(e) => eprintln!("[audio] resample error: {e}"),
                }
            }
        }
    }

    // Flush whatever's left when capture stops. AssemblyAI still requires >=50ms per
    // frame, so pad a short trailing remainder with silence rather than send a runt frame.
    if !out_buffer.is_empty() {
        let min_bytes = 50 * BYTES_PER_MS;
        if out_buffer.len() < min_bytes {
            out_buffer.resize(min_bytes, 0);
        }
        let _ = pcm_tx.send(out_buffer);
    }
}

fn f32_to_pcm16_bytes(samples: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(samples.len() * 2);
    for &s in samples {
        let clamped = s.clamp(-1.0, 1.0);
        let i16_sample = (clamped * i16::MAX as f32) as i16;
        bytes.extend_from_slice(&i16_sample.to_le_bytes());
    }
    bytes
}
