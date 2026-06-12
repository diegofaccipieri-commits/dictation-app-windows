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

fn get_api_key() -> Result<String, String> {
    if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
        if !key.trim().is_empty() {
            return Ok(key.trim().to_string());
        }
    }

    let settings = settings::load();
    if !settings.anthropic_api_key.is_empty() {
        return Ok(settings.anthropic_api_key);
    }

    Err("ANTHROPIC_API_KEY not set. Configure it as an environment variable or in app settings.".to_string())
}

pub fn translate_if_needed(text: &str, mode: &str) -> Result<String, String> {
    if mode == settings::TRANSLATION_OFF {
        return Ok(text.to_string());
    }

    let (source, target) =
        language_pair(mode).ok_or_else(|| format!("Invalid translation mode: {}", mode))?;

    let api_key = get_api_key()?;

    let body = serde_json::json!({
        "model": "claude-haiku-4-5-20251001",
        "max_tokens": 4096,
        "messages": [{
            "role": "user",
            "content": format!(
                "Translate the following text from {} to {}. Return ONLY the translated text, nothing else.\n\n{}",
                source, target, text
            )
        }]
    });

    let client = reqwest::blocking::Client::new();
    let response = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", &api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .body(body.to_string())
        .send()
        .map_err(|e| format!("HTTP request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().unwrap_or_default();
        return Err(format!("Claude API error {}: {}", status, text));
    }

    let json: serde_json::Value = response
        .json()
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    let translated = json["content"][0]["text"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_string();

    if translated.is_empty() {
        return Err("Claude returned empty translation".to_string());
    }

    Ok(translated)
}
