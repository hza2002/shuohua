use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextStats {
    pub words: usize,
}

pub fn compute(text: &str) -> TextStats {
    TextStats {
        words: count_words(text),
    }
}

pub fn count_words(text: &str) -> usize {
    text.split_word_bounds()
        .filter(|part| !part.trim().is_empty())
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_english_words_as_one_each() {
        assert_eq!(compute("Hello world").words, 2);
    }

    #[test]
    fn counts_chinese_chars_and_punctuation_as_words() {
        assert_eq!(compute("你好。").words, 3);
    }

    #[test]
    fn counts_mixed_text_with_unicode_boundaries() {
        let stats = compute("Hello world，你好。");
        assert_eq!(stats.words, 6);
    }

    #[test]
    fn ignores_whitespace_for_words() {
        assert_eq!(compute("Hi  Rust").words, 2);
    }
}
