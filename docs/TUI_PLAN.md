# TUI 改造计划

本计划只覆盖 TUI 相关改造。原则：先把 Status 和 History 做成日常可用的状态与历史管理工具，最后单独处理 Configure。配置文件仍是 source of truth；TUI 负责发现、展示、创建、打开编辑器和少量安全操作，不做通用 TOML 表单编辑器。

配套上下文：

- TUI 当前实现：`src/tui/**`
- i18n 当前实现：`src/i18n/**`、`assets/i18n/*.toml`
- history schema：`docs/SCHEMA.md`
- retained audio 约定：`~/.local/state/shuohua/audio/<recording_id>.wav`

## 总体顺序

1. Status 小修
2. History 音频展示与单条动作
3. History 筛选
4. History 标记与批量音频操作
5. History 音频清理
6. TUI i18n 整理
7. Configure 独立阶段

每个阶段必须独立可合并。Configure 之前不要引入配置写入器、模板系统或 profile/pipeline 向导。

## Phase 1: Status 小修

目标：只修展示，不重做信息架构。

范围：

- 音频柱状图按当前面板宽度自适应，而不是固定最多保留 160 个 meter。
- 宽屏时尽量填满；窄屏时显示尾部，保持当前低延迟刷新。
- TUI 渲染节奏对齐 `voice::meter::METER_INTERVAL_MS`（当前 50ms），每帧之间持续 drain IPC/key event，避免 audio meter 事件把 per-client IPC queue 打满。
- Status 页离开后不积累 meter；回到 Status 页从新的实时输入重新画。meter 是当前输入电平参考，不是历史 waveform。
- 不改变 Status 页布局、字段、快捷键或 UDS 协议。

验证：

- 单测 meter 渲染长度和裁剪逻辑。
- 手动在普通窗口和全屏 TUI 下确认柱状图能使用可用宽度。
- daemon 日志不应再持续刷 `IPC client queue full`；该 warn 已在 IPC 层做 1s 节流，避免异常时日志放大。

## Phase 2: History 音频展示与单条动作

目标：让 retained audio 在 History 里可见、可打开、可定位、可删除。

范围：

- 由 `record.id` 推导音频路径：`state_dir()/audio/<id>.wav`。
- 列表显示音频状态：
  - 有音频：文件存在。
  - 无音频：文件不存在；可能是当时 `record_audio = false`，也可能是写入失败或之后被删除。
  - 文件缺失不改变 history record。
- 右侧 `History details` 内联 audio block，展示有无音频、大小、mtime；不单独拆 Audio 详情页。
- 增加单条动作：
  - 打开音频文件。
  - Finder 定位音频文件。
  - 删除单条音频，必须二次确认。
- 删除音频只删除 `.wav`，不删除 history JSONL。
- 删除成功后列表状态立即按文件存在性刷新为无音频；history JSONL 不重写、不追加 tombstone。

建议快捷键：

- `o` 打开音频文件。
- `r` Finder 定位音频文件。
- `d` 打开删除确认；第一版只支持删除当前记录的音频。

产品语义：

- history 文本和音频是两个独立资源，用 recording id join。
- 删除音频后 history 记录保留，状态变为无音频。
- 第一阶段不做删除 history record，因为 JSONL 是 append-only 事实源，删除需要重写月文件和额外备份/回滚设计。

验证：

- 单测 audio path 推导。
- 单测文件存在、大小、缺失状态。
- 单测删除动作只影响 wav。
- macOS 手动验证打开、Finder reveal、删除音频。

## Phase 3: History 筛选

目标：保留当前全文搜索，并加入低复杂度筛选。先做 filter menu，不做复杂日期输入器或查询语言。

范围：

- 保留 `/` 全文搜索。
- 增加快捷筛选：
  - Today
  - Last 7 days
  - Last 30 days
  - This month
  - Has audio
  - Missing audio
  - Current app（如果当前 app 可得）
- 搜索和筛选同时生效。
- 统计区显示当前 filter、显示数量、总数量。

暂不做：

- 自由输入时间范围。
- `since:` / `until:` 查询语言。
- 正则搜索。

验证：

- 时间范围过滤单测。
- 音频状态过滤单测。
- 搜索和 filter 组合单测。

## Phase 4: History 标记与批量音频操作

目标：引入文件管理器式 selection 语义，为后续清理做准备。

选择语义：

- `Space` 标记/取消当前记录。
- 有 marked items 时，批量动作作用于 marked items。
- 没有 marked items 时，动作作用于当前记录。
- `Esc` 优先清除搜索/确认框；再清除标记。

范围：

- 显示 marked 数量。
- 批量删除音频，必须 confirmation，显示文件数和总大小。
- 批量播放、批量打开、批量 Finder reveal 暂不做。

验证：

- 标记状态按 record id 保存，不按列表 index 保存。
- 筛选变化后不可见 marked items 不误操作；确认框必须显示实际作用数量。
- 批量删除 dry-run 统计正确。

## Phase 5: History 音频清理

目标：对 retained audio 做可解释、可回滚成本低的清理。

范围：

- `Clean audio` 菜单：
  - older than 7 / 30 / 90 days
  - keep latest N days
  - delete orphan audio（有 wav 但找不到对应 history record）
- 执行前必须显示 dry-run：
  - 文件数
  - 总大小
  - 最老/最新时间
  - 少量样例路径
- 确认后只删除音频文件，不删除文本 history。

暂不做：

- 删除 history JSONL 记录。
- 删除 history + audio。
- 重写月文件。

如果后续要做 history 删除，必须单独设计：月文件 rewrite、备份、并发、失败恢复和 UI 确认语义。

## Phase 6: TUI i18n 整理

目标：TUI 文案系统化，不再混杂英文硬编码。

范围：

- 补齐所有 TUI 页面和动作文案。
- key 按页面与动作组织：
  - `tui.status.*`
  - `tui.history.*`
  - `tui.history.audio.*`
  - `tui.action.*`
  - `tui.confirm.*`
  - `tui.error.*`
  - `tui.configure.*`
- History detail title、footer、状态栏反馈、确认框、错误消息都走 i18n。
- 增加测试保证 `zh-CN` 和 `en-US` key 对齐。

可以穿插在前面阶段做，但在 History 功能稳定后必须集中补齐一次。

## Phase 7: Configure 独立阶段

目标：Configure 做成配置文件管理器 + 官方模板生成器 + 验证入口 + 编辑器 launcher，不做通用 TOML 表单编辑器。

原则：

- 配置文件仍是 source of truth。
- TUI 负责发现、展示、创建 LLM post component、打开编辑器、Finder 定位、刷新和验证配置文件。
- 复杂编辑交给 `$EDITOR` / `$VISUAL` / macOS 默认编辑器。
- 官方模板位于 `assets/config/**`；`config::template` registry 渲染并测试校验这些文件，避免模板与 spec 静默漂移。

第一步范围：

- 配置总览。
- 打开默认编辑器。
- 官方模板机制。
- 新建 LLM post component 向导。

暂不做：

- ASR provider 创建。
- profile pipeline 自动接入。
- 网络验证。
- 全量 TOML 表单编辑器。

### 模板与配置文件布局

模板文件当前放在：

```text
assets/config/
  manifest.toml
  main.toml
  profile/default.toml
  post/rule/zh_filter.toml
  post/llm/
    deepseek.toml
    openai.toml
    anthropic.toml
    custom-openai.toml
    custom-anthropic.toml
```

语义：

- `main.toml` / `profile/default.toml` / `post/rule/zh_filter.toml`：官方基础模板。
- `post/llm/`：TUI 新建 LLM component 使用的模板。
- `manifest.toml` 只描述模板元信息，不重复模板正文。

模板内容由 `config::template` registry 渲染，并由测试逐字节校验 `assets/config/**`。文档不要复制大片配置内容。

### 新建 LLM post component 向导

第一版只支持新增 LLM post component，路径固定：

```text
~/.config/shuohua/post/llm/<file_id>.toml
```

交互：

1. 选择模板：DeepSeek / OpenAI compatible / Anthropic / Custom。
2. 输入 file id。不能和现有文件重复。
3. 输入 provider name。默认等于 file id；不能和现有 `post/llm/*.toml` 里的 `name` 重复。
4. 选择接口形式：`openai` 或 `anthropic`。模板可预选。
5. 输入 base URL。模板可预填，但用户必须能改。
6. 输入 model。模板可预填，但用户必须能改。
7. 可选 provider-specific flags。第一版只给 DeepSeek 写 `extra_body.thinking = disabled`。
8. 创建文件，打开编辑器，返回 TUI 后刷新 inventory。

第一版不做网络验证，不自动加入 profile pipeline。创建配置和接入 profile 是两个动作。

当前已实现：

- Configure 模块导航：Overview / Main / Profile / PostProcessor / ASR Provider / Theme。
- Overview 首次进入运行 `shuo doctor` 并显示输出；`v` 手动刷新 inventory 并重跑 doctor。
- `o` 打开选中配置文件，`r` Finder 定位选中配置文件或配置目录。
- `R` 发送现有 UDS `reload_config`，并刷新 Configure inventory。
- PostProcessor 模块按 `n` 启动 LLM component wizard，创建 `post/llm/<file_id>.toml` 后打开编辑器。

## 当前剩余项

- History 时间范围筛选、批量标记、批量音频清理仍未实现。
- Configure 仍需把 `doctor` 和 TUI Overview 收敛到同一套结构化 diagnostics，而不是只显示 `shuo doctor` stdout。
- Profile route 辅助 workflow 仍未实现；新增 LLM component 不会自动修改 profile chain。
- 打开文件、Finder reveal、创建后自动打开编辑器这些 macOS 交互需要用户手动验证。
