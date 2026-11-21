use std::error::Error;
use std::path::Path;
use std::process::Command;
use crate::logger;
use crate::timeline::{merge_skip_ranges, SkipRange};

/// Returns true if the given extension/format corresponds to an audio format.
pub fn is_audio_format(format: &str) -> bool {
    let clean = format.trim().trim_start_matches('.').to_lowercase();
    matches!(
        clean.as_str(),
        "mp3"
            | "wav"
            | "aac"
            | "m4a"
            | "ogg"
            | "oga"
            | "flac"
            | "opus"
            | "wma"
            | "alac"
            | "aiff"
            | "aif"
    )
}

/// Derives the output filename from the input path, using the configured output format/extension (e.g. "mp4").
/// Video outputs include "-edited" (e.g. "media-edited.mp4").
/// Audio outputs omit "-edited" (e.g. "media.mp3").
pub fn get_output_filename(input_path: &str, output_format: &str) -> String {
    let path = Path::new(input_path);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("output");
    let parent = path.parent();

    let clean_fmt = output_format.trim().trim_start_matches('.');
    let ext = if !clean_fmt.is_empty() {
        clean_fmt
    } else {
        "mp4"
    };

    let new_filename = if is_audio_format(ext) {
        format!("{}.{}", stem, ext)
    } else {
        format!("{}-edited.{}", stem, ext)
    };

    if let Some(p) = parent {
        if !p.as_os_str().is_empty() {
            return p.join(new_filename).to_string_lossy().to_string();
        }
    }

    new_filename
}

/// Computes the non-skipped (playable) time segments from duration and skip ranges.
pub fn calculate_playable_segments(duration: f64, skips: &[SkipRange]) -> Vec<(f64, f64)> {
    let merged = merge_skip_ranges(skips.to_vec());
    let mut segments = Vec::new();
    let mut current = 0.0;

    for skip in merged {
        let s = skip.start.clamp(0.0, duration);
        let e = skip.end.clamp(0.0, duration);

        if s > current {
            segments.push((current, s));
        }
        current = current.max(e);
    }

    if current < duration {
        segments.push((current, duration));
    }

    // Keep only segments with meaningful duration (> 1ms)
    segments.retain(|&(start, end)| end - start > 0.001);
    segments
}

/// Probes whether the file contains video and audio streams.
pub fn probe_stream_types(filename: &str) -> (bool, bool) {
    let output = Command::new("ffprobe")
        .args([
            "-v", "error",
            "-show_entries", "stream=codec_type",
            "-of", "default=noprint_wrappers=1:nokey=1",
            filename,
        ])
        .output();

    if let Ok(out) = output {
        let text = String::from_utf8_lossy(&out.stdout);
        let has_video = text.lines().any(|l| l.trim() == "video");
        let has_audio = text.lines().any(|l| l.trim() == "audio");
        return (has_video, has_audio);
    }

    (false, true)
}

/// Builds the FFmpeg filter_complex and map arguments using select/aselect to avoid
/// the massive memory consumption and OOM kills caused by multi-branch trim + concat queues.
pub fn build_select_filter(
    segments: &[(f64, f64)],
    has_video: bool,
    has_audio: bool,
) -> (String, Vec<String>) {
    let mut expr_parts = Vec::new();
    for &(start, end) in segments {
        expr_parts.push(format!("between(t,{:.3},{:.3})", start, end));
    }
    let expr = expr_parts.join("+");

    let mut filter_parts = Vec::new();
    let mut map_args = Vec::new();

    if has_video {
        filter_parts.push(format!(
            "[0:v]select='{}',setpts=N/FRAME_RATE/TB[outv]",
            expr
        ));
        map_args.push("-map".to_string());
        map_args.push("[outv]".to_string());
    }

    if has_audio {
        filter_parts.push(format!(
            "[0:a]aselect='{}',asetpts=N/SR/TB[outa]",
            expr
        ));
        map_args.push("-map".to_string());
        map_args.push("[outa]".to_string());
    }

    let filter = filter_parts.join(";");
    (filter, map_args)
}

/// Builds the FFmpeg filter_complex and map arguments for trimming and concatenating segments.
#[allow(dead_code)]
pub fn build_trim_concat_filter(
    segments: &[(f64, f64)],
    has_video: bool,
    has_audio: bool,
) -> (String, Vec<String>) {
    let mut filter = String::new();
    let n = segments.len();

    for (i, &(start, end)) in segments.iter().enumerate() {
        if has_video {
            filter.push_str(&format!(
                "[0:v]trim=start={:.3}:end={:.3},setpts=PTS-STARTPTS[v{}];",
                start, end, i
            ));
        }
        if has_audio {
            filter.push_str(&format!(
                "[0:a]atrim=start={:.3}:end={:.3},asetpts=PTS-STARTPTS[a{}];",
                start, end, i
            ));
        }
    }

    let mut concat_inputs = String::new();
    for i in 0..n {
        if has_video {
            concat_inputs.push_str(&format!("[v{}]", i));
        }
        if has_audio {
            concat_inputs.push_str(&format!("[a{}]", i));
        }
    }

    let v_flag = if has_video { 1 } else { 0 };
    let a_flag = if has_audio { 1 } else { 0 };

    let mut map_args = Vec::new();
    let mut concat_outputs = String::new();

    if has_video {
        concat_outputs.push_str("[outv]");
        map_args.push("-map".to_string());
        map_args.push("[outv]".to_string());
    }
    if has_audio {
        concat_outputs.push_str("[outa]");
        map_args.push("-map".to_string());
        map_args.push("[outa]".to_string());
    }

    filter.push_str(&format!(
        "{}concat=n={}:v={}:a={}{}",
        concat_inputs, n, v_flag, a_flag, concat_outputs
    ));

    (filter, map_args)
}

/// Returns the appropriate encoder and preset flags based on output extension and available streams.
pub fn get_encoding_args(output_path: &str, has_video: bool, has_audio: bool) -> Vec<String> {
    let ext = Path::new(output_path)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();

    let mut args = Vec::new();

    match ext.as_str() {
        "mp4" | "m4v" | "mov" => {
            if has_video {
                args.push("-c:v".to_string());
                args.push("libx264".to_string());
                args.push("-preset".to_string());
                args.push("veryfast".to_string());
            }
            if has_audio {
                args.push("-c:a".to_string());
                args.push("aac".to_string());
                args.push("-b:a".to_string());
                args.push("192k".to_string());
            }
        }
        "webm" | "mkv" => {
            if has_video {
                args.push("-c:v".to_string());
                args.push("libx264".to_string());
                args.push("-preset".to_string());
                args.push("veryfast".to_string());
            }
            if has_audio {
                args.push("-c:a".to_string());
                args.push("aac".to_string());
            }
        }
        "mp3" => {
            if has_audio {
                args.push("-c:a".to_string());
                args.push("libmp3lame".to_string());
                args.push("-q:a".to_string());
                args.push("2".to_string());
            }
        }
        _ => {
            if has_video {
                args.push("-c:v".to_string());
                args.push("libx264".to_string());
                args.push("-preset".to_string());
                args.push("veryfast".to_string());
            }
            if has_audio {
                args.push("-c:a".to_string());
                args.push("aac".to_string());
            }
        }
    }

    args
}

/// Builds a human-readable, copy-pasteable FFmpeg command string for logging.
pub fn build_ffmpeg_command_string(
    input_path: &str,
    output_path: &str,
    filter_complex: &str,
    map_args: &[String],
    encoding_args: &[String],
) -> String {
    let mut parts = vec![
        "ffmpeg".to_string(),
        "-y".to_string(),
        "-nostdin".to_string(),
        "-v".to_string(),
        "warning".to_string(),
        "-i".to_string(),
        format!("\"{}\"", input_path),
        "-filter_complex".to_string(),
        format!("\"{}\"", filter_complex),
    ];

    for arg in map_args {
        if arg.starts_with('[') || arg.contains(' ') {
            parts.push(format!("\"{}\"", arg));
        } else {
            parts.push(arg.clone());
        }
    }

    for arg in encoding_args {
        if arg.contains(' ') {
            parts.push(format!("\"{}\"", arg));
        } else {
            parts.push(arg.clone());
        }
    }

    parts.push(format!("\"{}\"", output_path));
    parts.join(" ")
}

/// Exports the non-skipped sections to `output_path` using FFmpeg.
pub fn export_non_skipped(
    input_path: &str,
    output_path: &str,
    duration: f64,
    skips: &[SkipRange],
) -> Result<String, Box<dyn Error>> {
    let segments = calculate_playable_segments(duration, skips);
    if segments.is_empty() {
        return Err("No non-skipped content to export".into());
    }

    let (has_video, has_audio) = probe_stream_types(input_path);
    if !has_video && !has_audio {
        return Err("Input file has no detectable video or audio streams".into());
    }

    let (filter_complex, map_args) = build_select_filter(&segments, has_video, has_audio);
    let encoding_args = get_encoding_args(output_path, has_video, has_audio);

    // Build and log the final FFmpeg command
    let cmd_str = build_ffmpeg_command_string(
        input_path,
        output_path,
        &filter_complex,
        &map_args,
        &encoding_args,
    );
    logger::log(&format!("Final FFmpeg command:\n{}", cmd_str));

    let mut cmd = Command::new("ffmpeg");
    cmd.arg("-y")
       .arg("-nostdin")
       .arg("-v").arg("warning")
       .arg("-i").arg(input_path)
       .arg("-filter_complex").arg(&filter_complex);

    for arg in &map_args {
        cmd.arg(arg);
    }

    for arg in &encoding_args {
        cmd.arg(arg);
    }

    cmd.arg(output_path);

    let output = cmd.output()?;
    if !output.status.success() {
        let stderr_msg = String::from_utf8_lossy(&output.stderr);
        let stdout_msg = String::from_utf8_lossy(&output.stdout);
        let detail = if !stderr_msg.trim().is_empty() {
            stderr_msg.trim().to_string()
        } else if !stdout_msg.trim().is_empty() {
            stdout_msg.trim().to_string()
        } else {
            format!("Process exited with status: {}", output.status)
        };
        logger::log(&format!("FFmpeg export error:\n{}", detail));
        return Err(format!("FFmpeg export failed: {}", detail).into());
    }

    logger::log(&format!("FFmpeg export succeeded: {}", output_path));
    Ok(output_path.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_output_filename() {
        // Video outputs have "-edited"
        assert_eq!(get_output_filename("media.mp4", "mp4"), "media-edited.mp4");
        assert_eq!(get_output_filename("input.webm", "mp4"), "input-edited.mp4");
        assert_eq!(get_output_filename("/foo/bar/media.mp4", "mp4"), "/foo/bar/media-edited.mp4");
        assert_eq!(get_output_filename("sample.mkv", "mkv"), "sample-edited.mkv");
        assert_eq!(get_output_filename("video", "mp4"), "video-edited.mp4");
        assert_eq!(get_output_filename("rusty.webm", ".mp4"), "rusty-edited.mp4");

        // Audio outputs omit "-edited"
        assert_eq!(get_output_filename("song.mp3", "mp3"), "song.mp3");
        assert_eq!(get_output_filename("podcast.wav", "wav"), "podcast.wav");
        assert_eq!(get_output_filename("media.mp4", "mp3"), "media.mp3");
        assert_eq!(get_output_filename("media.mp4", ".aac"), "media.aac");
        assert_eq!(get_output_filename("/foo/bar/recording.m4a", "m4a"), "/foo/bar/recording.m4a");
        assert_eq!(get_output_filename("track.flac", "flac"), "track.flac");
        assert_eq!(get_output_filename("interview.ogg", "ogg"), "interview.ogg");
    }

    #[test]
    fn test_calculate_playable_segments() {
        let skips = vec![
            SkipRange { start: 10.0, end: 20.0 },
            SkipRange { start: 50.0, end: 60.0 },
        ];
        let segments = calculate_playable_segments(100.0, &skips);
        assert_eq!(
            segments,
            vec![(0.0, 10.0), (20.0, 50.0), (60.0, 100.0)]
        );

        // Skip right at start
        let skips_start = vec![SkipRange { start: 0.0, end: 15.0 }];
        assert_eq!(calculate_playable_segments(50.0, &skips_start), vec![(15.0, 50.0)]);

        // Skip right at end
        let skips_end = vec![SkipRange { start: 40.0, end: 50.0 }];
        assert_eq!(calculate_playable_segments(50.0, &skips_end), vec![(0.0, 40.0)]);

        // No skips
        assert_eq!(calculate_playable_segments(50.0, &[]), vec![(0.0, 50.0)]);
    }

    #[test]
    fn test_build_select_filter() {
        let segments = vec![(0.0, 10.0), (20.0, 30.0)];
        let (filter, map_args) = build_select_filter(&segments, true, true);
        assert!(filter.contains("select='between(t,0.000,10.000)+between(t,20.000,30.000)'"));
        assert!(filter.contains("setpts=N/FRAME_RATE/TB[outv]"));
        assert!(filter.contains("aselect='between(t,0.000,10.000)+between(t,20.000,30.000)'"));
        assert!(filter.contains("asetpts=N/SR/TB[outa]"));
        assert_eq!(map_args, vec!["-map", "[outv]", "-map", "[outa]"]);
    }

    #[test]
    fn test_build_trim_concat_filter() {
        let segments = vec![(0.0, 10.0), (20.0, 30.0)];
        let (filter, map_args) = build_trim_concat_filter(&segments, true, true);
        assert!(filter.contains("[0:v]trim=start=0.000:end=10.000"));
        assert!(filter.contains("[0:a]atrim=start=0.000:end=10.000"));
        assert!(filter.contains("concat=n=2:v=1:a=1[outv][outa]"));
        assert_eq!(map_args, vec!["-map", "[outv]", "-map", "[outa]"]);
    }

    #[test]
    fn test_build_ffmpeg_command_string() {
        let filter = "[0:v]trim=0:5[v0];[v0]concat[outv]";
        let map_args = vec!["-map".to_string(), "[outv]".to_string()];
        let encoding_args = vec!["-c:v".to_string(), "libx264".to_string(), "-preset".to_string(), "veryfast".to_string()];
        let cmd = build_ffmpeg_command_string("media.mp4", "media-edited.mp4", filter, &map_args, &encoding_args);
        assert_eq!(
            cmd,
            "ffmpeg -y -nostdin -v warning -i \"media.mp4\" -filter_complex \"[0:v]trim=0:5[v0];[v0]concat[outv]\" -map \"[outv]\" -c:v libx264 -preset veryfast \"media-edited.mp4\""
        );
    }

    #[test]
    fn test_export_non_skipped_integration() {
        if Path::new("text.mp4").exists() {
            let temp_out = "test_text_edited.mp4";
            let skips = vec![
                SkipRange { start: 1.0, end: 2.0 },
            ];
            // Export first 3 seconds with 1..2 skipped -> 2 seconds total
            let res = export_non_skipped("text.mp4", temp_out, 3.0, &skips);
            assert!(res.is_ok());
            assert!(Path::new(temp_out).exists());
            let _ = std::fs::remove_file(temp_out);
        }
    }
}
