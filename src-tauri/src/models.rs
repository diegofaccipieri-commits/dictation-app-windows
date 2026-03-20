// Model download and cache management
//
// Downloads and caches whisper.cpp GGML models from HuggingFace.
// Models are stored in %APPDATA%/DictationApp/models/ (via dirs::data_dir()).

use reqwest::blocking::Client;
use std::fs;
use std::io::Read;
use std::path::PathBuf;

const BASE_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/";

/// Returns the filename for a given model name.
fn model_filename(name: &str) -> &'static str {
    match name {
        "small" => "ggml-small.bin",
        "turbo" => "ggml-large-v3-turbo.bin",
        _ => panic!("Unknown model: {name}. Use \"small\" or \"turbo\"."),
    }
}

/// Returns the directory where models are cached.
pub fn models_dir() -> PathBuf {
    let base = dirs::data_dir().expect("Failed to resolve data directory");
    base.join("DictationApp").join("models")
}

/// Returns the expected local path for a model.
pub fn model_path(name: &str) -> PathBuf {
    models_dir().join(model_filename(name))
}

/// Checks whether a model has already been downloaded.
pub fn is_downloaded(name: &str) -> bool {
    model_path(name).exists()
}

/// Downloads a model from HuggingFace with progress reporting.
///
/// - Writes to a `.tmp` file first, then renames on success (atomic).
/// - `on_progress(downloaded_bytes, total_bytes)` is called after each chunk.
/// - Returns the final path on success.
pub fn download_model(
    name: &str,
    on_progress: impl Fn(u64, u64),
) -> Result<PathBuf, String> {
    let filename = model_filename(name);
    let url = format!("{BASE_URL}{filename}");
    let dest = model_path(name);

    // Already cached
    if dest.exists() {
        let size = fs::metadata(&dest)
            .map(|m| m.len())
            .unwrap_or(0);
        on_progress(size, size);
        return Ok(dest);
    }

    // Ensure models directory exists
    let dir = models_dir();
    fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create models directory: {e}"))?;

    let tmp_path = dir.join(format!("{filename}.tmp"));

    // Start download
    let client = Client::new();
    let response = client
        .get(&url)
        .send()
        .map_err(|e| format!("Failed to start download: {e}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "Download failed with status: {}",
            response.status()
        ));
    }

    let total_bytes = response.content_length().unwrap_or(0);
    let mut reader = response;
    let mut file = fs::File::create(&tmp_path)
        .map_err(|e| format!("Failed to create temp file: {e}"))?;

    let mut downloaded: u64 = 0;
    let mut buffer = [0u8; 64 * 1024]; // 64KB

    loop {
        let bytes_read = reader
            .read(&mut buffer)
            .map_err(|e| format!("Error reading download stream: {e}"))?;

        if bytes_read == 0 {
            break;
        }

        std::io::Write::write_all(&mut file, &buffer[..bytes_read])
            .map_err(|e| format!("Error writing to temp file: {e}"))?;

        downloaded += bytes_read as u64;
        on_progress(downloaded, total_bytes);
    }

    // Flush and close before rename
    drop(file);

    // Atomic rename
    fs::rename(&tmp_path, &dest)
        .map_err(|e| format!("Failed to rename temp file to final path: {e}"))?;

    Ok(dest)
}
