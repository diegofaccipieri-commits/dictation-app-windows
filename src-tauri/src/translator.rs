use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

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

fn output_path() -> PathBuf {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    std::env::temp_dir().join(format!("dictation_codex_translation_{}.txt", ts))
}

fn codex_candidates() -> Vec<String> {
    let mut out = Vec::new();
    out.push("codex".to_string());
    out.push("codex.exe".to_string());
    if let Ok(custom) = std::env::var("CODEX_BIN") {
        if !custom.trim().is_empty() {
            out.push(custom);
        }
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
        "You are a translation engine. Translate from {} to {}.\nReturn only the translated text, with no extra commentary.\n\nText:\n{}",
        source, target, text
    );

    let out_path = output_path();
    let args = vec![
        "-a".to_string(),
        "never".to_string(),
        "exec".to_string(),
        "-m".to_string(),
        "gpt-5.3-codex".to_string(),
        "-c".to_string(),
        "model_reasoning_effort=\"medium\"".to_string(),
        "--skip-git-repo-check".to_string(),
        "--ephemeral".to_string(),
        "--sandbox".to_string(),
        "read-only".to_string(),
        "--color".to_string(),
        "never".to_string(),
        "-o".to_string(),
        out_path.to_string_lossy().to_string(),
        prompt,
    ];

    let mut last_err = String::new();

    for bin in codex_candidates() {
        let mut cmd = Command::new(&bin);
        cmd.args(&args);
        cmd.env_remove("CLAUDECODE");
        cmd.env_remove("CLAUDE_CODE");
        cmd.env_remove("CLAUDE_CODE_SESSION");

        match cmd.output() {
            Ok(output) => {
                if !output.status.success() {
                    last_err = format!(
                        "{} exited with status {}: {}",
                        bin,
                        output.status,
                        String::from_utf8_lossy(&output.stderr)
                    );
                    continue;
                }

                let translated = std::fs::read_to_string(&out_path).unwrap_or_default();
                let translated = translated.trim().to_string();
                let _ = std::fs::remove_file(&out_path);

                if translated.is_empty() {
                    last_err = format!("{} returned empty translation output", bin);
                    continue;
                }

                return Ok(translated);
            }
            Err(e) => {
                last_err = format!("Failed to run {}: {}", bin, e);
            }
        }
    }

    let _ = std::fs::remove_file(&out_path);
    Err(format!("Translation failed. {}", last_err))
}
