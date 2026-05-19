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
    let mut map = std::collections::HashMap::new();
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
            emit_error(
                &app2,
                &format!("Erro ao baixar modelo {}: {}", model_name, e),
            );
        }
    });
}

#[tauri::command]
fn load_models(app: AppHandle, state: tauri::State<'_, AppState>) {
    if models::is_downloaded("small") {
        let path = models::model_path("small");
        let mut mgr = state.transcriber_manager.lock().unwrap();
        if let Err(e) = mgr.load_model("small", &path) {
            emit_error(&app, &format!("Falha ao carregar Small: {}", e));
        }
    }

    if models::is_downloaded("turbo") {
        let path = models::model_path("turbo");
        let mut mgr = state.transcriber_manager.lock().unwrap();
        if let Err(e) = mgr.load_model("turbo", &path) {
            emit_error(&app, &format!("Falha ao carregar Turbo: {}", e));
        }
    }

    let _ = app.emit("models-loaded", ());
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

fn start_recording(app: &AppHandle) {
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
        return;
    }

    st.transcribed_text.lock().unwrap().clear();

    {
        let mut recorder = st.recorder.lock().unwrap();
        if let Err(e) = recorder.start() {
            st.recording_state.store(STATE_IDLE, Ordering::SeqCst);
            emit_error(app, &format!("Falha ao iniciar gravação: {}", e));
            return;
        }
    }

    st.streaming_active.store(true, Ordering::SeqCst);
    st.hotkey_recording.store(true, Ordering::SeqCst);

    emit_state_change(app, "recording");

    let app_clone = app.clone();
    let streaming_flag = Arc::clone(&st.streaming_active);
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(2));

        if !streaming_flag.load(Ordering::SeqCst) {
            break;
        }

        let state = app_clone.state::<AppState>();
        if state.recording_state.load(Ordering::SeqCst) != STATE_RECORDING {
            continue;
        }

        let (samples, _) = state.recorder.lock().unwrap().current_samples();
        if samples.len() < 1600 {
            continue;
        }

        let preferred = state.live_model.lock().unwrap().clone();
        let result = state
            .transcriber_manager
            .lock()
            .unwrap()
            .transcribe_streaming_with_model(&samples, &preferred);

        match result {
            Ok(text) => {
                let cleaned = text_cleaner::clean(&text);
                *state.transcribed_text.lock().unwrap() = cleaned.clone();
                let _ = app_clone.emit("streaming-update", StreamingUpdate { text: cleaned });
            }
            Err(e) => {
                eprintln!("[streaming] transcription error: {}", e);
            }
        }
    });
}

fn stop_recording(app: &AppHandle) {
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
        return;
    }

    st.hotkey_recording.store(false, Ordering::SeqCst);
    st.streaming_active.store(false, Ordering::SeqCst);

    let wav_path = {
        let mut recorder = st.recorder.lock().unwrap();
        match recorder.stop() {
            Ok(path) => path,
            Err(e) => {
                st.recording_state.store(STATE_IDLE, Ordering::SeqCst);
                emit_state_change(app, "idle");
                emit_error(app, &format!("Falha ao encerrar gravação: {}", e));
                return;
            }
        }
    };

    emit_state_change(app, "transcribing");

    let fallback_text = st.transcribed_text.lock().unwrap().clone();
    let preferred_model = st.live_model.lock().unwrap().clone();
    let translation_mode = st.translation_mode.lock().unwrap().clone();

    let app_clone = app.clone();
    std::thread::spawn(move || {
        let (result_tx, result_rx) = crossbeam_channel::bounded(1);
        let app_for_worker = app_clone.clone();
        let wav_clone = wav_path.clone();
        let preferred_model_worker = preferred_model.clone();

        std::thread::spawn(move || {
            let state = app_for_worker.state::<AppState>();
            let result = state
                .transcriber_manager
                .lock()
                .unwrap()
                .transcribe_final_with_model(&wav_clone, &preferred_model_worker);
            let _ = result_tx.send(result);
        });

        let original_clean = match result_rx.recv_timeout(Duration::from_secs(45)) {
            Ok(Ok(text)) => text_cleaner::clean(&text),
            Ok(Err(e)) => {
                eprintln!("[final_transcription] error: {}", e);
                fallback_text.clone()
            }
            Err(_) => {
                eprintln!("[final_transcription] timeout, using streaming fallback");
                fallback_text.clone()
            }
        };

        if original_clean.is_empty() {
            let state = app_clone.state::<AppState>();
            state.recording_state.store(STATE_IDLE, Ordering::SeqCst);
            emit_state_change(&app_clone, "idle");
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
}

fn main() {
    let initial_settings = settings::load();

    let activation_mode = Arc::new(AtomicU8::new(hotkey_mode_from_settings(
        &initial_settings.activation_key,
    )));
    let hotkey_recording = Arc::new(AtomicBool::new(false));

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

            #[cfg(windows)]
            {
                let (hotkey_tx, hotkey_rx) = crossbeam_channel::unbounded();
                let hotkey_state = app_handle.state::<AppState>();

                hotkey::start(
                    hotkey_tx,
                    Arc::clone(&hotkey_state.hotkey_recording),
                    Arc::clone(&hotkey_state.activation_mode),
                );

                let hotkey_app = app_handle.clone();
                std::thread::spawn(move || {
                    for event in hotkey_rx {
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

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error running app");
}
