//! Bounded vocabulary prompts passed to Whisper's model tokenizer.
//!
//! Tokenize-then-truncate approach adapted from the MIT-licensed ActuallyFree
//! reference at commit e091a7a99b0681553b8ef6bb6a8e4b3948f8a940,
//! frontend/src-tauri/src/whisper_engine/whisper_engine.rs::tokenize_initial_prompt.
//! Existing repository LICENSE.md retains its Zackriya Solutions MIT notice.

// Whisper's text context reserves half of the usual 448-token window for
// the initial prompt. Count model tokens, not words or Unicode characters.
pub(crate) const MAX_INITIAL_PROMPT_TOKENS: usize = 224;

pub(crate) fn tokenize_prompt<Token, Error>(
    initial_prompt: Option<&str>,
    tokenize: impl FnOnce(&str, usize) -> Result<Vec<Token>, Error>,
) -> Result<(Vec<Token>, bool), Error> {
    let Some(prompt) = initial_prompt.filter(|text| !text.trim().is_empty()) else {
        return Ok((Vec::new(), false));
    };
    // UTF-8 byte length is a safe upper bound for the model's byte-level BPE
    // tokenizer. Giving it the eventual 224-token cap as capacity would fail
    // before we could truncate a larger vocabulary safely.
    let mut tokens = tokenize(prompt, prompt.len().saturating_add(1))?;
    let truncated = tokens.len() > MAX_INITIAL_PROMPT_TOKENS;
    tokens.truncate(MAX_INITIAL_PROMPT_TOKENS);
    Ok((tokens, truncated))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_or_unicode_whitespace_prompt_never_calls_the_tokenizer() {
        for prompt in [None, Some(""), Some(" \t\n\u{2003}\u{00a0}")] {
            let result = tokenize_prompt(prompt, |_, _| -> Result<Vec<i32>, &str> {
                panic!("empty vocabulary must not call native tokenization")
            })
            .unwrap();
            assert!(result.0.is_empty());
            assert!(!result.1);
        }
    }

    #[test]
    fn unicode_prompt_uses_utf8_byte_capacity_and_preserves_text() {
        let prompt = "КриптоПро, OpenAI, 张伟 🦀";
        let result = tokenize_prompt(Some(prompt), |text, capacity| {
            assert_eq!(text, prompt);
            assert_eq!(capacity, prompt.len() + 1);
            assert!(capacity > prompt.chars().count() + 1);
            Ok::<_, &str>(vec![71, 52, 99])
        })
        .unwrap();
        assert_eq!(result, (vec![71, 52, 99], false));
    }

    #[test]
    fn caps_actual_tokens_after_tokenization_preserving_priority_order() {
        let result = tokenize_prompt(Some("short text"), |_, _| {
            Ok::<_, &str>((0..300_i32).collect::<Vec<_>>())
        })
        .unwrap();
        assert_eq!(result.0, (0..224_i32).collect::<Vec<_>>());
        assert!(result.1);
    }

    #[test]
    fn exact_token_limit_is_not_reported_as_truncation() {
        let result = tokenize_prompt(Some("vocabulary"), |_, _| {
            Ok::<_, &str>(vec![5; MAX_INITIAL_PROMPT_TOKENS])
        })
        .unwrap();
        assert_eq!(result.0.len(), 224);
        assert!(!result.1);
    }

    #[test]
    fn tokenizer_failure_propagates_instead_of_silently_dropping_hints() {
        let result = tokenize_prompt(Some("vocabulary"), |_, _| {
            Err::<Vec<i32>, _>("invalid text")
        });
        assert_eq!(result.unwrap_err(), "invalid text");
    }
}
