#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod models;
mod audio;
mod transcriber;
mod text_cleaner;
mod paste;
mod batch;

#[cfg(windows)]
mod hotkey;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager,
};

// ---------------------------------------------------------------------------
// Recording states
// ---------------------------------------------------------------------------

const STATE_IDLE: u8 = 0;
const STATE_RECORDING: u8 = 1;
const STATE_TRANSCRIBING: u8 = 2;

fn state_label(v: u8) -> &'static str {
    match v {
        STATE_RECORDING => "recording",
        STATE_TRANSCRIBING => "transcribing",
        _ => "idle",
    }
}

// ---------------------------------------------------------------------------
// AppState
// ---------------------------------------------------------------------------

struct AppState {
    recorder: Mutex<audio::AudioRecorder>,
    transcriber_manager: Mutex<transcriber::TranscriberManager>,
    recording_state: AtomicU8,
    committed_text: Mutex<String>,
    transcribed_text: Mutex<String>,
    history: Mutex<Vec<String>>,
    live_model: Mutex<String>,
    batch_model: Mutex<String>,
    /// Flag used to stop the streaming loop when recording ends.
    streaming_active: Arc<AtomicBool>,
}

// ---------------------------------------------------------------------------
// Event payloads
// ---------------------------------------------------------------------------

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
    text: String,
}

#[derive(Clone, Serialize)]
struct StateChange {
    state: String,
}

#[derive(Clone, Serialize)]
struct BatchProgress {
    file: String,
    text: String,
    done: bool,
    error: Option<String>,
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[tauri::command]
fn check_models() -> HashMap<String, bool> {
    let mut map = HashMap::new();
    map.insert("small".to_string(), models::is_downloaded("small"));
    map.insert("turbo".to_string(), models::is_downloaded("turbo"));
    map
}

#[tauri::command]
fn download_model_cmd(name: String, app: AppHandle) {
    let app2 = app.clone();
    let model_name = name.clone();
    std::thread::spawn(move || {
        let result = models::download_model(&model_name, |downloaded, total| {
            let _ = app2.emit(
                "download-progress",
                DownloadProgress {
                    name: model_name.clone(),
                    downloaded,
                    total,
                },
            );
        });
        if let Err(e) = result {
            eprintln!("[download_model] Error downloading {}: {}", name, e);
        }
    });
}

#[tauri::command]
fn load_models(app: AppHandle, state: tauri::State<'_, AppState>) {
    // Load Small (required)
    if models::is_downloaded("small") {
        let path = models::model_path("small");
        let mut mgr = state.transcriber_manager.lock().unwrap();
        if let Err(e) = mgr.load_model("small", &path) {
            eprintln!("[load_models] Failed to load small model: {}", e);
        }
    }

    // Load Turbo (optional)
    if models::is_downloaded("turbo") {
        let path = models::model_path("turbo");
        let mut mgr = state.transcriber_manager.lock().unwrap();
        if let Err(e) = mgr.load_model("turbo", &path) {
            eprintln!("[load_models] Failed to load turbo model: {}", e);
        }
    }

    let _ = app.emit("models-loaded", ());
}

#[tauri::command]
fn get_state(state: tauri::State<'_, AppState>) -> String {
    state_label(state.recording_state.load(Ordering::SeqCst)).to_string()
}

#[tauri::command]
fn set_live_model(name: String, state: tauri::State<'_, AppState>) {
    *state.live_model.lock().unwrap() = name;
}

#[tauri::command]
fn set_batch_model(name: String, state: tauri::State<'_, AppState>) {
    *state.batch_model.lock().unwrap() = name;
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
    let mgr = state.transcriber_manager.lock().unwrap();
    let result = mgr.transcribe_final(&file_path);

    match result {
        Ok(text) => {
            let cleaned = text_cleaner::clean(&text);
            let _ = app.emit(
                "batch-progress",
                BatchProgress {
                    file: path,
                    text: cleaned,
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
                    .map(|ext| matches!(ext.to_lowercase().as_str(), "wav" | "mp3" | "m4a" | "ogg" | "flac"))
                    .unwrap_or(false)
            })
            .collect(),
        Err(e) => {
            let _ = app.emit(
                "batch-progress",
                BatchProgress {
                    file: path,
                    text: String::new(),
                    done: true,
                    error: Some(format!("Failed to read folder: {}", e)),
                },
            );
            return;
        }
    };

    let mgr = state.transcriber_manager.lock().unwrap();
    for entry in entries {
        let file_str = entry.to_string_lossy().to_string();
        match mgr.transcribe_final(&entry) {
            Ok(text) => {
                let cleaned = text_cleaner::clean(&text);
                let _ = app.emit(
                    "batch-progress",
                    BatchProgress {
                        file: file_str,
                        text: cleaned,
                        done: false,
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
                        done: false,
                        error: Some(e),
                    },
                );
            }
        }
    }

    // Signal completion
    let _ = app.emit(
        "batch-progress",
        BatchProgress {
            file: path,
            text: String::new(),
            done: true,
            error: None,
        },
    );
}

// ---------------------------------------------------------------------------
// History helper
// ---------------------------------------------------------------------------

fn push_history(state: &AppState, text: &str) {
    let mut history = state.history.lock().unwrap();
    // Remove duplicate if present
    history.retain(|item| item != text);
    // Prepend
    history.insert(0, text.to_string());
    // Cap at 5
    history.truncate(5);
}

// ---------------------------------------------------------------------------
// Recording flow helpers
// ---------------------------------------------------------------------------

fn start_recording(app: &AppHandle, state: &AppState) {
    // Only start if idle
    if state
        .recording_state
        .compare_exchange(STATE_IDLE, STATE_RECORDING, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }

    // Clear texts
    state.committed_text.lock().unwrap().clear();
    state.transcribed_text.lock().unwrap().clear();

    // Start audio capture
    {
        let mut recorder = state.recorder.lock().unwrap();
        if let Err(e) = recorder.start() {
            eprintln!("[start_recording] failed to start recorder: {}", e);
            state.recording_state.store(STATE_IDLE, Ordering::SeqCst);
            return;
        }
    }

    state.streaming_active.store(true, Ordering::SeqCst);

    let _ = app.emit("state-change", StateChange { state: "recording".to_string() });

    // Start streaming transcription loop in background thread
    let app_clone = app.clone();
    let streaming_flag = Arc::clone(&state.streaming_active);
    // We need to access state from AppHandle in the thread
    let app_for_state = app.clone();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_secs(2));

            if !streaming_flag.load(Ordering::SeqCst) {
                break;
            }

            let st = app_for_state.state::<AppState>();
            let (samples, _sr) = st.recorder.lock().unwrap().current_samples();

            if samples.is_empty() {
                continue;
            }

            let mgr = st.transcriber_manager.lock().unwrap();
            match mgr.transcribe_streaming(&samples) {
                Ok(text) => {
                    let cleaned = text_cleaner::clean(&text);
                    *st.transcribed_text.lock().unwrap() = cleaned.clone();
                    let _ = app_clone.emit("streaming-update", StreamingUpdate { text: cleaned });
                }
                Err(e) => {
                    eprintln!("[streaming] transcription error: {}", e);
                }
            }
        }
    });
}

fn stop_recording(app: &AppHandle, state: &AppState) {
    // Only stop if recording
    if state
        .recording_state
        .compare_exchange(STATE_RECORDING, STATE_TRANSCRIBING, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }

    // Stop streaming loop
    state.streaming_active.store(false, Ordering::SeqCst);

    // Stop recorder and get WAV path
    let wav_path = {
        let mut recorder = state.recorder.lock().unwrap();
        match recorder.stop() {
            Ok(path) => path,
            Err(e) => {
                eprintln!("[stop_recording] recorder.stop() failed: {}", e);
                state.recording_state.store(STATE_IDLE, Ordering::SeqCst);
                let _ = app.emit("state-change", StateChange { state: "idle".to_string() });
                return;
            }
        }
    };

    let _ = app.emit("state-change", StateChange { state: "transcribing".to_string() });

    // Grab streaming fallback text
    let fallback_text = state.transcribed_text.lock().unwrap().clone();

    // Spawn final transcription with timeout
    let app_clone = app.clone();
    let app_for_state = app.clone();
    std::thread::spawn(move || {
        let (result_tx, result_rx) = crossbeam_channel::bounded(1);

        // Worker thread for final transcription
        let wav_clone = wav_path.clone();
        let app_worker = app_for_state.clone();
        std::thread::spawn(move || {
            let st = app_worker.state::<AppState>();
            let mgr = st.transcriber_manager.lock().unwrap();
            let result = mgr.transcribe_final(&wav_clone);
            let _ = result_tx.send(result);
        });

        // Wait with 45s timeout
        let final_text = match result_rx.recv_timeout(Duration::from_secs(45)) {
            Ok(Ok(text)) => text_cleaner::clean(&text),
            Ok(Err(e)) => {
                eprintln!("[final_transcription] error: {}", e);
                if fallback_text.is_empty() {
                    String::new()
                } else {
                    fallback_text.clone()
                }
            }
            Err(_) => {
                eprintln!("[final_transcription] timed out after 45s, using streaming fallback");
                fallback_text.clone()
            }
        };

        let st = app_clone.state::<AppState>();

        if !final_text.is_empty() {
            // Paste result
            if let Err(e) = paste::copy_and_paste(&final_text) {
                eprintln!("[stop_recording] paste failed: {}", e);
            }

            // Add to history
            push_history(&st, &final_text);

            let _ = app_clone.emit(
                "transcription-complete",
                TranscriptionComplete { text: final_text },
            );
        }

        // Back to idle
        st.recording_state.store(STATE_IDLE, Ordering::SeqCst);
        let _ = app_clone.emit("state-change", StateChange { state: "idle".to_string() });

        // Clean up temp WAV
        let _ = std::fs::remove_file(&wav_path);
    });
}

fn cancel_recording(app: &AppHandle, state: &AppState) {
    let current = state.recording_state.load(Ordering::SeqCst);
    if current == STATE_IDLE {
        return;
    }

    // Stop streaming loop
    state.streaming_active.store(false, Ordering::SeqCst);

    // Stop recorder (ignore errors, might not be recording)
    {
        let mut recorder = state.recorder.lock().unwrap();
        let _ = recorder.stop();
    }

    // Clear state
    state.committed_text.lock().unwrap().clear();
    state.transcribed_text.lock().unwrap().clear();
    state.recording_state.store(STATE_IDLE, Ordering::SeqCst);

    let _ = app.emit("state-change", StateChange { state: "idle".to_string() });
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(AppState {
            recorder: Mutex::new(audio::AudioRecorder::new()),
            transcriber_manager: Mutex::new(transcriber::TranscriberManager::new()),
            recording_state: AtomicU8::new(STATE_IDLE),
            committed_text: Mutex::new(String::new()),
            transcribed_text: Mutex::new(String::new()),
            history: Mutex::new(Vec::new()),
            live_model: Mutex::new("small".to_string()),
            batch_model: Mutex::new("turbo".to_string()),
            streaming_active: Arc::new(AtomicBool::new(false)),
        })
        .invoke_handler(tauri::generate_handler![
            check_models,
            download_model_cmd,
            load_models,
            get_state,
            set_live_model,
            set_batch_model,
            get_history,
            reuse_history_item,
            transcribe_file_cmd,
            transcribe_folder_cmd,
        ])
        .setup(|app| {
            // ----- System tray -----
            let version = MenuItem::with_id(app, "version", "DictationApp v1.0", false, None::<&str>)?;
            let sep1 = PredefinedMenuItem::separator(app)?;
            let record = MenuItem::with_id(app, "record", "Start Recording", true, None::<&str>)?;
            let sep2 = PredefinedMenuItem::separator(app)?;
            let file_item =
                MenuItem::with_id(app, "transcribe_file", "Transcrever Arquivo...", true, None::<&str>)?;
            let folder_item =
                MenuItem::with_id(app, "transcribe_folder", "Transcrever Pasta...", true, None::<&str>)?;
            let sep3 = PredefinedMenuItem::separator(app)?;
            let update =
                MenuItem::with_id(app, "check_update", "Check for Updates...", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;

            let menu = Menu::with_items(app, &[
                &version, &sep1, &record, &sep2, &file_item, &folder_item, &sep3, &update, &quit,
            ])?;

            let _app_handle = app.handle().clone();
            TrayIconBuilder::new()
                .tooltip("DictationApp")
                .menu(&menu)
                .menu_on_left_click(false)
                .on_menu_event(move |app, event| match event.id.as_ref() {
                    "record" => {
                        let st = app.state::<AppState>();
                        let current = st.recording_state.load(Ordering::SeqCst);
                        if current == STATE_IDLE {
                            start_recording(app, &st);
                        } else if current == STATE_RECORDING {
                            stop_recording(app, &st);
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

            // ----- Hotkey listener (Windows only) -----
            #[cfg(windows)]
            {
                let (hotkey_tx, hotkey_rx) = crossbeam_channel::unbounded();
                let is_recording = Arc::new(AtomicBool::new(false));

                hotkey::start(hotkey_tx, is_recording.clone());

                let hotkey_app = _app_handle.clone();
                let is_recording_flag = is_recording;
                std::thread::spawn(move || {
                    for event in hotkey_rx {
                        let st = hotkey_app.state::<AppState>();
                        match event {
                            hotkey::HotkeyEvent::DoubleTapCtrl => {
                                let current = st.recording_state.load(Ordering::SeqCst);
                                if current == STATE_IDLE {
                                    is_recording_flag.store(true, Ordering::SeqCst);
                                    start_recording(&hotkey_app, &st);
                                } else if current == STATE_RECORDING {
                                    is_recording_flag.store(false, Ordering::SeqCst);
                                    stop_recording(&hotkey_app, &st);
                                }
                            }
                            hotkey::HotkeyEvent::Escape => {
                                is_recording_flag.store(false, Ordering::SeqCst);
                                cancel_recording(&hotkey_app, &st);
                            }
                        }
                    }
                });
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error running app");
}
