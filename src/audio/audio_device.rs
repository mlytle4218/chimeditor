
use std::sync::{Arc, Mutex};

use cpal::traits::DeviceTrait;

use super::playback::PlaybackState;

pub fn create_audio_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    state: Arc<Mutex<PlaybackState>>,
    channels: usize,
) -> Result<cpal::Stream, Box<dyn std::error::Error>> {

    let stream = device.build_output_stream(
        config,

        move |output: &mut [f32], _| {

            let mut s = match state.try_lock() {
                Ok(s) => s,
                Err(_) => {
                    output.fill(0.0);
                    return;
                }
            };

            let mut position = s.position;
            let speed = s.speed;
            let playing = s.playing;

            let base = s.cache.base_frame as f64;
            let cache_channels = s.cache.channels;

            let total_cached =
                s.cache.samples.len() / cache_channels;

            let end = base + total_cached as f64;

            let total_frames = s.total_frames as f64;

            for frame in output.chunks_mut(channels) {

                if !playing {
                    frame.fill(0.0);
                    continue;
                }

                if position < base || position >= end {
                    frame.fill(0.0);
                    continue;
                }

                let local = position - base;

                let index0 = local.floor() as usize;

                let index1 =
                    (index0 + 1).min(total_cached - 1);

                let fraction =
                    (local - index0 as f64) as f32;

                for (channel, sample) in frame.iter_mut().enumerate() {

                    let ch = channel.min(cache_channels - 1);

                    let a = s.cache.samples[
                        index0 * cache_channels + ch
                    ];

                    let b = s.cache.samples[
                        index1 * cache_channels + ch
                    ];

                    *sample = a + (b - a) * fraction;
                }

                position += speed;

                if position < 0.0 || position >= total_frames {

                    position = position.clamp(
                        0.0,
                        (total_frames - 1.0).max(0.0)
                    );

                    s.playing = false;
                    break;
                }
            }

            s.position = position;
        },

        |err| {
            eprintln!("CPAL audio error: {}", err);
        },

        None,
    )?;

    Ok(stream)
}
