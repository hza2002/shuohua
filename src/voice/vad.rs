//! 客户端 VAD：WebRTC VAD + 滑动窗口去抖 + 切换最小间隔防抖。
//!
//! 设计来源：DESIGN.md §2.9。
//!
//! 输入：16kHz s16le mono PCM，任意长度（cpal callback 一次 ~5–10ms，约 80–160
//! samples，不到一个 VAD 帧；feed 内部自动累积 20ms 帧再判）。
//! 输出：当前去抖后的 [`VadState`]；[`feed`](VadGate::feed) 返回这次调用产生的
//! 最后一次状态切换事件（如果有）。
//!
//! 去抖策略：
//! - 滑窗 5 帧 (100ms)，里面 ≥3 帧 voiced 才认为 Voiced；≥3 帧 unvoiced 才认为
//!   Unvoiced。短脉冲（一两帧）不切。
//! - 切换最小间隔 1s：防键盘声、咳嗽这类把 ON/OFF 抖来抖去。

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use webrtc_vad::{SampleRate, Vad, VadMode};

/// 16kHz 下 20ms 一帧 = 320 samples（WebRTC VAD 支持的合法帧长之一）。
const FRAME_SAMPLES: usize = 320;
/// 100ms 滑动窗口。
const HISTORY_LEN: usize = 5;
/// 进入 Voiced 所需的窗口内 voiced 帧数。
const VOICED_THRESHOLD: usize = 3;
/// 进入 Unvoiced 所需的窗口内 unvoiced 帧数（同样宽容度，对称）。
const UNVOICED_THRESHOLD: usize = 3;
/// 切换防抖最小间隔。
pub const DEFAULT_MIN_SWITCH_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VadState {
    Unvoiced,
    Voiced,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VadEvent {
    /// 去抖后的稳定状态发生了切换。
    Switched(VadState),
}

pub struct VadGate {
    vad: Vad,
    /// 累积不足一个 VAD 帧的尾巴 samples。
    pending: Vec<i16>,
    /// 最近 HISTORY_LEN 帧的 voiced bit。
    history: VecDeque<bool>,
    state: VadState,
    last_switch: Instant,
    min_switch_interval: Duration,
}

impl VadGate {
    pub fn new() -> Self {
        Self::with_min_switch_interval(DEFAULT_MIN_SWITCH_INTERVAL)
    }

    /// 测试入口：可以把切换防抖间隔设为 0 来跳过时序考量，独立验证滑窗逻辑。
    pub fn with_min_switch_interval(min_switch_interval: Duration) -> Self {
        Self {
            vad: Vad::new_with_rate_and_mode(SampleRate::Rate16kHz, VadMode::Aggressive),
            pending: Vec::with_capacity(FRAME_SAMPLES * 2),
            history: VecDeque::with_capacity(HISTORY_LEN),
            state: VadState::Unvoiced,
            // 起手就允许一次切换（避免 daemon 启动后第一次说话被 1s 防抖吞掉）。
            last_switch: Instant::now()
                .checked_sub(min_switch_interval)
                .unwrap_or_else(Instant::now),
            min_switch_interval,
        }
    }

    pub fn state(&self) -> VadState {
        self.state
    }

    pub fn feed(&mut self, pcm: &[i16]) -> Option<VadEvent> {
        self.feed_at(pcm, Instant::now())
    }

    /// 单测可显式注入 now。生产代码走 [`feed`](Self::feed)。
    pub fn feed_at(&mut self, pcm: &[i16], now: Instant) -> Option<VadEvent> {
        self.pending.extend_from_slice(pcm);
        let mut last_event = None;
        while self.pending.len() >= FRAME_SAMPLES {
            // drain 出一帧。drain 把头部 320 个搬走，剩下的留作下次。
            let frame: Vec<i16> = self.pending.drain(..FRAME_SAMPLES).collect();
            let voiced = self.vad.is_voice_segment(&frame).unwrap_or(false);
            if self.history.len() == HISTORY_LEN {
                self.history.pop_front();
            }
            self.history.push_back(voiced);
            if let Some(ev) = self.maybe_switch(now) {
                last_event = Some(ev);
            }
        }
        last_event
    }

    fn maybe_switch(&mut self, now: Instant) -> Option<VadEvent> {
        if self.history.len() < HISTORY_LEN {
            return None;
        }
        let voiced_count = self.history.iter().filter(|&&v| v).count();
        let candidate = match self.state {
            VadState::Unvoiced if voiced_count >= VOICED_THRESHOLD => VadState::Voiced,
            VadState::Voiced if (HISTORY_LEN - voiced_count) >= UNVOICED_THRESHOLD => {
                VadState::Unvoiced
            }
            _ => return None,
        };
        if now.duration_since(self.last_switch) < self.min_switch_interval {
            return None;
        }
        self.state = candidate;
        self.last_switch = now;
        Some(VadEvent::Switched(candidate))
    }
}

impl Default for VadGate {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 生成 320×n samples 的 440Hz 正弦波，幅度 ~0.3 满量程（明确人声水平）。
    fn tone(frames: usize) -> Vec<i16> {
        let mut v = Vec::with_capacity(FRAME_SAMPLES * frames);
        for i in 0..(FRAME_SAMPLES * frames) {
            let s = (2.0 * std::f32::consts::PI * 440.0 * (i as f32) / 16_000.0).sin() * 9_000.0;
            v.push(s as i16);
        }
        v
    }

    fn silence(frames: usize) -> Vec<i16> {
        vec![0i16; FRAME_SAMPLES * frames]
    }

    #[test]
    fn silence_only_stays_unvoiced() {
        let mut g = VadGate::with_min_switch_interval(Duration::ZERO);
        let ev = g.feed(&silence(20));
        assert!(ev.is_none());
        assert_eq!(g.state(), VadState::Unvoiced);
    }

    #[test]
    fn tone_switches_to_voiced_after_enough_frames() {
        let mut g = VadGate::with_min_switch_interval(Duration::ZERO);
        // 攒满 5 帧滑窗、3 帧 voiced 才能切。喂 5 帧 tone 应该至少触发一次切换。
        let ev = g.feed(&tone(5));
        assert_eq!(ev, Some(VadEvent::Switched(VadState::Voiced)));
        assert_eq!(g.state(), VadState::Voiced);
    }

    #[test]
    fn tone_then_silence_switches_back() {
        let mut g = VadGate::with_min_switch_interval(Duration::ZERO);
        g.feed(&tone(5));
        assert_eq!(g.state(), VadState::Voiced);
        // libfvad 在 tone→silence 突变后有 ~4 帧"刹车"期（噪声估计还没适应回来，
        // 全零仍被判为 voiced），所以喂 15 帧给它够长的窗口稳定到 unvoiced。
        // 真实场景里 ambient noise 一直在，这个迟滞会比测试小得多。
        let ev = g.feed(&silence(15));
        assert_eq!(ev, Some(VadEvent::Switched(VadState::Unvoiced)));
        assert_eq!(g.state(), VadState::Unvoiced);
    }

    #[test]
    fn short_blip_does_not_switch() {
        let mut g = VadGate::with_min_switch_interval(Duration::ZERO);
        // 把窗口先填到稳态 Unvoiced
        g.feed(&silence(5));
        assert_eq!(g.state(), VadState::Unvoiced);
        // 2 帧 tone 不够阈值 3，不应切
        let ev = g.feed(&tone(2));
        assert!(ev.is_none(), "got {ev:?}");
        assert_eq!(g.state(), VadState::Unvoiced);
    }

    #[test]
    fn switch_debounce_blocks_rapid_oscillation() {
        // 用足够大的 min_switch_interval 让第二次切换被吞掉。
        let mut g = VadGate::with_min_switch_interval(Duration::from_secs(10));
        let t0 = Instant::now();
        let ev1 = g.feed_at(&tone(5), t0);
        assert_eq!(ev1, Some(VadEvent::Switched(VadState::Voiced)));
        // 立刻喂回静音；候选 Unvoiced 满足，但 1s 防抖未到 → 不切
        let ev2 = g.feed_at(&silence(5), t0 + Duration::from_millis(100));
        assert!(ev2.is_none(), "got {ev2:?}");
        assert_eq!(g.state(), VadState::Voiced);
        // 等过防抖间隔再喂静音，这次应切
        let ev3 = g.feed_at(&silence(5), t0 + Duration::from_secs(11));
        assert_eq!(ev3, Some(VadEvent::Switched(VadState::Unvoiced)));
    }

    #[test]
    fn small_chunks_get_accumulated_into_frames() {
        let mut g = VadGate::with_min_switch_interval(Duration::ZERO);
        // 把 5 帧 tone 切成 80-sample 小块（模拟 cpal callback 一次 ~5ms）
        let big = tone(5);
        for chunk in big.chunks(80) {
            g.feed(chunk);
        }
        assert_eq!(g.state(), VadState::Voiced);
    }
}
