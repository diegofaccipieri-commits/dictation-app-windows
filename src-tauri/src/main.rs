#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
#[allow(dead_code)]
mod batch;
mod models;
mod paste;
mod settings;
mod text_cleaner;
mod transcriber;
mod translator;

#[cfg(windows)]
mod hotkey;

use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    webview::WebviewWindowBuilder,
    AppHandle, Emitter, Manager,
};

const STATE_IDLE: u8 = 0;
const STATE_RECORDING: u8 = 1;
const STATE_TRANSCRIBING: u8 = 2;

const HOTKEY_MODE_CTRL: u8 = 0;
const HOTKEY_MODE_WIN: u8 = 1;

fn state_label(v: u8) -> &'static str {
    match v {
        STATE_RECORDING => "recording",
        STATE_TRANSCRIBING => "transcribing",
        _ => "idle",
    }
}

fn hotkey_mode_from_settings(value: &str) -> u8 {
    if value == settings::ACTIVATION_WIN {
        HOTKEY_MODE_WIN
    } else {
        HOTKEY_MODE_CTRL
    }
}

struct AppState {
    recorder: Mutex<audio::AudioRecorder>,
    transcriber_manager: Mutex<transcriber::TranscriberManager>,
    recording_state: AtomicU8,
    transcribed_text: Mutex<String>,
    history: Mutex<Vec<String>>,
    live_model: Mutex<String>,
    batch_model: Mutex<String>,
    translation_mode: Mutex<String>,
    activation_key: Mutex<String>,
    activation_mode: Arc<AtomicU8>,
    streaming_active: Arc<AtomicBool>,
    hotkey_recording: Arc<AtomicBool>,
    transcription_busy: Arc<AtomicBool>,
}

#[derive(Clone, Serialize)]
struct DownloadProgress {
    name: String,
    downloaded: u64,
    total: u64,
}

#[derive(Clone, Serialize)]
struct StreamingUpdate {
    text: String,
}

#[derive(Clone, Serialize)]
struct TranscriptionComplete {
    original: String,
    final_text: String,
    translated: bool,
}

#[derive(Clone, Serialize)]
struct StateChange {
    state: String,
}

#[derive(Clone, Serialize)]
struct BatchProgress {
    file: String,
    text: String,
    index: usize,
    total: usize,
    done: bool,
    error: Option<String>,
}

#[derive(Clone, Serialize)]
struct ErrorEvent {
    message: String,
}

#[tauri::command]
fn check_models() -> std::collections::HashMap<String, bool> {
    let tiny_path = models::model_path("tiny");
    let small_path = models::model_path("small");
    let turbo_path = models::model_path("turbo");
    let tiny_exists = tiny_path.exists();
    let small_exists = small_path.exists();
    let turbo_exists = turbo_path.exists();

    log_debug(&format!("tiny path: {:?}, exists: {}", tiny_path, tiny_exists));
    log_debug(&format!("small path: {:?}, exists: {}", small_path, small_exists));
    log_debug(&format!("turbo path: {:?}, exists: {}", turbo_path, turbo_exists));

    let mut map = std::collections::HashMap::new();
    map.insert("tiny".to_string(), tiny_exists);
    map.insert("small".to_string(), small_exists);
    map.insert("turbo".to_string(), turbo_exists);
    map
}

#[tauri::command]
fn download_model_cmd(name: String, app: AppHandle) {
    let app2 = app.clone();
    let app3 = app.clone();
    let model_name = name.clone();
    let model_name2 = name.clone();

    eprintln!("[download] Starting download for model: {}", name);

    // Emit initial progress
    let _ = app.emit(
        "download-progress",
        DownloadProgress {
            name: name.clone(),
            downloaded: 0,
            total: 1,
        },
    );

    std::thread::spawn(move || {
        eprintln!("[download] Thread started for model: {}", model_name);

        let result = models::download_model(&model_name, |downloaded, total| {
            eprintln!("[download] Progress {}: {}/{}", model_name, downloaded, total);
            let _ = app2.emit(
                "download-progress",
                DownloadProgress {
                    name: model_name.clone(),
                    downloaded,
                    total,
                },
            );
        });

        match &result {
            Ok(path) => eprintln!("[download] Success: {:?}", path),
            Err(e) => {
                eprintln!("[download] Error: {}", e);
                emit_error(
                    &app3,
                    &format!("Erro ao baixar modelo {}: {}", model_name2, e),
                );
            }
        }
    });
}

#[tauri::command]
fn load_models(app: AppHandle) {
    let app_clone = app.clone();
    std::thread::spawn(move || {
        log_debug("load_models thread started");

        // Load tiny first (fastest, best for real-time)
        if models::is_downloaded("tiny") {
            log_debug("loading tiny model...");
            let path = models::model_path("tiny");
            let state = app_clone.state::<AppState>();
            let mut mgr = state.transcriber_manager.lock().unwrap();
            match mgr.load_model("tiny", &path) {
                Ok(()) => log_debug("tiny model loaded successfully"),
                Err(e) => {
                    log_debug(&format!("tiny model load FAILED: {}", e));
                    emit_error(&app_clone, &format!("Falha ao carregar Tiny: {}", e));
                }
            }
        }

        if models::is_downloaded("small") {
            log_debug("loading small model...");
            let path = models::model_path("small");
            let state = app_clone.state::<AppState>();
            let mut mgr = state.transcriber_manager.lock().unwrap();
            match mgr.load_model("small", &path) {
                Ok(()) => log_debug("small model loaded successfully"),
                Err(e) => {
                    log_debug(&format!("small model load FAILED: {}", e));
                    emit_error(&app_clone, &format!("Falha ao carregar Small: {}", e));
                }
            }
        }

        if models::is_downloaded("turbo") {
            log_debug("loading turbo model...");
            let path = models::model_path("turbo");
            let state = app_clone.state::<AppState>();
            let mut mgr = state.transcriber_manager.lock().unwrap();
            match mgr.load_model("turbo", &path) {
                Ok(()) => log_debug("turbo model loaded successfully"),
                Err(e) => {
                    log_debug(&format!("turbo model load FAILED: {}", e));
                    emit_error(&app_clone, &format!("Falha ao carregar Turbo: {}", e));
                }
            }
        }

        log_debug("models-loaded event emitted");
        let _ = app_clone.emit("models-loaded", ());
    });
}

#[tauri::command]
fn get_state(state: tauri::State<'_, AppState>) -> String {
    state_label(state.recording_state.load(Ordering::SeqCst)).to_string()
}

#[tauri::command]
fn get_settings(state: tauri::State<'_, AppState>) -> settings::AppSettings {
    settings::AppSettings {
        live_model: state.live_model.lock().unwrap().clone(),
        batch_model: state.batch_model.lock().unwrap().clone(),
        translation_mode: state.translation_mode.lock().unwrap().clone(),
        activation_key: state.activation_key.lock().unwrap().clone(),
        anthropic_api_key: settings::load().anthropic_api_key,
    }
}

#[tauri::command]
fn update_settings(
    new_settings: settings::AppSettings,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let normalized = settings::normalize(new_settings);

    *state.live_model.lock().unwrap() = normalized.live_model.clone();
    *state.batch_model.lock().unwrap() = normalized.batch_model.clone();
    *state.translation_mode.lock().unwrap() = normalized.translation_mode.clone();
    *state.activation_key.lock().unwrap() = normalized.activation_key.clone();

    state.activation_mode.store(
        hotkey_mode_from_settings(&normalized.activation_key),
        Ordering::SeqCst,
    );

    settings::save(&normalized)
}

#[tauri::command]
fn set_live_model(name: String, state: tauri::State<'_, AppState>) {
    let value = if name == settings::MODEL_TURBO {
        settings::MODEL_TURBO
    } else {
        settings::MODEL_SMALL
    };
    *state.live_model.lock().unwrap() = value.to_string();

    let snapshot = get_settings(state);
    let _ = settings::save(&snapshot);
}

#[tauri::command]
fn set_batch_model(name: String, state: tauri::State<'_, AppState>) {
    let value = if name == settings::MODEL_SMALL {
        settings::MODEL_SMALL
    } else {
        settings::MODEL_TURBO
    };
    *state.batch_model.lock().unwrap() = value.to_string();

    let snapshot = get_settings(state);
    let _ = settings::save(&snapshot);
}

#[tauri::command]
fn set_translation_mode(mode: String, state: tauri::State<'_, AppState>) {
    let value = if settings::is_translation_mode_valid(&mode) {
        mode
    } else {
        settings::TRANSLATION_OFF.to_string()
    };
    *state.translation_mode.lock().unwrap() = value;

    let snapshot = get_settings(state);
    let _ = settings::save(&snapshot);
}

#[tauri::command]
fn set_activation_key(key: String, state: tauri::State<'_, AppState>) {
    let value = if key == settings::ACTIVATION_WIN {
        settings::ACTIVATION_WIN
    } else {
        settings::ACTIVATION_CTRL
    };
    *state.activation_key.lock().unwrap() = value.to_string();
    state
        .activation_mode
        .store(hotkey_mode_from_settings(value), Ordering::SeqCst);

    let snapshot = get_settings(state);
    let _ = settings::save(&snapshot);
}

#[tauri::command]
fn get_history(state: tauri::State<'_, AppState>) -> Vec<String> {
    state.history.lock().unwrap().clone()
}

#[tauri::command]
fn reuse_history_item(text: String) {
    if let Err(e) = paste::copy_and_paste(&text) {
        eprintln!("[reuse_history_item] paste failed: {}", e);
    }
}

#[tauri::command]
fn transcribe_file_cmd(path: String, state: tauri::State<'_, AppState>, app: AppHandle) {
    let file_path = PathBuf::from(&path);
    let preferred = state.batch_model.lock().unwrap().clone();
    let mgr = state.transcriber_manager.lock().unwrap();

    let result = mgr
        .transcribe_final_with_model(&file_path, &preferred)
        .map(|text| text_cleaner::clean(&text));

    match result {
        Ok(text) => {
            let _ = app.emit(
                "batch-progress",
                BatchProgress {
                    file: path,
                    text,
                    index: 1,
                    total: 1,
                    done: true,
                    error: None,
                },
            );
        }
        Err(e) => {
            let _ = app.emit(
                "batch-progress",
                BatchProgress {
                    file: path,
                    text: String::new(),
                    index: 1,
                    total: 1,
                    done: true,
                    error: Some(e),
                },
            );
        }
    }
}

#[tauri::command]
fn transcribe_folder_cmd(path: String, state: tauri::State<'_, AppState>, app: AppHandle) {
    let folder = PathBuf::from(&path);
    let entries: Vec<PathBuf> = match std::fs::read_dir(&folder) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.extension()
                    .and_then(|ext| ext.to_str())
                    .map(|ext| {
                        matches!(
                            ext.to_lowercase().as_str(),
                            "wav" | "mp3" | "m4a" | "ogg" | "flac" | "mp4" | "mov" | "webm"
                        )
                    })
                    .unwrap_or(false)
            })
            .collect(),
        Err(e) => {
            let _ = app.emit(
                "batch-progress",
                BatchProgress {
                    file: path,
                    text: String::new(),
                    index: 0,
                    total: 0,
                    done: true,
                    error: Some(format!("Falha ao ler pasta: {}", e)),
                },
            );
            return;
        }
    };

    let total = entries.len();
    if total == 0 {
        let _ = app.emit(
            "batch-progress",
            BatchProgress {
                file: path,
                text: String::new(),
                index: 0,
                total: 0,
                done: true,
                error: Some("Nenhum arquivo de áudio/vídeo suportado na pasta".to_string()),
            },
        );
        return;
    }

    let preferred = state.batch_model.lock().unwrap().clone();
    let mgr = state.transcriber_manager.lock().unwrap();

    for (idx, entry) in entries.iter().enumerate() {
        let file_str = entry.to_string_lossy().to_string();
        match mgr.transcribe_final_with_model(entry, &preferred) {
            Ok(text) => {
                let cleaned = text_cleaner::clean(&text);
                let _ = app.emit(
                    "batch-progress",
                    BatchProgress {
                        file: file_str,
                        text: cleaned,
                        index: idx + 1,
                        total,
                        done: idx + 1 == total,
                        error: None,
                    },
                );
            }
            Err(e) => {
                let _ = app.emit(
                    "batch-progress",
                    BatchProgress {
                        file: file_str,
                        text: String::new(),
                        index: idx + 1,
                        total,
                        done: idx + 1 == total,
                        error: Some(e),
                    },
                );
            }
        }
    }
}

#[tauri::command]
fn pick_file_cmd() -> Option<String> {
    rfd::FileDialog::new()
        .add_filter(
            "Audio/Video",
            &["wav", "mp3", "m4a", "ogg", "flac", "mp4", "mov", "webm"],
        )
        .pick_file()
        .map(|p| p.to_string_lossy().to_string())
}

#[tauri::command]
fn pick_folder_cmd() -> Option<String> {
    rfd::FileDialog::new()
        .pick_folder()
        .map(|p| p.to_string_lossy().to_string())
}

fn emit_error(app: &AppHandle, message: &str) {
    let _ = app.emit(
        "transcription-error",
        ErrorEvent {
            message: message.to_string(),
        },
    );
}

fn push_history(app: &AppHandle, text: &str) {
    let st = app.state::<AppState>();
    let mut history = st.history.lock().unwrap();
    history.retain(|item| item != text);
    history.insert(0, text.to_string());
    history.truncate(5);
    let _ = app.emit("history-updated", ());
}

fn emit_state_change(app: &AppHandle, value: &str) {
    let _ = app.emit(
        "state-change",
        StateChange {
            state: value.to_string(),
        },
    );
}

fn get_overlay_position(app: &AppHandle) -> (f64, f64) {
    let monitor = app.primary_monitor().ok().flatten();
    if let Some(m) = monitor {
        let size = m.size();
        let pos = m.position();
        (pos.x as f64 + (size.width as f64) - 130.0, pos.y as f64 + 60.0)
    } else {
        (100.0, 100.0)
    }
}

fn show_overlay(app: &AppHandle) {
    if app.get_webview_window("overlay").is_some() {
        return;
    }

    let (x, y) = get_overlay_position(app);

    let _ = WebviewWindowBuilder::new(app, "overlay", tauri::WebviewUrl::App("overlay.html?state=recording".into()))
        .title("Recording")
        .inner_size(110.0, 36.0)
        .position(x, y)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .focused(false)
        .build();
}

fn close_overlay(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("overlay") {
        let _ = win.close();
    }
}

fn update_overlay_state(app: &AppHandle, state: &str) {
    if let Some(win) = app.get_webview_window("overlay") {
        let _ = win.close();
    }

    // Small delay to ensure window is fully closed before recreating
    std::thread::sleep(Duration::from_millis(50));

    let (x, y) = get_overlay_position(app);
    let url = format!("overlay.html?state={}", state);

    let _ = WebviewWindowBuilder::new(app, "overlay", tauri::WebviewUrl::App(url.into()))
        .title("Recording")
        .inner_size(110.0, 36.0)
        .position(x, y)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .focused(false)
        .build();
}

fn log_debug(msg: &str) {
    let log_path = dirs::data_dir().unwrap().join("DictationApp").join("debug.log");
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .and_then(|mut f| {
            use std::io::Write;
            writeln!(f, "[{}] {}", chrono::Local::now().format("%H:%M:%S"), msg)
        });
}

fn start_recording(app: &AppHandle) {
    log_debug("start_recording called");
    let st = app.state::<AppState>();

    if st
        .recording_state
        .compare_exchange(
            STATE_IDLE,
            STATE_RECORDING,
            Ordering::SeqCst,
            Ordering::SeqCst,
        )
        .is_err()
    {
        log_debug("start_recording: state was not IDLE, returning");
        return;
    }

    st.transcribed_text.lock().unwrap().clear();

    {
        let mut recorder = st.recorder.lock().unwrap();
        if let Err(e) = recorder.start() {
            log_debug(&format!("start_recording: recorder.start() failed: {}", e));
            st.recording_state.store(STATE_IDLE, Ordering::SeqCst);
            emit_error(app, &format!("Falha ao iniciar gravação: {}", e));
            return;
        }
        log_debug("start_recording: recorder started successfully");
    }

    st.streaming_active.store(true, Ordering::SeqCst);
    st.hotkey_recording.store(true, Ordering::SeqCst);

    emit_state_change(app, "recording");
    show_overlay(app);
}

fn stop_recording(app: &AppHandle) {
    log_debug("stop_recording called");
    let st = app.state::<AppState>();

    if st
        .recording_state
        .compare_exchange(
            STATE_RECORDING,
            STATE_TRANSCRIBING,
            Ordering::SeqCst,
            Ordering::SeqCst,
        )
        .is_err()
    {
        log_debug("stop_recording: state was not RECORDING, returning");
        return;
    }

    st.hotkey_recording.store(false, Ordering::SeqCst);
    st.streaming_active.store(false, Ordering::SeqCst);

    let wav_path = {
        let mut recorder = st.recorder.lock().unwrap();
        match recorder.stop() {
            Ok(path) => {
                log_debug(&format!("stop_recording: wav saved to {:?}", path));
                path
            }
            Err(e) => {
                log_debug(&format!("stop_recording: recorder.stop() failed: {}", e));
                st.recording_state.store(STATE_IDLE, Ordering::SeqCst);
                emit_state_change(app, "idle");
                close_overlay(app);
                emit_error(app, &format!("Falha ao encerrar gravação: {}", e));
                return;
            }
        }
    };

    // Check if a previous transcription is still running
    if st.transcription_busy.load(Ordering::SeqCst) {
        log_debug("transcription already in progress, aborting");
        st.recording_state.store(STATE_IDLE, Ordering::SeqCst);
        emit_state_change(app, "idle");
        close_overlay(app);
        emit_error(app, "Transcricao anterior ainda em andamento. Aguarde.");
        let _ = std::fs::remove_file(&wav_path);
        return;
    }

    emit_state_change(app, "transcribing");
    update_overlay_state(app, "transcribing");

    let fallback_text = st.transcribed_text.lock().unwrap().clone();
    let preferred_model = st.live_model.lock().unwrap().clone();
    let translation_mode = st.translation_mode.lock().unwrap().clone();
    log_debug(&format!("stop_recording: using model {}, fallback text len: {}", preferred_model, fallback_text.len()));

    // Mark transcription as busy
    let busy_flag = Arc::clone(&st.transcription_busy);
    busy_flag.store(true, Ordering::SeqCst);

    let app_clone = app.clone();
    std::thread::spawn(move || {
        let (result_tx, result_rx) = crossbeam_channel::bounded(1);
        let app_for_worker = app_clone.clone();
        let wav_clone = wav_path.clone();
        let preferred_model_worker = preferred_model.clone();
        let busy_flag_worker = Arc::clone(&busy_flag);

        std::thread::spawn(move || {
            log_debug("transcription worker started");

            // Try Vulkan CLI first (much faster with GPU acceleration)
            let model_path = models::model_path(&preferred_model_worker);
            log_debug(&format!("trying Vulkan CLI with model: {:?}", model_path));

            let result = transcriber::transcribe_with_vulkan_cli(&wav_clone, &model_path);

            if result.is_err() {
                log_debug(&format!("Vulkan CLI failed: {:?}, falling back to built-in", result));
                // Fallback to built-in whisper-rs
                let state = app_for_worker.state::<AppState>();
                let mgr = match state.transcriber_manager.try_lock() {
                    Ok(guard) => guard,
                    Err(_) => {
                        log_debug("lock busy, skipping fallback");
                        busy_flag_worker.store(false, Ordering::SeqCst);
                        let _ = result_tx.send(Err("Lock busy".to_string()));
                        return;
                    }
                };
                let fallback_result = mgr.transcribe_final_with_model(&wav_clone, &preferred_model_worker);
                drop(mgr);
                busy_flag_worker.store(false, Ordering::SeqCst);
                log_debug(&format!("fallback result: {:?}", fallback_result.as_ref().map(|s| s.len())));
                let _ = result_tx.send(fallback_result);
            } else {
                busy_flag_worker.store(false, Ordering::SeqCst);
                log_debug(&format!("Vulkan CLI result: {:?}", result.as_ref().map(|s| s.len())));
                let _ = result_tx.send(result);
            }
        });

        // Timeout: 30s for tiny, 60s for small, 120s for turbo
        let timeout_secs = match preferred_model.as_str() {
            "tiny" => 30,
            "small" => 60,
            _ => 120,
        };

        let original_clean = match result_rx.recv_timeout(Duration::from_secs(timeout_secs)) {
            Ok(Ok(text)) => {
                let cleaned = text_cleaner::clean(&text);
                log_debug(&format!("transcription success, cleaned len: {}", cleaned.len()));
                cleaned
            }
            Ok(Err(e)) => {
                log_debug(&format!("transcription error: {}", e));
                fallback_text.clone()
            }
            Err(_) => {
                log_debug("transcription timeout, using fallback");
                // Note: worker thread may still be running, but busy_flag will be cleared when it finishes
                fallback_text.clone()
            }
        };

        if original_clean.is_empty() {
            log_debug("original_clean is empty, returning to idle");
            let state = app_clone.state::<AppState>();
            state.recording_state.store(STATE_IDLE, Ordering::SeqCst);
            emit_state_change(&app_clone, "idle");
            close_overlay(&app_clone);
            let _ = std::fs::remove_file(&wav_path);
            return;
        }

        let mut final_text = original_clean.clone();
        let mut translated = false;

        if translation_mode != settings::TRANSLATION_OFF {
            match translator::translate_if_needed(&original_clean, &translation_mode) {
                Ok(text) if !text.trim().is_empty() => {
                    final_text = text.trim().to_string();
                    translated = true;
                }
                Ok(_) => {}
                Err(e) => {
                    eprintln!("[translation] {}", e);
                }
            }
        }

        if let Err(e) = paste::copy_and_paste(&final_text) {
            emit_error(&app_clone, &format!("Falha ao colar transcrição: {}", e));
        }

        push_history(&app_clone, &final_text);

        let _ = app_clone.emit(
            "transcription-complete",
            TranscriptionComplete {
                original: original_clean,
                final_text,
                translated,
            },
        );

        let state = app_clone.state::<AppState>();
        state.recording_state.store(STATE_IDLE, Ordering::SeqCst);
        emit_state_change(&app_clone, "idle");

        update_overlay_state(&app_clone, "done");
        std::thread::sleep(Duration::from_millis(800));
        close_overlay(&app_clone);

        let _ = std::fs::remove_file(&wav_path);
    });
}

#[cfg_attr(not(windows), allow(dead_code))]
fn cancel_recording(app: &AppHandle) {
    let st = app.state::<AppState>();
    let current = st.recording_state.load(Ordering::SeqCst);
    if current == STATE_IDLE {
        return;
    }

    st.hotkey_recording.store(false, Ordering::SeqCst);
    st.streaming_active.store(false, Ordering::SeqCst);

    {
        let mut recorder = st.recorder.lock().unwrap();
        let _ = recorder.stop();
    }

    st.transcribed_text.lock().unwrap().clear();
    st.recording_state.store(STATE_IDLE, Ordering::SeqCst);
    emit_state_change(app, "idle");
    close_overlay(app);
}

fn main() {
    std::panic::set_hook(Box::new(|info| {
        log_debug(&format!("PANIC: {:?}", info));
    }));

    log_debug("main() starting");
    let initial_settings = settings::load();
    log_debug("settings loaded");

    let activation_mode = Arc::new(AtomicU8::new(hotkey_mode_from_settings(
        &initial_settings.activation_key,
    )));
    let hotkey_recording = Arc::new(AtomicBool::new(false));
    log_debug("starting tauri builder");

    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(AppState {
            recorder: Mutex::new(audio::AudioRecorder::new()),
            transcriber_manager: Mutex::new(transcriber::TranscriberManager::new()),
            recording_state: AtomicU8::new(STATE_IDLE),
            transcribed_text: Mutex::new(String::new()),
            history: Mutex::new(Vec::new()),
            live_model: Mutex::new(initial_settings.live_model.clone()),
            batch_model: Mutex::new(initial_settings.batch_model.clone()),
            translation_mode: Mutex::new(initial_settings.translation_mode.clone()),
            activation_key: Mutex::new(initial_settings.activation_key.clone()),
            activation_mode: Arc::clone(&activation_mode),
            streaming_active: Arc::new(AtomicBool::new(false)),
            hotkey_recording: Arc::clone(&hotkey_recording),
            transcription_busy: Arc::new(AtomicBool::new(false)),
        })
        .invoke_handler(tauri::generate_handler![
            check_models,
            download_model_cmd,
            load_models,
            get_state,
            get_settings,
            update_settings,
            set_live_model,
            set_batch_model,
            set_translation_mode,
            set_activation_key,
            get_history,
            reuse_history_item,
            pick_file_cmd,
            pick_folder_cmd,
            transcribe_file_cmd,
            transcribe_folder_cmd,
        ])
        .setup(|app| {
            let _ = settings::save(&settings::load());

            let version_label = format!("DictationApp v{}", env!("CARGO_PKG_VERSION"));
            let version = MenuItem::with_id(app, "version", version_label, false, None::<&str>)?;
            let sep1 = PredefinedMenuItem::separator(app)?;
            let record = MenuItem::with_id(app, "record", "Start Recording", true, None::<&str>)?;
            let sep2 = PredefinedMenuItem::separator(app)?;
            let file_item = MenuItem::with_id(
                app,
                "transcribe_file",
                "Transcrever Arquivo...",
                true,
                None::<&str>,
            )?;
            let folder_item = MenuItem::with_id(
                app,
                "transcribe_folder",
                "Transcrever Pasta...",
                true,
                None::<&str>,
            )?;
            let sep3 = PredefinedMenuItem::separator(app)?;
            let update = MenuItem::with_id(
                app,
                "check_update",
                "Check for Updates...",
                true,
                None::<&str>,
            )?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;

            let menu = Menu::with_items(
                app,
                &[
                    &version,
                    &sep1,
                    &record,
                    &sep2,
                    &file_item,
                    &folder_item,
                    &sep3,
                    &update,
                    &quit,
                ],
            )?;

            TrayIconBuilder::new()
                .tooltip("DictationApp")
                .icon(tauri::image::Image::from_bytes(include_bytes!("../icons/icon.ico"))?)
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(move |app, event| match event.id.as_ref() {
                    "record" => {
                        let state = app.state::<AppState>();
                        let current = state.recording_state.load(Ordering::SeqCst);
                        if current == STATE_IDLE {
                            start_recording(app);
                        } else if current == STATE_RECORDING {
                            stop_recording(app);
                        }
                    }
                    "transcribe_file" => {
                        let _ = app.emit("open-file-dialog", ());
                    }
                    "transcribe_folder" => {
                        let _ = app.emit("open-folder-dialog", ());
                    }
                    "check_update" => {
                        let _ = app.emit("check-update", ());
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(win) = app.get_webview_window("main") {
                            if win.is_visible().unwrap_or(false) {
                                let _ = win.hide();
                            } else {
                                let _ = win.show();
                                let _ = win.set_focus();
                            }
                        }
                    }
                })
                .build(app)?;

            if let Some(window) = app.get_webview_window("main") {
                let win = window.clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = win.hide();
                    }
                });
            }

            #[cfg(windows)]
            {
                log_debug("starting hotkey listener");
                let app_handle = app.handle().clone();
                let (hotkey_tx, hotkey_rx) = crossbeam_channel::unbounded();
                let hotkey_state = app_handle.state::<AppState>();

                hotkey::start(
                    hotkey_tx,
                    Arc::clone(&hotkey_state.hotkey_recording),
                    Arc::clone(&hotkey_state.activation_mode),
                );
                log_debug("hotkey hook installed");

                let hotkey_app = app_handle.clone();
                std::thread::spawn(move || {
                    log_debug("hotkey receiver thread started");
                    for event in hotkey_rx {
                        log_debug(&format!("hotkey event received: {:?}", event));
                        let state = hotkey_app.state::<AppState>();
                        match event {
                            hotkey::HotkeyEvent::ActivationPressed => {
                                let current = state.recording_state.load(Ordering::SeqCst);
                                if current == STATE_IDLE {
                                    start_recording(&hotkey_app);
                                } else if current == STATE_RECORDING {
                                    stop_recording(&hotkey_app);
                                }
                            }
                            hotkey::HotkeyEvent::Escape => {
                                cancel_recording(&hotkey_app);
                            }
                        }
                    }
                });
            }

            log_debug("setup complete");
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error running app");
    log_debug("app exited normally");
}
