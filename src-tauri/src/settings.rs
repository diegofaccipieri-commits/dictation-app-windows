use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

pub const MODEL_TINY: &str = "tiny";
pub const MODEL_SMALL: &str = "small";
pub const MODEL_TURBO: &str = "turbo";

pub const TRANSLATION_OFF: &str = "off";
pub const TRANSLATION_PT_EN: &str = "pt_en";
pub const TRANSLATION_PT_ES: &str = "pt_es";
pub const TRANSLATION_EN_PT: &str = "en_pt";
pub const TRANSLATION_EN_ES: &str = "en_es";
pub const TRANSLATION_ES_PT: &str = "es_pt";
pub const TRANSLATION_ES_EN: &str = "es_en";

pub const ACTIVATION_CTRL: &str = "ctrl";
pub const ACTIVATION_WIN: &str = "win";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub live_model: String,
    pub batch_model: String,
    pub translation_mode: String,
    pub activation_key: String,
    #[serde(default)]
    pub anthropic_api_key: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            live_model: MODEL_TINY.to_string(),
            batch_model: MODEL_TURBO.to_string(),
            translation_mode: TRANSLATION_OFF.to_string(),
            activation_key: ACTIVATION_CTRL.to_string(),
            anthropic_api_key: String::new(),
        }
    }
}

pub fn settings_dir() -> PathBuf {
    let base = dirs::data_dir().unwrap_or_else(std::env::temp_dir);
    base.join("DictationApp")
}

pub fn settings_path() -> PathBuf {
    settings_dir().join("settings.json")
}

pub fn normalize(mut s: AppSettings) -> AppSettings {
    if s.live_model != MODEL_TINY && s.live_model != MODEL_SMALL && s.live_model != MODEL_TURBO {
        s.live_model = MODEL_TINY.to_string();
    }
    if s.batch_model != MODEL_TINY && s.batch_model != MODEL_SMALL && s.batch_model != MODEL_TURBO {
        s.batch_model = MODEL_TURBO.to_string();
    }
    if !is_translation_mode_valid(&s.translation_mode) {
        s.translation_mode = TRANSLATION_OFF.to_string();
    }
    if s.activation_key != ACTIVATION_CTRL && s.activation_key != ACTIVATION_WIN {
        s.activation_key = ACTIVATION_CTRL.to_string();
    }
    s
}

pub fn load() -> AppSettings {
    let path = settings_path();
    let data = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(_) => return AppSettings::default(),
    };

    let parsed: AppSettings = match serde_json::from_str(&data) {
        Ok(value) => value,
        Err(_) => return AppSettings::default(),
    };

    normalize(parsed)
}

pub fn save(settings: &AppSettings) -> Result<(), String> {
    let normalized = normalize(settings.clone());
    let dir = settings_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("Failed to create settings dir: {}", e))?;

    let data = serde_json::to_string_pretty(&normalized)
        .map_err(|e| format!("Failed to serialize settings: {}", e))?;
    fs::write(settings_path(), data).map_err(|e| format!("Failed to write settings: {}", e))
}

pub fn is_translation_mode_valid(mode: &str) -> bool {
    matches!(
        mode,
        TRANSLATION_OFF
            | TRANSLATION_PT_EN
            | TRANSLATION_PT_ES
            | TRANSLATION_EN_PT
            | TRANSLATION_EN_ES
            | TRANSLATION_ES_PT
            | TRANSLATION_ES_EN
    )
}
