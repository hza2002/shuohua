use super::{registry, Template, TemplateKind};

/// ASR 实例模板集合（registry 里 `kind == Asr` 的条目）。新建 ASR 用它列出可选
/// 实现并取对应 seed 模板；逐字段编辑走 File 编辑器，不再有独立 wizard 复刻 schema。
pub fn asr_templates() -> impl Iterator<Item = &'static Template> {
    registry()
        .iter()
        .filter(|template| template.kind == TemplateKind::Asr)
}
