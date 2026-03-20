// Batch transcription of audio/video files with timestamps and speaker diarization.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::transcriber::{Segment, Transcriber};

/// Audio/video extensions we support for batch transcription.
const SUPPORTED_EXTENSIONS: &[&str] = &["wav", "mp3", "m4a", "mp4", "mov", "webm", "ogg", "flac"];

/// Speaker gap threshold in seconds. If the silence between two consecutive
/// segments exceeds this value we switch the active speaker label.
const SPEAKER_GAP_THRESHOLD: f64 = 2.0;

// ---------------------------------------------------------------------------
// Timestamp / date helpers
// ---------------------------------------------------------------------------

/// Format seconds as `MM:SS.D` (e.g. "05:23.4").
pub fn format_timestamp(seconds: f64) -> String {
    let total_secs = seconds.max(0.0);
    let mins = (total_secs / 60.0).floor() as u64;
    let secs = total_secs - (mins as f64 * 60.0);
    // One decimal place for the fractional second.
    format!("{:02}:{:04.1}", mins, secs)
}

/// Format a `SystemTime` as `YYYY-MM-DD`.
fn format_date(time: SystemTime) -> String {
    let duration = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();

    // Simple civil-date derivation (no leap-second pedantry).
    let days = (secs / 86400) as i64;
    let (year, month, day) = days_to_ymd(days);
    format!("{:04}-{:02}-{:02}", year, month, day)
}

/// Convert days since Unix epoch to (year, month, day).
fn days_to_ymd(days_since_epoch: i64) -> (i64, u32, u32) {
    // Algorithm from Howard Hinnant (public domain).
    let z = days_since_epoch + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    (year, m, d)
}

/// Format total seconds as `MM:SS` for the header duration line.
fn format_duration(total_seconds: f64) -> String {
    let total = total_seconds.max(0.0).round() as u64;
    let mins = total / 60;
    let secs = total % 60;
    format!("{:02}:{:02}", mins, secs)
}

// ---------------------------------------------------------------------------
// Duration estimation
// ---------------------------------------------------------------------------

/// Try to get the audio duration for a file. For WAV we read the header via
/// `hound`; for everything else we fall back to the last segment end time
/// (passed in as `fallback`).
fn get_duration(path: &Path, fallback: f64) -> f64 {
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        if ext.eq_ignore_ascii_case("wav") {
            if let Ok(reader) = hound::WavReader::open(path) {
                let spec = reader.spec();
                let samples = reader.len() as f64;
                let rate = spec.sample_rate as f64;
                let channels = spec.channels as f64;
                if rate > 0.0 && channels > 0.0 {
                    return samples / channels / rate;
                }
            }
        }
    }
    fallback
}

// ---------------------------------------------------------------------------
// Speaker assignment
// ---------------------------------------------------------------------------

struct LabelledSegment {
    start: f64,
    end: f64,
    speaker: String,
    text: String,
}

/// Walk through segments and assign "Falante A" / "Falante B" based on
/// pause-gap heuristics.
fn assign_speakers(segments: &[Segment]) -> Vec<LabelledSegment> {
    if segments.is_empty() {
        return Vec::new();
    }

    let speakers = ["Falante A", "Falante B"];
    let mut current_idx: usize = 0; // index into `speakers`
    let mut result: Vec<LabelledSegment> = Vec::with_capacity(segments.len());

    for (i, seg) in segments.iter().enumerate() {
        if i > 0 {
            let prev_end = segments[i - 1].end;
            let gap = seg.start - prev_end;
            if gap > SPEAKER_GAP_THRESHOLD {
                current_idx = 1 - current_idx; // toggle 0 <-> 1
            }
        }

        result.push(LabelledSegment {
            start: seg.start,
            end: seg.end,
            speaker: speakers[current_idx].to_string(),
            text: seg.text.trim().to_string(),
        });
    }

    result
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Transcribe a single audio/video file, write a `.txt` alongside it, and
/// return the path to the generated text file.
pub fn transcribe_single_file(
    path: &Path,
    transcriber: &Transcriber,
    on_progress: impl Fn(&str),
) -> Result<PathBuf, String> {
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    on_progress(&format!("Transcrevendo: {}", file_name));

    // 1. Transcribe
    let segments = transcriber.transcribe_with_segments(path)?;

    // 2. Speaker assignment
    let labelled = assign_speakers(&segments);

    // 3. Duration
    let last_end = segments.last().map(|s| s.end).unwrap_or(0.0);
    let duration = get_duration(path, last_end);

    // 4. Date – prefer file modification time, fall back to now.
    let date_str = fs::metadata(path)
        .and_then(|m| m.modified())
        .map(|t| format_date(t))
        .unwrap_or_else(|_| format_date(SystemTime::now()));

    // 5. Build output text
    let mut output = String::new();
    output.push_str(&format!("Transcrição: {}\n", file_name));
    output.push_str(&format!("Duração: {}\n", format_duration(duration)));
    output.push_str(&format!("Data: {}\n", date_str));
    output.push_str("\n---\n\n");

    for seg in &labelled {
        output.push_str(&format!(
            "[{} - {}] {}: {}\n",
            format_timestamp(seg.start),
            format_timestamp(seg.end),
            seg.speaker,
            seg.text,
        ));
    }

    // 6. Write .txt
    let txt_path = path.with_extension("txt");
    fs::write(&txt_path, &output)
        .map_err(|e| format!("Failed to write {}: {}", txt_path.display(), e))?;

    Ok(txt_path)
}

/// Transcribe every supported audio/video file inside `folder`.
/// Returns `(completed, total)`.
pub fn transcribe_folder(
    folder: &Path,
    transcriber: &Transcriber,
    on_progress: impl Fn(&str),
) -> Result<(usize, usize), String> {
    let entries = fs::read_dir(folder)
        .map_err(|e| format!("Cannot read folder {}: {}", folder.display(), e))?;

    let mut files: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if SUPPORTED_EXTENSIONS
                    .iter()
                    .any(|&s| s.eq_ignore_ascii_case(ext))
                {
                    files.push(path);
                }
            }
        }
    }

    files.sort();
    let total = files.len();
    let mut completed: usize = 0;

    for (i, file_path) in files.iter().enumerate() {
        let name = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?");
        on_progress(&format!("Transcrevendo {}/{}: {}", i + 1, total, name));

        match transcribe_single_file(file_path, transcriber, &on_progress) {
            Ok(_) => completed += 1,
            Err(e) => {
                on_progress(&format!("Erro em {}: {}", name, e));
            }
        }
    }

    Ok((completed, total))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_timestamp() {
        assert_eq!(format_timestamp(0.0), "00:00.0");
        assert_eq!(format_timestamp(5.2), "00:05.2");
        assert_eq!(format_timestamp(63.4), "01:03.4");
        assert_eq!(format_timestamp(323.4), "05:23.4");
    }

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(0.0), "00:00");
        assert_eq!(format_duration(323.0), "05:23");
        assert_eq!(format_duration(60.0), "01:00");
    }

    #[test]
    fn test_speaker_assignment_no_gap() {
        let segs = vec![
            Segment { text: "Hello".into(), start: 0.0, end: 2.0 },
            Segment { text: "World".into(), start: 2.5, end: 4.0 },
        ];
        let labelled = assign_speakers(&segs);
        assert_eq!(labelled.len(), 2);
        assert_eq!(labelled[0].speaker, "Falante A");
        assert_eq!(labelled[1].speaker, "Falante A"); // gap 0.5 < 2.0
    }

    #[test]
    fn test_speaker_assignment_with_gap() {
        let segs = vec![
            Segment { text: "Hello".into(), start: 0.0, end: 2.0 },
            Segment { text: "World".into(), start: 5.0, end: 7.0 },
            Segment { text: "Again".into(), start: 7.5, end: 9.0 },
            Segment { text: "Switch".into(), start: 12.0, end: 14.0 },
        ];
        let labelled = assign_speakers(&segs);
        assert_eq!(labelled[0].speaker, "Falante A");
        assert_eq!(labelled[1].speaker, "Falante B"); // gap 3.0 > 2.0
        assert_eq!(labelled[2].speaker, "Falante B"); // gap 0.5
        assert_eq!(labelled[3].speaker, "Falante A"); // gap 3.0 > 2.0
    }

    #[test]
    fn test_days_to_ymd_epoch() {
        let (y, m, d) = days_to_ymd(0);
        assert_eq!((y, m, d), (1970, 1, 1));
    }

    #[test]
    fn test_format_date_epoch() {
        let s = format_date(SystemTime::UNIX_EPOCH);
        assert_eq!(s, "1970-01-01");
    }
}
