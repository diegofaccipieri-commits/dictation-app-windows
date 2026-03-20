// Global hotkey monitor (Windows only)
// Installs a low-level keyboard hook to detect double-tap Ctrl and ESC.

#[derive(Debug, Clone)]
pub enum HotkeyEvent {
    DoubleTapCtrl,
    Escape,
}

#[cfg(windows)]
mod platform {
    use super::HotkeyEvent;
    use crossbeam_channel::Sender;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Instant;

    use windows::Win32::Foundation::*;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::Input::KeyboardAndMouse::*;
    use windows::Win32::UI::WindowsAndMessaging::*;

    const VK_LCONTROL_U32: u32 = VK_LCONTROL.0 as u32;
    const VK_RCONTROL_U32: u32 = VK_RCONTROL.0 as u32;
    const VK_ESCAPE_U32: u32 = VK_ESCAPE.0 as u32;
    const DOUBLE_TAP_MS: u128 = 500;

    // Static state accessed from the hook callback.
    // Safety: only accessed from the hook thread (single-threaded message pump).
    static mut HOOK_TX: Option<Sender<HotkeyEvent>> = None;
    static mut HOOK_IS_RECORDING: Option<Arc<AtomicBool>> = None;
    static mut LAST_CTRL_DOWN: Option<Instant> = None;

    pub fn start(tx: Sender<HotkeyEvent>, is_recording: Arc<AtomicBool>) {
        std::thread::spawn(move || {
            unsafe {
                HOOK_TX = Some(tx);
                HOOK_IS_RECORDING = Some(is_recording);
                LAST_CTRL_DOWN = None;

                let h_instance = GetModuleHandleW(None).unwrap_or_default();
                let hook = SetWindowsHookExW(
                    WH_KEYBOARD_LL,
                    Some(low_level_keyboard_proc),
                    h_instance.into(),
                    0,
                );

                if hook.is_err() {
                    eprintln!("[hotkey] Failed to install keyboard hook");
                    return;
                }

                // Message pump — required for WH_KEYBOARD_LL to work.
                let mut msg = MSG::default();
                while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        });
    }

    unsafe extern "system" fn low_level_keyboard_proc(
        n_code: i32,
        w_param: WPARAM,
        l_param: LPARAM,
    ) -> LRESULT {
        if n_code >= 0 {
            let msg = w_param.0 as u32;
            // Only react to key-down events.
            if msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN {
                let kb = &*(l_param.0 as *const KBDLLHOOKSTRUCT);
                let vk = kb.vkCode;

                match vk {
                    VK_LCONTROL_U32 | VK_RCONTROL_U32 => {
                        handle_ctrl();
                    }
                    VK_ESCAPE_U32 => {
                        if let Some(ref tx) = HOOK_TX {
                            let _ = tx.try_send(HotkeyEvent::Escape);
                        }
                    }
                    _ => {}
                }
            }
        }

        CallNextHookEx(None, n_code, w_param, l_param)
    }

    unsafe fn handle_ctrl() {
        let recording = HOOK_IS_RECORDING
            .as_ref()
            .map(|r| r.load(Ordering::SeqCst))
            .unwrap_or(false);

        if recording {
            // While recording, a single Ctrl press stops recording.
            if let Some(ref tx) = HOOK_TX {
                let _ = tx.try_send(HotkeyEvent::DoubleTapCtrl);
            }
            LAST_CTRL_DOWN = None;
        } else {
            // Not recording: require double-tap within 500ms.
            let now = Instant::now();
            if let Some(prev) = LAST_CTRL_DOWN {
                if now.duration_since(prev).as_millis() <= DOUBLE_TAP_MS {
                    if let Some(ref tx) = HOOK_TX {
                        let _ = tx.try_send(HotkeyEvent::DoubleTapCtrl);
                    }
                    LAST_CTRL_DOWN = None;
                } else {
                    LAST_CTRL_DOWN = Some(now);
                }
            } else {
                LAST_CTRL_DOWN = Some(now);
            }
        }
    }
}

#[cfg(windows)]
pub fn start(
    tx: crossbeam_channel::Sender<HotkeyEvent>,
    is_recording: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    platform::start(tx, is_recording);
}

#[cfg(not(windows))]
pub fn start(
    _tx: crossbeam_channel::Sender<HotkeyEvent>,
    _is_recording: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    // No-op on non-Windows platforms.
}
