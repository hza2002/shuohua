use crate::config::schema::{self, SchemaId};
use crate::config::spec::ConfigSpec;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateKind {
    Config,
    Asr,
    Profile,
    PostRule,
    PostLlm,
}

#[derive(Debug, Clone, Copy)]
pub struct Template {
    pub id: &'static str,
    pub kind: TemplateKind,
    pub path: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    schema: SchemaId,
    pub(super) values: &'static [(&'static str, TemplateValue)],
}

impl Template {
    pub fn spec(&self) -> ConfigSpec {
        schema::spec_for(self.schema)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TemplateValue {
    String(&'static str),
    MultilineString(&'static str),
    Integer(i64),
    Float(f64),
    Bool(bool),
    StringArray(&'static [&'static str]),
    InlineTable(&'static [(&'static str, TemplateValue)]),
    Table(&'static [(&'static str, TemplateValue)]),
}

pub fn registry() -> &'static [Template] {
    TEMPLATES
}

#[derive(Debug, Clone, Copy)]
pub struct ThemePreset {
    pub id: &'static str,
    pub path: &'static str,
    pub(super) body: &'static str,
}

include!(concat!(env!("OUT_DIR"), "/embedded_themes.rs"));

pub fn theme_presets() -> &'static [ThemePreset] {
    THEME_PRESETS
}

pub fn theme_preset_body(name: &str) -> Option<&'static str> {
    let id = if name == crate::config::theme::DEFAULT_THEME_NAME
        || name == crate::config::theme::LEGACY_DEFAULT_THEME_NAME
    {
        crate::config::theme::DEFAULT_THEME_NAME
    } else {
        name
    };
    theme_presets()
        .iter()
        .find(|preset| preset.id == id)
        .map(|preset| preset.body)
}

const CONFIG_VALUES: &[(&str, TemplateValue)] = &[
    (
        "hotkey",
        TemplateValue::Table(&[
            ("trigger", TemplateValue::String("right_option:double")),
            ("cancel", TemplateValue::String("escape")),
            ("resume", TemplateValue::String("shift+right_option:double")),
        ]),
    ),
    (
        "voice",
        TemplateValue::Table(&[
            ("stop_delay_ms", TemplateValue::Integer(800)),
            ("record_audio", TemplateValue::String("off")),
            ("auto_paste", TemplateValue::Bool(true)),
        ]),
    ),
    (
        "voice.preprocess",
        TemplateValue::Table(&[("backend", TemplateValue::String("webrtc"))]),
    ),
    (
        "voice.vad",
        TemplateValue::Table(&[
            ("backend", TemplateValue::String("silero")),
            ("threshold", TemplateValue::Float(0.5)),
            ("pause_silence_ms", TemplateValue::Integer(1500)),
            ("pre_roll_ms", TemplateValue::Integer(300)),
            ("max_overlap_ms", TemplateValue::Integer(200)),
            ("min_start_voiced_frames", TemplateValue::Integer(2)),
        ]),
    ),
    (
        "dev",
        TemplateValue::Table(&[
            ("vad_trace", TemplateValue::Bool(false)),
            ("apple_backend_trace", TemplateValue::Bool(false)),
        ]),
    ),
    (
        "overlay",
        TemplateValue::Table(&[
            ("position", TemplateValue::String("bottom")),
            (
                "width",
                TemplateValue::Integer(crate::overlay::layout::constants::DEFAULT_WIDTH_PX as i64),
            ),
            ("max_text_lines", TemplateValue::Integer(5)),
        ]),
    ),
    (
        "post",
        TemplateValue::Table(&[("timeout_ms", TemplateValue::Integer(30_000))]),
    ),
    (
        "profile",
        TemplateValue::Table(&[
            ("default", TemplateValue::String("default")),
            (
                "chat",
                TemplateValue::StringArray(&[
                    "com.openai.chat",
                    "com.apple.MobileSMS",
                    "com.tencent.xinWeChat",
                    "com.tencent.qq",
                    "com.alibaba.DingTalkMac",
                    "com.electron.lark",
                    "ru.keepcoder.Telegram",
                    "com.tdesktop.Telegram",
                    "com.tinyspeck.slackmacgap",
                    "com.microsoft.teams2",
                ]),
            ),
            (
                "agent",
                TemplateValue::StringArray(&[
                    "com.openai.codex",
                    "com.anthropic.claudefordesktop",
                    "com.microsoft.VSCode",
                    "com.microsoft.VSCodeInsiders",
                    "com.cursor.Cursor",
                    "com.todesktop.230313mzl4w4u92",
                    "com.github.wez.wezterm",
                    "com.googlecode.iterm2",
                    "com.mitchellh.ghostty",
                    "com.apple.Terminal",
                    "dev.warp.Warp-Stable",
                    "com.apple.dt.Xcode",
                    "com.jetbrains.intellij",
                    "com.jetbrains.pycharm",
                    "com.jetbrains.WebStorm",
                    "com.jetbrains.goland",
                    "com.jetbrains.CLion",
                    "com.jetbrains.rustrover",
                    "com.jetbrains.AppCode",
                    "com.jetbrains.rider",
                    "com.google.android.studio",
                    "com.sublimetext.4",
                    "com.github.atom",
                    "org.vim.MacVim",
                    "com.neovide.neovide",
                    "org.gnu.Emacs",
                ]),
            ),
        ]),
    ),
    (
        "ui",
        TemplateValue::Table(&[
            ("language", TemplateValue::String("auto")),
            ("theme", TemplateValue::String("gruvbox-dark")),
            ("theme_tui", TemplateValue::String("")),
            ("theme_overlay", TemplateValue::String("")),
        ]),
    ),
];

const ASR_APPLE_VALUES: &[(&str, TemplateValue)] = &[
    ("type", TemplateValue::String("apple")),
    ("name", TemplateValue::String("Apple Local ASR")),
    ("language", TemplateValue::String("zh-CN")),
    ("install_assets", TemplateValue::Bool(true)),
    ("local_vad", TemplateValue::String("off")),
    ("open_timeout_ms", TemplateValue::Integer(5000)),
    ("finalize_timeout_ms", TemplateValue::Integer(5000)),
];

const ASR_DOUBAO_VALUES: &[(&str, TemplateValue)] = &[
    ("type", TemplateValue::String("doubao")),
    ("name", TemplateValue::String("Doubao ASR")),
    ("app_key", TemplateValue::String("")),
    ("access_key", TemplateValue::String("")),
    (
        "resource_id",
        TemplateValue::String("volc.bigasr.sauc.duration"),
    ),
    ("language", TemplateValue::String("auto")),
    ("enable_itn", TemplateValue::Bool(true)),
    ("enable_punc", TemplateValue::Bool(true)),
    ("enable_ddc", TemplateValue::Bool(true)),
    ("stream_mode", TemplateValue::Integer(2)),
    ("ai_vad", TemplateValue::Bool(false)),
    ("local_vad", TemplateValue::String("auto")),
    ("open_timeout_ms", TemplateValue::Integer(12_000)),
    ("finalize_timeout_ms", TemplateValue::Integer(12_000)),
];

const ASR_TENCENT_VALUES: &[(&str, TemplateValue)] = &[
    ("type", TemplateValue::String("tencent")),
    ("name", TemplateValue::String("Tencent ASR")),
    ("app_id", TemplateValue::String("")),
    ("secret_id", TemplateValue::String("")),
    ("secret_key", TemplateValue::String("")),
    ("engine_model_type", TemplateValue::String("16k_zh")),
    ("convert_num_mode", TemplateValue::Integer(1)),
    ("filter_modal", TemplateValue::Integer(1)),
    ("filter_punc", TemplateValue::Bool(false)),
    ("filter_dirty", TemplateValue::Integer(0)),
    ("need_vad", TemplateValue::Bool(false)),
    ("vad_silence_time", TemplateValue::Integer(1000)),
    ("max_speak_time", TemplateValue::Integer(60_000)),
    ("sentence_strategy", TemplateValue::Integer(0)),
    ("noise_threshold", TemplateValue::Float(0.0)),
    ("hotword_weight", TemplateValue::Integer(10)),
    ("hotword_id", TemplateValue::String("")),
    ("customization_id", TemplateValue::String("")),
    ("replace_text_id", TemplateValue::String("")),
    ("local_vad", TemplateValue::String("auto")),
    ("open_timeout_ms", TemplateValue::Integer(12_000)),
    ("finalize_timeout_ms", TemplateValue::Integer(12_000)),
];

const DEFAULT_PROFILE_VALUES: &[(&str, TemplateValue)] = &[
    ("name", TemplateValue::String("default")),
    (
        "asr",
        TemplateValue::Table(&[
            ("instance", TemplateValue::String("doubao")),
            ("hotwords", TemplateValue::StringArray(&[])),
        ]),
    ),
    (
        "post",
        TemplateValue::Table(&[(
            "chain",
            TemplateValue::StringArray(&["zh_filter", "deepseek"]),
        )]),
    ),
    (
        "post.overrides",
        TemplateValue::Table(&[(
            "deepseek",
            TemplateValue::Table(&[
                ("model", TemplateValue::String("deepseek-v4-flash")),
                (
                    "system_prompt",
                    TemplateValue::MultilineString(DEFAULT_SYSTEM_PROMPT),
                ),
                (
                    "prompt",
                    TemplateValue::MultilineString(DEFAULT_USER_PROMPT),
                ),
            ]),
        )]),
    ),
];

const CHAT_PROFILE_VALUES: &[(&str, TemplateValue)] = &[
    ("name", TemplateValue::String("chat")),
    (
        "asr",
        TemplateValue::Table(&[
            ("instance", TemplateValue::String("doubao")),
            ("hotwords", TemplateValue::StringArray(&[])),
        ]),
    ),
    (
        "post",
        TemplateValue::Table(&[(
            "chain",
            TemplateValue::StringArray(&["zh_filter", "deepseek"]),
        )]),
    ),
    (
        "post.overrides",
        TemplateValue::Table(&[(
            "deepseek",
            TemplateValue::Table(&[
                ("model", TemplateValue::String("deepseek-v4-flash")),
                (
                    "system_prompt",
                    TemplateValue::MultilineString(CHAT_SYSTEM_PROMPT),
                ),
                ("prompt", TemplateValue::MultilineString(CHAT_USER_PROMPT)),
            ]),
        )]),
    ),
];

const AGENT_PROFILE_VALUES: &[(&str, TemplateValue)] = &[
    ("name", TemplateValue::String("agent")),
    (
        "asr",
        TemplateValue::Table(&[
            ("instance", TemplateValue::String("doubao")),
            ("hotwords", TemplateValue::StringArray(&[])),
        ]),
    ),
    (
        "post",
        TemplateValue::Table(&[(
            "chain",
            TemplateValue::StringArray(&["zh_filter", "deepseek"]),
        )]),
    ),
    (
        "post.overrides",
        TemplateValue::Table(&[(
            "deepseek",
            TemplateValue::Table(&[
                ("model", TemplateValue::String("deepseek-v4-flash")),
                (
                    "system_prompt",
                    TemplateValue::MultilineString(AGENT_SYSTEM_PROMPT),
                ),
                ("prompt", TemplateValue::MultilineString(AGENT_USER_PROMPT)),
            ]),
        )]),
    ),
];

const DEFAULT_SYSTEM_PROMPT: &str = r#"你是 ASR 文本整理器。把语音识别文本整理成适合普通文本框、搜索框、提醒事项或网页对话框输入的最终文本。

核心原则：
- 以还原用户意图为主，表达清晰、简洁、高效。
- 可以轻微调整语序、合并重复、删除口误、补全标点，让文本更顺畅。
- 不做复杂重组，不主动分点，不改成正式文案、聊天话术或 Agent 指令。
- 不新增用户没说的事实、目标、要求、理由、时间或结论。
- 用户表达不明确时保持不明确；不要替用户补上下文或强行下结论。
- 保留原语言、语气、专有名词、技术词和中英混合表达。
- 只修正明确的 ASR 错误；不确定时保留原词。
- 不回答用户问题，不执行任务，不解释修改过程。

输出要求：
- 只输出整理后的最终文本。
- 不加“整理后如下”等前缀。"#;

const DEFAULT_USER_PROMPT: &str = r#"App 上下文只用于消歧 ASR 文本，不要输出：

<app>
name: {{app_name}}
bundle_id: {{bundle_id}}
</app>

把下面 ASR 文本整理成最终输入：

<asr_text>
{{text}}
</asr_text>

只输出整理后的最终文本。"#;

const CHAT_SYSTEM_PROMPT: &str = r#"你是 ASR 文本整理器。把语音识别文本整理成适合直接发送到微信或聊天软件的消息。

核心原则：
- 表达层积极整理：调整语序、合并重复、删除口误、补全标点、自然分段，让意思清楚顺畅。
- 语气层温和自然：保留用户真实态度和情绪倾向，可以适当加入连接词、缓和语气和轻量语气词，让表达清楚、礼貌、自然。
- 意图层严格保真：不新增用户没说的事实、理由、承诺、道歉、让步、责任归属、时间、要求或结论。

整理规则：
- 短输入保持简洁；长输入或多层意思可以自然分段，必要时用简短列表，但不要写成公文、客服话术或 Agent 指令。
- 用户卡顿、重复、跳跃表达或反复说明同一件事时，输出最终清晰版本，不复述思路历程。
- 用户自我纠正时，以后出现的明确修正为准，删除被否定的旧说法。
- 用户表达不明确时保持不明确；不要替用户补上下文或强行下结论。
- 指导别人做事时，可以把步骤和理由整理清楚，但不要把建议改成命令，不要把猜测改成事实。
- 情绪、强调和紧急程度可以转换成适合聊天的表达，不要保留无意义口语情绪词。
- 用户在口述要原样发送的文本、文案、标题、引用或消息内容时，只做必要 ASR 清理，不要过度改写。
- 不把拒绝改成同意，不把委婉改成强硬，不把中文强行翻译成英文。
- 不回答用户问题，不执行任务，不解释修改过程。

输出要求：
- 只输出整理后的最终消息。
- 不加“整理后如下”等前缀。
- 不默认添加 emoji。"#;

const CHAT_USER_PROMPT: &str = r#"App 上下文只用于判断聊天场景，不要输出：

<app>
name: {{app_name}}
bundle_id: {{bundle_id}}
</app>

把下面 ASR 文本整理成最终聊天消息：

<asr_text>
{{text}}
</asr_text>

只输出整理后的最终消息。"#;

const AGENT_SYSTEM_PROMPT: &str = r#"你是 ASR 文本整理器。把语音识别文本整理成可以直接发送给终端、IDE 或 coding agent 的最终用户输入。

核心原则：
- 表达层积极整理：调整语序、合并重复、删除口误、补全标点、分段分点，让用户真实意图更清楚、更适合 Agent 理解。
- 意图层严格保真：不新增用户没说的目标、文件、参数、路径、约束、测试命令、实现方案、背景信息或验收标准。
- 技术层默认保留：命令、代码、路径、URL、flag、环境变量、文件名、branch、commit hash、错误日志和代码符号，只有明确是 ASR 错误时才修正。

整理规则：
- 长输入或多意图输入可以使用 Markdown 标题、列表、编号和反引号组织；短输入保持简洁，不套固定模板。
- 用户卡顿、重复、跳跃表达或反复说明同一件事时，输出最终清晰版本，不复述思路历程。
- 用户自我纠正时，以后出现的明确修正为准，删除被否定的旧说法。
- 用户表达不明确时保持不明确；不要因为当前文本缺少上下文就补充、追问或替用户推断。
- 默认下游 Agent 拥有自己的上下文，让下游 Agent 根据上下文判断。
- “先看、先分析、先定位、先不要改、不要动别的”等表达是顺序或范围约束，必须保留，不要改写成更大的任务。
- 情绪、强调和紧急程度可以转换成适合 Agent 的约束、优先级或关注点。
- 用户在口述要保留的文本、文案、prompt、commit message、标题或注释时，只做必要 ASR 清理，不要过度 Agent 化改写。
- 不把疑问、猜测或可能性改写成事实；不把中文说明强行翻译成英文。
- 不回答用户问题，不执行任务，不解释修改过程。

输出要求：
- 只输出整理后的最终文本。
- 不加“整理后如下”等前缀。"#;

const AGENT_USER_PROMPT: &str = r#"App 上下文只用于消歧 ASR 中的技术词，不要输出：

<app>
name: {{app_name}}
bundle_id: {{bundle_id}}
</app>

把下面 ASR 文本整理成最终 Agent 输入：

<asr_text>
{{text}}
</asr_text>

只输出整理后的最终文本。"#;

const ZH_FILTER_VALUES: &[(&str, TemplateValue)] = &[
    ("name", TemplateValue::String("Chinese Filler Filter")),
    ("type", TemplateValue::String("rule")),
    (
        "patterns",
        TemplateValue::StringArray(&["嗯", "呃", "啊", "就是"]),
    ),
];

const DEEPSEEK_VALUES: &[(&str, TemplateValue)] = &[
    ("type", TemplateValue::String("llm")),
    ("format", TemplateValue::String("openai")),
    ("name", TemplateValue::String("deepseek")),
    (
        "base_url",
        TemplateValue::String("https://api.deepseek.com"),
    ),
    ("api_key", TemplateValue::String("")),
    ("model", TemplateValue::String("deepseek-chat")),
    (
        "system_prompt",
        TemplateValue::MultilineString(
            "你是 ASR 文本整理器。保留用户原意，只清理口误、重复、标点和明确的识别错误。只输出整理后的文本。",
        ),
    ),
    ("prompt", TemplateValue::String("{{text}}")),
    (
        "extra_body",
        TemplateValue::Table(&[(
            "thinking",
            TemplateValue::InlineTable(&[("type", TemplateValue::String("disabled"))]),
        )]),
    ),
];

const OPENAI_VALUES: &[(&str, TemplateValue)] = &[
    ("type", TemplateValue::String("llm")),
    ("format", TemplateValue::String("openai")),
    ("name", TemplateValue::String("openai")),
    (
        "base_url",
        TemplateValue::String("https://api.openai.com/v1"),
    ),
    ("api_key", TemplateValue::String("")),
    ("model", TemplateValue::String("gpt-4.1-mini")),
    ("prompt", TemplateValue::String("{{text}}")),
];

const ANTHROPIC_VALUES: &[(&str, TemplateValue)] = &[
    ("type", TemplateValue::String("llm")),
    ("format", TemplateValue::String("anthropic")),
    ("name", TemplateValue::String("anthropic")),
    (
        "base_url",
        TemplateValue::String("https://api.anthropic.com"),
    ),
    ("api_key", TemplateValue::String("")),
    ("model", TemplateValue::String("claude-haiku-4-5")),
    ("prompt", TemplateValue::String("{{text}}")),
];

const TEMPLATES: &[Template] = &[
    Template {
        id: "config",
        kind: TemplateKind::Config,
        path: "config.toml",
        title: "Config",
        description: "Top-level shuohua config.toml.",
        schema: SchemaId::Main,
        values: CONFIG_VALUES,
    },
    Template {
        id: "asr/apple",
        kind: TemplateKind::Asr,
        path: "asr/apple.toml",
        title: "Apple ASR",
        description: "Starter config for the local Apple SpeechAnalyzer provider.",
        schema: SchemaId::AsrApple,
        values: ASR_APPLE_VALUES,
    },
    Template {
        id: "asr/doubao",
        kind: TemplateKind::Asr,
        path: "asr/doubao.toml",
        title: "Doubao ASR",
        description: "Starter config for the Doubao provider.",
        schema: SchemaId::AsrDoubao,
        values: ASR_DOUBAO_VALUES,
    },
    Template {
        id: "asr/tencent",
        kind: TemplateKind::Asr,
        path: "asr/tencent.toml",
        title: "Tencent ASR",
        description: "Starter config for the Tencent Cloud realtime ASR provider.",
        schema: SchemaId::AsrTencent,
        values: ASR_TENCENT_VALUES,
    },
    Template {
        id: "profile/default",
        kind: TemplateKind::Profile,
        path: "profile/default.toml",
        title: "Default profile",
        description: "Default profile using Doubao ASR and DeepSeek post-processing.",
        schema: SchemaId::Profile,
        values: DEFAULT_PROFILE_VALUES,
    },
    Template {
        id: "profile/chat",
        kind: TemplateKind::Profile,
        path: "profile/chat.toml",
        title: "Chat profile",
        description: "Chat app profile using Doubao ASR and a chat-focused DeepSeek prompt.",
        schema: SchemaId::Profile,
        values: CHAT_PROFILE_VALUES,
    },
    Template {
        id: "profile/agent",
        kind: TemplateKind::Profile,
        path: "profile/agent.toml",
        title: "Agent profile",
        description:
            "Terminal, IDE, and coding-agent profile using a task-focused DeepSeek prompt.",
        schema: SchemaId::Profile,
        values: AGENT_PROFILE_VALUES,
    },
    Template {
        id: "post/zh_filter",
        kind: TemplateKind::PostRule,
        path: "post/zh_filter.toml",
        title: "Chinese speech cleanup rule",
        description: "Rule processor for common Chinese filler words.",
        schema: SchemaId::PostRule,
        values: ZH_FILTER_VALUES,
    },
    Template {
        id: "post/deepseek",
        kind: TemplateKind::PostLlm,
        path: "post/deepseek.toml",
        title: "DeepSeek",
        description: "OpenAI-compatible DeepSeek post-processing preset.",
        schema: SchemaId::PostLlm,
        values: DEEPSEEK_VALUES,
    },
    Template {
        id: "post/openai",
        kind: TemplateKind::PostLlm,
        path: "post/openai.toml",
        title: "OpenAI",
        description: "OpenAI post-processing preset.",
        schema: SchemaId::PostLlm,
        values: OPENAI_VALUES,
    },
    Template {
        id: "post/anthropic",
        kind: TemplateKind::PostLlm,
        path: "post/anthropic.toml",
        title: "Anthropic",
        description: "Anthropic post-processing preset.",
        schema: SchemaId::PostLlm,
        values: ANTHROPIC_VALUES,
    },
];
