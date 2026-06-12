use std::process::{Command, Stdio};
#[cfg(windows)]
use std::os::windows::process::CommandExt;

use crate::settings;

fn language_pair(mode: &str) -> Option<(&'static str, &'static str)> {
    match mode {
        settings::TRANSLATION_PT_EN => Some(("Portuguese", "English")),
        settings::TRANSLATION_PT_ES => Some(("Portuguese", "Spanish")),
        settings::TRANSLATION_EN_PT => Some(("English", "Portuguese")),
        settings::TRANSLATION_EN_ES => Some(("English", "Spanish")),
        settings::TRANSLATION_ES_PT => Some(("Spanish", "Portuguese")),
        settings::TRANSLATION_ES_EN => Some(("Spanish", "English")),
        _ => None,
    }
}

fn claude_candidates() -> Vec<String> {
    let mut out = vec!["claude".to_string(), "claude.exe".to_string()];
    if let Ok(home) = std::env::var("USERPROFILE") {
        out.push(format!("{}\\.local\\bin\\claude.exe", home));
    }
    out
}

pub fn translate_if_needed(text: &str, mode: &str) -> Result<String, String> {
    if mode == settings::TRANSLATION_OFF {
        return Ok(text.to_string());
    }

    let (source, target) =
        language_pair(mode).ok_or_else(|| format!("Invalid translation mode: {}", mode))?;

    let prompt = format!(
        "Translate from {} to {}. Return ONLY the translated text, nothing else.\n\n{}",
        source, target, text
    );

    let args = [
        "-p",
        &prompt,
        "--output-format", "text",
        "--max-turns", "1",
        "--model", "haiku",
    ];

    let mut last_err = String::new();

    for bin in claude_candidates() {
        let result = Command::new(&bin)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .creation_flags(0x08000000) // CREATE_NO_WINDOW
            .output();

        match result {
            Ok(output) => {
                if !output.status.success() {
                    last_err = format!(
                        "{} exited {}: {}",
                        bin,
                        output.status,
                        String::from_utf8_lossy(&output.stderr)
                    );
                    continue;
                }

                let translated = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if translated.is_empty() {
                    last_err = format!("{} returned empty output", bin);
                    continue;
                }

                return Ok(translated);
            }
            Err(e) => {
                last_err = format!("Failed to run {}: {}", bin, e);
            }
        }
    }

    Err(format!("Translation failed. {}", last_err))
}
