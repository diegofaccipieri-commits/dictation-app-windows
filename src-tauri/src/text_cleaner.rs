use regex::Regex;
use std::sync::LazyLock;

// ---------------------------------------------------------------------------
// Punctuation commands (order matters: longer phrases first)
// ---------------------------------------------------------------------------

static PUNCTUATION_COMMANDS: &[(&str, &str)] = &[
    ("ponto de interrogação", "?"),
    ("ponto de exclamação", "!"),
    ("ponto e vírgula", ";"),
    ("ponto final", "."),
    ("dois pontos", ":"),
    ("nova linha", "\n"),
    ("parágrafo", "\n\n"),
    ("reticências", "..."),
    ("interrogação", "?"),
    ("exclamação", "!"),
    ("vírgula", ","),
    ("question mark", "?"),
    ("exclamation mark", "!"),
    ("semicolon", ";"),
    ("comma", ","),
    ("colon", ":"),
    ("period", "."),
    ("dot", "."),
];

// ---------------------------------------------------------------------------
// Known Whisper hallucinations
// ---------------------------------------------------------------------------

static HALLUCINATIONS: &[&str] = &[
    "thank you",
    "thank you.",
    "thank you!",
    "thanks for watching",
    "thanks for watching!",
    "thank you for watching",
    "thank you for watching.",
    "thanks for listening",
    "please subscribe",
    "subtitles by",
    "transcribed by",
    "obrigado",
    "obrigada",
    "obrigado.",
    "obrigada.",
];

// ---------------------------------------------------------------------------
// LazyLock-compiled regexes
// ---------------------------------------------------------------------------

static WHISPER_TOKEN_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<\|[^|]*\|>").expect("invalid whisper token regex"));

/// Build a case-insensitive regex for a punctuation command word.
/// Pattern: (^|\s)WORD[.,!?;:]*(?=[\s.,!?;:]|$)
fn build_command_regex(word: &str) -> Regex {
    let escaped = regex::escape(word);
    let pattern = format!(r"(?i)(^|\s){}[.,!?;:]*(?=[\s.,!?;:]|$)", escaped);
    Regex::new(&pattern).expect("invalid punctuation command regex")
}

static COMMAND_REGEXES: LazyLock<Vec<(Regex, &str)>> = LazyLock::new(|| {
    PUNCTUATION_COMMANDS
        .iter()
        .map(|(word, symbol)| (build_command_regex(word), *symbol))
        .collect()
});

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Main entry point: applies all cleaning steps in order.
pub fn clean(text: &str) -> String {
    let step1 = strip_whisper_tokens(text);
    let step2 = strip_hallucinations(&step1);
    let step3 = apply_punctuation_commands(&step2);
    step3.trim().to_string()
}

/// Remove Whisper special tokens like `<|en|>`, `<|0.00|>`, etc.
pub fn strip_whisper_tokens(text: &str) -> String {
    WHISPER_TOKEN_RE.replace_all(text, "").to_string()
}

/// If the entire text is a known hallucination, return empty string.
/// If the text ends with a hallucination phrase, strip it.
pub fn strip_hallucinations(text: &str) -> String {
    let trimmed = text.trim();
    let lower = trimmed.to_lowercase();

    // Whole text is a hallucination
    for h in HALLUCINATIONS {
        if lower == *h {
            return String::new();
        }
    }

    // Text ends with a hallucination — strip it
    let mut result = trimmed.to_string();
    for h in HALLUCINATIONS {
        let lower_result = result.to_lowercase();
        if lower_result.ends_with(h) {
            let new_len = result.len() - h.len();
            result.truncate(new_len);
            result = result.trim_end().to_string();
        }
    }

    result
}

/// Replace spoken punctuation commands with their symbols, then capitalize.
pub fn apply_punctuation_commands(text: &str) -> String {
    let mut result = text.to_string();

    for (re, symbol) in COMMAND_REGEXES.iter() {
        // The regex captures an optional leading whitespace in group 1.
        // We want to replace the whole match with just the symbol (no leading space).
        result = re
            .replace_all(&result, |_caps: &regex::Captures| symbol.to_string())
            .to_string();
    }

    capitalize_after_sentence_end(&result)
}

/// Capitalize the first letter after sentence-ending punctuation (. ? !).
pub fn capitalize_after_sentence_end(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut capitalize_next = true; // capitalize first char of the text too

    for ch in text.chars() {
        if capitalize_next && ch.is_alphabetic() {
            for upper in ch.to_uppercase() {
                result.push(upper);
            }
            capitalize_next = false;
        } else {
            result.push(ch);
            if ch == '.' || ch == '?' || ch == '!' {
                capitalize_next = true;
            } else if !ch.is_whitespace() && ch != '\n' {
                capitalize_next = false;
            }
        }
    }

    result
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_whisper_tokens() {
        assert_eq!(
            strip_whisper_tokens("<|en|>Hello world<|0.00|>"),
            "Hello world"
        );
        assert_eq!(strip_whisper_tokens("No tokens here"), "No tokens here");
    }

    #[test]
    fn test_strip_hallucinations_full() {
        assert_eq!(strip_hallucinations("Thank you."), "");
        assert_eq!(strip_hallucinations("  obrigado  "), "");
        assert_eq!(strip_hallucinations("Thanks for watching!"), "");
    }

    #[test]
    fn test_strip_hallucinations_trailing() {
        assert_eq!(strip_hallucinations("Hello world thank you"), "Hello world");
        assert_eq!(strip_hallucinations("Great talk obrigado."), "Great talk");
    }

    #[test]
    fn test_punctuation_commands_pt() {
        let input = "Olá vírgula como vai ponto de interrogação";
        let output = apply_punctuation_commands(input);
        assert!(output.contains(','));
        assert!(output.contains('?'));
    }

    #[test]
    fn test_punctuation_commands_en() {
        let input = "Hello comma how are you question mark";
        let output = apply_punctuation_commands(input);
        assert!(output.contains(','));
        assert!(output.contains('?'));
    }

    #[test]
    fn test_capitalize_after_sentence() {
        let input = "hello. world? yes! ok";
        let output = capitalize_after_sentence_end(input);
        assert_eq!(output, "Hello. World? Yes! Ok");
    }

    #[test]
    fn test_clean_full_pipeline() {
        let input = "<|en|>hello comma this is a test period thank you";
        let output = clean(input);
        // Should strip whisper token, apply punctuation, strip hallucination, capitalize
        assert!(output.starts_with('H'));
        assert!(output.contains(','));
        assert!(output.contains('.'));
        assert!(!output.contains("thank you"));
    }
}
