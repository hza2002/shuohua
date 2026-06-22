# CLI 命令设计

10 个面向用户的命令，扁平、无嵌套。`clap` derive 实现。

> 配套文档：[architecture.md](./architecture.md) | [schema.md](./schema.md) | [CHANGELOG.md](../CHANGELOG.md)

## 1. 命令列表

```
shuo                  # 主入口：智能 fallback
                      #   - UDS 存在 → 当 TUI 客户端连进去
                      #   - UDS 不存在 → fork daemon + 起 TUI
shuo --daemon         # 纯 daemon，不开 TUI（launchd plist 用这个）

shuo doctor           # 环境检查：
                      #   - 权限：Accessibility / Microphone 是否授权
                      #   - 录音输入：默认麦克风设备是否存在、格式是否可用
                      #   - 配置校验：解析主 config.toml、profile/*.toml、
                      #               asr/*.toml 和 post/**/*.toml（本地，不跑 daemon）
                      #   - 打印 effective config：merge 后实际生效的配置
                      #     （voice.record_audio = off | lossless | compact）
                      #   - ASR / LLM Provider：`--runtime` 显式触发；走实际可运行性检查
                      #   - launchd 状态：plist 是否装、daemon 是否在跑

shuo config-template  # 一次性导出全部内置模板 registry + theme presets 到指定目录
                      # （默认 $XDG_CONFIG_HOME/shuohua/templates）；写入前预检全部目标路径，
                      # 任一文件已存在则拒绝覆盖且不写任何文件
                      #   --lang <auto|en-US|zh-CN> 控制生成注释语言

shuo install          # 装 launchd plist（~/Library/LaunchAgents/）+ launchctl bootstrap
                      # plist ProgramArguments = ["shuo", "--daemon"]
shuo uninstall        # launchctl bootout + 删 plist

shuo start            # launchctl kickstart（daemon 已装但停了）
shuo stop             # 通过 UDS 请求 daemon 正常退出，确认旧 PID 消失后才返回
shuo restart          # stop 成功且旧 PID 已退出后再 start
shuo status           # 走 UDS daemon_status op：PID、起来多久、当前是否在录音

shuo version          # 同 --version；shuo help / --help / -h 同。clap 默认 alias，保留
```

## 2. 设计要点

- **CLI runtime 由 dispatcher 单点持有**：`cli::run_command` 为显式子命令创建一个
  current-thread Tokio runtime，再进入 async dispatch。子命令模块不创建或嵌套
  runtime；需要 Tokio I/O 的 handler 写成 `async fn`，纯文件/进程操作保持同步。
  daemon 和 smart fallback/TUI 各自持有独立 runtime，不归 CLI dispatcher 管理。
- **`install` / `uninstall` 不管 binary**。binary 装哪里靠 `cargo install` / brew / 手动 cp，CLI 不掺和。
- **`--daemon` 是 flag 而非子命令**：避免污染顶层命令列表，仅 launchd plist 实际调用。
- **删除原 `shuo config` 子命令族**：用户用编辑器（lazyvim 等）直接编辑 `~/.config/shuohua/config.toml`、`~/.config/shuohua/profile/*.toml` 和 `~/.config/shuohua/post/**/*.toml`。校验/打印归 `doctor`。
- **删除原 `shuo history` 子命令族**：TUI 看历史。脚本要 grep 直接读 `~/.local/state/shuohua/history/YYYY-MM.jsonl`，schema 见 [schema.md](./schema.md)。
- **VAD shadow trace 不是 CLI 命令**。开发观测走 dev sidecar：
  `cargo run --features dev`，并在 `~/.config/shuohua/config.toml` 的 `[dev]` 里设置 `vad_trace = true`。
  Trace 写入 state dir 的 `traces/<recording_id>.jsonl`，便于评估 pause/resume
  切分质量。完整事件清单以 [schema.md §4](schema.md#4-vad-trace开发期-sidecar) 为准。
- **`doctor` 有退出码语义**：所有本地检查都会尽量跑完；只要存在会阻断正常使用的
  ERROR，命令最终返回非 0。warning / skipped 不影响退出码。`--runtime` 未开启时
  runtime skipped 不算失败；开启后 ASR / LLM runtime 失败算失败。daemon status
  查询超时为 1s；LLM runtime 单项超时为 15s。
- **`status` 不无限等待 daemon**：UDS daemon_status 查询超时为 1s；超时返回非 0。
- **`stop` 等待真实退出**：Shutdown 回包必须是带有效 PID 的 `daemon_status`；
  CLI 随后最多等待 20s，确认该 PID 消失后才打印 stopped。20s 上限覆盖 daemon
  最多 15s 的 active session graceful shutdown。超时返回非 0，不发送 signal，
  不强杀进程。
- **`restart` 不跨过失败的 stop**：只有 stop 成功并确认旧 PID 已退出后才执行
  launchctl start；Shutdown IPC、PID 等待或超时错误都会原样阻止 start。
- **默认快捷键兼顾普通键盘**：内置 config template 使用
  `right_option:double` toggle 录音、`escape` 取消。用户可在 `config.toml`
  改成任意支持的纯按键、修饰键组合或 double 形式，完整语法见
  [hotkey](modules/hotkey.md)。
- **macOS 兼容边界**：发布构建使用 macOS 26 SDK、deployment target macOS 15。
  应用在 macOS 15+ 可运行；Apple ASR provider 依赖 SpeechAnalyzer，仅 macOS 26+
  可用。云 ASR provider（例如 Doubao）在 macOS 15+ 均可使用；macOS 15–25 配置
  `provider = "apple"` 会在录音开始前 fail-fast，不会静默切到云端。

## 3. 用户旅程

```
1. brew install shuohua           # 或从源码 cargo install --path .
2. shuo doctor                    # 看权限缺哪个，按指示授权
3. shuo install                   # 装 launchd plist + 启动
4. 双击右 Option 录音             # 默认快捷键，可在 config.toml 修改
5. shuo                           # 想看实时状态：弹 TUI
6. 想改配置：TUI Configure 打开/创建配置，或直接编辑 toml 文件
7. shuo restart                   # 出问题就重启
8. shuo uninstall + cargo uninstall  # 不用了
```

## 4. launchd plist 模板

`shuo install` 生成下面这份 plist 到 `~/Library/LaunchAgents/com.hza2002.shuohua.plist`，然后 `launchctl bootstrap gui/$UID` 它。

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>com.hza2002.shuohua</string>

  <key>ProgramArguments</key>
  <array>
    <string>/Users/<USER>/.local/bin/shuo</string>     <!-- 安装时替换为当前 shuo 可执行文件绝对路径 -->
    <string>--daemon</string>
  </array>

  <key>RunAtLoad</key>
  <true/>

  <key>KeepAlive</key>
  <dict>
    <key>SuccessfulExit</key>
    <false/>                                            <!-- 崩了重启，主动 stop 不重启 -->
  </dict>

  <key>ThrottleInterval</key>
  <integer>10</integer>                                 <!-- 崩溃重启间隔下限 10s，防抖 -->

  <key>ProcessType</key>
  <string>Interactive</string>                          <!-- AppKit overlay 必须 -->

  <key>StandardOutPath</key>
  <string>/Users/<USER>/.local/state/shuohua/launchd.stdout.log</string>

  <key>StandardErrorPath</key>
  <string>/Users/<USER>/.local/state/shuohua/launchd.stderr.log</string>
</dict>
</plist>
```

**关键决策**：

- **Label**：`com.hza2002.shuohua`（reverse-DNS，参考 yabai `com.koekeishiya.yabai` 约定）
- **KeepAlive { SuccessfulExit: false }**：daemon 崩了自动重启；`shuo stop` 走 UDS graceful shutdown，daemon runtime 收到 `shutdown` 后先停止当前录音并等待 bounded 收尾，再退出 0，不触发重启。UDS 不可达时 stop 返回错误，避免用 signal kill 触发 KeepAlive 重启。
- **ThrottleInterval=10s**：防止崩溃循环把系统打爆
- **ProcessType=Interactive**：AppKit GUI 必须，否则 `NSPanel` 显不出来
- **StandardOutPath / StandardErrorPath**：保留为兜底。正式 daemon 日志写入 `~/.local/state/shuohua/logs/shuo-YYYY-MM-DD.log`；launchd stdout/stderr 只用于 panic、极早期失败、logger 初始化失败等正式 logger 尚未接管的情况。
- **不写 WorkingDirectory**：daemon 用绝对路径访问 `~/.config/shuohua/`、`~/.local/state/shuohua/`，cwd 无关
