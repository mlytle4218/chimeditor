use std::error::Error;
use std::fs::File;
use std::io::{Read, Write};
use std::process::{Command, Stdio};

pub fn decode_to_disk(
    filename: &str,
    cache_path: &str,
    sample_rate: u32,
    channels: usize,
) -> Result<u64, Box<dyn Error>> {

    println!("FFmpeg: decoding audio...");

    let rate = sample_rate.to_string();
    let channel_count = channels.to_string();

    let mut child = Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-v", "error",
            "-i", filename,
            "-map", "0:a:0",
            "-vn",
            "-ac", &channel_count,
            "-ar", &rate,
            "-c:a", "pcm_f32le",
            "-f", "f32le",
            "pipe:1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;

    let mut stdout = child.stdout.take().unwrap();
    let mut cache = File::create(cache_path)?;

    let mut buffer = vec![0u8; 1024 * 1024];

    loop {
        let count = stdout.read(&mut buffer)?;

        if count == 0 {
            break;
        }

        cache.write_all(&buffer[..count])?;
    }

    cache.flush()?;

    let status = child.wait()?;

    if !status.success() {
        return Err("FFmpeg decoding failed".into());
    }

    let file_size = cache.metadata()?.len();

    let bytes_per_frame = (channels * 4) as u64;

    if file_size == 0 || file_size % bytes_per_frame != 0 {
        return Err("Invalid PCM cache".into());
    }

    let total_frames = file_size / bytes_per_frame;

    println!(
        "Decoded {:.2} seconds",
        total_frames as f64 / sample_rate as f64
    );

    Ok(total_frames)
}