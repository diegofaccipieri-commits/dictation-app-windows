use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;
use hound::{WavSpec, WavWriter};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

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
    worker: Option<JoinHandle<()>>,
    raw_buffer: Arc<Mutex<Vec<f32>>>,
    resampled_buffer: Arc<Mutex<Vec<f32>>>,
    recording: Arc<AtomicBool>,
}

impl AudioRecorder {
    pub fn new() -> Self {
        Self {
            worker: None,
            raw_buffer: Arc::new(Mutex::new(Vec::new())),
            resampled_buffer: Arc::new(Mutex::new(Vec::new())),
            recording: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn start(&mut self) -> Result<(), String> {
        if self.recording.load(Ordering::SeqCst) {
            return Ok(());
        }

        // Clear buffers
        self.raw_buffer.lock().unwrap().clear();
        self.resampled_buffer.lock().unwrap().clear();

        let raw_buf = Arc::clone(&self.raw_buffer);
        let resampled_buf = Arc::clone(&self.resampled_buffer);
        let recording = Arc::clone(&self.recording);
        let (ready_tx, ready_rx) = crossbeam_channel::bounded::<Result<(), String>>(1);
        self.recording.store(true, Ordering::SeqCst);

        self.worker = Some(std::thread::spawn(move || {
            let host = cpal::default_host();
            let device = match host.default_input_device() {
                Some(device) => device,
                None => {
                    let _ = ready_tx.send(Err("No default input device found".to_string()));
                    return;
                }
            };

            let config = match device.default_input_config() {
                Ok(config) => config,
                Err(e) => {
                    let _ =
                        ready_tx.send(Err(format!("Failed to get default input config: {}", e)));
                    return;
                }
            };

            let native_sample_rate = config.sample_rate().0;
            let native_channels = config.channels();
            let sample_format = config.sample_format();
            let err_fn = |err: cpal::StreamError| {
                eprintln!("Audio stream error: {}", err);
            };

            let stream_config: cpal::StreamConfig = config.into();
            let stream_result = match sample_format {
                SampleFormat::F32 => device.build_input_stream(
                    &stream_config,
                    {
                        let raw_buf_f32 = Arc::clone(&raw_buf);
                        let resampled_buf_f32 = Arc::clone(&resampled_buf);
                        let recording_f32 = Arc::clone(&recording);
                        move |data: &[f32], _: &cpal::InputCallbackInfo| {
                            if !recording_f32.load(Ordering::SeqCst) {
                                return;
                            }
                            raw_buf_f32.lock().unwrap().extend_from_slice(data);
                            let mono = to_mono(data, native_channels);
                            let resampled = resample(&mono, native_sample_rate, TARGET_SAMPLE_RATE);
                            resampled_buf_f32
                                .lock()
                                .unwrap()
                                .extend_from_slice(&resampled);
                        }
                    },
                    err_fn,
                    None,
                ),
                SampleFormat::I16 => device.build_input_stream(
                    &stream_config,
                    {
                        let raw_buf_i16 = Arc::clone(&raw_buf);
                        let resampled_buf_i16 = Arc::clone(&resampled_buf);
                        let recording_i16 = Arc::clone(&recording);
                        move |data: &[i16], _: &cpal::InputCallbackInfo| {
                            if !recording_i16.load(Ordering::SeqCst) {
                                return;
                            }

                            let float_data: Vec<f32> =
                                data.iter().map(|&s| s as f32 / i16::MAX as f32).collect();

                            raw_buf_i16.lock().unwrap().extend_from_slice(&float_data);
                            let mono = to_mono(&float_data, native_channels);
                            let resampled = resample(&mono, native_sample_rate, TARGET_SAMPLE_RATE);
                            resampled_buf_i16
                                .lock()
                                .unwrap()
                                .extend_from_slice(&resampled);
                        }
                    },
                    err_fn,
                    None,
                ),
                SampleFormat::U16 => device.build_input_stream(
                    &stream_config,
                    {
                        let raw_buf_u16 = Arc::clone(&raw_buf);
                        let resampled_buf_u16 = Arc::clone(&resampled_buf);
                        let recording_u16 = Arc::clone(&recording);
                        move |data: &[u16], _: &cpal::InputCallbackInfo| {
                            if !recording_u16.load(Ordering::SeqCst) {
                                return;
                            }

                            let float_data: Vec<f32> = data
                                .iter()
                                .map(|&s| (s as f32 / u16::MAX as f32) * 2.0 - 1.0)
                                .collect();

                            raw_buf_u16.lock().unwrap().extend_from_slice(&float_data);
                            let mono = to_mono(&float_data, native_channels);
                            let resampled = resample(&mono, native_sample_rate, TARGET_SAMPLE_RATE);
                            resampled_buf_u16
                                .lock()
                                .unwrap()
                                .extend_from_slice(&resampled);
                        }
                    },
                    err_fn,
                    None,
                ),
                _ => {
                    let _ = ready_tx.send(Err(format!(
                        "Unsupported sample format: {:?}",
                        sample_format
                    )));
                    return;
                }
            };

            let stream = match stream_result {
                Ok(stream) => stream,
                Err(e) => {
                    let _ = ready_tx.send(Err(format!("Failed to build input stream: {}", e)));
                    return;
                }
            };

            if let Err(e) = stream.play() {
                let _ = ready_tx.send(Err(format!("Failed to start audio stream: {}", e)));
                return;
            }

            let _ = ready_tx.send(Ok(()));

            while recording.load(Ordering::SeqCst) {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }

            drop(stream);
        }));

        match ready_rx.recv_timeout(std::time::Duration::from_secs(3)) {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                self.recording.store(false, Ordering::SeqCst);
                if let Some(worker) = self.worker.take() {
                    let _ = worker.join();
                }
                return Err(e);
            }
            Err(_) => {
                self.recording.store(false, Ordering::SeqCst);
                if let Some(worker) = self.worker.take() {
                    let _ = worker.join();
                }
                return Err("Timed out while starting audio stream".to_string());
            }
        }

        Ok(())
    }

    pub fn stop(&mut self) -> Result<PathBuf, String> {
        self.recording.store(false, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }

        let resampled = self.resampled_buffer.lock().unwrap().clone();

        if resampled.is_empty() {
            return Err("No audio samples captured".to_string());
        }

        // Write to WAV
        let filename = format!(
            "dictation_{}.wav",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
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
}
