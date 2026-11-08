
use std::error::Error;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::io;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

struct AudioData {
    samples: Vec<f32>,
    sample_rate: u32,
    channels: usize,
}

struct PlaybackState {
    position: f64,
    playing: bool,
    speed: f64,
}

// Decode MP4 audio using FFmpeg.
fn decode_mp4(
    filename: &str,
    sample_rate: u32,
    channels: usize,
) -> Result<AudioData, Box<dyn Error>> {

    println!("Decoding MP4 with FFmpeg...");

    let output = Command::new("ffmpeg")
        .args([
            "-v", "error",
            "-i", filename,
            "-map", "0:a:0",
            "-vn",
            "-ac", &channels.to_string(),
            "-ar", &sample_rate.to_string(),
            "-c:a", "pcm_f32le",
            "-f", "f32le",
            "pipe:1",
        ])
        .output()?;

    if !output.status.success() {
        return Err(
            format!(
                "FFmpeg decoding failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ).into()
        );
    }

    // Convert little-endian bytes into f32 samples.
    let samples: Vec<f32> = output.stdout
        .chunks_exact(4)
        .map(|bytes| {
            f32::from_le_bytes([
                bytes[0],
                bytes[1],
                bytes[2],
                bytes[3],
            ])
        })
        .collect();

    if samples.is_empty() {
        return Err("No audio samples found in MP4".into());
    }

    println!("FFmpeg decoding completed!");
    println!("Sample rate: {} Hz", sample_rate);
    println!("Channels: {}", channels);
    println!("Total samples: {}", samples.len());

    Ok(AudioData {
        samples,
        sample_rate,
        channels,
    })
}

fn main() -> Result<(), Box<dyn Error>> {

    let filename = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "input.mp4".to_string());

    // Initialize CPAL.
    let host = cpal::default_host();

    let device = host
        .default_output_device()
        .ok_or("No audio output device found")?;

    let supported_config = device.default_output_config()?;

    if supported_config.sample_format() != cpal::SampleFormat::F32 {
        return Err("This example requires F32 audio output".into());
    }

    let config = supported_config.config();

    let sample_rate = config.sample_rate.0;
    let channels = config.channels as usize;

    // Decode audio using FFmpeg at the output device's format.
    let audio = Arc::new(
        decode_mp4(&filename, sample_rate, channels)?
    );

    let total_frames = audio.samples.len() / audio.channels;

    let state = Arc::new(Mutex::new(PlaybackState {
        position: 0.0,
        playing: false,
        speed: 1.0,
    }));

    let audio_callback = Arc::clone(&audio);
    let state_callback = Arc::clone(&state);

    // Create CPAL audio stream.
    let stream = device.build_output_stream(
        &config,

        move |output: &mut [f32], _| {

            let mut state = match state_callback.lock() {
                Ok(s) => s,
                Err(_) => {
                    output.fill(0.0);
                    return;
                }
            };

            let total_frames =
                audio_callback.samples.len() / audio_callback.channels;

            for frame in output.chunks_mut(audio_callback.channels) {

                if !state.playing {
                    frame.fill(0.0);
                    continue;
                }

                let position = state.position;

                if position < 0.0 ||
                   position >= total_frames as f64 {

                    state.playing = false;
                    frame.fill(0.0);
                    continue;
                }

                let index0 = position.floor() as usize;
                let index1 = (index0 + 1).min(total_frames - 1);

                let fraction = (position - index0 as f64) as f32;

                // Interpolate PCM samples.
                for (channel, sample) in frame.iter_mut().enumerate() {

                    let base0 =
                        index0 * audio_callback.channels + channel;

                    let base1 =
                        index1 * audio_callback.channels + channel;

                    let a = audio_callback.samples[base0];
                    let b = audio_callback.samples[base1];

                    *sample = a + (b - a) * fraction;
                }

                // Advance playback position.
                state.position += state.speed;
            }
        },

        |err| eprintln!("Audio output error: {}", err),

        None,
    )?;

    stream.play()?;

    println!("\n================================");
    println!("       CHIMEDITOR PLAYER");
    println!("================================");

    println!("play       - Play");
    println!("pause      - Pause");
    println!("f          - Fast forward 2x");
    println!("r          - Reverse 1x");
    println!("s 30       - Seek to 30 seconds");
    println!("q          - Quit");

    let stdin = io::stdin();

    loop {

        let mut command = String::new();

        stdin.read_line(&mut command)?;

        let command = command.trim();

        let mut state = state.lock().unwrap();

        match command {

            "play" => {

                if state.position >= total_frames as f64 {
                    state.position = 0.0;
                }

                state.speed = 1.0;
                state.playing = true;

                println!("Playing...");
            }

            "pause" => {

                state.playing = false;

                println!("Paused.");
            }

            "f" => {

                state.speed = 2.0;
                state.playing = true;

                println!("Fast forward 2x.");
            }

            "r" => {

                if state.position <= 0.0 {
                    state.position = (total_frames - 1) as f64;
                }

                state.speed = -1.0;
                state.playing = true;

                println!("Reverse playback.");
            }

            "q" => {

                println!("Exiting ChimEditor.");
                break;
            }

            cmd if cmd.starts_with("s ") => {

                if let Ok(seconds) =
                    cmd[2..].trim().parse::<f64>() {

                    state.position = (
                        seconds * audio.sample_rate as f64
                    ).clamp(0.0, total_frames as f64);

                    println!("Seeked to {:.2} seconds.", seconds);
                }
            }

            _ => println!("Unknown command."),
        }
    }

    Ok(())
}
