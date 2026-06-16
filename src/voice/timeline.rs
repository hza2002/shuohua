//! Sample-indexed PCM timeline + ring buffer.
//!
//! M10 voice 层需要按"recording 时间轴上的样本索引"对齐 VAD 决策、resume
//! pre-roll 和 ASR session 的发送窗口。本模块提供：
//!
//! - 单调递增的 `next_sample`：等于已被 `push()` 的样本总数。
//! - 有界 ring buffer：超过 `max_retained_ms` 的旧样本被丢弃，`oldest_sample`
//!   随之上调。
//! - `slice_from(start_sample)`：取从某个样本索引开始仍保留的 PCM；start 早
//!   于 `oldest_sample` 时自动 clamp，调用方拿到的 chunk 永远语义安全。
//!
//! 采样率固定为 canonical 16kHz s16le mono；不暴露任何 wall-clock 时间，
//! 上层用 `samples_to_ms` / `ms_to_samples` 跟配置参数互换。

pub const SAMPLE_RATE: u64 = 16_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PcmChunk {
    pub start_sample: u64,
    pub samples: Vec<i16>,
}

impl PcmChunk {
    pub fn end_sample(&self) -> u64 {
        self.start_sample + self.samples.len() as u64
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct PcmTimeline {
    next_sample: u64,
    retained_start: u64,
    retained: Vec<i16>,
    max_retained_samples: usize,
}

impl PcmTimeline {
    /// 创建一个 ring buffer，最多保留最近 `max_retained_ms` 毫秒的 PCM。
    pub fn new(max_retained_ms: u32) -> Self {
        let cap = ms_to_samples(max_retained_ms) as usize;
        Self {
            next_sample: 0,
            retained_start: 0,
            retained: Vec::with_capacity(cap),
            max_retained_samples: cap,
        }
    }

    /// 把 `samples` 追加到时间轴，返回这一段对应的 `[start_sample, end)` chunk。
    /// 总样本数超过 `max_retained_ms` 时，最旧的样本被丢弃，`oldest_sample`
    /// 随之上调。
    pub fn push(&mut self, samples: &[i16]) -> PcmChunk {
        let chunk_start = self.next_sample;
        self.retained.extend_from_slice(samples);
        self.next_sample += samples.len() as u64;
        if self.retained.len() > self.max_retained_samples {
            let excess = self.retained.len() - self.max_retained_samples;
            self.retained.drain(..excess);
            self.retained_start += excess as u64;
        }
        PcmChunk {
            start_sample: chunk_start,
            samples: samples.to_vec(),
        }
    }

    /// 下一次 push 将分配的起始样本索引；同时等于到目前为止总样本数。
    #[cfg(test)]
    pub fn next_sample(&self) -> u64 {
        self.next_sample
    }

    /// ring buffer 里最旧保留样本的索引。
    pub fn oldest_sample(&self) -> u64 {
        self.retained_start
    }

    /// 从 `start_sample` 起仍保留的 PCM。`start_sample` 早于 `oldest_sample`
    /// 时自动 clamp；晚于 `next_sample` 返回空 chunk（起点取 `next_sample`）。
    pub fn slice_from(&self, start_sample: u64) -> PcmChunk {
        let effective = start_sample.max(self.retained_start);
        if effective >= self.next_sample {
            return PcmChunk {
                start_sample: self.next_sample,
                samples: Vec::new(),
            };
        }
        let offset = (effective - self.retained_start) as usize;
        PcmChunk {
            start_sample: effective,
            samples: self.retained[offset..].to_vec(),
        }
    }
}

pub fn ms_to_samples(ms: u32) -> u64 {
    (ms as u64) * SAMPLE_RATE / 1000
}

#[cfg(test)]
fn samples_to_ms(samples: u64) -> u64 {
    samples.saturating_mul(1000) / SAMPLE_RATE
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(start: i16, len: usize) -> Vec<i16> {
        (0..len).map(|i| start + i as i16).collect()
    }

    #[test]
    fn ms_samples_round_trip_on_exact_values() {
        assert_eq!(ms_to_samples(0), 0);
        assert_eq!(ms_to_samples(1000), 16_000);
        assert_eq!(ms_to_samples(1500), 24_000);
        assert_eq!(samples_to_ms(16_000), 1000);
        assert_eq!(samples_to_ms(24_000), 1500);
    }

    #[test]
    fn push_assigns_monotonic_chunk_ranges() {
        let mut tl = PcmTimeline::new(1000);
        let a = tl.push(&[1, 2, 3, 4]);
        let b = tl.push(&[5, 6]);
        assert_eq!(a.start_sample, 0);
        assert_eq!(a.end_sample(), 4);
        assert_eq!(b.start_sample, 4);
        assert_eq!(b.end_sample(), 6);
        assert_eq!(tl.next_sample(), 6);
    }

    #[test]
    fn slice_from_returns_available_samples_from_index() {
        let mut tl = PcmTimeline::new(1000);
        tl.push(&[10, 11, 12, 13, 14]);
        let slice = tl.slice_from(2);
        assert_eq!(slice.start_sample, 2);
        assert_eq!(slice.samples, vec![12, 13, 14]);
    }

    #[test]
    fn slice_from_clamps_to_oldest_retained_sample() {
        // 100ms retention => 1600 samples.
        let mut tl = PcmTimeline::new(100);
        let frame1 = frame(0, 1000);
        let frame2 = frame(1000, 1000); // 总共 2000 samples
        tl.push(&frame1);
        tl.push(&frame2);
        // 超过 1600 => 丢掉最旧 400
        assert_eq!(tl.oldest_sample(), 400);
        assert_eq!(tl.next_sample(), 2000);

        let slice = tl.slice_from(0);
        // clamp 到 oldest_sample，不丢已被保留的样本
        assert_eq!(slice.start_sample, 400);
        assert_eq!(slice.samples.len(), 1600);
        assert_eq!(slice.samples[0], frame1[400] as i16);
    }

    #[test]
    fn slice_from_future_index_is_empty_at_next_sample() {
        let mut tl = PcmTimeline::new(1000);
        tl.push(&[1, 2, 3]);
        let s = tl.slice_from(99);
        assert!(s.is_empty());
        assert_eq!(s.start_sample, tl.next_sample());
    }

    #[test]
    fn retention_keeps_at_least_pre_roll_plus_overlap() {
        // pre_roll_ms=300, max_overlap_ms=200 -> 500ms = 8000 samples
        let retention_ms = 500;
        let mut tl = PcmTimeline::new(retention_ms);
        // push ~3 秒 PCM 模拟长录音
        for _ in 0..30 {
            tl.push(&vec![0i16; 1600]); // 100ms of silence
        }
        let retained = tl.next_sample() - tl.oldest_sample();
        assert!(
            retained >= ms_to_samples(retention_ms),
            "retained {retained} samples should be at least {} ms worth",
            retention_ms
        );
        // 不会无限增长
        assert!(retained <= ms_to_samples(retention_ms) + 1600);
    }

    #[test]
    fn empty_push_is_a_noop() {
        let mut tl = PcmTimeline::new(1000);
        let c = tl.push(&[]);
        assert_eq!(c.start_sample, 0);
        assert!(c.is_empty());
        assert_eq!(tl.next_sample(), 0);
        assert_eq!(tl.oldest_sample(), 0);
    }
}
