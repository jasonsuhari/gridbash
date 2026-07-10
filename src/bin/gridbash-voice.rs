#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("gridbash-voice is only available on Linux");
    std::process::exit(1);
}

#[cfg(target_os = "linux")]
fn main() -> anyhow::Result<()> {
    linux::run()
}

#[cfg(target_os = "linux")]
mod linux {
    use std::{
        path::PathBuf,
        sync::{Arc, Mutex},
        thread,
        time::Duration,
    };

    use anyhow::{Context, Result, anyhow, bail};
    use clap::Parser;
    use cpal::{
        FromSample, Sample, SampleFormat, SizedSample, Stream, StreamConfig,
        traits::{DeviceTrait, HostTrait, StreamTrait},
    };
    use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

    const TARGET_SAMPLE_RATE: u32 = 16_000;
    const MAX_CAPTURE_SECONDS: usize = 15;
    const END_SILENCE_SECONDS: f32 = 1.5;
    const SPEECH_THRESHOLD: f32 = 0.012;
    const NO_SPEECH_EXIT_CODE: i32 = 2;

    #[derive(Debug, Parser)]
    #[command(
        name = "gridbash-voice",
        version,
        about = "GridBash offline Linux dictation helper"
    )]
    struct Args {
        #[arg(long)]
        model: Option<PathBuf>,
    }

    #[derive(Debug)]
    struct CaptureState {
        mono: Vec<f32>,
        speech_started: bool,
        silent_frames: usize,
        sample_rate: u32,
        max_frames: usize,
        end_silence_frames: usize,
        done: bool,
        error: Option<String>,
    }

    impl CaptureState {
        fn new(sample_rate: u32) -> Self {
            Self {
                mono: Vec::with_capacity(sample_rate as usize * MAX_CAPTURE_SECONDS),
                speech_started: false,
                silent_frames: 0,
                sample_rate,
                max_frames: sample_rate as usize * MAX_CAPTURE_SECONDS,
                end_silence_frames: (sample_rate as f32 * END_SILENCE_SECONDS) as usize,
                done: false,
                error: None,
            }
        }

        fn push_frame(&mut self, sample: f32) {
            if self.done {
                return;
            }

            let amplitude = sample.abs();
            if amplitude >= SPEECH_THRESHOLD {
                self.speech_started = true;
                self.silent_frames = 0;
            } else if self.speech_started {
                self.silent_frames = self.silent_frames.saturating_add(1);
            }

            self.mono.push(sample.clamp(-1.0, 1.0));
            self.done = self.mono.len() >= self.max_frames
                || (self.speech_started && self.silent_frames >= self.end_silence_frames);
        }
    }

    pub fn run() -> Result<()> {
        let args = Args::parse();
        let model = args
            .model
            .or_else(|| std::env::var_os("GRIDBASH_VOICE_MODEL").map(PathBuf::from))
            .ok_or_else(|| anyhow!("pass --model or set GRIDBASH_VOICE_MODEL"))?;
        if !model.is_file() {
            bail!("voice model not found: {}", model.display());
        }

        let captured = capture_default_microphone()?;
        if !captured.speech_started {
            std::process::exit(NO_SPEECH_EXIT_CODE);
        }

        let audio = trim_and_resample(&captured.mono, captured.sample_rate, TARGET_SAMPLE_RATE);
        if audio.is_empty() {
            std::process::exit(NO_SPEECH_EXIT_CODE);
        }

        let transcript = transcribe(&model, &audio)?;
        if transcript.is_empty() {
            std::process::exit(NO_SPEECH_EXIT_CODE);
        }
        print!("{transcript}");
        Ok(())
    }

    fn capture_default_microphone() -> Result<CaptureState> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| anyhow!("no default microphone is available"))?;
        let supported = device
            .default_input_config()
            .context("failed to read the default microphone format")?;
        let sample_rate = supported.sample_rate();
        let channels = supported.channels() as usize;
        if channels == 0 {
            bail!("default microphone reported zero channels");
        }

        let state = Arc::new(Mutex::new(CaptureState::new(sample_rate)));
        let stream_config: StreamConfig = supported.clone().into();
        let stream = match supported.sample_format() {
            SampleFormat::I8 => build_stream::<i8>(&device, &stream_config, channels, &state)?,
            SampleFormat::I16 => build_stream::<i16>(&device, &stream_config, channels, &state)?,
            SampleFormat::I32 => build_stream::<i32>(&device, &stream_config, channels, &state)?,
            SampleFormat::I64 => build_stream::<i64>(&device, &stream_config, channels, &state)?,
            SampleFormat::U8 => build_stream::<u8>(&device, &stream_config, channels, &state)?,
            SampleFormat::U16 => build_stream::<u16>(&device, &stream_config, channels, &state)?,
            SampleFormat::U32 => build_stream::<u32>(&device, &stream_config, channels, &state)?,
            SampleFormat::U64 => build_stream::<u64>(&device, &stream_config, channels, &state)?,
            SampleFormat::F32 => build_stream::<f32>(&device, &stream_config, channels, &state)?,
            SampleFormat::F64 => build_stream::<f64>(&device, &stream_config, channels, &state)?,
            format => bail!("unsupported microphone sample format: {format}"),
        };

        stream
            .play()
            .context("failed to start microphone capture")?;
        loop {
            if state.lock().expect("capture state poisoned").done {
                break;
            }
            thread::sleep(Duration::from_millis(25));
        }
        drop(stream);

        let state = Arc::try_unwrap(state)
            .map_err(|_| anyhow!("microphone capture did not stop cleanly"))?
            .into_inner()
            .map_err(|_| anyhow!("microphone capture state was poisoned"))?;
        if let Some(error) = &state.error {
            bail!("microphone stream failed: {error}");
        }
        Ok(state)
    }

    fn build_stream<T>(
        device: &cpal::Device,
        config: &StreamConfig,
        channels: usize,
        state: &Arc<Mutex<CaptureState>>,
    ) -> Result<Stream>
    where
        T: Sample + SizedSample,
        f32: FromSample<T>,
    {
        let capture = Arc::clone(state);
        let error_capture = Arc::clone(state);
        device
            .build_input_stream(
                config,
                move |data: &[T], _| append_input(data, channels, &capture),
                move |error| {
                    if let Ok(mut state) = error_capture.lock() {
                        state.error = Some(error.to_string());
                        state.done = true;
                    }
                },
                None,
            )
            .context("failed to open the default microphone")
    }

    fn append_input<T>(data: &[T], channels: usize, state: &Arc<Mutex<CaptureState>>)
    where
        T: Sample,
        f32: FromSample<T>,
    {
        let Ok(mut state) = state.try_lock() else {
            return;
        };
        for frame in data.chunks_exact(channels) {
            let mono = frame.iter().copied().map(f32::from_sample).sum::<f32>() / channels as f32;
            state.push_frame(mono);
            if state.done {
                break;
            }
        }
    }

    fn trim_and_resample(input: &[f32], source_rate: u32, target_rate: u32) -> Vec<f32> {
        let start = input
            .iter()
            .position(|sample| sample.abs() >= SPEECH_THRESHOLD)
            .unwrap_or(0)
            .saturating_sub((source_rate / 5) as usize);
        let input = &input[start..];
        if input.is_empty() || source_rate == 0 || target_rate == 0 {
            return Vec::new();
        }
        if source_rate == target_rate {
            return input.to_vec();
        }

        let output_len = input.len().saturating_mul(target_rate as usize) / source_rate as usize;
        let ratio = source_rate as f64 / target_rate as f64;
        (0..output_len)
            .map(|index| {
                let source = index as f64 * ratio;
                let left = source.floor() as usize;
                let right = (left + 1).min(input.len() - 1);
                let blend = (source - left as f64) as f32;
                input[left] * (1.0 - blend) + input[right] * blend
            })
            .collect()
    }

    fn transcribe(model: &PathBuf, audio: &[f32]) -> Result<String> {
        let context = WhisperContext::new_with_params(model, WhisperContextParameters::default())
            .context("failed to load the offline voice model")?;
        let mut state = context
            .create_state()
            .context("failed to initialize offline voice recognition")?;
        let mut params = FullParams::new(SamplingStrategy::BeamSearch {
            beam_size: 5,
            patience: -1.0,
        });
        params.set_n_threads(
            thread::available_parallelism()
                .map(|count| count.get().min(8) as i32)
                .unwrap_or(2),
        );
        params.set_translate(false);
        params.set_language(None);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_no_context(true);

        state
            .full(params, audio)
            .context("offline voice transcription failed")?;
        let transcript = state
            .as_iter()
            .map(|segment| segment.to_string())
            .collect::<String>();
        Ok(transcript.split_whitespace().collect::<Vec<_>>().join(" "))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn capture_stops_after_ending_silence() {
            let mut state = CaptureState::new(100);
            for _ in 0..20 {
                state.push_frame(0.2);
            }
            for _ in 0..150 {
                state.push_frame(0.0);
            }
            assert!(state.speech_started);
            assert!(state.done);
        }

        #[test]
        fn resampling_preserves_duration() {
            let input = vec![0.2; 48_000];
            let output = trim_and_resample(&input, 48_000, 16_000);
            assert_eq!(output.len(), 16_000);
        }
    }
}
