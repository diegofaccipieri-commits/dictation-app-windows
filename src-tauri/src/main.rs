#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod models;
mod audio;
mod transcriber;
mod text_cleaner;
mod paste;
mod batch;

#[cfg(windows)]
mod hotkey;

use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager,
};

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            let version = MenuItem::with_id(app, "version", "DictationApp v1.0", false, None::<&str>)?;
            let sep1 = PredefinedMenuItem::separator(app)?;
            let record = MenuItem::with_id(app, "record", "Start Recording", true, None::<&str>)?;
            let sep2 = PredefinedMenuItem::separator(app)?;
            let file_item = MenuItem::with_id(app, "transcribe_file", "Transcrever Arquivo...", true, None::<&str>)?;
            let folder_item = MenuItem::with_id(app, "transcribe_folder", "Transcrever Pasta...", true, None::<&str>)?;
            let sep3 = PredefinedMenuItem::separator(app)?;
            let update = MenuItem::with_id(app, "check_update", "Check for Updates...", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;

            let menu = Menu::with_items(app, &[
                &version, &sep1, &record, &sep2,
                &file_item, &folder_item, &sep3,
                &update, &quit,
            ])?;

            TrayIconBuilder::new()
                .tooltip("DictationApp")
                .menu(&menu)
                .menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
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

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error running app");
}
