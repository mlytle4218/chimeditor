
use std::process::Command;

#[derive(Debug)]
pub struct VideoInfo {
    pub codec: String,
    pub width: u32,
    pub height: u32,
    pub duration: f64,
}

pub fn probe_video(
    filename: &str,
) -> Result<VideoInfo, Box<dyn std::error::Error>> {

    let output = Command::new("ffprobe")
        .args([
            "-v", "error",
            "-select_streams", "v:0",
            "-show_entries",
            "stream=codec_name,width,height",
            "-show_entries",
            "format=duration",
            "-of", "default=noprint_wrappers=1",
            filename,
        ])
        .output()?;

    if !output.status.success() {
        return Err("FFprobe failed".into());
    }

    let text = String::from_utf8(output.stdout)?;

    let mut codec = String::new();
    let mut width = 0;
    let mut height = 0;
    let mut duration = 0.0;

    for line in text.lines() {

        if let Some((key, value)) = line.split_once('=') {

            match key {
                "codec_name" => codec = value.to_string(),
                "width" => width = value.parse()?,
                "height" => height = value.parse()?,
                "duration" => duration = value.parse()?,
                _ => {}
            }
        }
    }

    if codec.is_empty() {
        return Err("No video stream found".into());
    }

    Ok(VideoInfo {
        codec,
        width,
        height,
        duration,
    })
}
