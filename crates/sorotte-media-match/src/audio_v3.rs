use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Instant;

use rustfft::{Fft, FftPlanner, num_complex::Complex};
use serde::{Deserialize, Serialize};

use crate::{
    MediaAudioStreamMetrics, MediaFingerprintError,
    tuning::{
        V3_AUDIO_HOP_SAMPLES, V3_AUDIO_MAX_FREQ_HZ, V3_AUDIO_MAX_PEAKS_PER_FRAME,
        V3_AUDIO_MIN_FREQ_HZ, V3_AUDIO_PAIR_CANDIDATE_RETAIN, V3_AUDIO_PAIR_DELTA_STRIDE_FRAMES,
        V3_AUDIO_PAIR_FANOUT, V3_AUDIO_PAIR_MAX_DELTA_FRAMES, V3_AUDIO_PAIR_MIN_DELTA_FRAMES,
        V3_AUDIO_PEAK_NEIGHBORHOOD, V3_AUDIO_RAW_REGION_RETAIN_LIMIT,
        V3_AUDIO_RAW_REGION_TRIM_BURST, V3_AUDIO_VERIFY_LANDMARK_LIMIT, V3_AUDIO_WINDOW_SAMPLES,
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
    raw_landmarks: AudioLandmarkReservoirV3,
    next_frame_index: usize,
    peak_frames: usize,
    max_buffer_samples: usize,
    analyzer_nanos: u128,
    compaction_nanos: u128,
    pairing_nanos: u128,
}

impl AudioConstellationV3Builder {
    pub(crate) fn new(sample_rate: u32) -> Self {
        let analyzer = (sample_rate != 0).then(|| AudioSpectralAnalyzerV3::new(sample_rate));
        Self {
            sample_rate,
            analyzer,
            rolling_samples: Vec::with_capacity(V3_AUDIO_WINDOW_SAMPLES),
            recent_frames: VecDeque::new(),
            raw_landmarks: AudioLandmarkReservoirV3::new(),
            next_frame_index: 0,
            peak_frames: 0,
            max_buffer_samples: 0,
            analyzer_nanos: 0,
            compaction_nanos: 0,
            pairing_nanos: 0,
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
                let analyzer_started_at = Instant::now();
                let peaks = self
                    .analyzer
                    .as_mut()
                    .expect("analyzer exists")
                    .peaks_for_frame(&self.rolling_samples[..V3_AUDIO_WINDOW_SAMPLES]);
                self.analyzer_nanos += analyzer_started_at.elapsed().as_nanos();
                self.process_peak_frame(self.next_frame_index, peaks);
                self.next_frame_index += 1;
                self.rolling_samples.drain(..V3_AUDIO_HOP_SAMPLES);
                self.max_buffer_samples = self.max_buffer_samples.max(self.rolling_samples.len());
            }
        }
    }

    fn process_peak_frame(&mut self, frame_index: usize, peaks: Vec<AudioSpectralPeakV3>) {
        let pairing_started_at = Instant::now();
        for anchor_frame in &mut self.recent_frames {
            let delta_frames = frame_index.saturating_sub(anchor_frame.frame_index);
            if !(V3_AUDIO_PAIR_MIN_DELTA_FRAMES..=V3_AUDIO_PAIR_MAX_DELTA_FRAMES)
                .contains(&delta_frames)
                || !audio_pair_delta_frame_is_sampled_v3(delta_frames)
            {
                continue;
            }
            for (peak_index, anchor_peak) in anchor_frame.peaks.iter().enumerate() {
                for target_peak in &peaks {
                    push_audio_pair_target_candidate_v3(
                        &mut anchor_frame.target_candidates_per_peak[peak_index],
                        AudioPairTargetCandidateV3::new(anchor_peak, target_peak, delta_frames),
                    );
                }
            }
        }
        self.pairing_nanos += pairing_started_at.elapsed().as_nanos();
        while self.recent_frames.front().is_some_and(|frame| {
            frame_index.saturating_sub(frame.frame_index) >= V3_AUDIO_PAIR_MAX_DELTA_FRAMES
        }) {
            if let Some(frame) = self.recent_frames.pop_front() {
                self.emit_closed_peak_frame(frame);
            }
        }
        self.peak_frames += 1;
        let target_candidates_per_peak = vec![Vec::new(); peaks.len()];
        self.recent_frames.push_back(AudioPeakFrameV3 {
            frame_index,
            peaks,
            target_candidates_per_peak,
        });
    }

    fn emit_closed_peak_frame(&mut self, frame: AudioPeakFrameV3) {
        let t_ms = audio_frame_timestamp_ms(frame.frame_index, self.sample_rate);
        for (anchor_peak, candidates) in frame.peaks.iter().zip(frame.target_candidates_per_peak) {
            for candidate in select_audio_pair_targets_v3(candidates) {
                let hash = audio_landmark_hash_v3(
                    anchor_peak.bin,
                    candidate.target_bin,
                    candidate.delta_frames,
                );
                let strength = ((anchor_peak.magnitude + candidate.target_magnitude) * 4.0)
                    .round()
                    .clamp(1.0, f32::from(u8::MAX)) as u8;
                let compaction_started_at = Instant::now();
                self.raw_landmarks.push(AudioLandmarkV3 {
                    hash,
                    t_ms,
                    weight: strength,
                });
                self.compaction_nanos += compaction_started_at.elapsed().as_nanos();
            }
        }
    }

    fn finish_with_metrics(
        mut self,
        duration_seconds: Option<f64>,
    ) -> (Vec<AudioLandmarkV3>, MediaAudioStreamMetrics) {
        while let Some(frame) = self.recent_frames.pop_front() {
            self.emit_closed_peak_frame(frame);
        }
        let raw_emitted = self.raw_landmarks.emitted_count;
        let raw_count = self.raw_landmarks.len();
        let max_retained = self.raw_landmarks.max_retained.max(raw_count);
        let max_raw_landmarks_after_compaction = max_retained;
        let raw_landmark_compactions = self.raw_landmarks.trim_count;
        let selection_started_at = Instant::now();
        let landmarks = finish_bounded_audio_landmarks_v3(
            self.raw_landmarks.into_landmarks(),
            duration_seconds,
        );
        let final_selection_millis = selection_started_at.elapsed().as_millis();
        let metrics = MediaAudioStreamMetrics {
            peak_frames: self.peak_frames,
            raw_landmarks_before_bounding: raw_count,
            final_landmarks: landmarks.len(),
            max_buffer_samples: self.max_buffer_samples,
            raw_landmarks_emitted: raw_emitted,
            max_raw_landmarks_seen: max_retained,
            max_raw_landmarks_after_compaction,
            raw_landmark_compactions,
            analyzer_millis: self.analyzer_nanos / 1_000_000,
            compaction_millis: self.compaction_nanos / 1_000_000,
            pairing_millis: self.pairing_nanos / 1_000_000,
            final_selection_millis,
            ..MediaAudioStreamMetrics::default()
        };
        (landmarks, metrics)
    }
}

struct AudioLandmarkReservoirV3 {
    regions: HashMap<u32, Vec<AudioLandmarkV3>>,
    emitted_count: usize,
    max_retained: usize,
    trim_count: usize,
}

impl AudioLandmarkReservoirV3 {
    fn new() -> Self {
        Self {
            regions: HashMap::new(),
            emitted_count: 0,
            max_retained: 0,
            trim_count: 0,
        }
    }

    fn push(&mut self, landmark: AudioLandmarkV3) {
        self.emitted_count += 1;
        let region = landmark.t_ms / 60_000;
        let bucket = self.regions.entry(region).or_default();
        bucket.push(landmark);
        if bucket.len() > V3_AUDIO_RAW_REGION_RETAIN_LIMIT + V3_AUDIO_RAW_REGION_TRIM_BURST {
            trim_audio_landmark_region_v3(bucket, V3_AUDIO_RAW_REGION_RETAIN_LIMIT);
            self.trim_count += 1;
        }
        self.max_retained = self.max_retained.max(self.len());
    }

    fn len(&self) -> usize {
        self.regions.values().map(Vec::len).sum()
    }

    fn into_landmarks(self) -> Vec<AudioLandmarkV3> {
        let mut landmarks = Vec::with_capacity(self.len());
        let mut regions = self.regions.into_iter().collect::<Vec<_>>();
        regions.sort_by_key(|(region, _)| *region);
        for (_, mut region_landmarks) in regions {
            trim_audio_landmark_region_v3(&mut region_landmarks, V3_AUDIO_RAW_REGION_RETAIN_LIMIT);
            landmarks.extend(region_landmarks);
        }
        landmarks.sort_by_key(|landmark| {
            (
                landmark.t_ms,
                landmark.hash,
                std::cmp::Reverse(landmark.weight),
            )
        });
        landmarks
    }
}

#[derive(Debug)]
struct AudioPeakFrameV3 {
    frame_index: usize,
    peaks: Vec<AudioSpectralPeakV3>,
    target_candidates_per_peak: Vec<Vec<AudioPairTargetCandidateV3>>,
}

#[derive(Debug, Clone, Copy)]
struct AudioPairTargetCandidateV3 {
    target_bin: usize,
    target_magnitude: f32,
    delta_frames: usize,
    delta_bucket: u32,
    score: f32,
}

impl AudioPairTargetCandidateV3 {
    fn new(
        anchor_peak: &AudioSpectralPeakV3,
        target_peak: &AudioSpectralPeakV3,
        delta_frames: usize,
    ) -> Self {
        let frequency_separation = anchor_peak.bin.abs_diff(target_peak.bin) as f32;
        let delta_bucket = audio_delta_bucket_v3(delta_frames);
        let score = (target_peak.magnitude * 2.0)
            + anchor_peak.magnitude
            + (frequency_separation + 1.0).ln()
            + (delta_bucket as f32 * 0.015);
        Self {
            target_bin: target_peak.bin,
            target_magnitude: target_peak.magnitude,
            delta_frames,
            delta_bucket,
            score,
        }
    }
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
    bounded_time_distributed_audio_landmarks_v3_for_duration(
        &mut raw,
        V3_AUDIO_VERIFY_LANDMARK_LIMIT,
        duration_seconds,
    )
}

fn trim_audio_landmark_region_v3(landmarks: &mut Vec<AudioLandmarkV3>, retain_limit: usize) {
    dedupe_audio_landmarks_v3(landmarks);
    if landmarks.len() <= retain_limit {
        return;
    }
    landmarks.sort_by_key(|landmark| {
        (
            std::cmp::Reverse(landmark.weight),
            landmark.t_ms,
            landmark.hash,
        )
    });
    landmarks.truncate(retain_limit);
    landmarks.sort_by_key(|landmark| {
        (
            landmark.t_ms,
            landmark.hash,
            std::cmp::Reverse(landmark.weight),
        )
    });
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
    audio_constellation_landmarks_v3_from_pcm_streaming(samples, sample_rate, duration_seconds).0
}

#[derive(Debug, Clone, Copy)]
struct AudioSpectralPeakV3 {
    bin: usize,
    magnitude: f32,
}

fn audio_frame_timestamp_ms(frame_index: usize, sample_rate: u32) -> u32 {
    let samples = frame_index.saturating_mul(V3_AUDIO_HOP_SAMPLES) as u64;
    ((samples * 1000) / u64::from(sample_rate)).min(u64::from(u32::MAX)) as u32
}

fn select_audio_pair_targets_v3(
    mut candidates: Vec<AudioPairTargetCandidateV3>,
) -> Vec<AudioPairTargetCandidateV3> {
    if candidates.is_empty() {
        return Vec::new();
    }
    candidates.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.delta_bucket.cmp(&right.delta_bucket))
            .then_with(|| left.delta_frames.cmp(&right.delta_frames))
            .then_with(|| left.target_bin.cmp(&right.target_bin))
    });
    let mut selected = Vec::with_capacity(V3_AUDIO_PAIR_FANOUT);
    let mut selected_delta_buckets = HashSet::new();
    for candidate in &candidates {
        if selected.len() >= V3_AUDIO_PAIR_FANOUT {
            break;
        }
        if selected_delta_buckets.insert(candidate.delta_bucket) {
            selected.push(*candidate);
        }
    }
    let mut selected_keys = selected
        .iter()
        .map(|candidate| (candidate.target_bin, candidate.delta_frames))
        .collect::<HashSet<_>>();
    for candidate in candidates {
        if selected.len() >= V3_AUDIO_PAIR_FANOUT {
            break;
        }
        if selected_keys.insert((candidate.target_bin, candidate.delta_frames)) {
            selected.push(candidate);
        }
    }
    selected.sort_by_key(|candidate| (candidate.delta_frames, candidate.target_bin));
    selected
}

fn push_audio_pair_target_candidate_v3(
    candidates: &mut Vec<AudioPairTargetCandidateV3>,
    candidate: AudioPairTargetCandidateV3,
) {
    if let Some(existing) = candidates
        .iter_mut()
        .find(|existing| existing.delta_bucket == candidate.delta_bucket)
    {
        if audio_pair_target_candidate_cmp(&candidate, existing).is_lt() {
            *existing = candidate;
        }
        return;
    }
    candidates.push(candidate);
    if candidates.len() > V3_AUDIO_PAIR_CANDIDATE_RETAIN {
        compact_audio_pair_target_candidates_v3(candidates);
    }
}

fn compact_audio_pair_target_candidates_v3(candidates: &mut Vec<AudioPairTargetCandidateV3>) {
    candidates.sort_by(audio_pair_target_candidate_cmp);
    candidates.truncate(V3_AUDIO_PAIR_CANDIDATE_RETAIN);
    candidates.sort_by_key(|candidate| (candidate.delta_bucket, candidate.delta_frames));
}

fn audio_pair_target_candidate_cmp(
    left: &AudioPairTargetCandidateV3,
    right: &AudioPairTargetCandidateV3,
) -> std::cmp::Ordering {
    right
        .score
        .total_cmp(&left.score)
        .then_with(|| left.delta_bucket.cmp(&right.delta_bucket))
        .then_with(|| left.delta_frames.cmp(&right.delta_frames))
        .then_with(|| left.target_bin.cmp(&right.target_bin))
}

fn audio_landmark_hash_v3(anchor_bin: usize, target_bin: usize, delta_frames: usize) -> u32 {
    let anchor_bin = audio_frequency_band_v3(anchor_bin);
    let target_bin = audio_frequency_band_v3(target_bin);
    let delta = audio_delta_bucket_v3(delta_frames).min(0x3ff);
    let packed = anchor_bin | (target_bin << 10) | (delta << 20);
    stable_hash_u64(packed.to_le_bytes()) as u32
}

fn audio_frequency_band_v3(bin: usize) -> u32 {
    if bin == 0 {
        return 0;
    }
    (((bin as f32 + 1.0).ln() * 96.0).round() as u32).min(0x3ff)
}

fn audio_delta_bucket_v3(delta_frames: usize) -> u32 {
    (delta_frames as u32).div_ceil(2).min(0x3ff)
}

fn audio_pair_delta_frame_is_sampled_v3(delta_frames: usize) -> bool {
    delta_frames == V3_AUDIO_PAIR_MIN_DELTA_FRAMES
        || delta_frames == V3_AUDIO_PAIR_MAX_DELTA_FRAMES
        || delta_frames
            .saturating_sub(V3_AUDIO_PAIR_MIN_DELTA_FRAMES)
            .is_multiple_of(V3_AUDIO_PAIR_DELTA_STRIDE_FRAMES)
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

pub(crate) fn bounded_time_distributed_audio_landmarks_v3_for_duration(
    landmarks: &mut [AudioLandmarkV3],
    max_landmarks: usize,
    duration_seconds: Option<f64>,
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
    let Some(duration_seconds) =
        duration_seconds.filter(|value| value.is_finite() && *value > 600.0)
    else {
        return select_time_distributed_audio_landmarks_v3(landmarks, max_landmarks);
    };
    let duration_ms = (duration_seconds * 1000.0).min(f64::from(u32::MAX)) as u32;
    let edge_ms = audio_selection_edge_region_ms(duration_ms);
    let mut body = Vec::new();
    let mut start_edge = Vec::new();
    let mut end_edge = Vec::new();
    for landmark in landmarks.iter().copied() {
        if landmark.t_ms < edge_ms {
            start_edge.push(landmark);
        } else if landmark.t_ms >= duration_ms.saturating_sub(edge_ms) {
            end_edge.push(landmark);
        } else {
            body.push(landmark);
        }
    }

    let edge_total_limit = (max_landmarks / 5).clamp(32, max_landmarks);
    let start_limit = edge_total_limit / 2;
    let end_limit = edge_total_limit.saturating_sub(start_limit);
    let mut selected = Vec::with_capacity(max_landmarks);
    selected.extend(select_time_distributed_audio_landmarks_v3(
        &mut body,
        max_landmarks.saturating_sub(edge_total_limit),
    ));
    selected.extend(select_time_distributed_audio_landmarks_v3(
        &mut start_edge,
        start_limit,
    ));
    selected.extend(select_time_distributed_audio_landmarks_v3(
        &mut end_edge,
        end_limit,
    ));

    let mut selected_keys = selected
        .iter()
        .map(|landmark| (landmark.hash, landmark.t_ms))
        .collect::<HashSet<_>>();
    if selected.len() < max_landmarks {
        let mut fill = landmarks.to_vec();
        fill.sort_by_key(|landmark| {
            (
                landmark.t_ms / 60_000,
                std::cmp::Reverse(landmark.weight),
                landmark.t_ms,
                landmark.hash,
            )
        });
        for landmark in fill {
            if selected.len() >= max_landmarks {
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
    selected.truncate(max_landmarks);
    selected
}

fn select_time_distributed_audio_landmarks_v3(
    landmarks: &mut [AudioLandmarkV3],
    max_landmarks: usize,
) -> Vec<AudioLandmarkV3> {
    if max_landmarks == 0 || landmarks.is_empty() {
        return Vec::new();
    }
    if landmarks.len() <= max_landmarks {
        let mut selected = landmarks.to_vec();
        selected.sort_by_key(|landmark| {
            (
                landmark.t_ms,
                landmark.hash,
                std::cmp::Reverse(landmark.weight),
            )
        });
        return selected;
    }
    let mut by_region = HashMap::<u32, Vec<AudioLandmarkV3>>::new();
    for landmark in landmarks.iter().copied() {
        by_region
            .entry(landmark.t_ms / 60_000)
            .or_default()
            .push(landmark);
    }
    let mut regions = by_region.into_iter().collect::<Vec<_>>();
    regions.sort_by_key(|(region, _)| *region);
    for (_, region_landmarks) in &mut regions {
        region_landmarks.sort_by_key(|landmark| {
            (
                std::cmp::Reverse(landmark.weight),
                landmark.t_ms,
                landmark.hash,
            )
        });
    }
    let mut selected = Vec::with_capacity(max_landmarks);
    let mut positions = vec![0usize; regions.len()];
    while selected.len() < max_landmarks {
        let mut advanced = false;
        for (region_index, (_, region_landmarks)) in regions.iter().enumerate() {
            if selected.len() >= max_landmarks {
                break;
            }
            if let Some(landmark) = region_landmarks.get(positions[region_index]).copied() {
                selected.push(landmark);
                positions[region_index] += 1;
                advanced = true;
            }
        }
        if !advanced {
            break;
        }
    }
    selected.sort_by_key(|landmark| {
        (
            landmark.t_ms,
            landmark.hash,
            std::cmp::Reverse(landmark.weight),
        )
    });
    selected
}

fn audio_selection_edge_region_ms(duration_ms: u32) -> u32 {
    ((f64::from(duration_ms) * 0.10).round() as u32).clamp(120_000, 240_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delayed_pair_selection_prefers_later_stronger_targets() {
        let anchor = AudioSpectralPeakV3 {
            bin: 100,
            magnitude: 2.0,
        };
        let weak = AudioSpectralPeakV3 {
            bin: 160,
            magnitude: 0.5,
        };
        let strong = AudioSpectralPeakV3 {
            bin: 260,
            magnitude: 8.0,
        };
        let candidates = (V3_AUDIO_PAIR_MIN_DELTA_FRAMES
            ..V3_AUDIO_PAIR_MIN_DELTA_FRAMES + V3_AUDIO_PAIR_FANOUT)
            .map(|delta| AudioPairTargetCandidateV3::new(&anchor, &weak, delta))
            .chain(std::iter::once(AudioPairTargetCandidateV3::new(
                &anchor,
                &strong,
                V3_AUDIO_PAIR_MIN_DELTA_FRAMES + 24,
            )))
            .collect::<Vec<_>>();

        let selected = select_audio_pair_targets_v3(candidates);

        assert!(
            selected
                .iter()
                .any(|candidate| candidate.target_bin == strong.bin),
            "{selected:?}"
        );
        assert!(
            selected
                .iter()
                .map(|candidate| candidate.delta_bucket)
                .collect::<HashSet<_>>()
                .len()
                > 1,
            "{selected:?}"
        );
    }

    #[test]
    fn audio_hash_quantization_tolerates_small_bin_and_delta_changes() {
        assert_eq!(
            audio_landmark_hash_v3(800, 900, 20),
            audio_landmark_hash_v3(801, 901, 19)
        );
    }

    #[test]
    fn long_audio_selection_reserves_body_landmarks() {
        let mut landmarks = (0..400)
            .map(|index| AudioLandmarkV3 {
                hash: index,
                t_ms: index * 250,
                weight: 12,
            })
            .chain((0..1200).map(|index| AudioLandmarkV3 {
                hash: 1_000 + index,
                t_ms: 300_000 + (index * 500),
                weight: 6,
            }))
            .chain((0..400).map(|index| AudioLandmarkV3 {
                hash: 3_000 + index,
                t_ms: 1_300_000 + (index * 250),
                weight: 12,
            }))
            .collect::<Vec<_>>();

        let selected = bounded_time_distributed_audio_landmarks_v3_for_duration(
            &mut landmarks,
            200,
            Some(1500.0),
        );
        let body = selected
            .iter()
            .filter(|landmark| landmark.t_ms >= 240_000 && landmark.t_ms < 1_260_000)
            .count();
        let edge = selected.len().saturating_sub(body);

        assert!(body > edge, "body={body} edge={edge}");
        assert!(selected.len() <= 200);
    }
}
