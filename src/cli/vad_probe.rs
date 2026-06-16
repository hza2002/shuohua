use anyhow::{anyhow, Context, Result};
use clap::Args;
use std::path::PathBuf;

#[derive(Debug, Args)]
pub struct VadProbeArgs {
    /// 16kHz mono s16le WAV file to evaluate.
    #[arg(long)]
    pub wav: PathBuf,
    /// Silero speech probability threshold.
    #[arg(long, default_value_t = 0.5)]
    pub threshold: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProbeEvent {
    pub speech: bool,
    pub start_ms: u64,
    pub end_ms: u64,
    pub probability: f32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeSummary {
    pub frames: usize,
    pub speech_frames: usize,
    pub speech_windows: usize,
    pub speech_ms: u64,
}

pub fn run(args: VadProbeArgs) -> Result<()> {
    if !(0.0..=1.0).contains(&args.threshold) {
        return Err(anyhow!("--threshold must be between 0.0 and 1.0"));
    }
    let events = run_silero(&args.wav, args.threshold)?;
    let summary = summarize_events(&events);
    print_summary(&summary, &events);
    Ok(())
}

fn run_silero(wav: &PathBuf, threshold: f32) -> Result<Vec<ProbeEvent>> {
    let samples = load_probe_wav(wav)?;
    let mut vad = voice_activity_detector::VoiceActivityDetector::builder()
        .sample_rate(16_000)
        .chunk_size(SILERO_CHUNK_SAMPLES)
        .build()
        .map_err(|e| anyhow!("create Silero VAD: {e}"))?;

    let mut events = Vec::new();
    for (index, chunk) in samples.chunks(SILERO_CHUNK_SAMPLES).enumerate() {
        let probability = vad.predict(chunk.iter().copied());
        let start_ms = samples_to_ms((index * SILERO_CHUNK_SAMPLES) as u64);
        let end_ms = samples_to_ms(((index * SILERO_CHUNK_SAMPLES) + chunk.len()) as u64);
        events.push(ProbeEvent {
            speech: probability >= threshold,
            start_ms,
            end_ms,
            probability,
        });
    }
    Ok(events)
}

fn summarize_events(events: &[ProbeEvent]) -> ProbeSummary {
    let mut speech_windows = 0;
    let mut prev_speech = false;
    let mut speech_ms = 0;
    let mut speech_frames = 0;

    for event in events {
        if event.speech {
            speech_frames += 1;
            speech_ms += event.end_ms.saturating_sub(event.start_ms);
            if !prev_speech {
                speech_windows += 1;
            }
        }
        prev_speech = event.speech;
    }

    ProbeSummary {
        frames: events.len(),
        speech_frames,
        speech_windows,
        speech_ms,
    }
}

fn print_summary(summary: &ProbeSummary, events: &[ProbeEvent]) {
    println!("backend=silero");
    println!("frames={}", summary.frames);
    println!("speech_frames={}", summary.speech_frames);
    println!("speech_windows={}", summary.speech_windows);
    println!("speech_ms={}", summary.speech_ms);
    for event in events {
        println!(
            "{}..{}ms speech={} probability={:.4}",
            event.start_ms, event.end_ms, event.speech, event.probability
        );
    }
}

const PROBE_SAMPLE_RATE: u32 = 16_000;
const SILERO_CHUNK_SAMPLES: usize = 512;

fn load_probe_wav(path: &PathBuf) -> Result<Vec<i16>> {
    let mut reader =
        hound::WavReader::open(path).with_context(|| format!("open wav {}", path.display()))?;
    let spec = reader.spec();
    if spec.channels != 1
        || spec.sample_rate != PROBE_SAMPLE_RATE
        || spec.bits_per_sample != 16
        || spec.sample_format != hound::SampleFormat::Int
    {
        return Err(anyhow!(
            "vad-probe expects 16kHz mono s16le WAV, got {}Hz {}ch {}bit {:?}",
            spec.sample_rate,
            spec.channels,
            spec.bits_per_sample,
            spec.sample_format
        ));
    }
    reader
        .samples::<i16>()
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("read wav samples")
}

fn samples_to_ms(samples: u64) -> u64 {
    samples.saturating_mul(1000) / PROBE_SAMPLE_RATE as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarize_events_counts_speech_windows() {
        let summary = summarize_events(&[
            ProbeEvent {
                speech: false,
                start_ms: 0,
                end_ms: 32,
                probability: 0.1,
            },
            ProbeEvent {
                speech: true,
                start_ms: 32,
                end_ms: 64,
                probability: 0.8,
            },
            ProbeEvent {
                speech: true,
                start_ms: 64,
                end_ms: 96,
                probability: 0.7,
            },
            ProbeEvent {
                speech: false,
                start_ms: 96,
                end_ms: 128,
                probability: 0.2,
            },
        ]);

        assert_eq!(summary.frames, 4);
        assert_eq!(summary.speech_frames, 2);
        assert_eq!(summary.speech_windows, 1);
        assert_eq!(summary.speech_ms, 64);
    }
}
