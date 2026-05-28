use std::collections::{BinaryHeap, HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Instant;

use rustfft::{Fft, FftPlanner, num_complex::Complex};
use serde::{Deserialize, Serialize};

use crate::{
    MediaAudioStreamMetrics, MediaFingerprintError,
    settings::MediaDenseAudioProfile,
    tuning::{
        V3_AUDIO_HOP_SAMPLES, V3_AUDIO_MAX_FREQ_HZ, V3_AUDIO_MAX_PEAKS_PER_FRAME,
        V3_AUDIO_MIN_FREQ_HZ, V3_AUDIO_PAIR_CANDIDATE_RETAIN, V3_AUDIO_PAIR_DELTA_STRIDE_FRAMES,
        V3_AUDIO_PAIR_FANOUT, V3_AUDIO_PAIR_MAX_DELTA_FRAMES, V3_AUDIO_PAIR_MIN_DELTA_FRAMES,
        V3_AUDIO_PEAK_NEIGHBORHOOD, V3_AUDIO_RAW_REGION_RETAIN_LIMIT,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AudioConstellationV3Config {
    pub sample_rate: u32,
    pub window_samples: usize,
    pub hop_samples: usize,
    pub max_peaks_per_frame: usize,
    pub peak_neighborhood: usize,
    pub pair_min_delta_frames: usize,
    pub pair_max_delta_frames: usize,
    pub pair_delta_stride_frames: usize,
    pub pair_fanout: usize,
    pub pair_candidate_retain: usize,
    pub anchor_peaks_per_frame: usize,
    pub target_peaks_per_frame: usize,
}

impl AudioConstellationV3Config {
    pub(crate) fn dense(profile: MediaDenseAudioProfile) -> Self {
        match profile {
            MediaDenseAudioProfile::DenseCurrent | MediaDenseAudioProfile::DenseRealfft => {
                Self::default_dense()
            }
            MediaDenseAudioProfile::Dense8k => Self {
                sample_rate: 8_000,
                ..Self::default_dense()
            },
            MediaDenseAudioProfile::DenseHop2048 => {
                Self::with_hop_preserving_target_zone(Self::default_dense(), 2048)
            }
            MediaDenseAudioProfile::Dense8kHop2048 => {
                let config = Self {
                    sample_rate: 8_000,
                    ..Self::default_dense()
                };
                Self::with_hop_preserving_target_zone(config, 2048)
            }
            MediaDenseAudioProfile::Dense8kWindow1024Hop1024 => {
                let config = Self {
                    sample_rate: 8_000,
                    window_samples: 1024,
                    ..Self::default_dense()
                };
                Self::with_hop_preserving_target_zone(config, 1024)
            }
            MediaDenseAudioProfile::DenseMaxPeaks4 => Self {
                max_peaks_per_frame: 4,
                anchor_peaks_per_frame: 4,
                target_peaks_per_frame: 4,
                ..Self::default_dense()
            },
            MediaDenseAudioProfile::DensePairRetain16 => Self {
                pair_candidate_retain: 16,
                ..Self::default_dense()
            },
            MediaDenseAudioProfile::DenseGated => Self {
                pair_candidate_retain: 12,
                pair_fanout: 4,
                anchor_peaks_per_frame: 3,
                target_peaks_per_frame: 3,
                ..Self::default_dense()
            },
            MediaDenseAudioProfile::DenseFastCombinedCandidate => {
                let config = Self {
                    sample_rate: 8_000,
                    window_samples: 1024,
                    max_peaks_per_frame: 4,
                    pair_candidate_retain: 16,
                    pair_fanout: 6,
                    anchor_peaks_per_frame: 4,
                    target_peaks_per_frame: 4,
                    ..Self::default_dense()
                };
                Self::with_hop_preserving_target_zone(config, 2048)
            }
        }
    }

    pub(crate) fn default_dense() -> Self {
        Self {
            sample_rate: crate::tuning::V3_AUDIO_SAMPLE_RATE,
            window_samples: V3_AUDIO_WINDOW_SAMPLES,
            hop_samples: V3_AUDIO_HOP_SAMPLES,
            max_peaks_per_frame: V3_AUDIO_MAX_PEAKS_PER_FRAME,
            peak_neighborhood: V3_AUDIO_PEAK_NEIGHBORHOOD,
            pair_min_delta_frames: V3_AUDIO_PAIR_MIN_DELTA_FRAMES,
            pair_max_delta_frames: V3_AUDIO_PAIR_MAX_DELTA_FRAMES,
            pair_delta_stride_frames: V3_AUDIO_PAIR_DELTA_STRIDE_FRAMES,
            pair_fanout: V3_AUDIO_PAIR_FANOUT,
            pair_candidate_retain: V3_AUDIO_PAIR_CANDIDATE_RETAIN,
            anchor_peaks_per_frame: V3_AUDIO_MAX_PEAKS_PER_FRAME,
            target_peaks_per_frame: V3_AUDIO_MAX_PEAKS_PER_FRAME,
        }
    }

    pub(crate) fn with_sample_rate(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            ..Self::default_dense()
        }
    }

    fn with_hop_preserving_target_zone(mut config: Self, hop_samples: usize) -> Self {
        let old_hop = config.hop_samples.max(1);
        config.hop_samples = hop_samples.max(1);
        config.pair_min_delta_frames =
            rescale_frame_delta(config.pair_min_delta_frames, old_hop, config.hop_samples).max(1);
        config.pair_max_delta_frames =
            rescale_frame_delta(config.pair_max_delta_frames, old_hop, config.hop_samples)
                .max(config.pair_min_delta_frames + 1);
        config.pair_delta_stride_frames =
            rescale_frame_delta(config.pair_delta_stride_frames, old_hop, config.hop_samples)
                .max(1);
        config
    }
}

fn rescale_frame_delta(delta: usize, old_hop: usize, new_hop: usize) -> usize {
    ((delta.saturating_mul(old_hop) + (new_hop / 2)) / new_hop).max(1)
}

pub(crate) struct AudioConstellationV3PcmStream {
    pending_byte: Option<u8>,
    builder: AudioConstellationV3Builder,
    streamed_bytes: usize,
    streamed_samples: usize,
}

impl AudioConstellationV3PcmStream {
    pub(crate) fn new(sample_rate: u32) -> Self {
        Self::with_config(
            AudioConstellationV3Config::with_sample_rate(sample_rate),
            V3_AUDIO_VERIFY_LANDMARK_LIMIT,
        )
    }

    pub(crate) fn with_config(config: AudioConstellationV3Config, landmark_limit: usize) -> Self {
        Self {
            pending_byte: None,
            builder: AudioConstellationV3Builder::new(config, landmark_limit),
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
    config: AudioConstellationV3Config,
    landmark_limit: usize,
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
    peak_selection_nanos: u128,
    reservoir_nanos: u128,
    candidate_pairs_considered: usize,
    candidate_pairs_skipped_by_anchor_gate: usize,
    candidate_pairs_skipped_by_target_gate: usize,
    candidate_pairs_emitted: usize,
    anchor_peaks_considered: usize,
    anchor_peaks_selected: usize,
    target_peaks_considered: usize,
    target_peaks_selected: usize,
}

impl AudioConstellationV3Builder {
    pub(crate) fn new(config: AudioConstellationV3Config, landmark_limit: usize) -> Self {
        let analyzer = (config.sample_rate != 0).then(|| AudioSpectralAnalyzerV3::new(config));
        Self {
            config,
            landmark_limit,
            analyzer,
            rolling_samples: Vec::with_capacity(config.window_samples),
            recent_frames: VecDeque::new(),
            raw_landmarks: AudioLandmarkReservoirV3::new(),
            next_frame_index: 0,
            peak_frames: 0,
            max_buffer_samples: 0,
            analyzer_nanos: 0,
            compaction_nanos: 0,
            pairing_nanos: 0,
            peak_selection_nanos: 0,
            reservoir_nanos: 0,
            candidate_pairs_considered: 0,
            candidate_pairs_skipped_by_anchor_gate: 0,
            candidate_pairs_skipped_by_target_gate: 0,
            candidate_pairs_emitted: 0,
            anchor_peaks_considered: 0,
            anchor_peaks_selected: 0,
            target_peaks_considered: 0,
            target_peaks_selected: 0,
        }
    }

    fn push_pcm_i16(&mut self, samples: &[i16]) {
        if self.analyzer.is_none() || samples.is_empty() {
            return;
        }
        let mut cursor = 0usize;
        while cursor < samples.len() {
            let needed = self
                .config
                .window_samples
                .saturating_sub(self.rolling_samples.len());
            let take = needed.min(samples.len() - cursor).max(1);
            self.rolling_samples
                .extend_from_slice(&samples[cursor..cursor + take]);
            cursor += take;
            self.max_buffer_samples = self.max_buffer_samples.max(self.rolling_samples.len());
            while self.rolling_samples.len() >= self.config.window_samples {
                let analyzer_started_at = Instant::now();
                let (peaks, peak_selection_nanos) = self
                    .analyzer
                    .as_mut()
                    .expect("analyzer exists")
                    .peaks_for_frame(&self.rolling_samples[..self.config.window_samples]);
                self.analyzer_nanos += analyzer_started_at.elapsed().as_nanos();
                self.peak_selection_nanos += peak_selection_nanos;
                self.process_peak_frame(self.next_frame_index, peaks);
                self.next_frame_index += 1;
                let remaining = self
                    .config
                    .window_samples
                    .saturating_sub(self.config.hop_samples);
                self.rolling_samples
                    .copy_within(self.config.hop_samples..self.config.window_samples, 0);
                self.rolling_samples.truncate(remaining);
                self.max_buffer_samples = self.max_buffer_samples.max(self.rolling_samples.len());
            }
        }
    }

    fn process_peak_frame(&mut self, frame_index: usize, peaks: Vec<AudioSpectralPeakV3>) {
        let pairing_started_at = Instant::now();
        for anchor_frame in &mut self.recent_frames {
            let delta_frames = frame_index.saturating_sub(anchor_frame.frame_index);
            if !(self.config.pair_min_delta_frames..=self.config.pair_max_delta_frames)
                .contains(&delta_frames)
                || !audio_pair_delta_frame_is_sampled_v3(delta_frames, self.config)
            {
                continue;
            }
            let anchor_limit = self
                .config
                .anchor_peaks_per_frame
                .min(anchor_frame.peaks.len());
            let target_limit = self.config.target_peaks_per_frame.min(peaks.len());
            self.anchor_peaks_considered = self
                .anchor_peaks_considered
                .saturating_add(anchor_frame.peaks.len());
            self.anchor_peaks_selected = self.anchor_peaks_selected.saturating_add(anchor_limit);
            self.target_peaks_considered = self.target_peaks_considered.saturating_add(peaks.len());
            self.target_peaks_selected = self.target_peaks_selected.saturating_add(target_limit);
            self.candidate_pairs_skipped_by_anchor_gate =
                self.candidate_pairs_skipped_by_anchor_gate.saturating_add(
                    anchor_frame
                        .peaks
                        .len()
                        .saturating_sub(anchor_limit)
                        .saturating_mul(peaks.len()),
                );
            self.candidate_pairs_skipped_by_target_gate =
                self.candidate_pairs_skipped_by_target_gate.saturating_add(
                    anchor_limit.saturating_mul(peaks.len().saturating_sub(target_limit)),
                );
            for (peak_index, anchor_peak) in
                anchor_frame.peaks.iter().take(anchor_limit).enumerate()
            {
                self.candidate_pairs_considered =
                    self.candidate_pairs_considered.saturating_add(target_limit);
                for target_peak in peaks.iter().take(target_limit) {
                    push_audio_pair_target_candidate_v3(
                        &mut anchor_frame.target_candidates_per_peak[peak_index],
                        AudioPairTargetCandidateV3::new(anchor_peak, target_peak, delta_frames),
                        self.config,
                    );
                }
            }
        }
        self.pairing_nanos += pairing_started_at.elapsed().as_nanos();
        while self.recent_frames.front().is_some_and(|frame| {
            frame_index.saturating_sub(frame.frame_index) >= self.config.pair_max_delta_frames
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
        let t_ms = audio_frame_timestamp_ms(frame.frame_index, self.config);
        for (anchor_peak, candidates) in frame.peaks.iter().zip(frame.target_candidates_per_peak) {
            for candidate in select_audio_pair_targets_v3(candidates, self.config) {
                let hash = audio_landmark_hash_v3(
                    anchor_peak.bin,
                    candidate.target_bin,
                    candidate.delta_frames,
                );
                let strength = ((anchor_peak.magnitude + candidate.target_magnitude) * 4.0)
                    .round()
                    .clamp(1.0, f32::from(u8::MAX)) as u8;
                let reservoir_started_at = Instant::now();
                self.raw_landmarks.push(AudioLandmarkV3 {
                    hash,
                    t_ms,
                    weight: strength,
                });
                self.candidate_pairs_emitted = self.candidate_pairs_emitted.saturating_add(1);
                self.reservoir_nanos += reservoir_started_at.elapsed().as_nanos();
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
        let accepted_into_reservoir = self.raw_landmarks.accepted_count;
        let rejected_by_reservoir = self.raw_landmarks.rejected_count;
        let raw_count = self.raw_landmarks.len();
        let max_retained = self.raw_landmarks.max_retained.max(raw_count);
        let max_raw_landmarks_after_compaction = max_retained;
        let raw_landmark_compactions = self.raw_landmarks.trim_count;
        let selection_started_at = Instant::now();
        let landmarks = finish_bounded_audio_landmarks_v3(
            self.raw_landmarks.into_landmarks(),
            duration_seconds,
            self.landmark_limit,
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
            reservoir_millis: self.reservoir_nanos / 1_000_000,
            pairing_millis: self.pairing_nanos / 1_000_000,
            peak_selection_millis: self.peak_selection_nanos / 1_000_000,
            final_selection_millis,
            candidate_pairs_considered: self.candidate_pairs_considered,
            candidate_pairs_skipped_by_anchor_gate: self.candidate_pairs_skipped_by_anchor_gate,
            candidate_pairs_skipped_by_target_gate: self.candidate_pairs_skipped_by_target_gate,
            candidate_pairs_skipped_by_saturation: rejected_by_reservoir,
            candidate_pairs_emitted: self.candidate_pairs_emitted,
            landmarks_accepted_into_reservoir: accepted_into_reservoir,
            landmarks_rejected_by_reservoir: rejected_by_reservoir,
            anchor_peaks_considered: self.anchor_peaks_considered,
            anchor_peaks_selected: self.anchor_peaks_selected,
            anchor_peaks_skipped_by_gate: self
                .anchor_peaks_considered
                .saturating_sub(self.anchor_peaks_selected),
            target_peaks_considered: self.target_peaks_considered,
            target_peaks_selected: self.target_peaks_selected,
            ..MediaAudioStreamMetrics::default()
        };
        (landmarks, metrics)
    }
}

struct AudioLandmarkReservoirV3 {
    regions: HashMap<u32, AudioLandmarkRegionReservoirV3>,
    emitted_count: usize,
    accepted_count: usize,
    rejected_count: usize,
    max_retained: usize,
    trim_count: usize,
}

impl AudioLandmarkReservoirV3 {
    fn new() -> Self {
        Self {
            regions: HashMap::new(),
            emitted_count: 0,
            accepted_count: 0,
            rejected_count: 0,
            max_retained: 0,
            trim_count: 0,
        }
    }

    fn push(&mut self, landmark: AudioLandmarkV3) {
        let region = landmark.t_ms / 60_000;
        self.emitted_count += 1;
        let accepted = self
            .regions
            .entry(region)
            .or_insert_with(|| {
                AudioLandmarkRegionReservoirV3::new(V3_AUDIO_RAW_REGION_RETAIN_LIMIT)
            })
            .push(landmark);
        if accepted {
            self.accepted_count += 1;
        } else {
            self.rejected_count += 1;
        }
        self.max_retained = self.max_retained.max(self.len());
    }

    fn len(&self) -> usize {
        self.regions
            .values()
            .map(AudioLandmarkRegionReservoirV3::len)
            .sum()
    }

    fn into_landmarks(self) -> Vec<AudioLandmarkV3> {
        let mut landmarks = Vec::with_capacity(self.len());
        let mut regions = self.regions.into_iter().collect::<Vec<_>>();
        regions.sort_by_key(|(region, _)| *region);
        for (_, region_landmarks) in regions {
            landmarks.extend(region_landmarks.into_landmarks());
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

struct AudioLandmarkRegionReservoirV3 {
    retain_limit: usize,
    landmarks: HashMap<AudioLandmarkKeyV3, AudioLandmarkV3>,
    heap: BinaryHeap<AudioLandmarkHeapEntryV3>,
}

impl AudioLandmarkRegionReservoirV3 {
    fn new(retain_limit: usize) -> Self {
        Self {
            retain_limit,
            landmarks: HashMap::new(),
            heap: BinaryHeap::new(),
        }
    }

    fn len(&self) -> usize {
        self.landmarks.len()
    }

    fn push(&mut self, landmark: AudioLandmarkV3) -> bool {
        if self.retain_limit == 0 {
            return false;
        }
        let key = AudioLandmarkKeyV3::from_landmark(landmark);
        if let Some(existing) = self.landmarks.get_mut(&key) {
            if landmark.weight > existing.weight {
                *existing = landmark;
                self.heap
                    .push(AudioLandmarkHeapEntryV3::from_landmark(landmark));
                return true;
            }
            return false;
        }
        if self.landmarks.len() < self.retain_limit {
            self.landmarks.insert(key, landmark);
            self.heap
                .push(AudioLandmarkHeapEntryV3::from_landmark(landmark));
            return true;
        }
        self.discard_stale_heap_entries();
        let candidate = AudioLandmarkHeapEntryV3::from_landmark(landmark);
        let Some(worst) = self.heap.peek().copied() else {
            self.landmarks.insert(key, landmark);
            self.heap.push(candidate);
            return true;
        };
        if candidate >= worst {
            return false;
        }
        self.heap.pop();
        self.landmarks.remove(&worst.key);
        self.landmarks.insert(key, landmark);
        self.heap.push(candidate);
        true
    }

    fn discard_stale_heap_entries(&mut self) {
        while self.heap.peek().is_some_and(|entry| {
            self.landmarks
                .get(&entry.key)
                .is_none_or(|landmark| landmark.weight != entry.weight)
        }) {
            self.heap.pop();
        }
    }

    fn into_landmarks(self) -> Vec<AudioLandmarkV3> {
        let mut landmarks = self.landmarks.into_values().collect::<Vec<_>>();
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct AudioLandmarkKeyV3 {
    hash: u32,
    t_ms: u32,
}

impl AudioLandmarkKeyV3 {
    fn from_landmark(landmark: AudioLandmarkV3) -> Self {
        Self {
            hash: landmark.hash,
            t_ms: landmark.t_ms,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AudioLandmarkHeapEntryV3 {
    key: AudioLandmarkKeyV3,
    weight: u8,
}

impl AudioLandmarkHeapEntryV3 {
    fn from_landmark(landmark: AudioLandmarkV3) -> Self {
        Self {
            key: AudioLandmarkKeyV3::from_landmark(landmark),
            weight: landmark.weight,
        }
    }

    fn inverse_weight(self) -> u8 {
        u8::MAX.saturating_sub(self.weight)
    }
}

impl Ord for AudioLandmarkHeapEntryV3 {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.inverse_weight()
            .cmp(&other.inverse_weight())
            .then_with(|| self.key.t_ms.cmp(&other.key.t_ms))
            .then_with(|| self.key.hash.cmp(&other.key.hash))
    }
}

impl PartialOrd for AudioLandmarkHeapEntryV3 {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
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
    config: AudioConstellationV3Config,
    min_bin: usize,
    max_bin: usize,
    hann: Vec<f32>,
    fft: Arc<dyn Fft<f32>>,
    buffer: Vec<Complex<f32>>,
    magnitudes: Vec<(usize, f32)>,
}

impl AudioSpectralAnalyzerV3 {
    pub(crate) fn new(config: AudioConstellationV3Config) -> Self {
        let (min_bin, max_bin) = v3_audio_bin_range(config.sample_rate, config.window_samples);
        let hann = v3_audio_hann_window(config.window_samples);
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(config.window_samples);
        Self {
            config,
            min_bin,
            max_bin,
            hann,
            fft,
            buffer: vec![Complex::new(0.0f32, 0.0f32); config.window_samples],
            magnitudes: Vec::with_capacity(max_bin.saturating_sub(min_bin)),
        }
    }

    fn peaks_for_frame(&mut self, samples: &[i16]) -> (Vec<AudioSpectralPeakV3>, u128) {
        for (index, slot) in self.buffer.iter_mut().enumerate() {
            let sample = samples[index] as f32 / f32::from(i16::MAX);
            *slot = Complex::new(sample * self.hann[index], 0.0);
        }
        self.fft.process(&mut self.buffer);
        let peak_started_at = Instant::now();
        let peaks = audio_spectral_peaks_from_fft_bins(
            &self.buffer,
            self.min_bin,
            self.max_bin,
            &mut self.magnitudes,
            self.config.max_peaks_per_frame,
            self.config.peak_neighborhood,
        );
        (peaks, peak_started_at.elapsed().as_nanos())
    }
}

#[cfg(test)]
pub(crate) fn audio_constellation_landmarks_v3_from_pcm_streaming(
    samples: &[i16],
    sample_rate: u32,
    duration_seconds: Option<f64>,
) -> (Vec<AudioLandmarkV3>, MediaAudioStreamMetrics) {
    let mut builder = AudioConstellationV3Builder::new(
        AudioConstellationV3Config::with_sample_rate(sample_rate),
        V3_AUDIO_VERIFY_LANDMARK_LIMIT,
    );
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
    let mut builder = AudioConstellationV3Builder::new(
        AudioConstellationV3Config::with_sample_rate(sample_rate),
        V3_AUDIO_VERIFY_LANDMARK_LIMIT,
    );
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
    landmark_limit: usize,
) -> Vec<AudioLandmarkV3> {
    dedupe_audio_landmarks_v3(&mut raw);
    if let Some(duration) = duration_seconds.filter(|value| value.is_finite() && *value > 120.0) {
        downweight_edge_audio_landmarks_v3(&mut raw, duration);
    }
    bounded_time_distributed_audio_landmarks_v3_for_duration(
        &mut raw,
        landmark_limit,
        duration_seconds,
    )
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

fn v3_audio_bin_range(sample_rate: u32, window_samples: usize) -> (usize, usize) {
    if sample_rate == 0 {
        return (1, window_samples / 2);
    }
    let min_bin = ((V3_AUDIO_MIN_FREQ_HZ * window_samples as f32) / sample_rate as f32).ceil();
    let max_bin = ((V3_AUDIO_MAX_FREQ_HZ * window_samples as f32) / sample_rate as f32).floor();
    let min_bin = (min_bin as usize).clamp(1, (window_samples / 2).saturating_sub(1));
    let max_bin = (max_bin as usize).clamp(min_bin + 1, window_samples / 2);
    (min_bin, max_bin)
}

fn v3_audio_hann_window(window_samples: usize) -> Vec<f32> {
    (0..window_samples)
        .map(|index| {
            let phase = (std::f32::consts::TAU * index as f32) / (window_samples - 1) as f32;
            0.5 - (0.5 * phase.cos())
        })
        .collect()
}

fn audio_spectral_peaks_from_fft_bins(
    buffer: &[Complex<f32>],
    min_bin: usize,
    max_bin: usize,
    magnitudes: &mut Vec<(usize, f32)>,
    max_peaks_per_frame: usize,
    peak_neighborhood: usize,
) -> Vec<AudioSpectralPeakV3> {
    magnitudes.clear();
    magnitudes.extend((min_bin..max_bin).map(|bin| {
        let value = buffer[bin].norm_sqr().max(f32::MIN_POSITIVE).log10();
        (bin, value)
    }));
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
        let left = local_index.saturating_sub(peak_neighborhood);
        let right = (local_index + peak_neighborhood + 1).min(magnitudes.len());
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
    peaks.truncate(max_peaks_per_frame);
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

fn audio_frame_timestamp_ms(frame_index: usize, config: AudioConstellationV3Config) -> u32 {
    let samples = frame_index.saturating_mul(config.hop_samples) as u64;
    ((samples * 1000) / u64::from(config.sample_rate)).min(u64::from(u32::MAX)) as u32
}

fn select_audio_pair_targets_v3(
    mut candidates: Vec<AudioPairTargetCandidateV3>,
    config: AudioConstellationV3Config,
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
    let mut selected = Vec::with_capacity(config.pair_fanout);
    let mut selected_delta_buckets = HashSet::new();
    for candidate in &candidates {
        if selected.len() >= config.pair_fanout {
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
        if selected.len() >= config.pair_fanout {
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
    config: AudioConstellationV3Config,
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
    if candidates.len() > config.pair_candidate_retain {
        compact_audio_pair_target_candidates_v3(candidates, config.pair_candidate_retain);
    }
}

fn compact_audio_pair_target_candidates_v3(
    candidates: &mut Vec<AudioPairTargetCandidateV3>,
    retain: usize,
) {
    candidates.sort_by(audio_pair_target_candidate_cmp);
    candidates.truncate(retain);
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

fn audio_pair_delta_frame_is_sampled_v3(
    delta_frames: usize,
    config: AudioConstellationV3Config,
) -> bool {
    delta_frames == config.pair_min_delta_frames
        || delta_frames == config.pair_max_delta_frames
        || delta_frames
            .saturating_sub(config.pair_min_delta_frames)
            .is_multiple_of(config.pair_delta_stride_frames)
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

        let selected =
            select_audio_pair_targets_v3(candidates, AudioConstellationV3Config::default_dense());

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
    fn audio_region_reservoir_reports_acceptance_and_rejection() {
        let mut reservoir = AudioLandmarkReservoirV3::new();
        for index in 0..V3_AUDIO_RAW_REGION_RETAIN_LIMIT {
            reservoir.push(AudioLandmarkV3 {
                hash: index as u32,
                t_ms: 60_000,
                weight: 20,
            });
        }
        reservoir.push(AudioLandmarkV3 {
            hash: 999_001,
            t_ms: 60_000,
            weight: 1,
        });
        reservoir.push(AudioLandmarkV3 {
            hash: 999_002,
            t_ms: 60_000,
            weight: 40,
        });

        assert_eq!(reservoir.len(), V3_AUDIO_RAW_REGION_RETAIN_LIMIT);
        assert_eq!(
            reservoir.emitted_count,
            V3_AUDIO_RAW_REGION_RETAIN_LIMIT + 2
        );
        assert_eq!(
            reservoir.accepted_count,
            V3_AUDIO_RAW_REGION_RETAIN_LIMIT + 1
        );
        assert_eq!(reservoir.rejected_count, 1);
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

    #[test]
    fn dense_fast_profile_lowers_pairing_cost_inputs() {
        let current = AudioConstellationV3Config::dense(MediaDenseAudioProfile::DenseCurrent);
        let fast =
            AudioConstellationV3Config::dense(MediaDenseAudioProfile::DenseFastCombinedCandidate);

        assert!(fast.sample_rate < current.sample_rate);
        assert!(fast.hop_samples > current.hop_samples);
        assert!(fast.max_peaks_per_frame < current.max_peaks_per_frame);
        assert!(fast.pair_candidate_retain < current.pair_candidate_retain);
    }

    #[test]
    fn dense_gated_profile_preserves_spectral_defaults_but_limits_pairing() {
        let current = AudioConstellationV3Config::dense(MediaDenseAudioProfile::DenseCurrent);
        let gated = AudioConstellationV3Config::dense(MediaDenseAudioProfile::DenseGated);

        assert_eq!(gated.sample_rate, current.sample_rate);
        assert_eq!(gated.window_samples, current.window_samples);
        assert_eq!(gated.hop_samples, current.hop_samples);
        assert_eq!(gated.max_peaks_per_frame, current.max_peaks_per_frame);
        assert!(gated.anchor_peaks_per_frame < current.anchor_peaks_per_frame);
        assert!(gated.target_peaks_per_frame < current.target_peaks_per_frame);
        assert!(gated.pair_candidate_retain < current.pair_candidate_retain);
        assert!(gated.pair_fanout < current.pair_fanout);
    }
}
