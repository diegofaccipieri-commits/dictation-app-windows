// Whisper transcription engine

use std::path::Path;
use std::sync::Arc;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

/// A single transcribed segment with timestamp information.
#[allow(dead_code)]
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
        let start = std::time::Instant::now();
        eprintln!("[whisper] Loading model from {:?}", model_path);

        let path_str = model_path
            .to_str()
            .ok_or_else(|| "Model path contains invalid UTF-8".to_string())?;

        let params = WhisperContextParameters::default();
        let ctx = WhisperContext::new_with_params(path_str, params)
            .map_err(|e| format!("Failed to load Whisper model: {}", e))?;

        eprintln!("[whisper] Model loaded in {:?}", start.elapsed());
        Ok(Self { ctx: Arc::new(ctx) })
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
        let start = std::time::Instant::now();
        eprintln!("[whisper] Starting transcription of {} samples ({:.1}s audio)", samples.len(), samples.len() as f32 / 16000.0);

        let mut state = self
            .ctx
            .create_state()
            .map_err(|e| format!("Failed to create whisper state: {}", e))?;
        eprintln!("[whisper] State created in {:?}", start.elapsed());

        let params = Self::default_params();
        let inference_start = std::time::Instant::now();
        state
            .full(params, samples)
            .map_err(|e| format!("Whisper inference failed: {}", e))?;
        eprintln!("[whisper] Inference completed in {:?}", inference_start.elapsed());

        let n = state.full_n_segments();

        let mut text = String::new();
        for i in 0..n {
            if let Some(seg) = state.get_segment(i) {
                let seg_text = seg
                    .to_str_lossy()
                    .map_err(|e| format!("Failed to get segment {} text: {}", i, e))?;
                text.push_str(seg_text.as_ref());
            }
        }

        Ok(text.trim().to_string())
    }

    /// Read a WAV file and transcribe it, returning the joined text.
    pub fn transcribe_file(&self, path: &Path) -> Result<String, String> {
        let samples = read_wav_16khz(path)?;
        self.transcribe_samples(&samples)
    }

    /// Read a WAV file and transcribe it, returning individual segments with timestamps.
    #[allow(dead_code)]
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

        let n = state.full_n_segments();

        let mut segments = Vec::with_capacity(n as usize);
        for i in 0..n {
            if let Some(seg) = state.get_segment(i) {
                let text = seg
                    .to_str_lossy()
                    .map_err(|e| format!("Failed to get segment {} text: {}", i, e))?;

                segments.push(Segment {
                    text: text.trim().to_string(),
                    start: seg.start_timestamp() as f64 / 100.0,
                    end: seg.end_timestamp() as f64 / 100.0,
                });
            }
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

/// Manages Tiny, Small and Turbo Whisper model instances.
pub struct TranscriberManager {
    tiny: Option<Transcriber>,
    small: Option<Transcriber>,
    turbo: Option<Transcriber>,
}

impl TranscriberManager {
    /// Create an empty manager with no models loaded.
    pub fn new() -> Self {
        Self {
            tiny: None,
            small: None,
            turbo: None,
        }
    }

    /// Load a model by name ("tiny", "small" or "turbo").
    pub fn load_model(&mut self, name: &str, path: &Path) -> Result<(), String> {
        let transcriber = Transcriber::new(path)?;
        match name {
            "tiny" => self.tiny = Some(transcriber),
            "small" => self.small = Some(transcriber),
            "turbo" => self.turbo = Some(transcriber),
            _ => return Err(format!("Unknown model name: {}", name)),
        }
        Ok(())
    }

    fn get_by_name(&self, name: &str) -> Option<&Transcriber> {
        match name {
            "tiny" => self.tiny.as_ref(),
            "small" => self.small.as_ref(),
            "turbo" => self.turbo.as_ref(),
            _ => None,
        }
    }

    /// Transcribe streaming samples using the preferred model, falling back to any loaded model.
    pub fn transcribe_streaming_with_model(
        &self,
        samples: &[f32],
        preferred: &str,
    ) -> Result<String, String> {
        if let Some(model) = self.get_by_name(preferred) {
            return model.transcribe_samples(samples);
        }

        if let Some(small) = &self.small {
            return small.transcribe_samples(samples);
        }
        if let Some(turbo) = &self.turbo {
            return turbo.transcribe_samples(samples);
        }

        Err("No transcription model loaded".to_string())
    }

    /// Transcribe a WAV file using the preferred model, falling back to any loaded model.
    pub fn transcribe_final_with_model(
        &self,
        path: &Path,
        preferred: &str,
    ) -> Result<String, String> {
        if let Some(model) = self.get_by_name(preferred) {
            return model.transcribe_file(path);
        }

        // Fallback: try tiny first (fastest), then small, then turbo
        if let Some(tiny) = &self.tiny {
            return tiny.transcribe_file(path);
        }
        if let Some(small) = &self.small {
            return small.transcribe_file(path);
        }
        if let Some(turbo) = &self.turbo {
            return turbo.transcribe_file(path);
        }

        Err("No transcription model loaded".to_string())
    }

    /// Transcribe with segments using the preferred model.
    #[allow(dead_code)]
    pub fn transcribe_with_segments_with_model(
        &self,
        path: &Path,
        preferred: &str,
    ) -> Result<Vec<Segment>, String> {
        if let Some(model) = self.get_by_name(preferred) {
            return model.transcribe_with_segments(path);
        }

        if let Some(turbo) = &self.turbo {
            return turbo.transcribe_with_segments(path);
        }
        if let Some(small) = &self.small {
            return small.transcribe_with_segments(path);
        }

        Err("No transcription model loaded".to_string())
    }

    /// Check if the Small model is loaded and ready.
    #[allow(dead_code)]
    pub fn is_small_ready(&self) -> bool {
        self.small.is_some()
    }

    /// Check if the Turbo model is loaded and ready.
    #[allow(dead_code)]
    pub fn is_turbo_ready(&self) -> bool {
        self.turbo.is_some()
    }
}

/// Transcribe a WAV file using the Vulkan-accelerated whisper-cli.exe
#[cfg(windows)]
pub fn transcribe_with_vulkan_cli(wav_path: &Path, model_path: &Path) -> Result<String, String> {
    use std::process::Command;
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x08000000;

    let exe_path = std::env::current_exe()
        .map_err(|e| format!("Failed to get exe path: {}", e))?;
    let exe_dir = exe_path
        .parent()
        .ok_or("Failed to get exe directory")?;

    // Try multiple locations for vulkan-bin
    let possible_paths = [
        exe_dir.join("vulkan-bin"),           // Development / same dir as exe
        exe_dir.join("resources").join("vulkan-bin"), // Tauri resources subfolder
        exe_dir.to_path_buf(),                // Files directly in exe dir (flat resources)
    ];

    let cli_path = possible_paths
        .iter()
        .map(|p| p.join("whisper-cli.exe"))
        .find(|p| p.exists())
        .ok_or_else(|| format!("whisper-cli.exe not found in any of: {:?}", possible_paths))?;

    let vulkan_dir = cli_path.parent().unwrap();

    eprintln!("[vulkan] Running whisper-cli from {:?}", cli_path);
    eprintln!("[vulkan] Model: {:?}", model_path);
    eprintln!("[vulkan] WAV: {:?}", wav_path);

    let output = Command::new(&cli_path)
        .arg("-m")
        .arg(model_path)
        .arg("-f")
        .arg(wav_path)
        .arg("-l")
        .arg("auto")
        .arg("-nt") // no timestamps in output
        .arg("-np") // no prints (cleaner output)
        .current_dir(vulkan_dir)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("Failed to run whisper-cli: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("whisper-cli failed: {}", stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let text = stdout.trim().to_string();

    eprintln!("[vulkan] Result: {}", text);
    Ok(text)
}

#[cfg(not(windows))]
pub fn transcribe_with_vulkan_cli(_wav_path: &Path, _model_path: &Path) -> Result<String, String> {
    Err("Vulkan CLI only available on Windows".to_string())
}
