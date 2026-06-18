//! 中文语音输入文本过滤。保守处理标点、空白、segment 边界和少量语气词。

use async_trait::async_trait;

use super::{AppContext, PipelineText, PostError, PostProcessor};

pub struct ZhFilter {
    name: String,
    fillers: Vec<String>,
}

impl ZhFilter {
    #[cfg(test)]
    pub fn new(fillers: &[&str]) -> Self {
        Self::with_name("filler", fillers)
    }

    pub fn with_name(name: impl Into<String>, fillers: &[&str]) -> Self {
        Self {
            name: name.into(),
            fillers: fillers.iter().map(|s| (*s).to_string()).collect(),
        }
    }

    #[cfg(test)]
    pub fn default_patterns() -> Self {
        Self::new(&["嗯", "呃", "啊"])
    }
}

#[async_trait]
impl PostProcessor for ZhFilter {
    fn name(&self) -> &str {
        &self.name
    }

    async fn process(
        &self,
        input: PipelineText,
        _ctx: &AppContext,
    ) -> Result<PipelineText, PostError> {
        let text = filter_zh_speech(&input, &self.fillers);
        Ok(PipelineText { text, ..input })
    }
}

fn filter_zh_speech(input: &PipelineText, fillers: &[String]) -> String {
    let segment_source = if input.segments.is_empty() {
        vec![input.text.clone()]
    } else {
        input.segments.clone()
    };
    let segments = segment_source
        .iter()
        .map(|s| normalize_zh_speech_segment(s))
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>();
    let joined = join_zh_speech_segments(&segments);
    let cleaned = normalize_zh_speech_text(&joined);
    let without_fillers = remove_conservative_fillers(&cleaned, fillers);
    final_cleanup(&without_fillers)
}

fn normalize_zh_speech_segment(text: &str) -> String {
    let text = normalize_space_chars(text);
    let text = map_ascii_speech_punct_to_zh(&text);
    let text = collapse_punctuation_runs(&text);
    let text = normalize_punctuation_spacing(&text);
    text.trim().to_string()
}

fn normalize_zh_speech_text(text: &str) -> String {
    let text = normalize_space_chars(text);
    let text = map_ascii_speech_punct_to_zh(&text);
    let text = collapse_punctuation_runs(&text);
    let text = normalize_punctuation_spacing(&text);
    let text = clean_edge_punctuation(&text);
    text.trim().to_string()
}

fn final_cleanup(text: &str) -> String {
    let text = collapse_punctuation_runs(text);
    let text = normalize_punctuation_spacing(&text);
    let text = clean_edge_punctuation(&text);
    text.trim().to_string()
}

fn normalize_space_chars(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\t' | '\n' | '\r' | '\u{3000}' => ' ',
            _ => c,
        })
        .collect()
}

fn map_ascii_speech_punct_to_zh(text: &str) -> String {
    let chars = text.chars().collect::<Vec<_>>();
    let mut out = String::with_capacity(text.len());
    for (idx, c) in chars.iter().copied().enumerate() {
        let prev = idx.checked_sub(1).and_then(|i| chars.get(i)).copied();
        let next = chars.get(idx + 1).copied();
        let mapped = match c {
            ',' => '，',
            '.' if prev.is_some_and(|c| c.is_ascii_alphanumeric())
                && next.is_some_and(|c| c.is_ascii_alphanumeric()) =>
            {
                '.'
            }
            '.' => '。',
            '?' => '？',
            '!' => '！',
            ';' => '；',
            ':' if chars.get(idx + 1) == Some(&'/') && chars.get(idx + 2) == Some(&'/') => ':',
            ':' => '：',
            '(' => '（',
            ')' => '）',
            _ => c,
        };
        out.push(mapped);
    }
    out
}

fn join_zh_speech_segments(segments: &[String]) -> String {
    let mut out = String::new();
    for segment in segments {
        let right = segment.trim();
        if right.is_empty() {
            continue;
        }
        if out.is_empty() {
            out.push_str(right);
            continue;
        }
        append_segment(&mut out, right);
    }
    out
}

fn append_segment(out: &mut String, right: &str) {
    trim_end_in_place(out);
    let right = right.trim_start();
    if right.is_empty() {
        return;
    }

    let Some(left_last) = out.chars().last() else {
        out.push_str(right);
        return;
    };
    let Some(right_first) = right.chars().next() else {
        return;
    };

    if should_drop_right_boundary_punct(left_last, right_first) {
        out.push_str(trim_start_zh_punctuation(right));
    } else if is_pause_punctuation(left_last) && is_sentence_punctuation(right_first) {
        pop_last_char(out);
        out.push(right_first);
        out.push_str(&right[right_first.len_utf8()..]);
    } else {
        if left_last.is_ascii_alphanumeric() && right_first.is_ascii_alphanumeric() {
            out.push(' ');
        }
        out.push_str(right);
    }
}

fn should_drop_right_boundary_punct(left: char, right: char) -> bool {
    (is_sentence_punctuation(left) && is_zh_speech_punctuation(right))
        || (is_pause_punctuation(left) && is_pause_punctuation(right))
        || (is_right_paren(left) && is_right_paren(right))
        || (is_left_paren(left) && is_left_paren(right))
}

fn trim_start_zh_punctuation(text: &str) -> &str {
    text.trim_start_matches(is_zh_speech_punctuation)
}

fn collapse_punctuation_runs(text: &str) -> String {
    let chars = text.chars().collect::<Vec<_>>();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if is_collapsible_punctuation(c) {
            let mut best = c;
            i += 1;
            while i < chars.len() && is_collapsible_punctuation(chars[i]) {
                best = dominant_punctuation(best, chars[i]);
                i += 1;
            }
            out.push(best);
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

fn dominant_punctuation(left: char, right: char) -> char {
    match (left, right) {
        (_, '？') | ('？', _) => '？',
        (_, '！') | ('！', _) => '！',
        (_, '。') | ('。', _) => '。',
        (_, '；') | ('；', _) => '；',
        (_, '：') | ('：', _) => '：',
        (_, '、') | ('、', _) => '、',
        _ => left,
    }
}

fn normalize_punctuation_spacing(text: &str) -> String {
    let chars = text.chars().collect::<Vec<_>>();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            let prev = out.chars().last();
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            let next = chars.get(i).copied();
            if should_keep_single_space(prev, next) {
                out.push(' ');
            }
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

fn should_keep_single_space(prev: Option<char>, next: Option<char>) -> bool {
    let (Some(prev), Some(next)) = (prev, next) else {
        return false;
    };
    if is_zh_speech_punctuation(next) || is_left_paren(next) || is_right_paren(prev) {
        return false;
    }
    if is_zh_speech_punctuation(prev) {
        return next.is_ascii_alphanumeric();
    }
    if is_cjk(prev) && is_cjk(next) {
        return false;
    }
    true
}

fn clean_edge_punctuation(text: &str) -> String {
    let text = text.trim_start_matches(['，', '。', '！', '？', '、', '；', '：']);
    let text = text.trim_end_matches(['，', '、', '；', '：']);
    text.to_string()
}

fn remove_conservative_fillers(text: &str, fillers: &[String]) -> String {
    if fillers.is_empty() {
        return text.to_string();
    }

    let chars = text.chars().collect::<Vec<_>>();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if let Some((filler, filler_len)) = match_filler(&chars, i, fillers) {
            let prev = out.chars().last();
            let next = chars.get(i + filler_len).copied();
            if should_remove_filler(&filler, prev, next, &chars, i, filler_len) {
                i += filler_len;
                while matches!(chars.get(i), Some('，' | '。')) {
                    i += 1;
                }
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn match_filler(chars: &[char], start: usize, fillers: &[String]) -> Option<(String, usize)> {
    fillers
        .iter()
        .filter_map(|filler| {
            let filler_chars = filler.chars().collect::<Vec<_>>();
            if filler_chars.is_empty() || start + filler_chars.len() > chars.len() {
                return None;
            }
            if chars[start..start + filler_chars.len()] == filler_chars[..] {
                Some((filler.clone(), filler_chars.len()))
            } else {
                None
            }
        })
        .max_by_key(|(_, len)| *len)
}

fn should_remove_filler(
    filler: &str,
    prev: Option<char>,
    next: Option<char>,
    chars: &[char],
    start: usize,
    len: usize,
) -> bool {
    if prev.is_some_and(|c| c.is_ascii_alphanumeric())
        || next.is_some_and(|c| c.is_ascii_alphanumeric())
    {
        return false;
    }
    if prev.is_none() || prev.is_some_and(is_zh_speech_punctuation) {
        return true;
    }
    if has_adjacent_same_filler(chars, start, len) {
        return true;
    }
    if filler == "啊" && next.is_none() && prev.is_some_and(is_cjk) {
        return false;
    }
    matches!(filler, "嗯" | "呃" | "啊")
}

fn has_adjacent_same_filler(chars: &[char], start: usize, len: usize) -> bool {
    start + len * 2 <= chars.len()
        && chars[start..start + len] == chars[start + len..start + len * 2]
}

fn trim_end_in_place(s: &mut String) {
    let trimmed_len = s.trim_end().len();
    s.truncate(trimmed_len);
}

fn pop_last_char(s: &mut String) {
    if let Some((idx, _)) = s.char_indices().next_back() {
        s.truncate(idx);
    }
}

fn is_sentence_punctuation(c: char) -> bool {
    matches!(c, '。' | '！' | '？')
}

fn is_pause_punctuation(c: char) -> bool {
    matches!(c, '，' | '、' | '；' | '：')
}

fn is_left_paren(c: char) -> bool {
    c == '（'
}

fn is_right_paren(c: char) -> bool {
    c == '）'
}

fn is_zh_speech_punctuation(c: char) -> bool {
    is_sentence_punctuation(c) || is_pause_punctuation(c) || is_left_paren(c) || is_right_paren(c)
}

fn is_collapsible_punctuation(c: char) -> bool {
    is_sentence_punctuation(c) || is_pause_punctuation(c)
}

fn is_cjk(c: char) -> bool {
    matches!(
        c as u32,
        0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xF900..=0xFAFF
            | 0x3040..=0x30FF
            | 0xAC00..=0xD7AF
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn run(filler: &ZhFilter, input: &str) -> String {
        run_segments(filler, &[input]).await
    }

    async fn run_segments(filler: &ZhFilter, segments: &[&str]) -> String {
        let segment_texts = segments
            .iter()
            .map(|s| (*s).to_string())
            .collect::<Vec<_>>();
        let raw = segment_texts.concat();
        let pt = PipelineText::new(raw, segment_texts);
        filler
            .process(pt, &AppContext::default())
            .await
            .unwrap()
            .text
    }

    #[tokio::test]
    async fn maps_common_spoken_ascii_punctuation_to_chinese() {
        let f = ZhFilter::default_patterns();
        assert_eq!(
            run(&f, "你好, 今天可以吗? 可以! (测试): ok;").await,
            "你好，今天可以吗？可以！（测试）： ok"
        );
    }

    #[tokio::test]
    async fn leaves_non_spoken_ascii_symbols_alone() {
        let f = ZhFilter::default_patterns();
        assert_eq!(
            run(&f, "curl http://a.com/path --flag_name=x@y").await,
            "curl http://a.com/path --flag_name=x@y"
        );
    }

    #[tokio::test]
    async fn keeps_unknown_symbols_and_emoji_without_extra_spacing() {
        let f = ZhFilter::default_patterns();
        assert_eq!(run_segments(&f, &["可以🙂", "继续"]).await, "可以🙂继续");
        assert_eq!(run_segments(&f, &["状态✓", "正常"]).await, "状态✓正常");
    }

    #[tokio::test]
    async fn handles_parenthesis_boundaries() {
        let f = ZhFilter::default_patterns();
        assert_eq!(run(&f, "测试 ( 版本一 ) 可以").await, "测试（版本一）可以");
        assert_eq!(
            run_segments(&f, &["测试", "（版本一）", "可以"]).await,
            "测试（版本一）可以"
        );
    }

    #[tokio::test]
    async fn collapses_spaces_and_repeated_punctuation() {
        let f = ZhFilter::default_patterns();
        assert_eq!(
            run(&f, "  你好  ，，，  世界。。。  可以？？？ ").await,
            "你好，世界。可以？"
        );
    }

    #[tokio::test]
    async fn joins_segments_by_boundary_format_rules() {
        let f = ZhFilter::default_patterns();
        assert_eq!(
            run_segments(&f, &["hello", "", "world", "，", "今天", "。", "，明天"]).await,
            "hello world，今天。明天"
        );
        assert_eq!(run_segments(&f, &["你好，", "。世界"]).await, "你好。世界");
    }

    #[tokio::test]
    async fn does_not_guess_missing_sentence_boundaries_between_segments() {
        let f = ZhFilter::default_patterns();
        assert_eq!(
            run_segments(&f, &["我今天去公司", "然后开了个会"]).await,
            "我今天去公司然后开了个会"
        );
    }

    #[tokio::test]
    async fn removes_conservative_fillers_without_deleting_semantic_words() {
        let f = ZhFilter::default_patterns();
        assert_eq!(
            run(&f, "嗯，今天呃天气啊挺好，嗯可以").await,
            "今天天气挺好，可以"
        );
        assert_eq!(
            run(&f, "那个文件这就是问题可以啊").await,
            "那个文件这就是问题可以啊"
        );
    }

    #[tokio::test]
    async fn preserves_raw_field() {
        let f = ZhFilter::default_patterns();
        let pt = PipelineText::new("嗯你好".into(), vec!["嗯你好".into()]);
        let out = f.process(pt, &AppContext::default()).await.unwrap();
        assert_eq!(out.text, "你好");
        assert_eq!(out.raw, "嗯你好");
    }

    #[tokio::test]
    async fn is_idempotent() {
        let f = ZhFilter::default_patterns();
        let once = run(&f, "嗯，，你好  , world。。。可以啊").await;
        let twice = run(&f, &once).await;
        assert_eq!(twice, once);
    }
}
