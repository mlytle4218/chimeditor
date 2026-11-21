use std::sync::{Arc, Mutex};

use cpal::traits::DeviceTrait;

use super::playback::PlaybackState;

/// Synthesizes a frequency-modulated boundary earcon tone.
/// Linear frequency sweep from `f_start` to `f_end` over `duration_secs` with
/// smooth cosine attack and release envelopes to prevent clicks.
pub fn generate_boundary_tone(
    sample_rate: u32,
    f_start: f32,
    f_end: f32,
    duration_secs: f32,
    amplitude: f32,
) -> Vec<f32> {
    let sr = sample_rate as f32;
    let total_samples = (duration_secs * sr) as usize;
    let fade_in = (0.015 * sr) as usize;
    let fade_out = (0.030 * sr) as usize;
    let mut samples = Vec::with_capacity(total_samples);

    for n in 0..total_samples {
        let tau = n as f32 / sr;
        let theta = 2.0
            * std::f32::consts::PI
            * (f_start * tau + (f_end - f_start) / (2.0 * duration_secs) * (tau * tau));

        let env = if n < fade_in && fade_in > 0 {
            ((std::f32::consts::FRAC_PI_2) * (n as f32 / fade_in as f32)).sin()
        } else if n >= total_samples.saturating_sub(fade_out) && fade_out > 0 {
            let offset = n - (total_samples - fade_out);
            ((std::f32::consts::FRAC_PI_2) * (offset as f32 / fade_out as f32)).cos()
        } else {
            1.0
        };

        // Fundamental (75%) + 2nd harmonic octave (25%) for a rich, warm tone
        let val = env * (0.75 * theta.sin() + 0.25 * (2.0 * theta).sin()) * amplitude;
        samples.push(val);
    }

    samples
}

pub fn create_audio_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    state: Arc<Mutex<PlaybackState>>,
    channels: usize,
) -> Result<cpal::Stream, Box<dyn std::error::Error>> {

    let sample_rate = config.sample_rate.0;
    // Ascending sound for start of skip (400 Hz -> 850 Hz)
    let ascending_tone = Arc::new(generate_boundary_tone(sample_rate, 400.0, 850.0, 0.15, 0.35));
    // Descending tone for end of skip (850 Hz -> 400 Hz)
    let descending_tone = Arc::new(generate_boundary_tone(sample_rate, 850.0, 400.0, 0.15, 0.35));

    let mut active_tone: Option<(Arc<Vec<f32>>, usize)> = None;

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
                let prev_pos = position;

                if !s.editing_mode {
                    // Normal mode: jump over skip sections
                    let skip_target = if speed >= 0.0 {
                        s.skip_ranges.iter()
                            .find(|&&(start, end)| {
                                position >= start as f64 &&
                                position < end as f64
                            })
                            .map(|&(_, end)| end)
                    } else {
                        s.skip_ranges.iter()
                            .find(|&&(start, end)| {
                                position > start as f64 &&
                                position <= end as f64
                            })
                            .map(|&(start, _)| start)
                    };

                    if let Some(target) = skip_target {
                        position = target as f64;

                        // Force the disk cache reader to load the new position.
                        s.generation += 1;
                        s.loading = false;

                        frame.fill(0.0);
                        continue;
                    }
                } else {
                    // Editing mode: do not skip skips.
                    // Play an ascending sound at the beginning of the skip,
                    // and a descending tone at the end of the skip.
                    if playing && speed != 0.0 {
                        let next_pos = position + speed;
                        for &(start, end) in &s.skip_ranges {
                            let start_f = start as f64;
                            let end_f = end as f64;

                            if speed > 0.0 {
                                // Crossing beginning of skip forward -> ascending sound
                                if prev_pos <= start_f && next_pos > start_f {
                                    active_tone = Some((Arc::clone(&ascending_tone), 0));
                                    break;
                                }
                                // Crossing end of skip forward -> descending tone
                                if prev_pos <= end_f && next_pos > end_f {
                                    active_tone = Some((Arc::clone(&descending_tone), 0));
                                    break;
                                }
                            } else {
                                // Crossing end of skip in reverse -> descending tone
                                if prev_pos >= end_f && next_pos < end_f {
                                    active_tone = Some((Arc::clone(&descending_tone), 0));
                                    break;
                                }
                                // Crossing beginning of skip in reverse -> ascending sound
                                if prev_pos >= start_f && next_pos < start_f {
                                    active_tone = Some((Arc::clone(&ascending_tone), 0));
                                    break;
                                }
                            }
                        }
                    }
                }

                // Advance boundary tone if one is currently active
                let tone_sample = if let Some((ref tone_buf, ref mut idx)) = active_tone {
                    if *idx < tone_buf.len() {
                        let val = tone_buf[*idx];
                        *idx += 1;
                        val
                    } else {
                        0.0
                    }
                } else {
                    0.0
                };

                if let Some((ref tone_buf, idx)) = active_tone {
                    if idx >= tone_buf.len() {
                        active_tone = None;
                    }
                }

                if !playing {
                    if tone_sample != 0.0 {
                        frame.fill(tone_sample);
                    } else {
                        frame.fill(0.0);
                    }
                    continue;
                }

                if position < base || position >= end {
                    if tone_sample != 0.0 {
                        frame.fill(tone_sample);
                    } else {
                        frame.fill(0.0);
                    }
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

                    let media_sample = a + (b - a) * fraction;

                    let mixed = if tone_sample != 0.0 {
                        media_sample * 0.75 + tone_sample
                    } else {
                        media_sample
                    };

                    *sample = mixed.clamp(-1.0, 1.0);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_boundary_tone() {
        let tone = generate_boundary_tone(48000, 400.0, 850.0, 0.15, 0.35);
        assert_eq!(tone.len(), (48000.0 * 0.15) as usize);
        assert!(tone[0].abs() < 1e-4);
        assert!(tone.last().unwrap().abs() < 1e-3);
        for &s in &tone {
            assert!(s >= -0.4 && s <= 0.4);
        }
    }
}
