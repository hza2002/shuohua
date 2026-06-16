//! Debug-only overlay introspection. 整个文件由 `#[cfg(debug_assertions)]` 在
//! mod.rs 处门禁，release build 完全不编译，零开销。
//!
//! 当 macOS 升级 / Apple 改动 `NSGlassEffectView` 私有 SPI 命名或参数语义时，
//! 跑一次前台 daemon 就能从 tracing mirror 看到：
//!
//! - [`dump_glass_selectors`] 列出真实方法名 + ObjC 类型编码
//! - [`probe_glass_state_ranges`] 对 setter+getter 对做 0..=31 round-trip，看 Apple 是否 clamp
//!
//! 找到答案后修改 `view.rs::apply_glass_settings` 里的 selector 字符串 / 调整 config 范围。

use core::ffi::{c_char, c_uint, CStr};

use objc2::msg_send;
use objc2::runtime::{AnyClass, AnyObject, MessageReceiver, Sel};
use objc2_app_kit::NSGlassEffectView;

#[allow(non_camel_case_types)]
type Method = *mut core::ffi::c_void;

unsafe extern "C" {
    fn object_getClass(obj: *const AnyObject) -> *const AnyClass;
    fn class_copyMethodList(cls: *const AnyClass, count: *mut c_uint) -> *mut Method;
    fn method_getName(method: Method) -> Sel;
    fn method_getTypeEncoding(method: Method) -> *const c_char;
    fn sel_getName(sel: Sel) -> *const c_char;
}

/// 一行 glass 相关 trace，自动加 area 字段方便过滤。
pub fn trace(msg: impl AsRef<str>) {
    tracing::debug!(area = "overlay_glass", message = %msg.as_ref());
}

/// 枚举 glass 实例的运行时 class 上**所有**实例方法，过滤含
/// `subdued / dim / variant / style / opaque / blur` 关键词的
/// selector 名字打印到 debug 日志，**附带 ObjC 类型编码**（确认 ABI 是 `q`=NSInteger
/// 还是 `Q`=NSUInteger / `c`=BOOL，避免再次像 BOOL→long long 那样错传）。
///
/// 编码字符串格式示例 `v24@0:8q16` ——
/// 返回 `v`(void)，栈帧 24B，arg0=`@`(self) 在偏移 0，arg1=`:`(SEL) 在 8，arg2=`q`(NSInteger) 在 16。
/// 常用 primitive 编码: `c`=char/BOOL, `i`=int, `q`=long long/NSInteger,
/// `Q`=unsigned long long/NSUInteger, `B`=C99 bool, `f`=float, `d`=double, `@`=id, `:`=SEL.
pub fn dump_glass_selectors(glass: &NSGlassEffectView) {
    let needles = ["subdued", "dim", "variant", "style", "opaque", "blur"];

    unsafe {
        let obj: *const AnyObject = msg_send![glass, self];
        let cls = object_getClass(obj);
        let mut count: c_uint = 0;
        let methods = class_copyMethodList(cls, &mut count);
        if methods.is_null() {
            return;
        }
        for i in 0..count as isize {
            let method = *methods.offset(i);
            let name = CStr::from_ptr(sel_getName(method_getName(method))).to_string_lossy();
            let lower = name.to_ascii_lowercase();
            if needles.iter().any(|n| lower.contains(n)) {
                let encoding = CStr::from_ptr(method_getTypeEncoding(method)).to_string_lossy();
                tracing::debug!(area = "overlay_glass", selector = %name, encoding = %encoding, "glass selector");
            }
        }
        // 调试一次性调用，buffer 一直存活直到进程退出；不引 libc 只为 free()
    }
}

/// 对 (setter, getter) 对做 round-trip 写入 0..=31 / 读回，定位 Apple 是否在 ivar 层做 clamp。
///
/// **只处理 setter arg 和 getter ret 都是 `q`（NSInteger/long long）的 pair**。
/// 碰到其他 ABI（如 `@`=object 或 `c`=BOOL）直接跳过，防止 `send_message` 传错类型导致 panic。
///
/// 输出形如：
///
/// ```text
/// [overlay glass range] set_variant:       0..=31 stored as-is (no clamp probed)     ← 任意存储，视觉范围另测
/// [overlay glass range] set_subduedState:  skipped (setter arg=T, getter ret=q — type mismatch)
/// [overlay glass range] set_subduedState:  not available (setter missing)            ← 这个 macOS build 没装
/// ```
///
/// 注意：**storage range ≠ visual range**。Apple 可能存 0~99 但只在 0/1 有视觉差。
/// 这个 probe 只能缩小 visual 测试的搜索区间，最终视觉范围还得 config 调值肉眼扫。
///
/// probe 完每个 setter 会恢复成原值，不污染后续 `apply_glass_settings`。
pub fn probe_glass_state_ranges(glass: &NSGlassEffectView) {
    let pairs = [
        (c"set_variant:", c"_variant"),
        (c"set_subduedState:", c"_subduedState"),
    ];

    const PROBE_MAX: i64 = 31;

    unsafe {
        let obj: &AnyObject = msg_send![glass, self];
        for (setter_name, getter_name) in pairs {
            let setter = Sel::register(setter_name);
            let getter = Sel::register(getter_name);
            let setter_responds: bool = msg_send![obj, respondsToSelector: setter];
            let getter_responds: bool = msg_send![obj, respondsToSelector: getter];

            if !setter_responds {
                tracing::debug!(
                    area = "overlay_glass",
                    setter = %setter_name.to_string_lossy(),
                    "glass setter not available"
                );
                continue;
            }
            if !getter_responds {
                tracing::debug!(
                    area = "overlay_glass",
                    setter = %setter_name.to_string_lossy(),
                    getter = %getter_name.to_string_lossy(),
                    "glass getter missing"
                );
                continue;
            }

            // 拿 encoding 检查 ABI。setter arg#2 和 getter return 必须都是 q。
            let setter_enc = encoding_for_selector(obj, setter);
            let getter_enc = encoding_for_selector(obj, getter);
            let setter_arg = arg_type_at(&setter_enc, 2); // arg#0=self, arg#1=SEL, arg#2=value
            let getter_ret = ret_type(&getter_enc);

            if setter_arg != Some('q') || getter_ret != Some('q') {
                tracing::debug!(
                    area = "overlay_glass",
                    setter = %setter_name.to_string_lossy(),
                    setter_arg = %setter_arg.map(|c| c.to_string()).unwrap_or_else(|| "?".to_string()),
                    getter_ret = %getter_ret.map(|c| c.to_string()).unwrap_or_else(|| "?".to_string()),
                    "glass probe skipped for ABI mismatch"
                );
                continue;
            }

            // 保留原值，结束时恢复
            let original: i64 = obj.send_message(getter, ());

            let mut readbacks: Vec<i64> = Vec::with_capacity(PROBE_MAX as usize + 1);
            for v in 0..=PROBE_MAX {
                let _: () = obj.send_message(setter, (v,));
                let r: i64 = obj.send_message(getter, ());
                readbacks.push(r);
            }

            let first_mismatch = readbacks
                .iter()
                .enumerate()
                .find(|(w, r)| (*w as i64) != **r);
            let summary = match first_mismatch {
                None => format!("0..={PROBE_MAX} stored as-is (no clamp probed)"),
                Some((w, &r)) if w == 0 => {
                    format!("storage anomaly: wrote 0 read back {r}")
                }
                Some((w, &r)) => {
                    format!("stored 0..={} as-is; wrote {w} read back {r}", w - 1)
                }
            };

            let _: () = obj.send_message(setter, (original,));

            tracing::debug!(
                area = "overlay_glass",
                setter = %setter_name.to_string_lossy(),
                summary = %summary,
                "glass range probe"
            );
        }
    }
}

/// 从 ObjC method encoding 里查某个 selector 的类型编码。
/// encoding 字符串格式如 `v24@0:8q16`（setter）或 `q16@0:8`（getter）。
fn encoding_for_selector(obj: &AnyObject, sel: Sel) -> String {
    extern "C" {
        fn method_getTypeEncoding(method: Method) -> *const c_char;
        fn class_getInstanceMethod(cls: *const AnyClass, sel: Sel) -> Method;
        fn object_getClass(obj: *const AnyObject) -> *const AnyClass;
    }
    unsafe {
        let cls = object_getClass(obj as *const _ as *const AnyObject);
        let method = class_getInstanceMethod(cls, sel);
        if method.is_null() {
            return String::new();
        }
        CStr::from_ptr(method_getTypeEncoding(method))
            .to_string_lossy()
            .into()
    }
}

/// 返回 method encoding 里的 return type（第一个非数字字符）。
fn ret_type(enc: &str) -> Option<char> {
    enc.chars().find(|c| !c.is_ascii_digit())
}

/// 返回 method encoding 里第 `index` 个参数的 type char。
/// index 按 ObjC 约定：0=self, 1=SEL, 2=arg0, 3=arg1...
fn arg_type_at(enc: &str, index: usize) -> Option<char> {
    let types: Vec<char> = enc.chars().filter(|c| !c.is_ascii_digit()).collect();
    // types[0] = return type, types[1] = self, types[2] = SEL, types[3] = arg0 ...
    types.get(index + 1).copied()
}
