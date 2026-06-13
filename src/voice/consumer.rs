//! PCM 消费端：cpal 帧 → 500ms pre-roll buffer + VAD 旁路决策。
//!
//! M2.5.d1 阶段：feed() 只追加 pre-roll + 跑 VAD 决策；返回的 VadEvent 用于
//! 上报日志，session 仍由 finish.rs 单开单关，行为对用户不变。
//! M2.5.d2 升级：finish.rs 会消费 VadEvent 决定关/开新 session，并在开新
//! session 时调 drain_preroll() 一次性 dump 历史给 ASR，避免辅音/弱起被丢。

use std::collections::VecDeque;
use std::time::Instant;

use crate::voice::vad::{VadEvent, VadGate, VadState};

/// 500ms @ 16kHz = 8000 samples。DESIGN §2.9：足够覆盖辅音/弱起，再大无收益。
pub const PREROLL_CAP_SAMPLES: usize = 8_000;

pub struct PcmConsumer {
    vad: VadGate,
    preroll: VecDeque<i16>,
}

impl PcmConsumer {
    pub fn new() -> Self {
        Self::with_vad(VadGate::new())
    }

    /// 测试入口：可注入 VadGate（自定义 min_switch_interval）。
    pub fn with_vad(vad: VadGate) -> Self {
        Self { vad, preroll: VecDeque::with_capacity(PREROLL_CAP_SAMPLES) }
    }

    /// 喂一批 PCM samples。追加到 pre-roll（容量满了 FIFO 丢最早），同时喂 VAD。
    /// 返回 VAD 在这次 feed 内触发的最后一次状态切换（若有）。
    pub fn feed(&mut self, pcm: &[i16]) -> Option<VadEvent> {
        self.feed_at(pcm, Instant::now())
    }

    pub fn feed_at(&mut self, pcm: &[i16], now: Instant) -> Option<VadEvent> {
        // 先 pop 掉会被新数据顶出的旧 samples，再一次性 extend；比逐个 push+pop 少分支。
        if self.preroll.len() + pcm.len() > PREROLL_CAP_SAMPLES {
            let need_to_drop = self.preroll.len() + pcm.len() - PREROLL_CAP_SAMPLES;
            // 即使 pcm 单批超过 PREROLL_CAP，也只 drop 现有的（不会负数 underflow）
            let drop_from_existing = need_to_drop.min(self.preroll.len());
            self.preroll.drain(..drop_from_existing);
            // 如果 pcm 本身就 > PREROLL_CAP，只保留末尾 PREROLL_CAP 个
            if pcm.len() >= PREROLL_CAP_SAMPLES {
                self.preroll.clear();
                let tail = &pcm[pcm.len() - PREROLL_CAP_SAMPLES..];
                self.preroll.extend(tail);
            } else {
                self.preroll.extend(pcm.iter().copied());
            }
        } else {
            self.preroll.extend(pcm.iter().copied());
        }
        self.vad.feed_at(pcm, now)
    }

    pub fn vad_state(&self) -> VadState {
        self.vad.state()
    }

    pub fn preroll_len(&self) -> usize {
        self.preroll.len()
    }

    /// 一次性取出所有 pre-roll samples，清空 buffer。M2.5.d2 开新 ASR session
    /// 时调用，把最近 500ms 历史喂给新 session（避免辅音/弱起被丢，DESIGN §2.9）。
    pub fn drain_preroll(&mut self) -> Vec<i16> {
        self.preroll.drain(..).collect()
    }
}

impl Default for PcmConsumer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn tone_samples(n: usize) -> Vec<i16> {
        (0..n)
            .map(|i| {
                let v = (2.0 * std::f32::consts::PI * 440.0 * (i as f32) / 16_000.0).sin()
                    * 9_000.0;
                v as i16
            })
            .collect()
    }

    #[test]
    fn new_starts_empty_unvoiced() {
        let c = PcmConsumer::new();
        assert_eq!(c.preroll_len(), 0);
        assert_eq!(c.vad_state(), VadState::Unvoiced);
    }

    #[test]
    fn small_feed_under_cap_grows_preroll() {
        let mut c = PcmConsumer::new();
        c.feed(&vec![0i16; 1000]);
        assert_eq!(c.preroll_len(), 1000);
    }

    #[test]
    fn preroll_caps_at_500ms() {
        let mut c = PcmConsumer::new();
        // 喂总计 12000 samples（750ms），超出 8000 cap
        for _ in 0..15 {
            c.feed(&vec![0i16; 800]);
        }
        assert_eq!(c.preroll_len(), PREROLL_CAP_SAMPLES);
    }

    #[test]
    fn single_oversized_chunk_keeps_tail_only() {
        let mut c = PcmConsumer::new();
        let big: Vec<i16> = (0..12_000).map(|i| i as i16).collect();
        c.feed(&big);
        assert_eq!(c.preroll_len(), PREROLL_CAP_SAMPLES);
        // 尾部 8000 个：从 big[4000] 开始；front() 应等 4000
        assert_eq!(*c.preroll.front().unwrap(), 4_000);
        assert_eq!(*c.preroll.back().unwrap(), 11_999);
    }

    #[test]
    fn drain_preroll_returns_and_clears() {
        let mut c = PcmConsumer::new();
        c.feed(&vec![7i16; 1000]);
        let out = c.drain_preroll();
        assert_eq!(out.len(), 1000);
        assert!(out.iter().all(|&v| v == 7));
        assert_eq!(c.preroll_len(), 0);
        // 再喂应从 0 开始攒
        c.feed(&vec![9i16; 50]);
        assert_eq!(c.preroll_len(), 50);
    }

    #[test]
    fn feed_returns_vad_switch_event() {
        let mut c =
            PcmConsumer::with_vad(VadGate::with_min_switch_interval(Duration::ZERO));
        // 喂 5 帧 tone，应触发 Voiced 切换
        let ev = c.feed(&tone_samples(320 * 5));
        assert!(matches!(ev, Some(VadEvent::Switched(VadState::Voiced))));
        assert_eq!(c.vad_state(), VadState::Voiced);
    }
}
