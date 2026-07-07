#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResumeSeed {
    pub text: String,
}

impl ResumeSeed {
    /// seed 文本去空白后非空才算「有内容」。display（engine 回显）与 record
    /// （finish 拼 history/post）共用这一个判定，避免两处对「空 seed」理解漂移。
    pub(crate) fn non_empty_text(&self) -> Option<&str> {
        Some(self.text.as_str()).filter(|text| !text.trim().is_empty())
    }
}

/// 一次 recording 是怎么开始的——决定 overlay 起始提示与 resume 相关行为。
/// 由 daemon 决策后放进 `SessionParams`，engine 在清屏后据此发提示。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) enum RecordingStart {
    /// 普通 trigger/toggle 开始，无提示。
    #[default]
    Fresh,
    /// 按了 resume 热键但没有可恢复的记录 → 照常开新录音，但提示「新录音」，
    /// 让用户知道热键生效、且确实没有可续写的内容。
    NewFromResume,
    /// 按了 resume 热键且最新记录可恢复 → 用旧 ASR 文本做 seed 续写。
    Seed(ResumeSeed),
}

impl RecordingStart {
    /// seed 存在时返回它（供 finish 拼 history/post、engine 回显）。
    pub(crate) fn seed(&self) -> Option<&ResumeSeed> {
        match self {
            Self::Seed(seed) => Some(seed),
            _ => None,
        }
    }

    /// 本次是否带 seed 续写——决定「有内容」判据是否收紧（见 voice.md）。
    pub(crate) fn is_seed(&self) -> bool {
        matches!(self, Self::Seed(_))
    }
}
