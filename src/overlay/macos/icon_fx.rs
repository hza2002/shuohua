//! 状态图标的自绘动画效果（挂在 state_icon 背后的 FX 宿主 layer 上）。
//!
//! 每个状态一种"有设计感"的动效，比 SF Symbol 变量值更生动：
//! - Idle → 呼吸光晕（radial 渐变放大淡出，心跳/待命）
//! - Connecting → 雷达扩散环（3 个同心环错相位向外扩散淡出）
//! - Recording → 音频电平条（dB 感知响度驱动高度 + 错相位摆动，数据驱动每帧更新）
//! - Thinking → 跳动三点（经典"思考中"，3 点错相位上下弹）
//! - Stopping → 彗星尾 spinner（conic 渐变被圆环 mask 裁成环带，旋转）
//!
//! 设计约束：动画用无限 `CABasicAnimation`（渲染服务器驱动，创建一次，不每帧重加）；
//! 每帧只更新几何/颜色/显隐。效果各自惰性创建。Error 抖动是一次性的，留在 view.rs
//! 直接作用于符号本身。

use std::time::Instant;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_core_graphics::{CGColor, CGPath};
use objc2_foundation::{ns_string, NSArray, NSNumber, NSPoint, NSRect, NSSize, NSString};
use objc2_quartz_core::{
    kCAGradientLayerConic, kCAGradientLayerRadial, CABasicAnimation, CAGradientLayer, CALayer,
    CAMediaTiming, CAShapeLayer,
};

use super::chrome::color_from_rgb_alpha;
use crate::overlay::OverlayState;

const RADAR_RINGS: usize = 3;
const RADAR_PERIOD: f64 = 1.8;
const DOT_COUNT: usize = 3;
const DOT_PERIOD: f64 = 0.5;
const BAR_COUNT: usize = 4;
/// 各竖条相对电平的高度权重（中间高、两侧低），让电平条有「波形」轮廓。
const BAR_SHAPE: [f64; BAR_COUNT] = [0.55, 0.95, 1.0, 0.7];
/// 音量包络每帧朝目标逼近的比例；越小越平滑/越「黏」。
const BAR_SMOOTH: f64 = 0.4;
/// 静音时也保留的最小高度比例，配合摆动让电平条始终有点「活气」。
const BAR_FLOOR: f64 = 0.18;
/// 每条独立摆动的基础角速度（rad/s），错相位形成跳动波浪。
const BAR_WIGGLE_SPEED: f64 = 7.0;
/// 人声感知响度（dB）映射窗口：低于地板算静音，高于顶算满格。把线性 RMS 转成对数后
/// 拉伸到 0–1，正常说话才能填满量程（线性 RMS 很小，直接用几乎不动）。
const LOUD_FLOOR_DB: f64 = -55.0;
const LOUD_TOP_DB: f64 = -12.0;
/// 效果切换淡入时长。
const FADE_IN: f64 = 0.18;

/// 彗星尾 spinner：conic 渐变（头亮尾淡）被圆环 mask 裁成环带，整体旋转。
struct Comet {
    gradient: Retained<CAGradientLayer>,
    mask: Retained<CAShapeLayer>,
}

/// 图标背后的 FX 管理器。各效果惰性创建并复用；`render` 每帧显隐+更新对应效果。
/// 静态效果（光晕/雷达/跳点/彗星尾）靠脏检查避免每帧重建；Recording 电平条是数据
/// 驱动（跟 level 走），每帧更新。
pub(super) struct IconFx {
    host: Retained<CALayer>,
    comet: Option<Comet>,
    halo: Option<Retained<CAGradientLayer>>,
    radar: Option<Vec<Retained<CAShapeLayer>>>,
    dots: Option<Vec<Retained<CAShapeLayer>>>,
    bars: Option<Vec<Retained<CALayer>>>,
    /// 平滑后的音量包络（0–1），驱动电平条整体高度。
    bar_loud: f64,
    /// 电平条摆动相位的时间基准。
    started: Instant,
    /// 上次应用的 (state, color, 宽, 高)，相同则跳过静态效果的几何/颜色重建。
    last_key: Option<(OverlayState, u32, i32, i32)>,
}

impl IconFx {
    pub(super) fn new(host: Retained<CALayer>) -> Self {
        Self {
            host,
            comet: None,
            halo: None,
            radar: None,
            dots: None,
            bars: None,
            bar_loud: 0.0,
            started: Instant::now(),
            last_key: None,
        }
    }

    /// 每帧调用。`bounds` 是图标自身坐标系下的尺寸（0,0,w,h），`level` 是当前录音电平
    /// （0–1）。返回 true 表示该效果独占图标位、调用方应隐藏 SF 符号。
    pub(super) fn render(
        &mut self,
        bounds: NSRect,
        state: OverlayState,
        color_rgb: u32,
        level: f32,
    ) -> bool {
        let key = (
            state,
            color_rgb,
            bounds.size.width as i32,
            bounds.size.height as i32,
        );
        let activated = self.last_key.map(|k| k.0) != Some(state);
        let dirty = self.last_key != Some(key);

        // Recording 电平条每帧都要更新（level 在变）；其余静态效果只在 state/颜色/尺寸
        // 变化时重建。
        if !dirty && state != OverlayState::Recording {
            return hides_symbol(state);
        }
        self.last_key = Some(key);
        if dirty {
            self.hide_all();
        }

        match state {
            OverlayState::Idle => self.show_halo(bounds, color_rgb),
            OverlayState::Connecting => self.show_radar(bounds, color_rgb),
            OverlayState::Thinking => self.show_dots(bounds, color_rgb, activated),
            OverlayState::Stopping => self.show_comet(bounds, color_rgb, activated),
            OverlayState::Recording => self.show_bars(bounds, color_rgb, level, activated),
            OverlayState::Error => {}
        }
        hides_symbol(state)
    }

    fn hide_all(&self) {
        if let Some(comet) = &self.comet {
            comet.gradient.setHidden(true);
        }
        if let Some(halo) = &self.halo {
            halo.setHidden(true);
        }
        for ring in self.radar.iter().flatten() {
            ring.setHidden(true);
        }
        for dot in self.dots.iter().flatten() {
            dot.setHidden(true);
        }
        for bar in self.bars.iter().flatten() {
            bar.setHidden(true);
        }
    }

    // ── Idle：呼吸光晕 ──────────────────────────────────────────────

    fn show_halo(&mut self, bounds: NSRect, color_rgb: u32) {
        if self.halo.is_none() {
            let halo = CAGradientLayer::new();
            unsafe { halo.setType(kCAGradientLayerRadial) };
            halo.setStartPoint(NSPoint::new(0.5, 0.5));
            halo.setEndPoint(NSPoint::new(1.0, 1.0));
            // 放大 + 淡出的心跳脉冲。
            halo.add_loop(ns_string!("transform.scale"), 0.55, 1.6, 2.4, false, 0.0);
            halo.add_loop(ns_string!("opacity"), 0.55, 0.0, 2.4, false, 0.0);
            self.host.addSublayer(&halo);
            self.halo = Some(halo);
        }
        let halo = self.halo.as_ref().unwrap();
        halo.setFrame(bounds);
        let head = color_from_rgb_alpha(color_rgb, 0.5).CGColor();
        let edge = color_from_rgb_alpha(color_rgb, 0.0).CGColor();
        let colors = NSArray::from_slice(&[as_any(&head), as_any(&edge)]);
        unsafe { halo.setColors(Some(&colors)) };
        halo.setHidden(false);
    }

    // ── Connecting：雷达扩散环 ──────────────────────────────────────

    fn show_radar(&mut self, bounds: NSRect, color_rgb: u32) {
        if self.radar.is_none() {
            let mut rings = Vec::with_capacity(RADAR_RINGS);
            for i in 0..RADAR_RINGS {
                let ring = CAShapeLayer::new();
                ring.setFillColor(None);
                ring.setLineWidth(1.5);
                let offset = RADAR_PERIOD * i as f64 / RADAR_RINGS as f64;
                // 从中心小圈放大到满，同时淡出 → 一圈圈向外发。
                ring.add_loop(
                    ns_string!("transform.scale"),
                    0.2,
                    1.0,
                    RADAR_PERIOD,
                    false,
                    offset,
                );
                ring.add_loop(
                    ns_string!("opacity"),
                    0.85,
                    0.0,
                    RADAR_PERIOD,
                    false,
                    offset,
                );
                self.host.addSublayer(&ring);
                rings.push(ring);
            }
            self.radar = Some(rings);
        }
        let path = unsafe { CGPath::with_ellipse_in_rect(inset(bounds, 1.5), std::ptr::null()) };
        let stroke = color_from_rgb_alpha(color_rgb, 1.0).CGColor();
        for ring in self.radar.as_ref().unwrap() {
            ring.setFrame(bounds);
            ring.setPath(Some(&path));
            ring.setStrokeColor(Some(&stroke));
            ring.setHidden(false);
        }
    }

    // ── Thinking：跳动三点 ──────────────────────────────────────────

    fn show_dots(&mut self, bounds: NSRect, color_rgb: u32, activated: bool) {
        let diameter = (bounds.size.height * 0.2).max(3.0);
        let spacing = diameter * 1.7;
        if self.dots.is_none() {
            let mut dots = Vec::with_capacity(DOT_COUNT);
            for i in 0..DOT_COUNT {
                let dot = CAShapeLayer::new();
                let offset = DOT_PERIOD * i as f64 / DOT_COUNT as f64;
                // 上下弹跳（autoreverse），错相位形成"波浪"。
                dot.add_loop(
                    ns_string!("transform.translation.y"),
                    0.0,
                    diameter * 0.9,
                    DOT_PERIOD,
                    true,
                    offset,
                );
                self.host.addSublayer(&dot);
                dots.push(dot);
            }
            self.dots = Some(dots);
        }
        let path = unsafe {
            CGPath::with_ellipse_in_rect(
                NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(diameter, diameter)),
                std::ptr::null(),
            )
        };
        let fill = color_from_rgb_alpha(color_rgb, 1.0).CGColor();
        let cx = bounds.size.width / 2.0;
        let cy = bounds.size.height / 2.0;
        for (i, dot) in self.dots.as_ref().unwrap().iter().enumerate() {
            let x = cx + (i as f64 - (DOT_COUNT as f64 - 1.0) / 2.0) * spacing - diameter / 2.0;
            dot.setFrame(NSRect::new(
                NSPoint::new(x, cy - diameter / 2.0),
                NSSize::new(diameter, diameter),
            ));
            dot.setPath(Some(&path));
            dot.setFillColor(Some(&fill));
            dot.setHidden(false);
            if activated {
                fade_in(dot);
            }
        }
    }

    // ── Recording：音频电平条 ───────────────────────────────────────

    fn show_bars(&mut self, bounds: NSRect, color_rgb: u32, level: f32, activated: bool) {
        let bar_w = (bounds.size.width / (BAR_COUNT as f64 * 2.0)).max(2.0);
        let gap = bar_w;
        let max_h = bounds.size.height * 0.9;
        if self.bars.is_none() {
            let mut bars = Vec::with_capacity(BAR_COUNT);
            for _ in 0..BAR_COUNT {
                let bar = CALayer::new();
                bar.setCornerRadius(bar_w / 2.0);
                self.host.addSublayer(&bar);
                bars.push(bar);
            }
            self.bars = Some(bars);
        }
        // dB 感知响度 → 平滑成稳定的音量包络（避免抖动）。
        let loud = perceptual_loudness(level);
        self.bar_loud += (loud - self.bar_loud) * BAR_SMOOTH;
        // 整体高度 = 地板 + 响度撑起的部分；摆动让各条错相位跳动。
        let base = BAR_FLOOR + self.bar_loud * (1.0 - BAR_FLOOR);
        let t = self.started.elapsed().as_secs_f64();

        let fill = color_from_rgb_alpha(color_rgb, 1.0).CGColor();
        let cx = bounds.size.width / 2.0;
        let cy = bounds.size.height / 2.0;
        let total_w = BAR_COUNT as f64 * bar_w + (BAR_COUNT as f64 - 1.0) * gap;
        let left = cx - total_w / 2.0;
        for (i, bar) in self.bars.as_ref().unwrap().iter().enumerate() {
            let wiggle = 0.6 + 0.4 * (t * BAR_WIGGLE_SPEED + i as f64 * 1.3).sin();
            let v = (base * BAR_SHAPE[i] * wiggle).clamp(0.0, 1.0);
            let h = (v * max_h).clamp(bar_w, max_h);
            let x = left + i as f64 * (bar_w + gap);
            bar.setFrame(NSRect::new(
                NSPoint::new(x, cy - h / 2.0),
                NSSize::new(bar_w, h),
            ));
            bar.setBackgroundColor(Some(&fill));
            bar.setHidden(false);
            if activated {
                fade_in(bar);
            }
        }
    }

    // ── Stopping：彗星尾 spinner ────────────────────────────────────

    fn show_comet(&mut self, bounds: NSRect, color_rgb: u32, activated: bool) {
        if self.comet.is_none() {
            let gradient = CAGradientLayer::new();
            unsafe { gradient.setType(kCAGradientLayerConic) };
            gradient.setStartPoint(NSPoint::new(0.5, 0.5));
            gradient.setEndPoint(NSPoint::new(0.5, 0.0));

            let mask = CAShapeLayer::new();
            mask.setFillColor(None);
            mask.setLineWidth(2.5);
            // mask 用 alpha 通道裁剪，描边须不透明；颜色本身不显示。
            mask.setStrokeColor(Some(&color_from_rgb_alpha(0xFFFFFF, 1.0).CGColor()));
            unsafe { gradient.setMask(Some(&mask)) };

            gradient.add_loop(
                ns_string!("transform.rotation.z"),
                0.0,
                std::f64::consts::TAU,
                0.9,
                false,
                0.0,
            );
            self.host.addSublayer(&gradient);
            self.comet = Some(Comet { gradient, mask });
        }
        let comet = self.comet.as_ref().unwrap();
        comet.gradient.setFrame(bounds);
        comet.mask.setFrame(bounds);
        let mask_path =
            unsafe { CGPath::with_ellipse_in_rect(inset(bounds, 2.5), std::ptr::null()) };
        comet.mask.setPath(Some(&mask_path));
        // 头亮尾淡：满色扫到透明，沿环带形成彗星尾。
        let head = color_from_rgb_alpha(color_rgb, 1.0).CGColor();
        let tail = color_from_rgb_alpha(color_rgb, 0.0).CGColor();
        let colors = NSArray::from_slice(&[as_any(&head), as_any(&tail)]);
        unsafe { comet.gradient.setColors(Some(&colors)) };
        comet.gradient.setHidden(false);
        if activated {
            fade_in(&comet.gradient);
        }
    }
}

/// 线性 RMS（0–1）→ 人声感知响度（0–1）。先转 dB（对数），再把人声常用 dB 窗口拉伸到
/// 满量程。正常说话的线性 RMS 很小，不做这步电平条几乎不动。
fn perceptual_loudness(rms: f32) -> f64 {
    let rms = rms as f64;
    if rms <= 1e-5 {
        return 0.0;
    }
    let db = 20.0 * rms.log10();
    ((db - LOUD_FLOOR_DB) / (LOUD_TOP_DB - LOUD_FLOOR_DB)).clamp(0.0, 1.0)
}

/// 哪些状态的效果独占图标位（隐藏 SF 符号）。
fn hides_symbol(state: OverlayState) -> bool {
    matches!(
        state,
        OverlayState::Recording | OverlayState::Thinking | OverlayState::Stopping
    )
}

/// 一次性 opacity 0→1 淡入（removedOnCompletion 默认 true）。只用在没有无限 opacity
/// 动画的效果上（彗星尾/跳点/电平条），避免和光晕/雷达的循环 opacity 打架。
fn fade_in(layer: &CALayer) {
    let anim = CABasicAnimation::animationWithKeyPath(Some(ns_string!("opacity")));
    unsafe {
        anim.setFromValue(Some(&NSNumber::numberWithDouble(0.0)));
        anim.setToValue(Some(&NSNumber::numberWithDouble(1.0)));
    }
    anim.setDuration(FADE_IN);
    layer.addAnimation_forKey(&anim, Some(ns_string!("fadein")));
}

/// CALayer 扩展：挂一条无限循环的标量动画（from→to）。`offset` 用 timeOffset 错相位；
/// `autoreverse` 让来回往返（跳点用）。
trait LoopAnim {
    fn add_loop(
        &self,
        key_path: &NSString,
        from: f64,
        to: f64,
        duration: f64,
        autoreverse: bool,
        offset: f64,
    );
}

impl LoopAnim for CALayer {
    fn add_loop(
        &self,
        key_path: &NSString,
        from: f64,
        to: f64,
        duration: f64,
        autoreverse: bool,
        offset: f64,
    ) {
        let anim = CABasicAnimation::animationWithKeyPath(Some(key_path));
        unsafe {
            anim.setFromValue(Some(&NSNumber::numberWithDouble(from)));
            anim.setToValue(Some(&NSNumber::numberWithDouble(to)));
        }
        anim.setDuration(duration);
        anim.setRepeatCount(f32::MAX);
        anim.setAutoreverses(autoreverse);
        anim.setTimeOffset(offset);
        anim.setRemovedOnCompletion(false);
        self.addAnimation_forKey(&anim, Some(key_path));
    }
}

fn inset(rect: NSRect, by: f64) -> NSRect {
    NSRect::new(
        NSPoint::new(by / 2.0, by / 2.0),
        NSSize::new(
            (rect.size.width - by).max(0.0),
            (rect.size.height - by).max(0.0),
        ),
    )
}

/// CGColor 是 CF 类型；CAGradientLayer.colors 期望 CGColorRef 数组。运行时 CGColorRef 即
/// 合法 objc 对象，指针级转换到 `AnyObject` 安全。
fn as_any(color: &CGColor) -> &AnyObject {
    unsafe { &*(color as *const CGColor as *const AnyObject) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loudness_maps_speech_rms_into_useful_range() {
        // 静音 → 0；线性极小值也算静音。
        assert_eq!(perceptual_loudness(0.0), 0.0);
        assert_eq!(perceptual_loudness(1e-6), 0.0);
        // 正常说话的线性 RMS（~0.02–0.1）应落在量程中上段，而非贴底。
        let quiet = perceptual_loudness(0.02); // ≈ -34 dB
        let normal = perceptual_loudness(0.06); // ≈ -24 dB
        assert!((0.3..0.6).contains(&quiet), "quiet={quiet}");
        assert!((0.55..0.85).contains(&normal), "normal={normal}");
        assert!(normal > quiet);
        // 大声 → 满格。
        assert_eq!(perceptual_loudness(0.5), 1.0);
    }
}
