use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

use rustfft::{Fft, FftPlanner, num_complex::Complex};
use serde::{Deserialize, Serialize};

use crate::{
    MediaAudioStreamMetrics, MediaFingerprintError,
    tuning::{
        V3_AUDIO_HOP_SAMPLES, V3_AUDIO_MAX_FREQ_HZ, V3_AUDIO_MAX_PEAKS_PER_FRAME,
        V3_AUDIO_MIN_FREQ_HZ, V3_AUDIO_PAIR_FANOUT, V3_AUDIO_PAIR_MAX_DELTA_FRAMES,
        V3_AUDIO_PAIR_MIN_DELTA_FRAMES, V3_AUDIO_PEAK_NEIGHBORHOOD,
        V3_AUDIO_RAW_LANDMARK_BUFFER_LIMIT, V3_AUDIO_RAW_LANDMARK_RETAIN_LIMIT,
        V3_AUDIO_VERIFY_LANDMARK_LIMIT, V3_AUDIO_WINDOW_SAMPLES,
    },
    video_v3::stable_hash_u64,
};

#[cfg(test)]
use crate::tuning::V3_AUDIO_SAMPLE_RATE;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioLandmarkV3 {
    pub hash: u32,
    pub t_ms: u32,
    pub weight: u8,
}

pub(crate) struct AudioConstellationV3PcmStream {
    pending_byte: Option<u8>,
    builder: AudioConstellationV3Builder,
    streamed_bytes: usize,
    streamed_samples: usize,
}

impl AudioConstellationV3PcmStream {
    pub(crate) fn new(sample_rate: u32) -> Self {
        Self {
            pending_byte: None,
            builder: AudioConstellationV3Builder::new(sample_rate),
            streamed_bytes: 0,
            streamed_samples: 0,
        }
    }

    pub(crate) fn push_bytes(&mut self, bytes: &[u8]) -> Result<(), MediaFingerprintError> {
        self.streamed_bytes += bytes.len();
        let mut samples =
            Vec::with_capacity((bytes.len() + usize::from(self.pending_byte.is_some())) / 2);
        let mut cursor = 0usize;
        if let Some(left) = self.pending_byte.take() {
            if let Some(right) = bytes.first().copied() {
                samples.push(i16::from_le_bytes([left, right]));
                cursor = 1;
            } else {
                self.pending_byte = Some(left);
                return Ok(());
            }
        }
        let chunks = bytes[cursor..].chunks_exact(2);
        let remainder = chunks.remainder();
        for chunk in chunks {
            samples.push(i16::from_le_bytes([chunk[0], chunk[1]]));
        }
        if let Some(byte) = remainder.first().copied() {
            self.pending_byte = Some(byte);
        }
        self.streamed_samples += samples.len();
        self.builder.push_pcm_i16(&samples);
        Ok(())
    }

    pub(crate) fn finish(
        self,
        duration_seconds: Option<f64>,
    ) -> Result<(Vec<AudioLandmarkV3>, MediaAudioStreamMetrics), MediaFingerprintError> {
        if self.pending_byte.is_some() {
            return Err(MediaFingerprintError::InvalidToolOutput {
                tool: "ffmpeg",
                reason: "decoded PCM had a partial trailing sample".to_owned(),
            });
        }
        let streamed_bytes = self.streamed_bytes;
        let streamed_samples = self.streamed_samples;
        let (landmarks, mut metrics) = self.builder.finish_with_metrics(duration_seconds);
        metrics.streamed_bytes = streamed_bytes;
        metrics.streamed_samples = streamed_samples;
        Ok((landmarks, metrics))
    }
}

struct AudioConstellationV3Builder {
    sample_rate: u32,
    analyzer: Option<AudioSpectralAnalyzerV3>,
    rolling_samples: Vec<i16>,
    recent_frames: VecDeque<AudioPeakFrameV3>,
    raw_landmarks: Vec<AudioLandmarkV3>,
    next_frame_index: usize,
    peak_frames: usize,
    max_buffer_samples: usize,
    max_raw_landmarks_seen: usize,
    max_raw_landmarks_after_compaction: usize,
    raw_landmark_compactions: usize,
}

impl AudioConstellationV3Builder {
    pub(crate) fn new(sample_rate: u32) -> Self {
        let analyzer = (sample_rate != 0).then(|| AudioSpectralAnalyzerV3::new(sample_rate));
        Self {
            sample_rate,
            analyzer,
            rolling_samples: Vec::with_capacity(V3_AUDIO_WINDOW_SAMPLES),
            recent_frames: VecDeque::new(),
            raw_landmarks: Vec::new(),
            next_frame_index: 0,
            peak_frames: 0,
            max_buffer_samples: 0,
            max_raw_landmarks_seen: 0,
            max_raw_landmarks_after_compaction: 0,
            raw_landmark_compactions: 0,
        }
    }

    fn push_pcm_i16(&mut self, samples: &[i16]) {
        if self.analyzer.is_none() || samples.is_empty() {
            return;
        }
        let mut cursor = 0usize;
        while cursor < samples.len() {
            let needed = V3_AUDIO_WINDOW_SAMPLES.saturating_sub(self.rolling_samples.len());
            let take = needed.min(samples.len() - cursor).max(1);
            self.rolling_samples
                .extend_from_slice(&samples[cursor..cursor + take]);
            cursor += take;
            self.max_buffer_samples = self.max_buffer_samples.max(self.rolling_samples.len());
            while self.rolling_samples.len() >= V3_AUDIO_WINDOW_SAMPLES {
                let peaks = self
                    .analyzer
                    .as_mut()
                    .expect("analyzer exists")
                    .peaks_for_frame(&self.rolling_samples[..V3_AUDIO_WINDOW_SAMPLES]);
                self.process_peak_frame(self.next_frame_index, peaks);
                self.next_frame_index += 1;
                self.rolling_samples.drain(..V3_AUDIO_HOP_SAMPLES);
                self.max_buffer_samples = self.max_buffer_samples.max(self.rolling_samples.len());
            }
        }
    }

    fn process_peak_frame(&mut self, frame_index: usize, peaks: Vec<AudioSpectralPeakV3>) {
        let mut needs_compaction = false;
        for anchor_frame in &mut self.recent_frames {
            let delta_frames = frame_index.saturating_sub(anchor_frame.frame_index);
            if !(V3_AUDIO_PAIR_MIN_DELTA_FRAMES..=V3_AUDIO_PAIR_MAX_DELTA_FRAMES)
                .contains(&delta_frames)
            {
                continue;
            }
            for (peak_index, anchor_peak) in anchor_frame.peaks.iter().enumerate() {
                let mut emitted = anchor_frame.emitted_per_peak[peak_index];
                if emitted >= V3_AUDIO_PAIR_FANOUT {
                    continue;
                }
                for target_peak in &peaks {
                    let t_ms = audio_frame_timestamp_ms(anchor_frame.frame_index, self.sample_rate);
                    let hash =
                        audio_landmark_hash_v3(anchor_peak.bin, target_peak.bin, delta_frames);
                    let strength = ((anchor_peak.magnitude + target_peak.magnitude) * 4.0)
                        .round()
                        .clamp(1.0, f32::from(u8::MAX)) as u8;
                    self.raw_landmarks.push(AudioLandmarkV3 {
                        hash,
                        t_ms,
                        weight: strength,
                    });
                    self.max_raw_landmarks_seen =
                        self.max_raw_landmarks_seen.max(self.raw_landmarks.len());
                    needs_compaction |=
                        self.raw_landmarks.len() > V3_AUDIO_RAW_LANDMARK_BUFFER_LIMIT;
                    emitted += 1;
                    if emitted >= V3_AUDIO_PAIR_FANOUT {
                        break;
                    }
                }
                anchor_frame.emitted_per_peak[peak_index] = emitted;
            }
        }
        if needs_compaction {
            self.compact_raw_landmarks_if_needed();
        }
        while self.recent_frames.front().is_some_and(|frame| {
            frame_index.saturating_sub(frame.frame_index) >= V3_AUDIO_PAIR_MAX_DELTA_FRAMES
        }) {
            self.recent_frames.pop_front();
        }
        self.peak_frames += 1;
        let emitted_per_peak = vec![0; peaks.len()];
        self.recent_frames.push_back(AudioPeakFrameV3 {
            frame_index,
            peaks,
            emitted_per_peak,
        });
    }

    fn compact_raw_landmarks_if_needed(&mut self) {
        if self.raw_landmarks.len() > V3_AUDIO_RAW_LANDMARK_BUFFER_LIMIT {
            compact_audio_landmark_buffer_v3(
                &mut self.raw_landmarks,
                V3_AUDIO_RAW_LANDMARK_RETAIN_LIMIT,
            );
            self.raw_landmark_compactions += 1;
            self.max_raw_landmarks_after_compaction = self
                .max_raw_landmarks_after_compaction
                .max(self.raw_landmarks.len());
        }
        self.max_raw_landmarks_after_compaction = self
            .max_raw_landmarks_after_compaction
            .max(self.raw_landmarks.len());
    }

    fn finish_with_metrics(
        self,
        duration_seconds: Option<f64>,
    ) -> (Vec<AudioLandmarkV3>, MediaAudioStreamMetrics) {
        let raw_count = self.raw_landmarks.len();
        let max_raw_landmarks_after_compaction = if self.raw_landmark_compactions == 0 {
            raw_count
        } else {
            self.max_raw_landmarks_after_compaction
        };
        let landmarks = finish_bounded_audio_landmarks_v3(self.raw_landmarks, duration_seconds);
        let metrics = MediaAudioStreamMetrics {
            peak_frames: self.peak_frames,
            raw_landmarks_before_bounding: raw_count,
            final_landmarks: landmarks.len(),
            max_buffer_samples: self.max_buffer_samples,
            max_raw_landmarks_seen: self.max_raw_landmarks_seen.max(raw_count),
            max_raw_landmarks_after_compaction,
            raw_landmark_compactions: self.raw_landmark_compactions,
            ..MediaAudioStreamMetrics::default()
        };
        (landmarks, metrics)
    }
}

#[derive(Debug)]
struct AudioPeakFrameV3 {
    frame_index: usize,
    peaks: Vec<AudioSpectralPeakV3>,
    emitted_per_peak: Vec<usize>,
}

struct AudioSpectralAnalyzerV3 {
    min_bin: usize,
    max_bin: usize,
    hann: Vec<f32>,
    fft: Arc<dyn Fft<f32>>,
    buffer: Vec<Complex<f32>>,
}

impl AudioSpectralAnalyzerV3 {
    pub(crate) fn new(sample_rate: u32) -> Self {
        let (min_bin, max_bin) = v3_audio_bin_range(sample_rate);
        let hann = v3_audio_hann_window();
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(V3_AUDIO_WINDOW_SAMPLES);
        Self {
            min_bin,
            max_bin,
            hann,
            fft,
            buffer: vec![Complex::new(0.0f32, 0.0f32); V3_AUDIO_WINDOW_SAMPLES],
        }
    }

    fn peaks_for_frame(&mut self, samples: &[i16]) -> Vec<AudioSpectralPeakV3> {
        for (index, slot) in self.buffer.iter_mut().enumerate() {
            let sample = samples[index] as f32 / f32::from(i16::MAX);
            *slot = Complex::new(sample * self.hann[index], 0.0);
        }
        self.fft.process(&mut self.buffer);
        audio_spectral_peaks_from_fft_bins(&self.buffer, self.min_bin, self.max_bin)
    }
}

#[cfg(test)]
pub(crate) fn audio_constellation_landmarks_v3_from_pcm_streaming(
    samples: &[i16],
    sample_rate: u32,
    duration_seconds: Option<f64>,
) -> (Vec<AudioLandmarkV3>, MediaAudioStreamMetrics) {
    let mut builder = AudioConstellationV3Builder::new(sample_rate);
    builder.push_pcm_i16(samples);
    let (landmarks, mut metrics) = builder.finish_with_metrics(duration_seconds);
    metrics.streamed_samples = samples.len();
    metrics.streamed_bytes = samples.len().saturating_mul(2);
    (landmarks, metrics)
}

#[cfg(test)]
pub(crate) fn audio_constellation_landmarks_v3_from_pcm_chunks(
    chunks: &[&[i16]],
    sample_rate: u32,
    duration_seconds: Option<f64>,
) -> (Vec<AudioLandmarkV3>, MediaAudioStreamMetrics) {
    let mut builder = AudioConstellationV3Builder::new(sample_rate);
    let mut samples = 0usize;
    for chunk in chunks {
        samples += chunk.len();
        builder.push_pcm_i16(chunk);
    }
    let (landmarks, mut metrics) = builder.finish_with_metrics(duration_seconds);
    metrics.streamed_samples = samples;
    metrics.streamed_bytes = samples.saturating_mul(2);
    (landmarks, metrics)
}

fn finish_bounded_audio_landmarks_v3(
    mut raw: Vec<AudioLandmarkV3>,
    duration_seconds: Option<f64>,
) -> Vec<AudioLandmarkV3> {
    dedupe_audio_landmarks_v3(&mut raw);
    if let Some(duration) = duration_seconds.filter(|value| value.is_finite() && *value > 120.0) {
        downweight_edge_audio_landmarks_v3(&mut raw, duration);
    }
    bounded_time_distributed_audio_landmarks_v3(&mut raw, V3_AUDIO_VERIFY_LANDMARK_LIMIT)
}

fn compact_audio_landmark_buffer_v3(landmarks: &mut Vec<AudioLandmarkV3>, retain_limit: usize) {
    dedupe_audio_landmarks_v3(landmarks);
    if landmarks.len() <= retain_limit {
        return;
    }
    let mut by_weight = landmarks.clone();
    by_weight.sort_by_key(|landmark| {
        (
            std::cmp::Reverse(landmark.weight),
            landmark.t_ms,
            landmark.hash,
        )
    });
    let high_weight_limit = retain_limit / 2;
    let mut selected = by_weight
        .into_iter()
        .take(high_weight_limit)
        .collect::<Vec<_>>();
    let mut selected_keys = selected
        .iter()
        .map(|landmark| (landmark.hash, landmark.t_ms))
        .collect::<HashSet<_>>();
    let distributed_limit = retain_limit.saturating_sub(selected.len());
    let mut distributed = bounded_time_distributed_audio_landmarks_v3(landmarks, distributed_limit);
    for landmark in distributed.drain(..) {
        if selected_keys.insert((landmark.hash, landmark.t_ms)) {
            selected.push(landmark);
        }
    }
    if selected.len() < retain_limit {
        let mut remaining = landmarks.clone();
        remaining.sort_by_key(|landmark| {
            (
                landmark.t_ms,
                std::cmp::Reverse(landmark.weight),
                landmark.hash,
            )
        });
        for landmark in remaining {
            if selected.len() >= retain_limit {
                break;
            }
            if selected_keys.insert((landmark.hash, landmark.t_ms)) {
                selected.push(landmark);
            }
        }
    }
    selected.sort_by_key(|landmark| {
        (
            landmark.t_ms,
            landmark.hash,
            std::cmp::Reverse(landmark.weight),
        )
    });
    *landmarks = selected;
}

#[cfg(test)]
fn finish_pcm_stream_for_test(
    chunks: &[&[u8]],
) -> Result<(Vec<AudioLandmarkV3>, MediaAudioStreamMetrics), MediaFingerprintError> {
    let mut stream = AudioConstellationV3PcmStream::new(V3_AUDIO_SAMPLE_RATE);
    for chunk in chunks {
        stream.push_bytes(chunk)?;
    }
    stream.finish(None)
}

#[cfg(test)]
pub(crate) fn audio_streaming_reference_overlap(
    left: &[AudioLandmarkV3],
    right: &[AudioLandmarkV3],
) -> f64 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let left = left
        .iter()
        .map(|landmark| (landmark.hash, landmark.t_ms))
        .collect::<HashSet<_>>();
    let right = right
        .iter()
        .map(|landmark| (landmark.hash, landmark.t_ms))
        .collect::<HashSet<_>>();
    left.intersection(&right).count() as f64 / left.len().max(right.len()) as f64
}

fn v3_audio_bin_range(sample_rate: u32) -> (usize, usize) {
    if sample_rate == 0 {
        return (1, V3_AUDIO_WINDOW_SAMPLES / 2);
    }
    let min_bin =
        ((V3_AUDIO_MIN_FREQ_HZ * V3_AUDIO_WINDOW_SAMPLES as f32) / sample_rate as f32).ceil();
    let max_bin =
        ((V3_AUDIO_MAX_FREQ_HZ * V3_AUDIO_WINDOW_SAMPLES as f32) / sample_rate as f32).floor();
    let min_bin = (min_bin as usize).clamp(1, (V3_AUDIO_WINDOW_SAMPLES / 2).saturating_sub(1));
    let max_bin = (max_bin as usize).clamp(min_bin + 1, V3_AUDIO_WINDOW_SAMPLES / 2);
    (min_bin, max_bin)
}

fn v3_audio_hann_window() -> Vec<f32> {
    (0..V3_AUDIO_WINDOW_SAMPLES)
        .map(|index| {
            let phase =
                (std::f32::consts::TAU * index as f32) / (V3_AUDIO_WINDOW_SAMPLES - 1) as f32;
            0.5 - (0.5 * phase.cos())
        })
        .collect()
}

fn audio_spectral_peaks_from_fft_bins(
    buffer: &[Complex<f32>],
    min_bin: usize,
    max_bin: usize,
) -> Vec<AudioSpectralPeakV3> {
    let magnitudes = (min_bin..max_bin)
        .map(|bin| {
            let value = buffer[bin].norm_sqr().max(f32::MIN_POSITIVE).log10();
            (bin, value)
        })
        .collect::<Vec<_>>();
    let mean = if magnitudes.is_empty() {
        0.0
    } else {
        magnitudes
            .iter()
            .map(|(_, magnitude)| *magnitude)
            .sum::<f32>()
            / magnitudes.len() as f32
    };
    let mut peaks = Vec::new();
    for (local_index, (bin, magnitude)) in magnitudes.iter().enumerate() {
        if *magnitude < mean + 0.35 {
            continue;
        }
        let left = local_index.saturating_sub(V3_AUDIO_PEAK_NEIGHBORHOOD);
        let right = (local_index + V3_AUDIO_PEAK_NEIGHBORHOOD + 1).min(magnitudes.len());
        if magnitudes[left..right]
            .iter()
            .all(|(_, neighbor)| *magnitude >= *neighbor)
        {
            peaks.push(AudioSpectralPeakV3 {
                bin: *bin,
                magnitude: *magnitude - mean,
            });
        }
    }
    peaks.sort_by(|left, right| {
        right
            .magnitude
            .total_cmp(&left.magnitude)
            .then_with(|| left.bin.cmp(&right.bin))
    });
    peaks.truncate(V3_AUDIO_MAX_PEAKS_PER_FRAME);
    peaks.sort_by_key(|peak| peak.bin);
    peaks
}

#[cfg(test)]
pub fn audio_constellation_stream_rejects_odd_trailing_byte_for_test(
    bytes: &[u8],
) -> Result<(), MediaFingerprintError> {
    finish_pcm_stream_for_test(&[bytes]).map(|_| ())
}

#[cfg(test)]
pub(crate) fn audio_constellation_streaming_decode_pcm_bytes_for_test(
    bytes: &[u8],
) -> Result<(Vec<AudioLandmarkV3>, MediaAudioStreamMetrics), MediaFingerprintError> {
    finish_pcm_stream_for_test(&[bytes])
}

#[cfg(test)]
pub(crate) fn audio_constellation_streaming_decode_split_bytes_for_test(
    bytes: &[u8],
) -> Result<MediaAudioStreamMetrics, MediaFingerprintError> {
    let chunks = bytes.chunks(3).collect::<Vec<_>>();
    finish_pcm_stream_for_test(&chunks).map(|(_, metrics)| metrics)
}

#[cfg(test)]
pub fn audio_constellation_landmarks_v3_from_pcm(
    samples: &[i16],
    sample_rate: u32,
    duration_seconds: Option<f64>,
) -> Vec<AudioLandmarkV3> {
    if samples.len() < V3_AUDIO_WINDOW_SAMPLES || sample_rate == 0 {
        return Vec::new();
    }
    let min_bin =
        ((V3_AUDIO_MIN_FREQ_HZ * V3_AUDIO_WINDOW_SAMPLES as f32) / sample_rate as f32).ceil();
    let max_bin =
        ((V3_AUDIO_MAX_FREQ_HZ * V3_AUDIO_WINDOW_SAMPLES as f32) / sample_rate as f32).floor();
    let min_bin = (min_bin as usize).clamp(1, (V3_AUDIO_WINDOW_SAMPLES / 2).saturating_sub(1));
    let max_bin = (max_bin as usize).clamp(min_bin + 1, V3_AUDIO_WINDOW_SAMPLES / 2);
    let frames = audio_spectral_peak_frames_v3(samples, min_bin, max_bin);
    if frames.is_empty() {
        return Vec::new();
    }
    let mut raw = Vec::new();
    for (frame_index, peaks) in frames.iter().enumerate() {
        for anchor_peak in peaks {
            let start = frame_index + V3_AUDIO_PAIR_MIN_DELTA_FRAMES;
            let end = (frame_index + V3_AUDIO_PAIR_MAX_DELTA_FRAMES + 1).min(frames.len());
            if start >= end {
                continue;
            }
            let mut emitted = 0usize;
            'targets: for (target_frame, target_peaks) in
                frames.iter().enumerate().take(end).skip(start)
            {
                let delta_frames = target_frame.saturating_sub(frame_index);
                for target_peak in target_peaks {
                    let t_ms = audio_frame_timestamp_ms(frame_index, sample_rate);
                    let hash =
                        audio_landmark_hash_v3(anchor_peak.bin, target_peak.bin, delta_frames);
                    let strength = ((anchor_peak.magnitude + target_peak.magnitude) * 4.0)
                        .round()
                        .clamp(1.0, f32::from(u8::MAX)) as u8;
                    raw.push(AudioLandmarkV3 {
                        hash,
                        t_ms,
                        weight: strength,
                    });
                    emitted += 1;
                    if emitted >= V3_AUDIO_PAIR_FANOUT {
                        break 'targets;
                    }
                }
            }
        }
    }
    dedupe_audio_landmarks_v3(&mut raw);
    if let Some(duration) = duration_seconds.filter(|value| value.is_finite() && *value > 120.0) {
        downweight_edge_audio_landmarks_v3(&mut raw, duration);
    }
    bounded_time_distributed_audio_landmarks_v3(&mut raw, V3_AUDIO_VERIFY_LANDMARK_LIMIT)
}

#[derive(Debug, Clone, Copy)]
struct AudioSpectralPeakV3 {
    bin: usize,
    magnitude: f32,
}

#[cfg(test)]
fn audio_spectral_peak_frames_v3(
    samples: &[i16],
    min_bin: usize,
    max_bin: usize,
) -> Vec<Vec<AudioSpectralPeakV3>> {
    let frame_count = (samples.len() - V3_AUDIO_WINDOW_SAMPLES) / V3_AUDIO_HOP_SAMPLES + 1;
    let hann = (0..V3_AUDIO_WINDOW_SAMPLES)
        .map(|index| {
            let phase =
                (std::f32::consts::TAU * index as f32) / (V3_AUDIO_WINDOW_SAMPLES - 1) as f32;
            0.5 - (0.5 * phase.cos())
        })
        .collect::<Vec<_>>();
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(V3_AUDIO_WINDOW_SAMPLES);
    let mut buffer = vec![Complex::new(0.0f32, 0.0f32); V3_AUDIO_WINDOW_SAMPLES];
    let mut frames = Vec::with_capacity(frame_count);
    for frame_index in 0..frame_count {
        let start = frame_index * V3_AUDIO_HOP_SAMPLES;
        for (index, slot) in buffer.iter_mut().enumerate() {
            let sample = samples[start + index] as f32 / f32::from(i16::MAX);
            *slot = Complex::new(sample * hann[index], 0.0);
        }
        fft.process(&mut buffer);
        let magnitudes = (min_bin..max_bin)
            .map(|bin| {
                let value = buffer[bin].norm_sqr().max(f32::MIN_POSITIVE).log10();
                (bin, value)
            })
            .collect::<Vec<_>>();
        let mean = if magnitudes.is_empty() {
            0.0
        } else {
            magnitudes
                .iter()
                .map(|(_, magnitude)| *magnitude)
                .sum::<f32>()
                / magnitudes.len() as f32
        };
        let mut peaks = Vec::new();
        for (local_index, (bin, magnitude)) in magnitudes.iter().enumerate() {
            if *magnitude < mean + 0.35 {
                continue;
            }
            let left = local_index.saturating_sub(V3_AUDIO_PEAK_NEIGHBORHOOD);
            let right = (local_index + V3_AUDIO_PEAK_NEIGHBORHOOD + 1).min(magnitudes.len());
            if magnitudes[left..right]
                .iter()
                .all(|(_, neighbor)| *magnitude >= *neighbor)
            {
                peaks.push(AudioSpectralPeakV3 {
                    bin: *bin,
                    magnitude: *magnitude - mean,
                });
            }
        }
        peaks.sort_by(|left, right| {
            right
                .magnitude
                .total_cmp(&left.magnitude)
                .then_with(|| left.bin.cmp(&right.bin))
        });
        peaks.truncate(V3_AUDIO_MAX_PEAKS_PER_FRAME);
        peaks.sort_by_key(|peak| peak.bin);
        frames.push(peaks);
    }
    frames
}

fn audio_frame_timestamp_ms(frame_index: usize, sample_rate: u32) -> u32 {
    let samples = frame_index.saturating_mul(V3_AUDIO_HOP_SAMPLES) as u64;
    ((samples * 1000) / u64::from(sample_rate)).min(u64::from(u32::MAX)) as u32
}

fn audio_landmark_hash_v3(anchor_bin: usize, target_bin: usize, delta_frames: usize) -> u32 {
    let anchor_bin = (anchor_bin as u32 / 2).min(0x3ff);
    let target_bin = (target_bin as u32 / 2).min(0x3ff);
    let delta = (delta_frames as u32).min(0x3ff);
    let packed = anchor_bin | (target_bin << 10) | (delta << 20);
    stable_hash_u64(packed.to_le_bytes()) as u32
}

fn dedupe_audio_landmarks_v3(landmarks: &mut Vec<AudioLandmarkV3>) {
    landmarks.sort_by_key(|landmark| {
        (
            landmark.t_ms,
            landmark.hash,
            std::cmp::Reverse(landmark.weight),
        )
    });
    landmarks.dedup_by(|left, right| left.t_ms == right.t_ms && left.hash == right.hash);
}

fn downweight_edge_audio_landmarks_v3(landmarks: &mut [AudioLandmarkV3], duration_seconds: f64) {
    let edge_ms = (duration_seconds * 1000.0 * 0.08).clamp(30_000.0, 120_000.0) as u32;
    let duration_ms = (duration_seconds * 1000.0).min(f64::from(u32::MAX)) as u32;
    for landmark in landmarks {
        if landmark.t_ms < edge_ms || landmark.t_ms > duration_ms.saturating_sub(edge_ms) {
            landmark.weight = landmark.weight.saturating_sub(landmark.weight / 2).max(1);
        }
    }
}

pub(crate) fn bounded_time_distributed_audio_landmarks_v3(
    landmarks: &mut [AudioLandmarkV3],
    max_landmarks: usize,
) -> Vec<AudioLandmarkV3> {
    landmarks.sort_by_key(|landmark| {
        (
            landmark.t_ms,
            landmark.hash,
            std::cmp::Reverse(landmark.weight),
        )
    });
    if max_landmarks == 0 {
        return Vec::new();
    }
    if landmarks.len() <= max_landmarks {
        return landmarks.to_vec();
    }
    let stride = landmarks.len() as f64 / max_landmarks as f64;
    (0..max_landmarks)
        .map(|index| landmarks[(index as f64 * stride).floor() as usize])
        .collect()
}
