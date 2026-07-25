//! 模型 prompt 模板：system + user 双消息，防止 prompt 泄露
//!
//! 设计要点：
//! - system 定义角色与硬性约束，user 只给原文和目标
//! - 严格禁止前缀/后缀/解释/代码块，第一条字必须是压缩结果
//! - 提供 3 套预设：Minimal / Standard / StrictChars（见 PresetPrompt）

const DEFAULT_SYSTEM: &str = r#"你是文本压缩助手。

任务：将用户输入的文本压缩到指定字数以内。

要求：
- 保留核心信息、关键事实和原始语义。
- 删除重复、冗余、修饰性内容。
- 保持原文表达意图，不新增、不改写观点。
- 优先保留专有名词、数字、条件和结论。

输出规则：
- 只输出压缩后的文本。
- 不添加说明、标题、前缀、后缀或代码块。"#;

const DEFAULT_USER_TEMPLATE: &str = r#"目标字数：{target}
原文字数：{orig}

原文：
{text}"#;

/// 压缩消息构造参数
pub struct PromptParams<'a> {
    pub text: &'a str,
    pub target_chars: usize,
    /// 自定义 system 提示词（None 用默认）
    pub custom_system: Option<&'a str>,
    /// 自定义 user 模板（None 用默认）
    /// 支持占位符：{text} / {target} / {orig} / {cut}
    pub custom_user_template: Option<&'a str>,
}

/// 返回 (system, user) 两条消息内容
pub fn build_compress_messages(text: &str, target_chars: usize) -> (String, String) {
    build_compress_messages_with(PromptParams {
        text,
        target_chars,
        custom_system: None,
        custom_user_template: None,
    })
}

/// 支持自定义模板的版本
pub fn build_compress_messages_with(params: PromptParams) -> (String, String) {
    let original_chars = params.text.chars().count();
    let cut_pct = if original_chars > 0 {
        ((1.0 - params.target_chars as f32 / original_chars as f32) * 100.0) as u32
    } else {
        0
    };

    let system = params
        .custom_system
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| DEFAULT_SYSTEM.to_string());

    let user_template = params
        .custom_user_template
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| DEFAULT_USER_TEMPLATE.to_string());

    // 占位符替换
    let user = user_template
        .replace("{text}", params.text)
        .replace("{target}", &params.target_chars.to_string())
        .replace("{orig}", &original_chars.to_string())
        .replace("{cut}", &cut_pct.to_string());

    (system, user)
}

/// 兼容旧接口：CLI 单 prompt 模式（仅 ollama /api/generate 用，已废弃）
#[allow(dead_code)]
pub fn build_compress_prompt(text: &str, target_chars: usize) -> String {
    let (system, user) = build_compress_messages(text, target_chars);
    format!("{}\n\n{}", system, user)
}

/// 返回默认 system 提示词（供前端展示）
pub fn default_system() -> &'static str {
    DEFAULT_SYSTEM
}

/// 返回默认 user 模板（供前端展示）
pub fn default_user_template() -> &'static str {
    DEFAULT_USER_TEMPLATE
}

// ==================== 预设提示词 ====================

const MINIMAL_SYSTEM: &str = "压缩文本，保留核心语义，直接输出结果。";
const MINIMAL_USER_TEMPLATE: &str = "压缩到 {target} 字以内：\n\n{text}";

const STRICT_CHARS_SYSTEM: &str = r#"你是文本压缩助手。

任务：将用户输入的文本压缩到指定字数以内。

要求：
- 保留核心信息、关键事实和原始语义。
- 删除重复、冗余、修饰性内容。
- 保持原文表达意图，不新增、不改写观点。
- 优先保留专有名词、数字、条件和结论。

硬性要求：输出字数必须 ≤ {target}，超出请重新压缩。

输出规则：
- 只输出压缩后的文本。
- 不添加说明、标题、前缀、后缀或代码块。"#;

const STRICT_CHARS_USER_TEMPLATE: &str = r#"目标字数：{target}
原文字数：{orig}

原文：
{text}"#;

/// 预设提示词 ID
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresetPrompt {
    Minimal,
    Standard,
    StrictChars,
}

impl PresetPrompt {
    pub fn from_str(s: &str) -> Self {
        match s {
            "minimal" => PresetPrompt::Minimal,
            "strict_chars" => PresetPrompt::StrictChars,
            _ => PresetPrompt::Standard,
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            PresetPrompt::Minimal => "minimal",
            PresetPrompt::Standard => "standard",
            PresetPrompt::StrictChars => "strict_chars",
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            PresetPrompt::Minimal => "极简",
            PresetPrompt::Standard => "标准",
            PresetPrompt::StrictChars => "严格字数",
        }
    }

    pub fn system(&self) -> &'static str {
        match self {
            PresetPrompt::Minimal => MINIMAL_SYSTEM,
            PresetPrompt::Standard => DEFAULT_SYSTEM,
            PresetPrompt::StrictChars => STRICT_CHARS_SYSTEM,
        }
    }

    pub fn user_template(&self) -> &'static str {
        match self {
            PresetPrompt::Minimal => MINIMAL_USER_TEMPLATE,
            PresetPrompt::Standard => DEFAULT_USER_TEMPLATE,
            PresetPrompt::StrictChars => STRICT_CHARS_USER_TEMPLATE,
        }
    }
}

/// 基于预设构造 (system, user) 消息
/// - custom_system / custom_user_template 优先于 preset
/// - StrictChars 的 system 含 {target} 占位符，需先替换
pub fn build_compress_messages_preset(
    text: &str,
    target_chars: usize,
    preset: PresetPrompt,
    custom_system: Option<&str>,
    custom_user_template: Option<&str>,
) -> (String, String) {
    let original_chars = text.chars().count();

    let system_raw = custom_system
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| preset.system().to_string());

    let user_template = custom_user_template
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| preset.user_template().to_string());

    // system 中也支持 {target} / {orig} 占位符（StrictChars 需要）
    let system = system_raw
        .replace("{target}", &target_chars.to_string())
        .replace("{orig}", &original_chars.to_string());

    let user = user_template
        .replace("{text}", text)
        .replace("{target}", &target_chars.to_string())
        .replace("{orig}", &original_chars.to_string());

    (system, user)
}
