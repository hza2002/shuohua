//! 日志门禁。分级规则见 [`docs/DESIGN.md`](../../docs/DESIGN.md) §2.13。
//!
//! 简版：
//!
//! - `eprintln!` 直调 = release + debug 都打印 → 错误 / 警告 / 启动 / 致命路径
//! - `crate::debug_println!` = 仅 debug build 打印 → narration / probe / 每步细节
//!
//! release binary 跑 launchd 时 stderr 走 `.plist` 的 `StandardErrorPath`，
//! 文件应该只长 error 行。narration 想看就 `cargo run`（debug build）。

// 两份 cfg 分支是 stable 写法。直接在 macro 展开里塞 `#[cfg(debug_assertions)]`
// 会落到"表达式属性"位置，目前仍 unstable。
// 内部不带尾分号，让调用者能用在 expression 位（如 match arm）也能当 stmt 用。
#[cfg(debug_assertions)]
#[macro_export]
macro_rules! debug_println {
    ($($arg:tt)*) => { eprintln!($($arg)*) };
}

// release 走 no-op，但用 `format_args!` 消耗一次入参，避免上层 `logid` / `seq` /
// `text` 这种"只为日志取值"的局部变量在 release 触发 unused_variables 警告。
// `format_args!` 返回 `Arguments<'_>`，纯栈对象、`let _ =` 接住即丢，零 IO 零分配。
#[cfg(not(debug_assertions))]
#[macro_export]
macro_rules! debug_println {
    ($($arg:tt)*) => {{
        let _ = format_args!($($arg)*);
    }};
}
