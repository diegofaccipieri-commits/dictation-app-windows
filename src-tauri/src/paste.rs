// Clipboard and auto-paste
//
// Copies text to the system clipboard (cross-platform via arboard)
// and simulates Ctrl+V on Windows via Win32 SendInput.

use arboard::Clipboard;

/// Copy text to the system clipboard.
pub fn copy_to_clipboard(text: &str) -> Result<(), String> {
    let mut clipboard = Clipboard::new().map_err(|e| e.to_string())?;
    clipboard.set_text(text).map_err(|e| e.to_string())?;
    Ok(())
}

/// Simulate Ctrl+V keystroke to paste into the currently focused application.
#[cfg(windows)]
pub fn paste_into_focused_app() -> Result<(), String> {
    use windows::Win32::UI::Input::KeyboardAndMouse::*;

    let inputs = [
        make_key_input(VK_CONTROL, KEYBD_EVENT_FLAGS(0)),  // Ctrl down
        make_key_input(VK_V, KEYBD_EVENT_FLAGS(0)),        // V down
        make_key_input(VK_V, KEYEVENTF_KEYUP),             // V up
        make_key_input(VK_CONTROL, KEYEVENTF_KEYUP),       // Ctrl up
    ];

    unsafe {
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }

    Ok(())
}

/// No-op on non-Windows (allows development/compilation on macOS).
#[cfg(not(windows))]
pub fn paste_into_focused_app() -> Result<(), String> {
    Ok(())
}

/// Copy text to clipboard then simulate paste with a short delay between.
pub fn copy_and_paste(text: &str) -> Result<(), String> {
    copy_to_clipboard(text)?;
    std::thread::sleep(std::time::Duration::from_millis(100));
    paste_into_focused_app()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Windows helper
// ---------------------------------------------------------------------------

#[cfg(windows)]
fn make_key_input(vk: VIRTUAL_KEY, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    use windows::Win32::UI::Input::KeyboardAndMouse::*;

    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}
