// Whisper transcription engine

use std::path::Path;
use std::sync::Arc;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

/// A single transcribed segment with timestamp information.
#[derive(Clone, serde::Serialize)]
pub struct Segment {
    pub text: String,
    pub start: f64, // seconds
    pub end: f64,   // seconds
}

/// Wraps a loaded whisper-rs model for transcription.
pub struct Transcriber {
    ctx: Arc<WhisperContext>,
}

impl Transcriber {
    /// Load a Whisper model from the given GGML file path.
    pub fn new(model_path: &Path) -> Result<Self, String> {
        let path_str = model_path
            .to_str()
            .ok_or_else(|| "Model path contains invalid UTF-8".to_string())?;

        let params = WhisperContextParameters::default();
        let ctx = WhisperContext::new_with_params(path_str, params)
            .map_err(|e| format!("Failed to load Whisper model: {}", e))?;

        Ok(Self {
            ctx: Arc::new(ctx),
        })
    }

    /// Build default FullParams for decoding.
    fn default_params() -> FullParams<'static, 'static> {
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(None); // auto-detect
        params.set_no_speech_thold(0.3);
        params
    }

    /// Transcribe pre-processed 16 kHz mono f32 samples, returning the joined text.
    pub fn transcribe_samples(&self, samples: &[f32]) -> Result<String, String> {
        let mut state = self
            .ctx
            .create_state()
            .map_err(|e| format!("Failed to create whisper state: {}", e))?;

        let params = Self::default_params();
        state
            .full(params, samples)
            .map_err(|e| format!("Whisper inference failed: {}", e))?;

        let n = state
            .full_n_segments()
            .map_err(|e| format!("Failed to get segment count: {}", e))?;

        let mut text = String::new();
        for i in 0..n {
            let seg = state
                .full_get_segment_text(i)
                .map_err(|e| format!("Failed to get segment {} text: {}", i, e))?;
            text.push_str(&seg);
        }

        Ok(text.trim().to_string())
    }

    /// Read a WAV file and transcribe it, returning the joined text.
    pub fn transcribe_file(&self, path: &Path) -> Result<String, String> {
        let samples = read_wav_16khz(path)?;
        self.transcribe_samples(&samples)
    }

    /// Read a WAV file and transcribe it, returning individual segments with timestamps.
    pub fn transcribe_with_segments(&self, path: &Path) -> Result<Vec<Segment>, String> {
        let samples = read_wav_16khz(path)?;

        let mut state = self
            .ctx
            .create_state()
            .map_err(|e| format!("Failed to create whisper state: {}", e))?;

        let params = Self::default_params();
        state
            .full(params, &samples)
            .map_err(|e| format!("Whisper inference failed: {}", e))?;

        let n = state
            .full_n_segments()
            .map_err(|e| format!("Failed to get segment count: {}", e))?;

        let mut segments = Vec::with_capacity(n as usize);
        for i in 0..n {
            let text = state
                .full_get_segment_text(i)
                .map_err(|e| format!("Failed to get segment {} text: {}", i, e))?;
            let t0 = state
                .full_get_segment_t0(i)
                .map_err(|e| format!("Failed to get segment {} t0: {}", i, e))?;
            let t1 = state
                .full_get_segment_t1(i)
                .map_err(|e| format!("Failed to get segment {} t1: {}", i, e))?;

            segments.push(Segment {
                text: text.trim().to_string(),
                start: t0 as f64 / 100.0,
                end: t1 as f64 / 100.0,
            });
        }

        Ok(segments)
    }
}

/// Read a WAV file, convert to mono 16 kHz f32 samples.
pub fn read_wav_16khz(path: &Path) -> Result<Vec<f32>, String> {
    let reader =
        hound::WavReader::open(path).map_err(|e| format!("Failed to open WAV file: {}", e))?;

    let spec = reader.spec();
    let channels = spec.channels;
    let sample_rate = spec.sample_rate;

    // Read all samples as f32
    let raw_samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .into_samples::<f32>()
            .map(|s| s.map_err(|e| format!("Failed to read float sample: {}", e)))
            .collect::<Result<Vec<f32>, String>>()?,
        hound::SampleFormat::Int => {
            let bits = spec.bits_per_sample;
            let max_val = (1u32 << (bits - 1)) as f32;
            reader
                .into_samples::<i32>()
                .map(|s| {
                    s.map(|v| v as f32 / max_val)
                        .map_err(|e| format!("Failed to read int sample: {}", e))
                })
                .collect::<Result<Vec<f32>, String>>()?
        }
    };

    // Convert to mono by averaging channels
    let mono = if channels > 1 {
        let ch = channels as usize;
        raw_samples
            .chunks_exact(ch)
            .map(|frame| frame.iter().sum::<f32>() / channels as f32)
            .collect()
    } else {
        raw_samples
    };

    // Resample to 16 kHz if needed
    if sample_rate != 16_000 {
        Ok(crate::audio::resample(&mono, sample_rate, 16_000))
    } else {
        Ok(mono)
    }
}

/// Manages Small and Turbo Whisper model instances.
pub struct TranscriberManager {
    small: Option<Transcriber>,
    turbo: Option<Transcriber>,
}

impl TranscriberManager {
    /// Create an empty manager with no models loaded.
    pub fn new() -> Self {
        Self {
            small: None,
            turbo: None,
        }
    }

    /// Load a model by name ("small" or "turbo").
    pub fn load_model(&mut self, name: &str, path: &Path) -> Result<(), String> {
        let transcriber = Transcriber::new(path)?;
        match name {
            "small" => self.small = Some(transcriber),
            "turbo" => self.turbo = Some(transcriber),
            _ => return Err(format!("Unknown model name: {}", name)),
        }
        Ok(())
    }

    /// Transcribe streaming samples using the Small model (low latency).
    pub fn transcribe_streaming(&self, samples: &[f32]) -> Result<String, String> {
        self.small
            .as_ref()
            .ok_or_else(|| "Small model not loaded".to_string())?
            .transcribe_samples(samples)
    }

    /// Transcribe a WAV file using the Turbo model, falling back to Small.
    pub fn transcribe_final(&self, path: &Path) -> Result<String, String> {
        if let Some(turbo) = &self.turbo {
            return turbo.transcribe_file(path);
        }
        if let Some(small) = &self.small {
            return small.transcribe_file(path);
        }
        Err("No transcription model loaded".to_string())
    }

    /// Transcribe with segments using the Turbo model.
    pub fn transcribe_with_segments(&self, path: &Path) -> Result<Vec<Segment>, String> {
        self.turbo
            .as_ref()
            .ok_or_else(|| "Turbo model not loaded".to_string())?
            .transcribe_with_segments(path)
    }

    /// Check if the Small model is loaded and ready.
    pub fn is_small_ready(&self) -> bool {
        self.small.is_some()
    }

    /// Check if the Turbo model is loaded and ready.
    pub fn is_turbo_ready(&self) -> bool {
        self.turbo.is_some()
    }
}
