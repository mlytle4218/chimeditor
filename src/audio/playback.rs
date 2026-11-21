
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use super::decoder::decode_to_disk;
use super::disk_cache::DiskCache;
use super::audio_device::create_audio_stream;

const WINDOW_SECONDS: u64 = 10;
const REFILL_SECONDS: u64 = 2;

pub struct AudioCache {
    pub samples: Vec<f32>,
    pub base_frame: u64,
    pub channels: usize,
}

impl AudioCache {

    pub fn end_frame(&self) -> u64 {
        self.base_frame
            + (self.samples.len() / self.channels) as u64
    }
}

pub struct PlaybackState {
    pub position: f64,
    pub playing: bool,
    pub speed: f64,

    pub cache: AudioCache,

    pub total_frames: u64,
    pub loading: bool,
    pub generation: u64,

    pub skip_ranges: Vec<(u64, u64)>,
    pub editing_mode: bool,
}



pub struct AudioEngine {
    pub state: Arc<Mutex<PlaybackState>>,
    pub sample_rate: u32,

    cache_path: String,

    _stream: cpal::Stream,

    worker_stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl AudioEngine {

    pub fn new(
        filename: &str,
    ) -> Result<Self, Box<dyn std::error::Error>> {

        let host = cpal::default_host();

        let device = host
            .default_output_device()
            .ok_or("No audio output device")?;

        let supported = device.default_output_config()?;

        if supported.sample_format() != cpal::SampleFormat::F32 {
            return Err("F32 output required".into());
        }

        let config = supported.config();

        let sample_rate = config.sample_rate.0;
        let channels = config.channels as usize;

        let cache_path = format!(
            "{}/chimeditor_{}_{}.pcm",
            std::env::temp_dir().display(),
            std::process::id(),
            sample_rate
        );

        let total_frames = decode_to_disk(
            filename,
            &cache_path,
            sample_rate,
            channels,
        )?;

        let state = Arc::new(Mutex::new(PlaybackState {
            skip_ranges: Vec::new(),
            editing_mode: false,

            position: 0.0,
            playing: false,
            speed: 1.0,

            cache: AudioCache {
                samples: Vec::new(),
                base_frame: 0,
                channels,
            },

            total_frames,
            loading: false,
            generation: 0,
        }));

        let cache = DiskCache::new(
            cache_path.clone(),
            channels,
            total_frames,
        );

        // Start background disk-cache reader.

        let worker_stop = Arc::new(
            std::sync::atomic::AtomicBool::new(false)
        );

        let thread_stop = Arc::clone(&worker_stop);

        let reader_state = Arc::clone(&state);

        let worker = thread::spawn(move || {

            let window_frames =
                WINDOW_SECONDS * sample_rate as u64;

            let margin_frames =
                REFILL_SECONDS * sample_rate as u64;

            loop {

                if thread_stop.load(
                    std::sync::atomic::Ordering::Relaxed
                ) {
                    break;
                }

                thread::sleep(Duration::from_millis(20));

                if thread_stop.load(
                    std::sync::atomic::Ordering::Relaxed
                ) {
                    break;
                }

                let request = {

                    let mut s = reader_state.lock().unwrap();

                    if s.loading {
                        continue;
                    }

                    let position = s.position.max(0.0);

                    let start = s.cache.base_frame as f64;
                    let end = s.cache.end_frame() as f64;

                    let empty = s.cache.samples.is_empty();

                    let outside =
                        empty || position < start || position >= end;

                    let near_boundary = if s.speed >= 0.0 {
                        position >= end - margin_frames as f64
                            && end < s.total_frames as f64
                    } else {
                        position <= start + margin_frames as f64
                            && start > 0.0
                    };

                    if !outside && !near_boundary {
                        continue;
                    }

                    let offset = if s.speed < 0.0 {
                        window_frames.saturating_sub(margin_frames)
                    } else {
                        margin_frames
                    };

                    let new_start = (position as u64)
                        .saturating_sub(offset)
                        .min(s.total_frames.saturating_sub(window_frames));

                    s.loading = true;

                    Some((new_start, s.generation))
                };

                if let Some((new_start, generation)) = request {

                    let frames = window_frames.min(
                        total_frames.saturating_sub(new_start)
                    );

                    let result = cache.read_window(
                        new_start,
                        frames
                    );

                    let mut s = reader_state.lock().unwrap();

                    if s.generation != generation {
                        s.loading = false;
                        continue;
                    }

                    match result {

                        Ok(samples) => {
                            s.cache = AudioCache {
                                samples,
                                base_frame: new_start,
                                channels,
                            };
                        }

                        Err(error) => {
                            eprintln!("Cache read error: {}", error);
                        }
                    }

                    s.loading = false;
                }
            }
        });


        let stream = create_audio_stream(
            &device,
            &config,
            Arc::clone(&state),
            channels,
        )?;

        stream.play()?;

        Ok(Self {
            state,
            sample_rate,
            cache_path,
            _stream: stream,
            worker_stop,
            worker: Some(worker),

        })
    }

    #[allow(dead_code)]
    pub fn play(&self) {

        let mut s = self.state.lock().unwrap();

        if s.position >= s.total_frames as f64 {
            s.position = 0.0;
        }

        s.speed = 1.0;
        s.playing = true;
    }

    pub fn resume(&self) {

        let mut s = self.state.lock().unwrap();

        if s.position >= s.total_frames as f64 {
            s.position = 0.0;
        }

        if s.speed == 0.0 {
            s.speed = 1.0;
        }

        s.playing = true;
        let _ = self._stream.play();
    }

    pub fn normal_speed(&self) {

        let mut s = self.state.lock().unwrap();

        if s.position >= s.total_frames as f64 {
            s.position = 0.0;
        }

        s.speed = 1.0;
        s.playing = true;
        let _ = self._stream.play();
    }

    pub fn pause(&self) {

        self.state.lock().unwrap().playing = false;
        let _ = self._stream.pause();
    }

    pub fn pause_stream(&self) {
        let _ = self._stream.pause();
    }

    #[allow(dead_code)]
    pub fn resume_stream(&self) {
        let _ = self._stream.play();
    }

    pub fn set_speed(&self, speed: f64) {

        let mut s = self.state.lock().unwrap();

        if speed < 0.0 && s.position <= 0.0 {
            s.position = s.total_frames.saturating_sub(1) as f64;
        } else if speed > 0.0 && s.position >= s.total_frames as f64 {
            s.position = 0.0;
        }

        s.speed = speed;
        s.playing = true;
        let _ = self._stream.play();
    }

    pub fn change_speed(&self, delta: f64) -> f64 {

        let mut s = self.state.lock().unwrap();

        let base_speed = if s.speed <= 0.0 {
            1.0
        } else {
            s.speed
        };

        let mut new_speed = (base_speed + delta).max(0.25);
        new_speed = (new_speed * 100.0).round() / 100.0;

        if new_speed > 0.0 && s.position >= s.total_frames as f64 {
            s.position = 0.0;
        }

        s.speed = new_speed;
        s.playing = true;
        let _ = self._stream.play();

        new_speed
    }

    #[allow(dead_code)]
    pub fn fast_forward(&self) {

        self.set_speed(2.0);
    }

    #[allow(dead_code)]
    pub fn reverse(&self) {

        self.set_speed(-1.0);
    }

    pub fn seek(&self, seconds: f64) {

        let mut s = self.state.lock().unwrap();

        s.position = (
            seconds.max(0.0) * self.sample_rate as f64
        ).clamp(0.0, s.total_frames as f64);

        // Invalidate old cache reads.
        s.generation += 1;
        s.loading = false;
    }

    pub fn seek_backward(&self, step_seconds: f64) -> f64 {

        let mut s = self.state.lock().unwrap();
        let rate = self.sample_rate as f64;
        let current_pos = s.position / rate;

        let skip_ranges: Vec<(f64, f64)> = if s.editing_mode {
            Vec::new()
        } else {
            s.skip_ranges
                .iter()
                .map(|&(start, end)| (start as f64 / rate, end as f64 / rate))
                .collect()
        };

        let new_pos = calculate_seek_backward(current_pos, step_seconds, &skip_ranges);

        s.position = (new_pos * rate).clamp(0.0, s.total_frames as f64);
        s.generation += 1;
        s.loading = false;

        new_pos
    }

    pub fn seek_forward(&self, step_seconds: f64) -> f64 {

        let mut s = self.state.lock().unwrap();
        let rate = self.sample_rate as f64;
        let current_pos = s.position / rate;
        let duration = s.total_frames as f64 / rate;

        let skip_ranges: Vec<(f64, f64)> = if s.editing_mode {
            Vec::new()
        } else {
            s.skip_ranges
                .iter()
                .map(|&(start, end)| (start as f64 / rate, end as f64 / rate))
                .collect()
        };

        let new_pos = calculate_seek_forward(current_pos, step_seconds, duration, &skip_ranges);

        s.position = (new_pos * rate).clamp(0.0, s.total_frames as f64);
        s.generation += 1;
        s.loading = false;

        new_pos
    }

    pub fn toggle_editing_mode(&self) -> bool {
        let mut s = self.state.lock().unwrap();
        s.editing_mode = !s.editing_mode;
        s.editing_mode
    }

    #[allow(dead_code)]
    pub fn is_editing_mode(&self) -> bool {
        self.state.lock().unwrap().editing_mode
    }

    #[allow(dead_code)]
    pub fn set_editing_mode(&self, enabled: bool) {
        let mut s = self.state.lock().unwrap();
        s.editing_mode = enabled;
    }

    pub fn position(&self) -> f64 {

        self.state.lock().unwrap().position
            / self.sample_rate as f64
    }

    pub fn duration(&self) -> f64 {

        let s = self.state.lock().unwrap();

        s.total_frames as f64 / self.sample_rate as f64
    }

    pub fn set_skip_ranges(
        &self,
        ranges: &[(f64, f64)],
    ) {
        let mut s = self.state.lock().unwrap();

        let mut converted = Vec::new();

        for &(start, end) in ranges {

            let start_frame =
                (start * self.sample_rate as f64) as u64;

            let end_frame =
                (end * self.sample_rate as f64) as u64;

            let start_frame = start_frame.min(s.total_frames);
            let end_frame = end_frame.min(s.total_frames);

            if end_frame > start_frame {
                converted.push((start_frame, end_frame));
            }
        }

        converted.sort_by_key(|&(start, _)| start);
        s.skip_ranges = converted;
        s.generation += 1;
        s.loading = false;
    }
}

/// Calculates new position when seeking backward by `step_seconds`.
/// Skip sections are bypassed like that audio does not count toward the seek step.
pub fn calculate_seek_backward(
    current_pos: f64,
    step_seconds: f64,
    skip_ranges: &[(f64, f64)],
) -> f64 {
    if step_seconds <= 0.0 {
        return current_pos.max(0.0);
    }

    let mut pos = current_pos.max(0.0);
    let mut rem = step_seconds;

    // Step 1: If current position is inside a skip range [start, end],
    // move to the start of the skip range without consuming seek distance.
    for &(start, end) in skip_ranges {
        if pos > start && pos <= end {
            pos = start;
            break;
        }
    }

    // Step 2: Traverse skip ranges ending before or at pos, from right to left.
    let mut prior_skips: Vec<(f64, f64)> = skip_ranges
        .iter()
        .copied()
        .filter(|&(_, end)| end <= pos)
        .collect();
    prior_skips.sort_by(|a, b| a.0.total_cmp(&b.0));

    for &(start, end) in prior_skips.iter().rev() {
        let playable_len = pos - end;
        if rem <= playable_len {
            pos -= rem;
            rem = 0.0;
            break;
        } else {
            rem -= playable_len;
            pos = start;
        }
    }

    // Step 3: Consume any remaining seek distance in the opening playable segment [0, pos].
    if rem > 0.0 {
        if rem <= pos {
            pos -= rem;
        } else {
            pos = 0.0;
        }
    }

    pos
}

/// Calculates new position when seeking forward by `step_seconds`.
/// Skip sections are bypassed like that audio does not count toward the seek step.
pub fn calculate_seek_forward(
    current_pos: f64,
    step_seconds: f64,
    duration: f64,
    skip_ranges: &[(f64, f64)],
) -> f64 {
    if step_seconds <= 0.0 {
        return current_pos.clamp(0.0, duration);
    }

    let mut pos = current_pos.clamp(0.0, duration);
    let mut rem = step_seconds;

    // Step 1: If current position is inside a skip range [start, end],
    // move to the end of the skip range without consuming seek distance.
    for &(start, end) in skip_ranges {
        if pos >= start && pos < end {
            pos = end.min(duration);
            break;
        }
    }

    // Step 2: Traverse skip ranges starting at or after pos, from left to right.
    let mut subsequent_skips: Vec<(f64, f64)> = skip_ranges
        .iter()
        .copied()
        .filter(|&(start, _)| start >= pos)
        .collect();
    subsequent_skips.sort_by(|a, b| a.0.total_cmp(&b.0));

    for &(start, end) in &subsequent_skips {
        let playable_len = (start - pos).max(0.0);
        if rem <= playable_len {
            pos += rem;
            rem = 0.0;
            break;
        } else {
            rem -= playable_len;
            pos = end.min(duration);
        }
    }

    // Step 3: Consume any remaining seek distance in the final playable segment [pos, duration].
    if rem > 0.0 {
        pos = (pos + rem).min(duration);
    }

    pos
}

impl Drop for AudioEngine {

    fn drop(&mut self) {

        self.worker_stop.store(true, Ordering::Relaxed);

        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }

        let _ = std::fs::remove_file(&self.cache_path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_seek_backward_over_skip() {
        let skips = vec![(10.0, 20.0)];

        // Case 1: seeking backward from 22.0 by 5.0s.
        // Moves 22.0 -> 20.0 (2s), skips [10.0, 20.0], then 10.0 -> 7.0 (3s). Total = 5s playable audio.
        let pos = calculate_seek_backward(22.0, 5.0, &skips);
        assert!((pos - 7.0).abs() < 1e-6);

        // Case 2: starting inside skip [10.0, 20.0] at 15.0s.
        // Skip audio doesn't count, so starts at 10.0, then moves 5s back to 5.0.
        let pos = calculate_seek_backward(15.0, 5.0, &skips);
        assert!((pos - 5.0).abs() < 1e-6);

        // Case 3: starting at skip boundary (20.0s).
        let pos = calculate_seek_backward(20.0, 5.0, &skips);
        assert!((pos - 5.0).abs() < 1e-6);

        // Case 4: seek does not cross skip (from 28.0s by 5.0s -> 23.0s).
        let pos = calculate_seek_backward(28.0, 5.0, &skips);
        assert!((pos - 23.0).abs() < 1e-6);

        // Case 5: multiple skip sections.
        let multi_skips = vec![(5.0, 15.0), (20.0, 30.0)];
        // 35.0 -> 30.0 (5s), skips to 20.0, 20.0 -> 15.0 (5s), skips to 5.0, 5.0 -> 3.0 (2s). Total = 12s.
        let pos = calculate_seek_backward(35.0, 12.0, &multi_skips);
        assert!((pos - 3.0).abs() < 1e-6);

        // Case 6: hits start of media (0.0).
        let pos = calculate_seek_backward(4.0, 10.0, &skips);
        assert!((pos - 0.0).abs() < 1e-6);

        // Case 7: no skips.
        let pos = calculate_seek_backward(10.0, 4.0, &[]);
        assert!((pos - 6.0).abs() < 1e-6);
    }

    #[test]
    fn test_seek_forward_over_skip() {
        let skips = vec![(10.0, 20.0)];

        // Case 1: seeking forward from 8.0 by 5.0s.
        // Moves 8.0 -> 10.0 (2s), skips [10.0, 20.0], then 20.0 -> 23.0 (3s). Total = 5s playable audio.
        let pos = calculate_seek_forward(8.0, 5.0, 100.0, &skips);
        assert!((pos - 23.0).abs() < 1e-6);

        // Case 2: starting inside skip at 15.0s.
        let pos = calculate_seek_forward(15.0, 5.0, 100.0, &skips);
        assert!((pos - 25.0).abs() < 1e-6);

        // Case 3: seek clamped at duration.
        let pos = calculate_seek_forward(98.0, 5.0, 100.0, &skips);
        assert!((pos - 100.0).abs() < 1e-6);
    }

    #[test]
    fn test_editing_mode_seeking_does_not_skip() {
        // In editing mode, seeking uses empty skip list so skips are NOT jumped
        let skips_empty = vec![];
        let pos = calculate_seek_forward(8.0, 5.0, 100.0, &skips_empty);
        assert_eq!(pos, 13.0);

        let pos_back = calculate_seek_backward(13.0, 5.0, &skips_empty);
        assert_eq!(pos_back, 8.0);
    }
}
