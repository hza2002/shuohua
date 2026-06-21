# hotkey — 全局热键

**TL;DR**：纯函数状态机（`RawEvent + Instant → HotkeyEvent`）+ CGEventTap 真吞事件；时间常数写死不暴露；suppress 漏吞会让前台 App modifier 状态泄漏。

> **何时读**：改热键语法、suppress 行为、tracker 状态机、CGEventTap 桥。
> **不在这里**：reload 时热替换 trigger 见 [config](config.md)。
> **代码**：`src/hotkey/`。`combo.rs`(类型+精确匹配) / `parse.rs`(grammar→Combo) / `tracker.rs`(状态机) / `suppressor.rs`(吞哪些) / `provider_darwin.rs`(CGEventTap+ModMask 解码) / `bindings.rs`(trigger/cancel + cancel-first TrackerSet)。

## 配置语法（grammar）

```text
trigger  := combo (":double")?
combo    := token ("+" token)*
token    := ("left_"|"right_")? mod | key
mod      := cmd|command | ctrl|control | opt|alt|option | shift   # canonical: cmd/ctrl/opt
key      := f1..f20 | a..z | 0..9 | space|tab|escape|esc|return|delete|backspace
         |  up|down|left|right | ;,./\[]'`-=
```

| 触发形态 | 例 | 时机 | suppress |
|---|---|---|---|
| 纯按键 | `f16` `escape` | KeyDown 且 mods 全无；auto-repeat 不重触发 | 该 key down + 配对 up |
| 修饰键+键 | `cmd+r` `left_cmd+shift+r` | KeyDown 时 mods 精确匹配（未指定的必须松开） | 仅 key 部分 down/up；modifier 全放行 |
| 修饰键单按 | `right_shift` `cmd+shift` | clean tap：按下→松开期间无普通键、无额外 modifier、<500ms | 不吞任何（modifier 太常用） |
| 双击 | 上面任一 `+:double` | 两次 tap 在 400ms 内 | keyed 两次候选 cycle 都吞；modifier-only 不吞 |

精确匹配：trigger 没写的 modifier 必须松开（`cmd+r` 下按 `cmd+shift+r` 不触发，跟 VSCode 一致）。别名输入全接受，`Display` 输出 canonical 以保 TUI capture round-trip 稳定。

## 时间常数（写死 `tracker.rs` 顶部，不暴露用户）

- `MOD_HOLD_THRESHOLD = 500ms`：modifier-only tap 上限，超出算长按。BetterTouchTool/Hammerspoon 社区收敛值（Karabiner 1000ms 偏慢，250ms 偏激进）。
- `DOUBLE_TAP_WINDOW = 400ms`：双击两次间隔上限。macOS Dictation Right Shift×2 实测 ~350ms，留 50ms 容错。
- 单按 trigger 不被双击窗口延迟：每个 trigger 只有一种解释，单按版本不等 400ms。

## 依赖红线

suppress 落地依赖 `core-graphics ≥ 0.25` 的 `CallbackResult::Drop`（真返回 NULL 给系统）。0.24 没有这条路径 → suppress 静默失效。回滚此依赖前先确认。

## 本模块持有的不变量

- **#1** CGEventTap 回调跑专用 OS 线程 CFRunLoop，不让出。
- **#2** C→Rust 事件桥用 pipe，不用 cgo callback。被 suppress 的事件也照写（tracker 仍要看见每个事件做 combo 匹配）。
- **#6** 热键注册在系统启动前完成；运行时新增（transient Esc/R）要保证 dispatcher 已起。
- **#7** down/up 配对吞：keydown 被吞则对应 keyup 必吞，否则前台 App 看到孤立 keyup → modifier 泄漏。回调里维护"已吞物理键集合"，keyup 查表。reload 中途换 binding 也靠这条保持安全。

## 测试

`tracker` / `suppressor` 是纯函数式状态机，`proptests.rs` 用 proptest 跟参考模型逐步等价；断言 KeyDown/KeyUp 配对、suppress 不变量（#7）。
