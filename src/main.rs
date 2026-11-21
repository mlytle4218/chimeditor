mod audio;
mod video;
mod timeline;
mod config;
pub mod logger;
mod project;

use audio::AudioEngine;
use video::{export_non_skipped, probe_media_details, probe_video};
use timeline::{merge_skip_ranges, SkipRange};
use config::{Action, Config};
use project::{is_cme_file, Project};

use crossterm::{
    cursor::MoveTo,
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType},
};

use std::error::Error;
use std::io::{stdout, Write};
use std::time::Duration;

// Prints status message at the top-left corner (0, 0) of the crossterm screen.
fn print_status(message: &str) {
    let mut out = stdout();
    let _ = execute!(out, MoveTo(0, 0), Clear(ClearType::CurrentLine));
    let _ = write!(out, "{}", message);
    let _ = out.flush();
}

// Formats seconds into HH:MM:SS (hours:minutes:seconds).
fn format_time(seconds: f64) -> String {
    let total_secs = seconds.max(0.0) as u64;
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let secs = total_secs % 60;
    format!("{:02}:{:02}:{:02}", hours, minutes, secs)
}

// Formats seconds into words: "X hours Y minutes Z seconds" (omits hours if under an hour).
pub fn format_time_words(seconds: f64) -> String {
    let total_secs = seconds.max(0.0) as u64;
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let secs = total_secs % 60;

    let mut parts = Vec::new();
    if hours > 0 {
        let h_unit = if hours == 1 { "hour" } else { "hours" };
        parts.push(format!("{} {}", hours, h_unit));
    }

    let m_unit = if minutes == 1 { "minute" } else { "minutes" };
    parts.push(format!("{} {}", minutes, m_unit));

    let s_unit = if secs == 1 { "second" } else { "seconds" };
    parts.push(format!("{} {}", secs, s_unit));

    parts.join(" ")
}

// Calculates the next skip index and target seek position (2 seconds before the skip, or 0.0 if starting <= 2.0s).
pub fn calculate_cycle_skip_target(
    current_idx: Option<usize>,
    skips: &[SkipRange],
) -> Option<(usize, f64)> {
    if skips.is_empty() {
        return None;
    }
    let next_idx = match current_idx {
        None => 0,
        Some(idx) => (idx + 1) % skips.len(),
    };
    let skip = &skips[next_idx];
    let target = if skip.start <= 2.0 {
        0.0
    } else {
        skip.start - 2.0
    };
    Some((next_idx, target))
}

// Deletes the skip range containing the given position, returning the removed skip if found.
pub fn delete_skip_at(position: f64, skips: &mut Vec<SkipRange>) -> Option<SkipRange> {
    if let Some(idx) = skips.iter().position(|r| position >= r.start && position <= r.end) {
        Some(skips.remove(idx))
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NudgeType {
    StartEarlier, // 'n': start -= 0.25s
    StartLater,   // 'm': start += 0.25s
    EndEarlier,   // 'N': end -= 0.25s
    EndLater,     // 'M': end += 0.25s
}

// Finds the index of the skip to nudge based on position and last active skip index.
pub fn find_current_skip_idx(
    pos: f64,
    last_idx: Option<usize>,
    skips: &[SkipRange],
) -> Option<usize> {
    if skips.is_empty() {
        return None;
    }
    // 1. Position is strictly inside a skip
    if let Some(idx) = skips.iter().position(|r| pos >= r.start && pos <= r.end) {
        return Some(idx);
    }
    // 2. Position is within 2.5 seconds before a skip's start (e.g. pre-roll from C or previous nudge)
    if let Some(idx) = skips
        .iter()
        .position(|r| pos >= (r.start - 2.5).max(0.0) && pos < r.start)
    {
        return Some(idx);
    }
    // 3. Position is near the last active/cycled skip (within 3.5s before start to 2.5s after end)
    if let Some(idx) = last_idx {
        if idx < skips.len() {
            let s = &skips[idx];
            if pos >= (s.start - 3.5).max(0.0) && pos <= s.end + 2.5 {
                return Some(idx);
            }
        }
    }
    // 4. Fallback to last_idx if valid
    if let Some(idx) = last_idx {
        if idx < skips.len() {
            return Some(idx);
        }
    }
    // 5. If only 1 skip exists, default to it
    if skips.len() == 1 {
        return Some(0);
    }
    None
}

// Nudges a skip boundary by 0.25s and returns the updated skip and target seek position.
pub fn nudge_skip(
    nudge: NudgeType,
    idx: usize,
    skips: &mut [SkipRange],
    duration: f64,
) -> Option<(SkipRange, f64)> {
    if idx >= skips.len() {
        return None;
    }

    let min_start = if idx > 0 { skips[idx - 1].end } else { 0.0 };
    let max_end = if idx + 1 < skips.len() {
        skips[idx + 1].start
    } else {
        duration
    };

    let skip = &mut skips[idx];

    match nudge {
        NudgeType::StartEarlier => {
            let new_start = (skip.start - 0.25).max(min_start);
            skip.start = new_start;
            let target = if new_start <= 2.0 { 0.0 } else { new_start - 2.0 };
            Some((skip.clone(), target))
        }
        NudgeType::StartLater => {
            let max_start = (skip.end - 0.1).max(0.0);
            let new_start = (skip.start + 0.25).min(max_start);
            skip.start = new_start;
            let target = if new_start <= 2.0 { 0.0 } else { new_start - 2.0 };
            Some((skip.clone(), target))
        }
        NudgeType::EndEarlier => {
            let min_end = skip.start + 0.1;
            let new_end = (skip.end - 0.25).max(min_end);
            skip.end = new_end;
            let target = if new_end <= 2.0 { 0.0 } else { new_end - 2.0 };
            Some((skip.clone(), target))
        }
        NudgeType::EndLater => {
            let new_end = (skip.end + 0.25).min(max_end);
            skip.end = new_end;
            let target = if new_end <= 2.0 { 0.0 } else { new_end - 2.0 };
            Some((skip.clone(), target))
        }
    }
}

#[cfg(unix)]
struct StderrGuard {
    saved_fd: Option<i32>,
}

#[cfg(unix)]
impl StderrGuard {
    fn new(log_path: &str) -> Self {
        use std::os::unix::io::AsRawFd;
        unsafe extern "C" {
            fn dup(fd: std::os::raw::c_int) -> std::os::raw::c_int;
            fn dup2(oldfd: std::os::raw::c_int, newfd: std::os::raw::c_int) -> std::os::raw::c_int;
        }

        if let Ok(file) = std::fs::OpenOptions::new().create(true).append(true).open(log_path) {
            unsafe {
                let saved = dup(2);
                if saved >= 0 {
                    dup2(file.as_raw_fd(), 2);
                    return Self { saved_fd: Some(saved) };
                }
            }
        }
        Self { saved_fd: None }
    }
}

#[cfg(unix)]
impl Drop for StderrGuard {
    fn drop(&mut self) {
        unsafe extern "C" {
            fn dup2(oldfd: std::os::raw::c_int, newfd: std::os::raw::c_int) -> std::os::raw::c_int;
            fn close(fd: std::os::raw::c_int) -> std::os::raw::c_int;
        }

        if let Some(saved) = self.saved_fd.take() {
            unsafe {
                dup2(saved, 2);
                close(saved);
            }
        }
    }
}

#[cfg(not(unix))]
struct StderrGuard;
#[cfg(not(unix))]
impl StderrGuard {
    fn new(_log_path: &str) -> Self { Self }
}

// Automatically restore terminal mode and standard error when exiting.
struct TerminalGuard {
    _stderr: StderrGuard,
}

impl TerminalGuard {
    fn new(log_path: &str) -> Result<Self, Box<dyn Error>> {
        let stderr = StderrGuard::new(log_path);
        enable_raw_mode()?;
        let mut out = stdout();
        let _ = execute!(out, Clear(ClearType::All), MoveTo(0, 0));
        let _ = out.flush();
        Ok(Self { _stderr: stderr })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let mut out = stdout();
        let _ = execute!(out, MoveTo(0, 1), Clear(ClearType::CurrentLine));
        println!();
        logger::log("ChimEditor closed.");
    }
}

fn main() -> Result<(), Box<dyn Error>> {

    let mut args = std::env::args().skip(1);

    let input_arg = args
        .next()
        .unwrap_or_else(|| "input.mp4".to_string());

    // Ensure user config directory exists (~/.config/shimeditor) and load configuration.
    let config_path = config::ensure_user_config_exists();
    let config = Config::load_from_file(&config_path.to_string_lossy());

    // Resolve log file path (~/.config/shimeditor/chimeditor.log).
    let log_path = config::get_log_file_path(&config.settings.log_file);
    let log_path_str = log_path.to_string_lossy().to_string();

    // Initialize program log file.
    let _ = logger::init(&log_path_str);

    logger::log(&format!("Loaded config: {}", config_path.display()));
    logger::log(&format!("Log file: {}", log_path_str));
    logger::log(&format!("Input argument: {}", input_arg));
    logger::log(&format!("Output format setting: {}", config.settings.output_format));

    let input_path = std::path::Path::new(&input_arg);

    // Initialize project:
    // If input is .cme, open the project archive and load its skips.
    // If input is a media file, create a new .cme zip archive with the media and blank skips.
    let mut project = if is_cme_file(input_path) {
        logger::log(&format!("Opening existing project archive: {}", input_arg));
        Project::open_cme(input_path)?
    } else {
        logger::log(&format!(
            "Media file passed ('{}'). Creating .cme project archive...",
            input_arg
        ));
        Project::create_from_media(input_path)?
    };

    let filename = project.media_path.to_string_lossy().to_string();
    let cme_path_display = project.cme_path.to_string_lossy().to_string();

    // Inspect and log FFprobe / FFmpeg information about the media file.
    let media_details = probe_media_details(&filename);
    logger::log("--- FFprobe Media Stream Details ---");
    logger::log(&media_details);

    match probe_video(&filename) {
        Ok(info) => {
            logger::log("--- Video Stream Summary ---");
            logger::log(&format!("Codec: {}", info.codec));
            logger::log(&format!("Resolution: {}x{}", info.width, info.height));
            logger::log(&format!("Duration: {:.2}s", info.duration));
        }
        Err(error) => {
            logger::log(&format!("Video stream info: {}", error));
        }
    }

    // Initialize audio engine (FFmpeg decoding logged to log file).
    let engine = AudioEngine::new(&filename)?;

    // Skips from project.
    let mut skip_ranges: Vec<SkipRange> = project.skip_ranges.clone();

    logger::log(&format!("Active skip sections: {}", skip_ranges.len()));
    for range in &skip_ranges {
        logger::log(&format!(
            "Skip {:.2}s -> {:.2}s",
            range.start,
            range.end
        ));
    }

    let engine_ranges: Vec<(f64, f64)> = skip_ranges
        .iter()
        .map(|r| (r.start, r.end))
        .collect();

    engine.set_skip_ranges(&engine_ranges);

    logger::log(&format!("Audio duration: {:.2}s", engine.duration()));

    // Automatically start playing audio.
    engine.play();

    let mut pending_skip_start: Option<f64> = None;
    let mut cycle_skip_idx: Option<usize> = None;
    let mut last_active_skip_idx: Option<usize> = None;

    let _terminal = TerminalGuard::new(&log_path_str)?;

    // Keyboard event loop.
    loop {

        if event::poll(Duration::from_millis(50))? {

            if let Event::Key(key) = event::read()? {

                // Ignore key release and repeat events.
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    break;
                }

                if let Some(&action) = config.keybindings.get(&key.code) {
                    match action {

                        // PLAY / PAUSE
                        Action::PlayPause => {

                            let playing = {
                                let state = engine.state.lock().unwrap();
                                state.playing
                            };

                            if playing {
                                engine.pause();
                            } else {
                                engine.resume();
                            }
                        }

                        // NORMAL SPEED
                        Action::NormalSpeed => {

                            engine.normal_speed();

                            print_status("Normal speed (1.0x)");
                        }

                        // SPEED UP
                        Action::SpeedUp => {

                            let new_speed = engine.change_speed(config.settings.speed_step);

                            print_status(&format!("Speed: {:.1}x", new_speed));
                        }

                        // SPEED DOWN
                        Action::SpeedDown => {

                            let new_speed = engine.change_speed(-config.settings.speed_step);

                            print_status(&format!("Speed: {:.1}x", new_speed));
                        }

                        // FAST FORWARD
                        Action::FastForward => {

                            engine.set_speed(config.settings.fast_forward_speed);

                            print_status(&format!(
                                "Fast forward {:.1}x",
                                config.settings.fast_forward_speed
                            ));
                        }

                        // REVERSE
                        Action::Reverse => {

                            engine.set_speed(config.settings.reverse_speed);

                            print_status(&format!(
                                "Reverse playback ({:.1}x)",
                                config.settings.reverse_speed
                            ));
                        }

                        // SEEK BACKWARD
                        Action::SeekBackward => {

                            let position =
                                engine.seek_backward(config.settings.seek_step_seconds);

                            print_status(&format_time(position));
                        }

                        // SEEK FORWARD
                        Action::SeekForward => {

                            let position =
                                engine.seek_forward(config.settings.seek_step_seconds);

                            print_status(&format_time(position));
                        }

                        // BEGINNING
                        Action::SeekBeginning => {
                            cycle_skip_idx = None;
                            last_active_skip_idx = None;

                            engine.seek(0.0);

                            print_status("Beginning");
                        }

                        // END
                        Action::SeekEnd => {

                            let duration = engine.duration();

                            engine.seek(duration);

                            engine.pause();

                            print_status("End of audio");
                        }

                        // DISPLAY POSITION
                        Action::DisplayPosition => {

                            print_status(&format!(
                                "{} / {}",
                                format_time(engine.position()),
                                format_time(engine.duration())
                            ));
                        }

                        // DISPLAY TIMESTAMP IN WORDS (KEY 'W')
                        Action::DisplayTimestampWords => {
                            let pos = engine.position();
                            let formatted = format_time_words(pos);
                            logger::log(&format!("Display timestamp in words: {}", formatted));
                            print_status(&formatted);
                        }

                        // MARK SKIP START
                        Action::MarkSkipStart => {

                            let pos = engine.position();
                            pending_skip_start = Some(pos);

                            logger::log(&format!(
                                "Mark skip start: {:.2}s ({})",
                                pos,
                                format_time(pos)
                            ));

                            print_status(&format_time(pos));
                        }

                        // MARK SKIP END
                        Action::MarkSkipEnd => {

                            let pos = engine.position();

                            if let Some(start) = pending_skip_start.take() {
                                let (s, e) = if start <= pos {
                                    (start, pos)
                                } else {
                                    (pos, start)
                                };

                                if e > s {
                                    skip_ranges.push(SkipRange { start: s, end: e });
                                    skip_ranges = merge_skip_ranges(skip_ranges);

                                    let engine_ranges: Vec<(f64, f64)> = skip_ranges
                                        .iter()
                                        .map(|r| (r.start, r.end))
                                        .collect();

                                    engine.set_skip_ranges(&engine_ranges);

                                    logger::log(&format!(
                                        "Mark skip end: {:.2}s ({}), new section: {} -> {}",
                                        pos,
                                        format_time(pos),
                                        format_time(s),
                                        format_time(e)
                                    ));

                                    if let Err(err) = project.save_skips(&skip_ranges) {
                                        logger::log(&format!("Failed to update project skips: {}", err));
                                    }

                                    print_status(&format!("{} -> {}", format_time(s), format_time(e)));
                                }
                            }
                        }

                        // SKIP FROM BEGINNING (CAPITAL 'B')
                        Action::SkipFromBeginning => {
                            let pos = engine.position();
                            let (s, e) = (0.0, pos);

                            if e > s {
                                skip_ranges.push(SkipRange { start: s, end: e });
                                skip_ranges = merge_skip_ranges(skip_ranges);

                                let engine_ranges: Vec<(f64, f64)> = skip_ranges
                                    .iter()
                                    .map(|r| (r.start, r.end))
                                    .collect();

                                engine.set_skip_ranges(&engine_ranges);

                                logger::log(&format!(
                                    "Skip from beginning: 0.00s ({}) -> {:.2}s ({})",
                                    format_time(s),
                                    e,
                                    format_time(e)
                                ));

                                if let Err(err) = project.save_skips(&skip_ranges) {
                                    logger::log(&format!("Failed to update project skips: {}", err));
                                }

                                print_status(&format!("{} -> {}", format_time(s), format_time(e)));
                            }
                        }

                        // SKIP TO END (CAPITAL 'E')
                        Action::SkipToEnd => {
                            let pos = engine.position();
                            let duration = engine.duration();
                            let (s, e) = (pos, duration);

                            if e > s {
                                skip_ranges.push(SkipRange { start: s, end: e });
                                skip_ranges = merge_skip_ranges(skip_ranges);

                                let engine_ranges: Vec<(f64, f64)> = skip_ranges
                                    .iter()
                                    .map(|r| (r.start, r.end))
                                    .collect();

                                engine.set_skip_ranges(&engine_ranges);

                                logger::log(&format!(
                                    "Skip to end: {:.2}s ({}) -> {:.2}s ({})",
                                    s,
                                    format_time(s),
                                    e,
                                    format_time(e)
                                ));

                                if let Err(err) = project.save_skips(&skip_ranges) {
                                    logger::log(&format!("Failed to update project skips: {}", err));
                                }

                                print_status(&format!("{} -> {}", format_time(s), format_time(e)));
                            }
                        }

                        // TOGGLE EDITING MODE (CAPITAL 'A')
                        Action::ToggleEditingMode => {
                            let enabled = engine.toggle_editing_mode();
                            if enabled {
                                logger::log("Editing mode enabled: skips will not be skipped, boundary tones active.");
                                print_status("Editing mode: ON");
                            } else {
                                logger::log("Editing mode disabled: skips will be skipped normally.");
                                print_status("Editing mode: OFF");
                            }
                        }

                        // DELETE SKIP (KEY 'D' - ONLY IN EDITING MODE)
                        Action::DeleteSkip => {
                            if !engine.is_editing_mode() {
                                logger::log("Delete skip ignored: not in editing mode.");
                                print_status("Not in edit mode");
                            } else {
                                let pos = engine.position();
                                if let Some(removed) = delete_skip_at(pos, &mut skip_ranges) {
                                    cycle_skip_idx = None;
                                    last_active_skip_idx = None;

                                    let engine_ranges: Vec<(f64, f64)> = skip_ranges
                                        .iter()
                                        .map(|r| (r.start, r.end))
                                        .collect();
                                    engine.set_skip_ranges(&engine_ranges);

                                    logger::log(&format!(
                                        "Deleted skip: {} -> {} ({:.2}s - {:.2}s)",
                                        format_time(removed.start),
                                        format_time(removed.end),
                                        removed.start,
                                        removed.end
                                    ));

                                    if let Err(err) = project.save_skips(&skip_ranges) {
                                        logger::log(&format!("Failed to update project skips after deletion: {}", err));
                                    }

                                    print_status(&format!(
                                        "Deleted: {} -> {}",
                                        format_time(removed.start),
                                        format_time(removed.end)
                                    ));
                                } else {
                                    logger::log(&format!(
                                        "Delete skip: position {} ({:.2}s) is not inside any skip.",
                                        format_time(pos),
                                        pos
                                    ));
                                    print_status("Not inside a skip");
                                }
                            }
                        }

                        // CYCLE SKIPS (CAPITAL 'C')
                        Action::CycleSkips => {
                            let sorted_skips = merge_skip_ranges(skip_ranges.clone());
                            match calculate_cycle_skip_target(cycle_skip_idx, &sorted_skips) {
                                None => {
                                    logger::log("Cycle skips: no skips defined.");
                                    print_status("No skips");
                                }
                                Some((next_idx, target)) => {
                                    cycle_skip_idx = Some(next_idx);
                                    last_active_skip_idx = Some(next_idx);
                                    engine.seek(target);

                                    let skip = &sorted_skips[next_idx];
                                    logger::log(&format!(
                                        "Cycle to skip {}/{} at {} (jumped to {:.2}s / {})",
                                        next_idx + 1,
                                        sorted_skips.len(),
                                        format_time(skip.start),
                                        target,
                                        format_time(target)
                                    ));

                                    print_status(&format!(
                                        "Skip {}/{}: {}",
                                        next_idx + 1,
                                        sorted_skips.len(),
                                        format_time(target)
                                    ));
                                }
                            }
                        }

                        // NUDGE SKIP BOUNDARIES (KEYS 'n', 'm', 'N', 'M' - ONLY IN EDITING MODE)
                        Action::NudgeStartEarlier
                        | Action::NudgeStartLater
                        | Action::NudgeEndEarlier
                        | Action::NudgeEndLater => {
                            if !engine.is_editing_mode() {
                                logger::log("Nudge skip ignored: not in editing mode.");
                                print_status("Not in edit mode");
                            } else if skip_ranges.is_empty() {
                                logger::log("Nudge skip: no skips defined.");
                                print_status("No skips");
                            } else {
                                let pos = engine.position();
                                if let Some(idx) = find_current_skip_idx(pos, last_active_skip_idx, &skip_ranges) {
                                    let nudge_type = match action {
                                        Action::NudgeStartEarlier => NudgeType::StartEarlier,
                                        Action::NudgeStartLater => NudgeType::StartLater,
                                        Action::NudgeEndEarlier => NudgeType::EndEarlier,
                                        Action::NudgeEndLater => NudgeType::EndLater,
                                        _ => unreachable!(),
                                    };

                                    if let Some((updated_skip, target)) = nudge_skip(nudge_type, idx, &mut skip_ranges, engine.duration()) {
                                        last_active_skip_idx = Some(idx);
                                        cycle_skip_idx = Some(idx);

                                        let engine_ranges: Vec<(f64, f64)> = skip_ranges
                                            .iter()
                                            .map(|r| (r.start, r.end))
                                            .collect();
                                        engine.set_skip_ranges(&engine_ranges);

                                        if let Err(err) = project.save_skips(&skip_ranges) {
                                            logger::log(&format!("Failed to save skips after nudge: {}", err));
                                        }

                                        engine.seek(target);

                                        let label = match nudge_type {
                                            NudgeType::StartEarlier => "Nudge start earlier (-0.25s)",
                                            NudgeType::StartLater => "Nudge start later (+0.25s)",
                                            NudgeType::EndEarlier => "Nudge end earlier (-0.25s)",
                                            NudgeType::EndLater => "Nudge end later (+0.25s)",
                                        };

                                        logger::log(&format!(
                                            "{}: skip {}/{} is now {:.2}s -> {:.2}s, seek to {:.2}s ({})",
                                            label,
                                            idx + 1,
                                            skip_ranges.len(),
                                            updated_skip.start,
                                            updated_skip.end,
                                            target,
                                            format_time(target)
                                        ));

                                        print_status(&format!(
                                            "{} -> {}",
                                            format_time(updated_skip.start),
                                            format_time(updated_skip.end)
                                        ));
                                    }
                                } else {
                                    logger::log(&format!(
                                        "Nudge skip: position {:.2}s ({}) is not near any skip.",
                                        pos,
                                        format_time(pos)
                                    ));
                                    print_status("Not inside a skip");
                                }
                            }
                        }

                        // EXPORT (LETTER 'O')
                        Action::Export => {

                            engine.pause();
                            engine.pause_stream();

                            let _ = project.save_skips(&skip_ranges);

                            let output_path = project.get_output_filename(&config.settings.output_format);

                            print_status(&format!("Exporting {}...", output_path));
                            logger::log(&format!("Starting FFmpeg export to '{}'...", output_path));

                            match export_non_skipped(
                                &filename,
                                &output_path,
                                engine.duration(),
                                &skip_ranges,
                            ) {
                                Ok(saved) => {
                                    logger::log(&format!("FFmpeg export succeeded: {}", saved));
                                    print_status(&format!("Saved {}", saved));
                                }
                                Err(err) => {
                                    logger::log(&format!("FFmpeg export failed: {}", err));
                                    print_status("Export failed");
                                }
                            }

                            // Discard any keystrokes pressed while the export was in progress
                            while event::poll(Duration::from_millis(0)).unwrap_or(false) {
                                let _ = event::read();
                            }

                            // Stop playback and quit after export
                            break;
                        }

                        // QUIT
                        Action::Quit => {

                            break;
                        }
                    }
                }
            }
        }
    }

    // Update project archive with latest skips on exit
    if let Err(err) = project.save_skips(&skip_ranges) {
        logger::log(&format!("Failed to save project skips on exit: {}", err));
    } else {
        logger::log(&format!(
            "Updated {} skip section(s) in '{}'",
            skip_ranges.len(),
            cme_path_display
        ));
    }

    // Engine drops after terminal mode is restored.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_time() {
        assert_eq!(format_time(0.0), "00:00:00");
        assert_eq!(format_time(5.0), "00:00:05");
        assert_eq!(format_time(59.0), "00:00:59");
        assert_eq!(format_time(60.0), "00:01:00");
        assert_eq!(format_time(65.0), "00:01:05");
        assert_eq!(format_time(3599.0), "00:59:59");
        assert_eq!(format_time(3600.0), "01:00:00");
        assert_eq!(format_time(3661.0), "01:01:01");
        assert_eq!(format_time(484.74), "00:08:04");
        assert_eq!(format_time(-10.0), "00:00:00");
    }

    #[test]
    fn test_calculate_cycle_skip_target() {
        // Empty skips returns None
        assert_eq!(calculate_cycle_skip_target(None, &[]), None);

        // Skips where first skip starts at 0.0 (or <= 2.0s)
        let skips = vec![
            SkipRange { start: 0.0, end: 5.0 },
            SkipRange { start: 10.0, end: 15.0 },
            SkipRange { start: 25.0, end: 30.0 },
        ];

        // 1st press: None -> 1st skip (idx 0), starts at 0.0 -> target 0.0
        let (idx1, target1) = calculate_cycle_skip_target(None, &skips).unwrap();
        assert_eq!(idx1, 0);
        assert_eq!(target1, 0.0);

        // 2nd press: Some(0) -> 2nd skip (idx 1), starts at 10.0 -> target 8.0 (2 seconds before)
        let (idx2, target2) = calculate_cycle_skip_target(Some(idx1), &skips).unwrap();
        assert_eq!(idx2, 1);
        assert_eq!(target2, 8.0);

        // 3rd press: Some(1) -> 3rd skip (idx 2), starts at 25.0 -> target 23.0 (2 seconds before)
        let (idx3, target3) = calculate_cycle_skip_target(Some(idx2), &skips).unwrap();
        assert_eq!(idx3, 2);
        assert_eq!(target3, 23.0);

        // 4th press: Some(2) -> cycles back to 1st skip (idx 0), target 0.0
        let (idx4, target4) = calculate_cycle_skip_target(Some(idx3), &skips).unwrap();
        assert_eq!(idx4, 0);
        assert_eq!(target4, 0.0);

        // Skips where first skip starts at e.g. 1.5s (within 2 seconds of beginning)
        let skips_close = vec![
            SkipRange { start: 1.5, end: 5.0 },
        ];
        let (idx_c, target_c) = calculate_cycle_skip_target(None, &skips_close).unwrap();
        assert_eq!(idx_c, 0);
        assert_eq!(target_c, 0.0);

        // Skips where first skip starts after 2.0s (e.g. 8.0s)
        let skips_offset = vec![
            SkipRange { start: 8.0, end: 12.0 },
        ];
        let (idx_o, target_o) = calculate_cycle_skip_target(None, &skips_offset).unwrap();
        assert_eq!(idx_o, 0);
        assert_eq!(target_o, 6.0);

        // Single skip cycling cycles back to itself
        let (idx_o2, target_o2) = calculate_cycle_skip_target(Some(0), &skips_offset).unwrap();
        assert_eq!(idx_o2, 0);
        assert_eq!(target_o2, 6.0);
    }

    #[test]
    fn test_format_time_words() {
        // Less than an hour: hours omitted
        assert_eq!(format_time_words(0.0), "0 minutes 0 seconds");
        assert_eq!(format_time_words(1.0), "0 minutes 1 second");
        assert_eq!(format_time_words(5.0), "0 minutes 5 seconds");
        assert_eq!(format_time_words(59.0), "0 minutes 59 seconds");
        assert_eq!(format_time_words(60.0), "1 minute 0 seconds");
        assert_eq!(format_time_words(61.0), "1 minute 1 second");
        assert_eq!(format_time_words(125.0), "2 minutes 5 seconds");
        assert_eq!(format_time_words(3599.0), "59 minutes 59 seconds");

        // 1 hour or more: hours included
        assert_eq!(format_time_words(3600.0), "1 hour 0 minutes 0 seconds");
        assert_eq!(format_time_words(3601.0), "1 hour 0 minutes 1 second");
        assert_eq!(format_time_words(3661.0), "1 hour 1 minute 1 second");
        assert_eq!(format_time_words(7200.0), "2 hours 0 minutes 0 seconds");
        assert_eq!(format_time_words(7325.0), "2 hours 2 minutes 5 seconds");
        assert_eq!(format_time_words(-5.0), "0 minutes 0 seconds");
    }

    #[test]
    fn test_delete_skip_at() {
        let mut skips = vec![
            SkipRange { start: 10.0, end: 20.0 },
            SkipRange { start: 30.0, end: 40.0 },
        ];

        // Before first skip: not found
        assert_eq!(delete_skip_at(5.0, &mut skips), None);
        assert_eq!(skips.len(), 2);

        // Inside first skip (at start boundary): deleted
        assert_eq!(
            delete_skip_at(10.0, &mut skips),
            Some(SkipRange { start: 10.0, end: 20.0 })
        );
        assert_eq!(skips.len(), 1);
        assert_eq!(skips[0].start, 30.0);

        // Between skips: not found
        assert_eq!(delete_skip_at(25.0, &mut skips), None);
        assert_eq!(skips.len(), 1);

        // Inside remaining skip (at end boundary): deleted
        assert_eq!(
            delete_skip_at(40.0, &mut skips),
            Some(SkipRange { start: 30.0, end: 40.0 })
        );
        assert_eq!(skips.len(), 0);

        // On empty list: not found
        assert_eq!(delete_skip_at(35.0, &mut skips), None);
    }

    #[test]
    fn test_find_current_skip_idx() {
        let skips = vec![
            SkipRange { start: 10.0, end: 20.0 },
            SkipRange { start: 30.0, end: 40.0 },
        ];

        // Empty skips
        assert_eq!(find_current_skip_idx(15.0, None, &[]), None);

        // Position strictly inside skip 0
        assert_eq!(find_current_skip_idx(15.0, None, &skips), Some(0));

        // Position strictly inside skip 1
        assert_eq!(find_current_skip_idx(35.0, None, &skips), Some(1));

        // Position 2s before skip 0 (e.g. pre-roll at 8.0s)
        assert_eq!(find_current_skip_idx(8.0, None, &skips), Some(0));

        // Position 2s before skip 1 (e.g. pre-roll at 28.0s)
        assert_eq!(find_current_skip_idx(28.0, None, &skips), Some(1));

        // Position far away with last_idx pointing to skip 0
        assert_eq!(find_current_skip_idx(100.0, Some(0), &skips), Some(0));
    }

    #[test]
    fn test_nudge_skip() {
        let duration = 100.0;
        let mut skips = vec![
            SkipRange { start: 10.0, end: 20.0 },
            SkipRange { start: 30.0, end: 40.0 },
        ];

        // 1. Nudge start earlier ('n'): 10.0 -> 9.75, seek target = 9.75 - 2.0 = 7.75
        let (s, target) = nudge_skip(NudgeType::StartEarlier, 0, &mut skips, duration).unwrap();
        assert_eq!(s.start, 9.75);
        assert_eq!(s.end, 20.0);
        assert_eq!(target, 7.75);

        // 2. Nudge start later ('m'): 9.75 -> 10.0, seek target = 10.0 - 2.0 = 8.0
        let (s, target) = nudge_skip(NudgeType::StartLater, 0, &mut skips, duration).unwrap();
        assert_eq!(s.start, 10.0);
        assert_eq!(s.end, 20.0);
        assert_eq!(target, 8.0);

        // 3. Nudge end earlier ('N'): 20.0 -> 19.75, seek target = 19.75 - 2.0 = 17.75
        let (s, target) = nudge_skip(NudgeType::EndEarlier, 0, &mut skips, duration).unwrap();
        assert_eq!(s.start, 10.0);
        assert_eq!(s.end, 19.75);
        assert_eq!(target, 17.75);

        // 4. Nudge end later ('M'): 19.75 -> 20.0, seek target = 20.0 - 2.0 = 18.0
        let (s, target) = nudge_skip(NudgeType::EndLater, 0, &mut skips, duration).unwrap();
        assert_eq!(s.start, 10.0);
        assert_eq!(s.end, 20.0);
        assert_eq!(target, 18.0);

        // Boundary test: start near 0 (<= 2.0s) -> target jumps to 0.0
        let mut skips_close = vec![SkipRange { start: 0.5, end: 5.0 }];
        let (s, target) = nudge_skip(NudgeType::StartEarlier, 0, &mut skips_close, duration).unwrap();
        assert_eq!(s.start, 0.25);
        assert_eq!(target, 0.0);

        // Boundary test: end near 0 (<= 2.0s) -> target jumps to 0.0
        let mut skips_end_close = vec![SkipRange { start: 0.1, end: 1.5 }];
        let (s, target) = nudge_skip(NudgeType::EndEarlier, 0, &mut skips_end_close, duration).unwrap();
        assert_eq!(s.end, 1.25);
        assert_eq!(target, 0.0);
    }
}
