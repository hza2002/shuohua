# config — 配置加载与热重载

**TL;DR**：`notify` 监听配置**目录**不监听文件（编辑器换 inode 会丢事件）；subscriber 自带 diff 只在关心字段变化时动作；`config.toml`/`theme/*.toml` 立即生效，profile/asr/post 下次录音才读。

> **何时读**：改配置 schema、热重载、profile 路由落地、theme。
> **不在这里**：profile 选哪套 ASR/post 的语义见 [post](post.md)；字段格式不是 schema（那是 history/UDS，见 [schema](../schema.md)）。
> **代码**：`src/config/`（`main.rs`/`profile.rs`/`theme.rs`/`schema.rs`/`spec.rs`/`diagnostics/`/`template/`/`asr/`/`post/`）；热重载在独立的 `src/reload.rs`。

## reload 模块边界

`reload.rs` 单向依赖 config/overlay/i18n/hotkey 的对外 API（`OverlayHandle`、`i18n::init`、`hotkey::parse`），**不被它们反向 import**——一个集中的"翻译层"：watcher 一个 source，subscriber N 个 sink。

- `watch_with_handle(path, overlay)` → notify watcher（专用 std::thread）+ 手动 reload handle
- `spawn_overlay` / `spawn_i18n` / `spawn_hotkey` 三个 subscriber，各自 diff `prev` 只对关心字段动作。
- `Rx = watch::Receiver<Arc<RuntimeConfig>>`（含主配置 + effective theme + fallback warning）。

## 实现要点

- **监听目录而非文件**：编辑器保存常 atomic rename 换 inode，监听文件本身丢事件。
- 自动 reload 只把 `config.toml` + `theme/*.toml` 当触发源；`profile/*.toml`/`asr/*.toml`/`post/**` 不触发 broadcast，下次录音开始同步读最新。
- **150ms debounce**（一次保存常触发 2-3 事件）。
- **parse 失败保留旧值**：只打日志 `config reload failed; keeping previous config`，不发空值。

## 字段覆盖矩阵

| 字段 | 生效 | 路径 |
|---|---|---|
| `[overlay].*` | 立即（next render） | `spawn_overlay` → rebuild_chrome |
| `ui.language` | 立即（重译 label） | `spawn_i18n` → `i18n::init` + `Relabel` |
| `[hotkey].trigger` / `[hotkey].cancel` / `[hotkey].resume` | 立即（下次按键） | `spawn_hotkey` → mpsc<Bindings> → 主循环换 Tracker+Suppressor |
| `[ui].theme*` / `theme/*.toml` | 立即 | spawn_overlay / TUI reload 重载 effective theme |
| `[voice].*` 全部 | 下次起 session | 主循环 `cfg_rx.borrow()` 取快照 |
| `[profile]` 路由 / `profile/*.toml` / `post/**` | 下次起 session | toggle ON 时选 Profile |
| 手动 `{"op":"reload_config"}` | 立即 | 走 UDS server，复用同一 parse+broadcast 入口 |

## 文件 ID 与展示名

`profile/*.toml`、`asr/*.toml`、`post/*.toml`、`theme/*.toml`
的文件名 stem 是机器 ID，必须以小写字母开头，后续只允许小写字母、数字、
`-`、`_`。profile 路由、`post.chain`、ASR instance lookup 都只引用这个 ID；
非法文件名在加载/doctor 阶段按 Error 处理。

`name` 字段是展示标签，允许任意 UTF-8，不参与路由和引用，可选、可重复。
所有实例类型（`profile`/`asr`/`post`/`theme`）的 `name` 都可不填；未填写时
展示层从文件 stem 派生可读名——把 `-`/`_` 转空格、各段首字母大写，例如
`my-profile.toml` 显示为 `My Profile`（裸 stem 全小写加连字符不适合直接展示）。

## 配置实例契约

所有可被引用的配置都是**文件 stem 实例**：

- **实例 ID = 文件 stem**（例如 `asr/work.toml` → ID 为 `work`）；引用方只认 ID，与实现无关。
- **`type` = 选实现**（闭合枚举，仅多实现家族有此字段）；缺失或值不合法时 resolver 报错。
  - `asr`：`type = "apple" | "aliyun" | "doubao" | "tencent"`（必填，无默认）
  - `post`：`type = "rule" | "llm"`（必填，无默认）
- **`name` = 人类可读备注/显示标签**（可选，任意 UTF-8）；从不用作引用键、不参与任何功能判断；可重复，缺失时展示层用 stem 派生名兜底。
- **其余键私有**：由各实现自己 deserialize，上层不依赖实现私有字段。
- **没有隐式 Apple 默认**：`asr.instance` 指向的 `asr/<id>.toml` 必须存在且带 `type`，文件不存在直接报错，不回退任何内置实例。

| 字段 | 含义 | 出现在 | 可被引用 |
|---|---|---|---|
| 文件 stem | 实例 ID | 所有实例文件 | ✅ 唯一引用键 |
| `type` | 选哪个实现（闭合枚举） | asr（`apple`\|`doubao`）、post（`rule`\|`llm`） | ❌ |
| `name` | 人类可读备注/显示标签 | 任意实例，可选 | ❌ |
| `format` | llm 协议方言（`openai`\|`anthropic`） | 仅 post `type = "llm"` | ❌ |

post `type = "llm"` 的 `base_url` 必填：实例文件必须显式写出 base_url，不从 `name` 或已知 provider 推断。

**Profile → ASR 引用链**：`profile.asr.instance` 的值是 ASR 实例 ID；resolver (`config::asr::instance::resolve_instance`) 以此查找 `asr/<id>.toml`，读取 `type` 字段得到 `AsrKind`，再构造对应 provider。引用键永远是实例 ID，不是实现名（`apple`/`doubao`）。

**`post.chain` 引用**：链元素是裸实例 ID（如 `deepseek`），resolver 查 `post/<id>.toml` 读 `type` 字段区分 rule/llm，与 ASR instance 同构。

**`[post.overrides.<id>]` profile 级 llm 覆盖**：`<id>` 必须是 `post.chain` 中的元素，且目标 `post/<id>.toml` 必须是 `type = "llm"` 组件；表内字段按 llm schema 校验，合并进该组件（override 优先）。三类硬错误（diagnostics 报 Error，path=`post.overrides`）：`<id>` 不在 chain 中（dangling）、`<id>` 目标是 rule 组件、表内含未知字段。运行期只对 llm 组件套用覆盖、对 rule 静默忽略（软链接语义在运行期，硬校验在 diagnostics）。

## voice preprocess

`[voice.preprocess].backend` 默认是 `webrtc`：cpal 采集后在 recorder 线程内用 WebRTC Audio Processing 做 noise suppression、high-pass 和保守 digital AGC，启动开销接近 off；`webrtc-audio-processing` 是普通依赖（`features = ["bundled"]`），随每次构建编入 bundled C++。`off` 用原始采集、不做预处理，是唯一留存完整原始音频的路径。`apple` 使用 macOS 原生语音处理采集（AEC/降噪/增益），效果好但每次启动建立连接会慢一点，作为音频环境差时的兜底。模板必须导出真实默认值，并在注释里说明已支持取值；未实现的后端不要写进用户模板说明。参数取舍见 [webrtc_backend.md](webrtc_backend.md)。

## Hotkey 热替换

CGEventTap 在 OS 层捕获所有键盘事件、不过滤——trigger/cancel/resume 切换只影响 `TrackerSet` 判定和 suppressor 规则。重置成本 = 主循环 select 收到新 bindings → 重建 tracker 状态（避免旧 binding 半按串到新），不拆 CGEventTap。parse 失败或三个 binding 任意冲突时保留旧 bindings。

## TUI 写入面

> `config.toml` 与 profile/asr/post 文件的标量字段都可在 TUI 内编辑；Table 字段仍 ReadOnly，Array 字段按每行一个元素在弹窗中编辑。鼠标点击即选中光标所在的模块/源/字段，滚轮滚动当前列表；顶部 Tab 栏点击可全局切页。字段列表下方有「选中项详情区」，展示选中字段的完整 key/值/默认值/说明（列表列会截断的长 key 在此显示全）；内容过长时用滚轮、PageUp/PageDown 或 Ctrl-U/Ctrl-D 滚动（详情区滚动状态在切换选中项时清零）；值保留原始换行。多行/数组弹窗编辑支持方向键移动光标（上下跨行、左右移动），用终端原生光标在真实位置显示、不位移文本。键盘导航只有两级焦点：左边 Modules、右边内容区（来源栏 + 字段合成一层）。模块列表用 j/k 移动、`l`/`Enter` 进入内容区；内容区里 j/k 移字段、h/l 横向切来源 tab（Main/Overview 无来源）、`Enter` 编辑、`Esc` 统一返回模块列表（进出用同一组语义）；Select/Toggle 编辑时上下左右与 hjkl 都可切值。字段列表超出可视高度时随选中项居中滚动（同 History 列表），长列表不会看不到下面的项。

- **控件派生**：`FieldSpec` 的 `kind`/`values`/`min`/`max` 决定控件类型（Toggle / Select / Number / Text）；ReadOnly 不允许写入。多行字段（`multiline`）、hotkey 字段（`keycapture`）和 secret 字段通过弹窗模态编辑。hotkey 字段是**纯文本输入**（弹窗预填当前值、`Enter` 保存），不做「按键捕获」：daemon 的全局 CGEventTap 会抢走目标键、终端也收不到本程序的热键词汇（修饰键单击 / 双击 `:double` / 左右侧 / F13-F20），捕获既不可靠又会误触录音。弹窗 hint 直接展示语法和示例；保存时按语法校验（见下）。`keycapture` 这个 `FieldSpec` flag 仅用来把 hotkey 字段路由到这个带语法说明的弹窗。
- **默认值**：`FieldSpec.default` 提供初始值；专门的 drift test 在 `cargo test` 时确认 schema 声明的默认与代码不脱节。
- **写入合约**（`field_write::set_field` / `apply_field` / `unset_field`）：
  - 保存前对整份文档跑 `validate_value`，按 `blocking_errors` 决定是否放行：结构性 Error（类型 / 范围 / 未知字段 / 枚举）阻止写盘；**未填 secret 这类 readiness Error 放行**——好让含多个空密钥的半成品逐字段填；Warning 一律放行。真正拦空密钥的是加载/运行时的 `reject_schema_diagnostics`（连 Warning 一起 bail），以及 provider 自己的非空校验。
  - `set_field` = `apply_field`（在内存 `DocumentMut` 上写入 + 校验）再原子写盘；新建草稿复用同一 `apply_field`，故新建与编辑同一套校验（见下「新建 = 编辑」）。
  - 写盘前先写临时文件，然后 `rename` 原子替换，避免写一半的配置文件。
  - `unset_field` 删除键，让字段回到 schema 默认；字段若原本不存在则静默成功。
- **新建 = 编辑**：新建实例不是独立表单，而是一份**未落盘的内存草稿文档**（ASR 用 `AsrDraftDoc`）。字段控件/校验完全复用 `field_view` + `field_write`——草稿只是「还没写盘的文件」：全字段（含 secret）就地填，`Ctrl-S` 一次落盘完整文件，`Esc` 干净丢弃、不留半成品。ASR 的 `type` 是首行可切 `Select`，切换即按 registry 模板重铺草稿（同 LLM 的 `preset`）；`file_id` 是实例文件名、不是 schema 字段。LLM/Profile 目前仍用各自的 `LlmComponentDraft`/`ProfileDraftForm`（字段单独定义，尚未并入 schema 驱动草稿）。
- **打开配置文件**：以 TUI 内编辑为主。`e` 用系统默认应用打开当前选中的配置文件（不走 `$EDITOR`，避免在 TUI 里 spawn 终端编辑器抢屏），`r` 在 Finder 中定位文件 / 打开所在目录。新建组件后不自动打开编辑器。
- **热重载**：写入 `config.toml` 后立即通过 `Command::ReloadConfig` 触发 daemon 重载（`reload_config`）；其他文件（profile/asr/post）的变更在下次录音会话开始时生效。
- **语义校验**：`hotkey.trigger` / `hotkey.cancel` / `hotkey.resume` 在保存前额外调用 hotkey bindings parser，解析失败或三者冲突会阻止写盘并回显错误。
- **Profile composer**：Profile 模块的字段区不是普通逐字段列表，而是专用的 composer——把 name / asr.instance / 按已解析 provider 展开的 asr 覆盖 / hotwords / post 链成员及每个 llm 成员按 PostLlm schema 展开的覆盖，合成一层可编辑视图。继承值灰、显式覆盖蓝、无效项（stale 覆盖 / 悬空链成员）红。新建 Profile 只创建容器并绑定一个已有 ASR 实例；没有 `asr/*.toml` 时不进入新建表单，post chain 创建后用 composer 的 `a` 添加。`a` 弹出成员选择器往链尾追加、`x` 移出选中链成员、Shift-J/K 重排、`D` 把选中覆盖/标量还原为继承、`X` 一次清掉全部无效覆盖；`Enter` 复用共享编辑器/弹窗改选中行。所有写入只落 profile 文件；覆盖在写盘前按**已解析的 provider/component schema** 校验（profile 自身的 `[asr]`/`[post.overrides]` 自由表拦不住类型错误），失败走同一套内联错误弹窗。

## Theme

`theme/<id>.toml` 描述 TUI+overlay 颜色和少量 overlay token；字段缺省从内置 `gruvbox-dark` 补齐，用户文件优先于同名内置 preset。`theme_tui`/`theme_overlay` 可单独覆盖，空字符串跟随 `theme`。内置 theme 唯一来源是 `assets/themes/*.toml`，`build.rs` 编译期校验并生成嵌入 registry。
