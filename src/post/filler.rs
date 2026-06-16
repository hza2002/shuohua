//! 规则去口语词。M2.5 引入。
//!
//! 配置示例（DESIGN §2.10）：
//! ```toml
//! [processors.filler]
//! patterns = ["嗯", "啊", "呃", "那个", "就是"]
//! ```
//!
//! M2.5 实现就是 regex.replace_all + 折叠产生的多余空白。`collapse_repeats`
//! 配置项 DESIGN 里写了但 M2.5 不实现——中文里"哈哈"/"妈妈"是合法重叠，
//! 简单 char 折叠会误伤；语义级折叠等 M7 LLM processor 一起处理。

use async_trait::async_trait;
use regex::Regex;

use super::{AppContext, PipelineText, PostError, PostProcessor};

pub struct RuleBasedFiller {
    name: String,
    pattern: Regex,
}

impl RuleBasedFiller {
    /// 用任意 filler 词列表构造。词会被 regex escape 后做 alternation。
    #[cfg(test)]
    pub fn new(patterns: &[&str]) -> Self {
        Self::with_name("filler", patterns)
    }

    pub fn with_name(name: impl Into<String>, patterns: &[&str]) -> Self {
        let alt = patterns
            .iter()
            .map(|p| regex::escape(p))
            .collect::<Vec<_>>()
            .join("|");
        // alt 来自 hardcoded 词列表，escape 后保证合法 → expect 安全
        let pattern = Regex::new(&alt).expect("filler patterns regex");
        Self {
            name: name.into(),
            pattern,
        }
    }

    /// 默认 5 词集合（DESIGN §2.10 示例）：嗯 啊 呃 那个 就是。
    #[cfg(test)]
    pub fn default_patterns() -> Self {
        Self::new(&["嗯", "啊", "呃", "那个", "就是"])
    }
}

#[async_trait]
impl PostProcessor for RuleBasedFiller {
    fn name(&self) -> &str {
        &self.name
    }

    async fn process(
        &self,
        input: PipelineText,
        _ctx: &AppContext,
    ) -> Result<PipelineText, PostError> {
        let stripped = self.pattern.replace_all(&input.text, "").into_owned();
        // 去 filler 后可能产生连续多空格（"hello 嗯 world" → "hello  world"）
        // 也可能首尾出现孤立空格。按 ASCII 空白折叠。注意 split_whitespace 会同时
        // 处理中文全角空格 \u{3000} 之外的 ASCII 空白，对剪贴板输出足够干净。
        let text = stripped.split_whitespace().collect::<Vec<_>>().join(" ");
        Ok(PipelineText { text, ..input })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn run(filler: &RuleBasedFiller, input: &str) -> String {
        let pt = PipelineText::new(input.into(), vec![input.into()]);
        filler
            .process(pt, &AppContext::default())
            .await
            .unwrap()
            .text
    }

    #[tokio::test]
    async fn removes_chinese_fillers() {
        let f = RuleBasedFiller::default_patterns();
        assert_eq!(run(&f, "嗯今天嗯天气真好").await, "今天天气真好");
    }

    #[tokio::test]
    async fn removes_consecutive_fillers() {
        let f = RuleBasedFiller::default_patterns();
        assert_eq!(run(&f, "嗯嗯嗯今天嗯啊呃下雨了").await, "今天下雨了");
    }

    #[tokio::test]
    async fn untouched_when_no_filler() {
        let f = RuleBasedFiller::default_patterns();
        assert_eq!(run(&f, "今天天气真好").await, "今天天气真好");
    }

    #[tokio::test]
    async fn collapses_whitespace_after_removal() {
        let f = RuleBasedFiller::default_patterns();
        // 段间空格分隔的多段："Rust 嗯 tokio" → 去 "嗯" 后变 "Rust  tokio"，折成 "Rust tokio"
        assert_eq!(run(&f, "Rust 嗯 tokio").await, "Rust tokio");
    }

    #[tokio::test]
    async fn preserves_english_and_punctuation() {
        let f = RuleBasedFiller::default_patterns();
        assert_eq!(
            run(&f, "I use Rust, tokio and macOS.").await,
            "I use Rust, tokio and macOS."
        );
    }

    #[tokio::test]
    async fn handles_multi_char_filler_words() {
        let f = RuleBasedFiller::default_patterns();
        // "那个" 和 "就是" 是多字词，验证 alternation 不被错误切分
        assert_eq!(
            run(&f, "我那个想说就是这个想法挺好的").await,
            "我想说这个想法挺好的"
        );
    }

    #[tokio::test]
    async fn custom_patterns_work() {
        let f = RuleBasedFiller::new(&["um", "uh"]);
        assert_eq!(run(&f, "um I uh think so").await, "I think so");
    }

    #[tokio::test]
    async fn raw_field_unchanged() {
        let f = RuleBasedFiller::default_patterns();
        let pt = PipelineText::new("嗯你好".into(), vec!["嗯你好".into()]);
        let out = f.process(pt, &AppContext::default()).await.unwrap();
        assert_eq!(out.text, "你好");
        assert_eq!(out.raw, "嗯你好"); // raw 不变
    }
}
