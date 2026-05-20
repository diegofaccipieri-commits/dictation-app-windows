// Global hotkey monitor (Windows only)
// Installs a low-level keyboard hook to detect double-tap activation key and ESC.

#[derive(Debug, Clone)]
pub enum HotkeyEvent {
    ActivationPressed,
    Escape,
}

pub const ACTIVATION_CTRL: u8 = 0;
pub const ACTIVATION_WIN: u8 = 1;

#[cfg(windows)]
mod platform {
    use super::{HotkeyEvent, ACTIVATION_CTRL, ACTIVATION_WIN};
    use crossbeam_channel::Sender;
    use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
    use std::sync::Arc;
    use std::time::Instant;

    use windows::Win32::Foundation::*;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::Input::KeyboardAndMouse::*;
    use windows::Win32::UI::WindowsAndMessaging::*;

    const VK_LCONTROL_U32: u32 = VK_LCONTROL.0 as u32;
    const VK_RCONTROL_U32: u32 = VK_RCONTROL.0 as u32;
    const VK_LWIN_U32: u32 = VK_LWIN.0 as u32;
    const VK_RWIN_U32: u32 = VK_RWIN.0 as u32;
    const VK_ESCAPE_U32: u32 = VK_ESCAPE.0 as u32;
    const DOUBLE_TAP_MS: u128 = 500;

    // Static state accessed from the hook callback.
    // Safety: only accessed from the hook thread (single-threaded message pump).
    static mut HOOK_TX: Option<Sender<HotkeyEvent>> = None;
    static mut HOOK_IS_RECORDING: Option<Arc<AtomicBool>> = None;
    static mut HOOK_ACTIVATION_MODE: Option<Arc<AtomicU8>> = None;
    static mut LAST_ACTIVATION_DOWN: Option<Instant> = None;
    static mut CTRL_DOWN: bool = false;
    static mut WIN_DOWN: bool = false;

    pub fn start(
        tx: Sender<HotkeyEvent>,
        is_recording: Arc<AtomicBool>,
        activation_mode: Arc<AtomicU8>,
    ) {
        std::thread::spawn(move || {
            unsafe {
                HOOK_TX = Some(tx);
                HOOK_IS_RECORDING = Some(is_recording);
                HOOK_ACTIVATION_MODE = Some(activation_mode);
                LAST_ACTIVATION_DOWN = None;
                CTRL_DOWN = false;
                WIN_DOWN = false;

                let h_instance = GetModuleHandleW(None).unwrap_or_default();
                let hook = match SetWindowsHookExW(
                    WH_KEYBOARD_LL,
                    Some(low_level_keyboard_proc),
                    h_instance,
                    0,
                ) {
                    Ok(hook) => hook,
                    Err(_) => {
                        eprintln!("[hotkey] Failed to install keyboard hook");
                        return;
                    }
                };

                // Message pump — required for WH_KEYBOARD_LL to work.
                let mut msg = MSG::default();
                while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }

                let _ = UnhookWindowsHookEx(hook);
            }
        });
    }

    unsafe extern "system" fn low_level_keyboard_proc(
        n_code: i32,
        w_param: WPARAM,
        l_param: LPARAM,
    ) -> LRESULT {
        let mut should_swallow = false;

        if n_code >= 0 {
            let msg = w_param.0 as u32;
            let kb = &*(l_param.0 as *const KBDLLHOOKSTRUCT);
            let vk = kb.vkCode;
            let is_keydown = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
            let is_keyup = msg == WM_KEYUP || msg == WM_SYSKEYUP;
            let activation_mode = HOOK_ACTIVATION_MODE
                .as_ref()
                .map(|mode| mode.load(Ordering::SeqCst))
                .unwrap_or(ACTIVATION_CTRL);

            match vk {
                VK_LCONTROL_U32 | VK_RCONTROL_U32 => {
                    if is_keydown && !CTRL_DOWN {
                        CTRL_DOWN = true;
                        if activation_mode == ACTIVATION_CTRL {
                            handle_activation_press();
                        }
                    } else if is_keyup {
                        CTRL_DOWN = false;
                    }
                }
                VK_LWIN_U32 | VK_RWIN_U32 => {
                    if activation_mode == ACTIVATION_WIN {
                        should_swallow = true;
                    }

                    if is_keydown && !WIN_DOWN {
                        WIN_DOWN = true;
                        if activation_mode == ACTIVATION_WIN {
                            handle_activation_press();
                        }
                    } else if is_keyup {
                        WIN_DOWN = false;
                    }
                }
                VK_ESCAPE_U32 if is_keydown => {
                    if let Some(ref tx) = HOOK_TX {
                        let _ = tx.try_send(HotkeyEvent::Escape);
                    }
                }
                _ => {}
            }
        }

        if should_swallow {
            return LRESULT(1);
        }

        CallNextHookEx(None, n_code, w_param, l_param)
    }

    unsafe fn handle_activation_press() {
        let recording = HOOK_IS_RECORDING
            .as_ref()
            .map(|r| r.load(Ordering::SeqCst))
            .unwrap_or(false);

        if recording {
            // While recording, a single activation key press stops recording.
            if let Some(ref tx) = HOOK_TX {
                let _ = tx.try_send(HotkeyEvent::ActivationPressed);
            }
            LAST_ACTIVATION_DOWN = None;
        } else {
            // Not recording: require double-tap within 500ms.
            let now = Instant::now();
            if let Some(prev) = LAST_ACTIVATION_DOWN {
                if now.duration_since(prev).as_millis() <= DOUBLE_TAP_MS {
                    if let Some(ref tx) = HOOK_TX {
                        let _ = tx.try_send(HotkeyEvent::ActivationPressed);
                    }
                    LAST_ACTIVATION_DOWN = None;
                } else {
                    LAST_ACTIVATION_DOWN = Some(now);
                }
            } else {
                LAST_ACTIVATION_DOWN = Some(now);
            }
        }
    }
}

#[cfg(windows)]
pub fn start(
    tx: crossbeam_channel::Sender<HotkeyEvent>,
    is_recording: std::sync::Arc<std::sync::atomic::AtomicBool>,
    activation_mode: std::sync::Arc<std::sync::atomic::AtomicU8>,
) {
    platform::start(tx, is_recording, activation_mode);
}

#[cfg(not(windows))]
pub fn start(
    _tx: crossbeam_channel::Sender<HotkeyEvent>,
    _is_recording: std::sync::Arc<std::sync::atomic::AtomicBool>,
    _activation_mode: std::sync::Arc<std::sync::atomic::AtomicU8>,
) {
    // No-op on non-Windows platforms.
}
