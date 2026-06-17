# CLI 命令设计

10 个面向用户的命令，扁平、无嵌套。`clap` derive 实现。

> 配套文档：[DESIGN.md](./DESIGN.md) | [SCHEMA.md](./SCHEMA.md) | [CHANGELOG.md](../CHANGELOG.md)

## 1. 命令列表

```
shuo                  # 主入口：智能 fallback
                      #   - UDS 存在 → 当 TUI 客户端连进去
                      #   - UDS 不存在 → fork daemon + 起 TUI
shuo --daemon         # 纯 daemon，不开 TUI（launchd plist 用这个）

shuo doctor           # 环境检查：
                      #   - 权限：Accessibility / Microphone 是否授权
                      #   - 录音输入：默认麦克风设备是否存在、格式是否可用
                      #   - 终端识别：识别当前终端 App，提示授权对象
                      #   - 配置校验：解析主 config.toml + 所有 per-app post/*.toml，
                      #               报 file:line 错误（不跑 daemon）
                      #   - 打印 effective config：merge 后实际生效的配置
                      #   - ASR 连通性：只测 WebSocket handshake + auth（不发 PCM，
                      #                 避免触发计费）；--full 才真发 1s 静音 PCM
                      #   - launchd 状态：plist 是否装、daemon 是否在跑

shuo install          # 装 launchd plist（~/Library/LaunchAgents/）+ launchctl bootstrap
                      # plist ProgramArguments = ["shuo", "--daemon"]
shuo uninstall        # launchctl bootout + 删 plist

shuo start            # launchctl kickstart（daemon 已装但停了）
shuo stop             # launchctl kill
shuo restart          # stop + start
shuo status           # 走 UDS daemon_status op：PID、起来多久、当前是否在录音

shuo version          # 同 --version；shuo help / --help / -h 同。clap 默认 alias，保留
```

## 2. 设计要点

- **`install` / `uninstall` 不管 binary**。binary 装哪里靠 `cargo install` / brew / 手动 cp，CLI 不掺和。
- **`--daemon` 是 flag 而非子命令**：避免污染顶层命令列表，仅 launchd plist 实际调用。
- **删除原 `shuo config` 子命令族**：用户用编辑器（lazyvim 等）直接编辑 `~/.config/shuohua/config.toml`、`~/.config/shuohua/profile/*.toml` 和 `~/.config/shuohua/post/**/*.toml`。校验/打印归 `doctor`。
- **删除原 `shuo history` 子命令族**：TUI 看历史。脚本要 grep 直接读 `~/.local/state/shuohua/history/YYYY-MM.jsonl`，schema 见 [SCHEMA.md](./SCHEMA.md)。
- **VAD shadow trace 不是 CLI 命令**。M10 默认 build 已包含 Silero，但开发观测仍只走 dev sidecar：
  `cargo run --features dev`，并在 `~/.config/shuohua/config.toml` 的 `[dev]` 里设置 `vad_trace = true`。
  Trace 写入 state dir 的 `traces/<recording_id>.jsonl`，包含 `vad_frame`、`vad_transition`、
  `session_start`、`session_finalize_start`、`session_done`、`session_open_error`、
  `asr_segment`、`recording_end` 等事件，便于评估 pause/resume 切分质量。

## 3. 用户旅程

```
1. cargo install shuohua          # 或 brew install
2. shuo doctor                    # 看权限缺哪个，按指示授权
3. shuo install                   # 装 launchd plist + 启动
4. 按 F9 录音                     # 直接用
5. shuo                           # 想看实时状态：弹 TUI
6. 想改配置：直接编辑 toml 文件   # lazyvim 友好
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
    <string>/Users/<USER>/.local/bin/shuo</string>     <!-- 安装时替换为 which shuo 的绝对路径 -->
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
- **KeepAlive { SuccessfulExit: false }**：daemon 崩了自动重启；`shuo stop` 主动停 = 正常退出码 = 不重启
- **ThrottleInterval=10s**：防止崩溃循环把系统打爆
- **ProcessType=Interactive**：AppKit GUI 必须，否则 `NSPanel` 显不出来
- **StandardOutPath / StandardErrorPath**：保留为兜底。正式 daemon 日志写入 `~/.local/state/shuohua/logs/shuo-YYYY-MM-DD.log`；launchd stdout/stderr 只用于 panic、极早期失败、logger 初始化失败等正式 logger 尚未接管的情况。
- **不写 WorkingDirectory**：daemon 用绝对路径访问 `~/.config/shuohua/`、`~/.local/state/shuohua/`，cwd 无关
