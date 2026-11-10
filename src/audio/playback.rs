
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

    pub fn play(&self) {

        let mut s = self.state.lock().unwrap();

        if s.position >= s.total_frames as f64 {
            s.position = 0.0;
        }

        s.speed = 1.0;
        s.playing = true;
    }

    pub fn pause(&self) {

        self.state.lock().unwrap().playing = false;
    }

    pub fn fast_forward(&self) {

        let mut s = self.state.lock().unwrap();

        s.speed = 2.0;
        s.playing = true;
    }

    pub fn reverse(&self) {

        let mut s = self.state.lock().unwrap();

        if s.position <= 0.0 {
            s.position = s.total_frames.saturating_sub(1) as f64;
        }

        s.speed = -1.0;
        s.playing = true;
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

    pub fn position(&self) -> f64 {

        self.state.lock().unwrap().position
            / self.sample_rate as f64
    }

    pub fn duration(&self) -> f64 {

        let s = self.state.lock().unwrap();

        s.total_frames as f64 / self.sample_rate as f64
    }
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
