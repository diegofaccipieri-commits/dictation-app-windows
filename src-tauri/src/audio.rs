use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream};
use hound::{WavSpec, WavWriter};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

const TARGET_SAMPLE_RATE: u32 = 16_000;

/// Resample audio using linear interpolation.
pub fn resample(samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == to_rate {
        return samples.to_vec();
    }
    let ratio = from_rate as f64 / to_rate as f64;
    let out_len = (samples.len() as f64 / ratio) as usize;
    (0..out_len)
        .map(|i| {
            let src = i as f64 * ratio;
            let idx = src as usize;
            let frac = (src - idx as f64) as f32;
            let a = samples.get(idx).copied().unwrap_or(0.0);
            let b = samples.get(idx + 1).copied().unwrap_or(a);
            a + frac * (b - a)
        })
        .collect()
}

/// Convert interleaved multi-channel samples to mono by averaging channels.
fn to_mono(samples: &[f32], channels: u16) -> Vec<f32> {
    if channels <= 1 {
        return samples.to_vec();
    }
    let ch = channels as usize;
    samples
        .chunks_exact(ch)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

pub struct AudioRecorder {
    stream: Option<Stream>,
    raw_buffer: Arc<Mutex<Vec<f32>>>,
    resampled_buffer: Arc<Mutex<Vec<f32>>>,
    recording: Arc<Mutex<bool>>,
    sample_rate: Arc<Mutex<u32>>,
    channels: Arc<Mutex<u16>>,
}

impl AudioRecorder {
    pub fn new() -> Self {
        Self {
            stream: None,
            raw_buffer: Arc::new(Mutex::new(Vec::new())),
            resampled_buffer: Arc::new(Mutex::new(Vec::new())),
            recording: Arc::new(Mutex::new(false)),
            sample_rate: Arc::new(Mutex::new(TARGET_SAMPLE_RATE)),
            channels: Arc::new(Mutex::new(1)),
        }
    }

    pub fn start(&mut self) -> Result<(), String> {
        // Clear buffers
        self.raw_buffer.lock().unwrap().clear();
        self.resampled_buffer.lock().unwrap().clear();

        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| "No default input device found".to_string())?;

        let config = device
            .default_input_config()
            .map_err(|e| format!("Failed to get default input config: {}", e))?;

        let native_sample_rate = config.sample_rate().0;
        let native_channels = config.channels();
        let sample_format = config.sample_format();

        *self.sample_rate.lock().unwrap() = native_sample_rate;
        *self.channels.lock().unwrap() = native_channels;

        let raw_buf = Arc::clone(&self.raw_buffer);
        let resampled_buf = Arc::clone(&self.resampled_buffer);
        let recording = Arc::clone(&self.recording);
        let sr = native_sample_rate;
        let ch = native_channels;

        let err_fn = |err: cpal::StreamError| {
            eprintln!("Audio stream error: {}", err);
        };

        let stream_config: cpal::StreamConfig = config.into();

        let stream = match sample_format {
            SampleFormat::F32 => device
                .build_input_stream(
                    &stream_config,
                    move |data: &[f32], _: &cpal::InputCallbackInfo| {
                        if !*recording.lock().unwrap() {
                            return;
                        }
                        // Store raw samples
                        raw_buf.lock().unwrap().extend_from_slice(data);

                        // Convert to mono, resample, and accumulate
                        let mono = to_mono(data, ch);
                        let resampled = resample(&mono, sr, TARGET_SAMPLE_RATE);
                        resampled_buf.lock().unwrap().extend_from_slice(&resampled);
                    },
                    err_fn,
                    None,
                )
                .map_err(|e| format!("Failed to build f32 input stream: {}", e))?,
            SampleFormat::I16 => {
                let raw_buf_i16 = Arc::clone(&self.raw_buffer);
                let resampled_buf_i16 = Arc::clone(&self.resampled_buffer);
                let recording_i16 = Arc::clone(&self.recording);
                device
                    .build_input_stream(
                        &stream_config,
                        move |data: &[i16], _: &cpal::InputCallbackInfo| {
                            if !*recording_i16.lock().unwrap() {
                                return;
                            }
                            // Convert i16 to f32
                            let float_data: Vec<f32> = data
                                .iter()
                                .map(|&s| s as f32 / i16::MAX as f32)
                                .collect();

                            raw_buf_i16.lock().unwrap().extend_from_slice(&float_data);

                            let mono = to_mono(&float_data, ch);
                            let resampled = resample(&mono, sr, TARGET_SAMPLE_RATE);
                            resampled_buf_i16
                                .lock()
                                .unwrap()
                                .extend_from_slice(&resampled);
                        },
                        err_fn,
                        None,
                    )
                    .map_err(|e| format!("Failed to build i16 input stream: {}", e))?
            }
            _ => return Err(format!("Unsupported sample format: {:?}", sample_format)),
        };

        stream
            .play()
            .map_err(|e| format!("Failed to start audio stream: {}", e))?;

        *self.recording.lock().unwrap() = true;
        self.stream = Some(stream);

        Ok(())
    }

    pub fn stop(&mut self) -> Result<PathBuf, String> {
        *self.recording.lock().unwrap() = false;

        // Drop the stream to stop capture
        self.stream.take();

        let resampled = self.resampled_buffer.lock().unwrap().clone();

        if resampled.is_empty() {
            return Err("No audio samples captured".to_string());
        }

        // Write to WAV
        let filename = format!("dictation_{}.wav", std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis());
        let path = std::env::temp_dir().join(filename);

        let spec = WavSpec {
            channels: 1,
            sample_rate: TARGET_SAMPLE_RATE,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };

        let mut writer = WavWriter::create(&path, spec)
            .map_err(|e| format!("Failed to create WAV file: {}", e))?;

        for &sample in &resampled {
            writer
                .write_sample(sample)
                .map_err(|e| format!("Failed to write WAV sample: {}", e))?;
        }

        writer
            .finalize()
            .map_err(|e| format!("Failed to finalize WAV file: {}", e))?;

        Ok(path)
    }

    /// Returns the resampled 16kHz mono samples accumulated so far and the sample rate.
    pub fn current_samples(&self) -> (Vec<f32>, f64) {
        let samples = self.resampled_buffer.lock().unwrap().clone();
        (samples, TARGET_SAMPLE_RATE as f64)
    }

    pub fn is_recording(&self) -> bool {
        *self.recording.lock().unwrap()
    }
}
