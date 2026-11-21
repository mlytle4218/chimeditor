use crossterm::event::KeyCode;
use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    PlayPause,
    NormalSpeed,
    SpeedUp,
    SpeedDown,
    FastForward,
    Reverse,
    SeekBackward,
    SeekForward,
    SeekBeginning,
    SeekEnd,
    DisplayPosition,
    DisplayTimestampWords,
    MarkSkipStart,
    MarkSkipEnd,
    SkipFromBeginning,
    SkipToEnd,
    ToggleEditingMode,
    DeleteSkip,
    CycleSkips,
    NudgeStartEarlier,
    NudgeStartLater,
    NudgeEndEarlier,
    NudgeEndLater,
    Export,
    Quit,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub log_file: String,
    pub output_format: String,
    pub speed_step: f64,
    pub seek_step_seconds: f64,
    pub fast_forward_speed: f64,
    pub reverse_speed: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            log_file: "chimeditor.log".to_string(),
            output_format: "mp4".to_string(),
            speed_step: 0.5,
            seek_step_seconds: 5.0,
            fast_forward_speed: 2.0,
            reverse_speed: -1.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub keybindings: HashMap<KeyCode, Action>,
    pub settings: Settings,
    pub key_names: HashMap<Action, Vec<String>>,
}

impl Default for Config {
    fn default() -> Self {
        let mut keybindings = HashMap::new();
        let mut key_names = HashMap::new();

        let defaults = vec![
            (Action::PlayPause, vec!["SPACE".to_string()]),
            (Action::NormalSpeed, vec!["Z".to_string()]),
            (Action::SpeedUp, vec!["UP".to_string()]),
            (Action::SpeedDown, vec!["DOWN".to_string()]),
            (Action::FastForward, vec!["F".to_string()]),
            (Action::Reverse, vec!["R".to_string()]),
            (Action::SeekBackward, vec!["LEFT".to_string()]),
            (Action::SeekForward, vec!["RIGHT".to_string()]),
            (Action::SeekBeginning, vec!["HOME".to_string()]),
            (Action::SeekEnd, vec!["END".to_string()]),
            (Action::DisplayPosition, vec!["P".to_string()]),
            (Action::DisplayTimestampWords, vec!["W".to_string()]),
            (Action::MarkSkipStart, vec!["b".to_string()]),
            (Action::SkipFromBeginning, vec!["B".to_string()]),
            (Action::MarkSkipEnd, vec!["e".to_string()]),
            (Action::SkipToEnd, vec!["E".to_string()]),
            (Action::ToggleEditingMode, vec!["A".to_string()]),
            (Action::DeleteSkip, vec!["d".to_string(), "D".to_string()]),
            (Action::CycleSkips, vec!["C".to_string()]),
            (Action::NudgeStartEarlier, vec!["n".to_string()]),
            (Action::NudgeStartLater, vec!["m".to_string()]),
            (Action::NudgeEndEarlier, vec!["N".to_string()]),
            (Action::NudgeEndLater, vec!["M".to_string()]),
            (Action::Export, vec!["O".to_string()]),
            (Action::Quit, vec!["Q".to_string(), "ESC".to_string()]),
        ];

        for (action, keys) in defaults {
            for key_str in &keys {
                if let Some(code) = parse_key_code(key_str) {
                    keybindings.insert(code, action);
                    // Also bind uppercase/lowercase for single alphabetic characters,
                    // except where case differentiates actions (e.g. 'b'/'B', 'e'/'E', 'a'/'A', 'c'/'C').
                    if let KeyCode::Char(c) = code {
                        if c.is_ascii_alphabetic()
                            && c != 'b'
                            && c != 'B'
                            && c != 'e'
                            && c != 'E'
                            && c != 'a'
                            && c != 'A'
                            && c != 'c'
                            && c != 'C'
                            && c != 'n'
                            && c != 'N'
                            && c != 'm'
                            && c != 'M'
                        {
                            keybindings.insert(KeyCode::Char(c.to_ascii_lowercase()), action);
                            keybindings.insert(KeyCode::Char(c.to_ascii_uppercase()), action);
                        }
                    }
                }
            }
            key_names.insert(action, keys);
        }

        Self {
            keybindings,
            settings: Settings::default(),
            key_names,
        }
    }
}

pub fn parse_key_code(s: &str) -> Option<KeyCode> {
    let trimmed = s.trim().trim_matches(|c| c == '\'' || c == '"').trim();
    let lower = trimmed.to_lowercase();
    match lower.as_str() {
        "space" | "spacebar" | " " => Some(KeyCode::Char(' ')),
        "left" | "arrowleft" | "arrow_left" => Some(KeyCode::Left),
        "right" | "arrowright" | "arrow_right" => Some(KeyCode::Right),
        "up" | "arrowup" | "arrow_up" => Some(KeyCode::Up),
        "down" | "arrowdown" | "arrow_down" => Some(KeyCode::Down),
        "home" => Some(KeyCode::Home),
        "end" => Some(KeyCode::End),
        "esc" | "escape" => Some(KeyCode::Esc),
        "enter" | "return" => Some(KeyCode::Enter),
        "tab" => Some(KeyCode::Tab),
        "backspace" => Some(KeyCode::Backspace),
        "pageup" | "page_up" | "pgup" => Some(KeyCode::PageUp),
        "pagedown" | "page_down" | "pgdown" => Some(KeyCode::PageDown),
        _ => {
            if trimmed.len() == 1 {
                Some(KeyCode::Char(trimmed.chars().next().unwrap()))
            } else if lower.starts_with('f') {
                if let Ok(num) = lower[1..].parse::<u8>() {
                    Some(KeyCode::F(num))
                } else {
                    None
                }
            } else {
                None
            }
        }
    }
}

pub fn parse_action(name: &str) -> Option<Action> {
    let normalized = name.trim().to_lowercase();
    match normalized.as_str() {
        "play_pause" | "play" | "pause" => Some(Action::PlayPause),
        "normal_speed" | "normal" | "reset_speed" => Some(Action::NormalSpeed),
        "speed_up" | "faster" | "increase_speed" => Some(Action::SpeedUp),
        "speed_down" | "slower" | "decrease_speed" => Some(Action::SpeedDown),
        "fast_forward" | "forward" | "ff" => Some(Action::FastForward),
        "reverse" | "rewind" | "rev" => Some(Action::Reverse),
        "seek_backward" | "seek_back" | "backward" => Some(Action::SeekBackward),
        "seek_forward" => Some(Action::SeekForward),
        "seek_beginning" | "beginning" | "start" => Some(Action::SeekBeginning),
        "seek_end" | "end" => Some(Action::SeekEnd),
        "display_position" | "position" => Some(Action::DisplayPosition),
        "display_timestamp_words"
        | "display_position_words"
        | "timestamp_words"
        | "words_position"
        | "words" => Some(Action::DisplayTimestampWords),
        "mark_skip_start" | "skip_start" | "mark_start" | "skip_beginning" => {
            Some(Action::MarkSkipStart)
        }
        "mark_skip_end" | "skip_end" | "mark_end" => Some(Action::MarkSkipEnd),
        "skip_from_beginning"
        | "skip_from_start"
        | "skip_beginning_to_current"
        | "skip_start_to_current"
        | "skip_to_here" => Some(Action::SkipFromBeginning),
        "skip_to_end"
        | "skip_current_to_end"
        | "skip_from_here"
        | "skip_end_from_current" => Some(Action::SkipToEnd),
        "toggle_editing_mode"
        | "editing_mode"
        | "edit_mode"
        | "toggle_edit_mode" => Some(Action::ToggleEditingMode),
        "delete_skip"
        | "delete"
        | "remove_skip"
        | "del_skip" => Some(Action::DeleteSkip),
        "cycle_skips"
        | "cycle_skip"
        | "next_skip"
        | "jump_skip"
        | "preview_skips" => Some(Action::CycleSkips),
        "nudge_start_earlier"
        | "nudge_start_prev"
        | "nudge_beginning_earlier"
        | "nudge_start_left" => Some(Action::NudgeStartEarlier),
        "nudge_start_later"
        | "nudge_start_next"
        | "nudge_beginning_later"
        | "nudge_start_right" => Some(Action::NudgeStartLater),
        "nudge_end_earlier"
        | "nudge_end_prev"
        | "nudge_end_left" => Some(Action::NudgeEndEarlier),
        "nudge_end_later"
        | "nudge_end_next"
        | "nudge_end_right" => Some(Action::NudgeEndLater),
        "export" | "output" | "render" | "save_output" => Some(Action::Export),
        "quit" | "exit" => Some(Action::Quit),
        _ => None,
    }
}

pub fn parse_config(contents: &str) -> Result<Config, Box<dyn Error>> {
    let mut config = Config::default();

    #[derive(PartialEq)]
    enum Section {
        None,
        Keybindings,
        Settings,
    }

    let mut current_section = Section::None;
    let mut current_action: Option<Action> = None;
    let mut custom_keys: HashMap<Action, Vec<String>> = HashMap::new();

    for line in contents.lines() {
        let trimmed = line.trim();

        // Skip comments and blank lines
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // Section header
        if trimmed == "keybindings:" {
            current_section = Section::Keybindings;
            current_action = None;
            continue;
        } else if trimmed == "settings:" {
            current_section = Section::Settings;
            current_action = None;
            continue;
        }

        match current_section {
            Section::Keybindings => {
                if trimmed.starts_with("- ") {
                    // List item: key for the current action
                    if let Some(action) = current_action {
                        let raw_key = trimmed[2..].trim();
                        let clean_key = raw_key.trim_matches(|c| c == '\'' || c == '"').trim();
                        if let Some(code) = parse_key_code(clean_key) {
                            config.keybindings.insert(code, action);
                            if let KeyCode::Char(c) = code {
                                if c.is_ascii_alphabetic()
                                    && c != 'b'
                                    && c != 'B'
                                    && c != 'e'
                                    && c != 'E'
                                    && c != 'a'
                                    && c != 'A'
                                    && c != 'c'
                                    && c != 'C'
                                    && c != 'n'
                                    && c != 'N'
                                    && c != 'm'
                                    && c != 'M'
                                {
                                    config.keybindings.insert(KeyCode::Char(c.to_ascii_lowercase()), action);
                                    config.keybindings.insert(KeyCode::Char(c.to_ascii_uppercase()), action);
                                }
                            }
                            custom_keys.entry(action).or_default().push(clean_key.to_string());
                        }
                    }
                } else if let Some((key_name, val)) = trimmed.split_once(':') {
                    let action_str = key_name.trim();
                    if let Some(action) = parse_action(action_str) {
                        current_action = Some(action);
                        let val_trimmed = val.trim();
                        // Support inline list: ["key1", "key2"]
                        if val_trimmed.starts_with('[') && val_trimmed.ends_with(']') {
                            let inner = &val_trimmed[1..val_trimmed.len() - 1];
                            for item in inner.split(',') {
                                let clean = item.trim().trim_matches(|c| c == '\'' || c == '"').trim();
                                if let Some(code) = parse_key_code(clean) {
                                    config.keybindings.insert(code, action);
                                    if let KeyCode::Char(c) = code {
                                        if c.is_ascii_alphabetic()
                                            && c != 'b'
                                            && c != 'B'
                                            && c != 'e'
                                            && c != 'E'
                                            && c != 'a'
                                            && c != 'A'
                                            && c != 'c'
                                            && c != 'C'
                                            && c != 'n'
                                            && c != 'N'
                                            && c != 'm'
                                            && c != 'M'
                                        {
                                            config.keybindings.insert(KeyCode::Char(c.to_ascii_lowercase()), action);
                                            config.keybindings.insert(KeyCode::Char(c.to_ascii_uppercase()), action);
                                        }
                                    }
                                    custom_keys.entry(action).or_default().push(clean.to_string());
                                }
                            }
                        } else if !val_trimmed.is_empty() {
                            // Single value on same line: action: key
                            let clean = val_trimmed.trim_matches(|c| c == '\'' || c == '"').trim();
                            if let Some(code) = parse_key_code(clean) {
                                config.keybindings.insert(code, action);
                                if let KeyCode::Char(c) = code {
                                    if c.is_ascii_alphabetic()
                                        && c != 'b'
                                        && c != 'B'
                                        && c != 'e'
                                        && c != 'E'
                                        && c != 'a'
                                        && c != 'A'
                                        && c != 'c'
                                        && c != 'C'
                                        && c != 'n'
                                        && c != 'N'
                                        && c != 'm'
                                        && c != 'M'
                                    {
                                        config.keybindings.insert(KeyCode::Char(c.to_ascii_lowercase()), action);
                                        config.keybindings.insert(KeyCode::Char(c.to_ascii_uppercase()), action);
                                    }
                                }
                                custom_keys.entry(action).or_default().push(clean.to_string());
                            }
                        }
                    }
                }
            }
            Section::Settings => {
                if let Some((k, v)) = trimmed.split_once(':') {
                    let key = k.trim();
                    let val = v.trim().split('#').next().unwrap_or("").trim();
                    match key {
                        "log_file" => {
                            let clean = val.trim_matches(|c| c == '\'' || c == '"').trim();
                            if !clean.is_empty() {
                                config.settings.log_file = clean.to_string();
                            }
                        }
                        "output_format" | "output_extension" | "export_format" => {
                            let clean = val
                                .trim_matches(|c| c == '\'' || c == '"')
                                .trim()
                                .trim_start_matches('.');
                            if !clean.is_empty() {
                                config.settings.output_format = clean.to_lowercase();
                            }
                        }
                        "speed_step" => {
                            if let Ok(num) = val.parse::<f64>() {
                                config.settings.speed_step = num;
                            }
                        }
                        "seek_step_seconds" => {
                            if let Ok(num) = val.parse::<f64>() {
                                config.settings.seek_step_seconds = num;
                            }
                        }
                        "fast_forward_speed" => {
                            if let Ok(num) = val.parse::<f64>() {
                                config.settings.fast_forward_speed = num;
                            }
                        }
                        "reverse_speed" => {
                            if let Ok(num) = val.parse::<f64>() {
                                config.settings.reverse_speed = num;
                            }
                        }
                        _ => {}
                    }
                }
            }
            Section::None => {}
        }
    }

    // Update key names for display if custom keys were parsed
    for (action, keys) in custom_keys {
        if !keys.is_empty() {
            config.key_names.insert(action, keys);
        }
    }

    Ok(config)
}

/// Returns the path to the user's configuration directory (~/.config/shimeditor or $XDG_CONFIG_HOME/shimeditor).
pub fn get_user_config_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join("shimeditor");
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".config").join("shimeditor");
    }
    PathBuf::from(".config").join("shimeditor")
}

/// Returns the configuration file path to load.
/// Checks for a local "config.yaml" first; otherwise uses ~/.config/shimeditor/config.yaml.
pub fn get_config_file_path() -> PathBuf {
    let local = Path::new("config.yaml");
    if local.exists() {
        return local.to_path_buf();
    }
    get_user_config_dir().join("config.yaml")
}

/// Ensures that the user's config directory (~/.config/shimeditor) exists,
/// seeds the default config.yaml template if none exists, and returns the path.
pub fn ensure_user_config_exists() -> PathBuf {
    let config_path = get_config_file_path();
    if !config_path.exists() {
        if let Some(parent) = config_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let default_config = include_str!("../config.yaml");
        let _ = fs::write(&config_path, default_config);
    }
    config_path
}

/// Resolves the log file path, placing relative filenames into the user's config directory (~/.config/shimeditor).
pub fn get_log_file_path(configured_log: &str) -> PathBuf {
    let p = Path::new(configured_log);
    if p.is_absolute() {
        return p.to_path_buf();
    }
    let user_dir = get_user_config_dir();
    let _ = fs::create_dir_all(&user_dir);
    user_dir.join(configured_log)
}

impl Config {
    pub fn load_from_file(path: &str) -> Self {
        if !Path::new(path).exists() {
            return Self::default();
        }

        match fs::read_to_string(path) {
            Ok(contents) => match parse_config(&contents) {
                Ok(config) => config,
                Err(err) => {
                    eprintln!("Warning: Failed to parse '{}': {}. Using defaults.", path, err);
                    Self::default()
                }
            },
            Err(err) => {
                eprintln!("Warning: Could not read '{}': {}. Using defaults.", path, err);
                Self::default()
            }
        }
    }

    #[allow(dead_code)]
    pub fn print_controls(&self) {
        let action_display = [
            (Action::PlayPause, "Play / Pause"),
            (Action::NormalSpeed, "Normal speed (1.0x)"),
            (
                Action::SpeedUp,
                &format!("Increase speed (+{:.1}x)", self.settings.speed_step) as &str,
            ),
            (
                Action::SpeedDown,
                &format!("Decrease speed (-{:.1}x)", self.settings.speed_step) as &str,
            ),
            (
                Action::FastForward,
                &format!("Fast forward {:.1}x", self.settings.fast_forward_speed) as &str,
            ),
            (
                Action::Reverse,
                &format!("Reverse ({:.1}x)", self.settings.reverse_speed) as &str,
            ),
            (
                Action::SeekBackward,
                &format!("Seek backward {:.1} seconds", self.settings.seek_step_seconds) as &str,
            ),
            (
                Action::SeekForward,
                &format!("Seek forward {:.1} seconds", self.settings.seek_step_seconds) as &str,
            ),
            (Action::SeekBeginning, "Go to beginning"),
            (Action::SeekEnd, "Go to end"),
            (Action::DisplayPosition, "Display position (HH:MM:SS)"),
            (Action::DisplayTimestampWords, "Display position in words (e.g. 5 minutes 30 seconds)"),
            (Action::MarkSkipStart, "Mark beginning of skip"),
            (Action::SkipFromBeginning, "Skip from beginning to current position"),
            (Action::MarkSkipEnd, "Mark end of skip"),
            (Action::SkipToEnd, "Skip from current position to end"),
            (Action::ToggleEditingMode, "Toggle editing mode (audible skip tones)"),
            (Action::DeleteSkip, "Delete skip under playhead (in edit mode)"),
            (Action::CycleSkips, "Cycle to 2s before next skip"),
            (Action::NudgeStartEarlier, "Nudge beginning of edit 0.25s earlier (n)"),
            (Action::NudgeStartLater, "Nudge beginning of edit 0.25s later (m)"),
            (Action::NudgeEndEarlier, "Nudge end of edit 0.25s earlier (Shift+N)"),
            (Action::NudgeEndLater, "Nudge end of edit 0.25s later (Shift+M)"),
            (Action::Export, "Export non-skipped media (FFmpeg)"),
            (Action::Quit, "Quit"),
        ];

        println!("\nKEYBOARD CONTROLS");
        println!("-----------------");
        for (action, description) in action_display {
            let key_str = self
                .key_names
                .get(&action)
                .map(|k| k.join(", "))
                .unwrap_or_else(|| "-".to_string());
            println!("{:<14} {}", key_str, description);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_key_code() {
        assert_eq!(parse_key_code("space"), Some(KeyCode::Char(' ')));
        assert_eq!(parse_key_code("left"), Some(KeyCode::Left));
        assert_eq!(parse_key_code("right"), Some(KeyCode::Right));
        assert_eq!(parse_key_code("up"), Some(KeyCode::Up));
        assert_eq!(parse_key_code("down"), Some(KeyCode::Down));
        assert_eq!(parse_key_code("home"), Some(KeyCode::Home));
        assert_eq!(parse_key_code("end"), Some(KeyCode::End));
        assert_eq!(parse_key_code("esc"), Some(KeyCode::Esc));
        assert_eq!(parse_key_code("f"), Some(KeyCode::Char('f')));
        assert_eq!(parse_key_code("F"), Some(KeyCode::Char('F')));
        assert_eq!(parse_key_code("z"), Some(KeyCode::Char('z')));
        assert_eq!(parse_key_code("Z"), Some(KeyCode::Char('Z')));
        assert_eq!(parse_key_code("f5"), Some(KeyCode::F(5)));
    }

    #[test]
    fn test_parse_config() {
        let yaml = r#"
keybindings:
  play_pause:
    - "space"
  normal_speed:
    - "z"
  speed_up:
    - "up"
  speed_down:
    - "down"
  fast_forward:
    - "f"
    - "F"
  seek_backward:
    - "left"
  seek_forward:
    - "right"

settings:
  output_format: "mp4"
  speed_step: 0.5
  seek_step_seconds: 10.0
  fast_forward_speed: 3.0
  reverse_speed: -2.0
"#;
        let config = parse_config(yaml).unwrap();
        assert_eq!(config.settings.output_format, "mp4");
        assert_eq!(config.settings.speed_step, 0.5);
        assert_eq!(config.settings.seek_step_seconds, 10.0);
        assert_eq!(config.settings.fast_forward_speed, 3.0);
        assert_eq!(config.settings.reverse_speed, -2.0);
        assert_eq!(config.keybindings.get(&KeyCode::Char(' ')), Some(&Action::PlayPause));
        assert_eq!(config.keybindings.get(&KeyCode::Char('z')), Some(&Action::NormalSpeed));
        assert_eq!(config.keybindings.get(&KeyCode::Char('Z')), Some(&Action::NormalSpeed));
        assert_eq!(config.keybindings.get(&KeyCode::Up), Some(&Action::SpeedUp));
        assert_eq!(config.keybindings.get(&KeyCode::Down), Some(&Action::SpeedDown));
        assert_eq!(config.keybindings.get(&KeyCode::Left), Some(&Action::SeekBackward));
    }

    #[test]
    fn test_load_existing_config_yaml() {
        let config = Config::load_from_file("config.yaml");
        assert_eq!(config.settings.output_format, "mp4");
        assert_eq!(config.settings.speed_step, 0.5);
        assert_eq!(config.settings.seek_step_seconds, 5.0);
        assert_eq!(config.settings.fast_forward_speed, 2.0);
        assert_eq!(config.settings.reverse_speed, -1.0);
        assert_eq!(config.keybindings.get(&KeyCode::Char(' ')), Some(&Action::PlayPause));
        assert_eq!(config.keybindings.get(&KeyCode::Char('z')), Some(&Action::NormalSpeed));
        assert_eq!(config.keybindings.get(&KeyCode::Char('Z')), Some(&Action::NormalSpeed));
        assert_eq!(config.keybindings.get(&KeyCode::Up), Some(&Action::SpeedUp));
        assert_eq!(config.keybindings.get(&KeyCode::Down), Some(&Action::SpeedDown));
        assert_eq!(config.keybindings.get(&KeyCode::Left), Some(&Action::SeekBackward));
        assert_eq!(config.keybindings.get(&KeyCode::Right), Some(&Action::SeekForward));
        assert_eq!(config.keybindings.get(&KeyCode::Char('p')), Some(&Action::DisplayPosition));
        assert_eq!(config.keybindings.get(&KeyCode::Char('P')), Some(&Action::DisplayPosition));
        assert_eq!(config.keybindings.get(&KeyCode::Char('w')), Some(&Action::DisplayTimestampWords));
        assert_eq!(config.keybindings.get(&KeyCode::Char('W')), Some(&Action::DisplayTimestampWords));
        assert_eq!(config.keybindings.get(&KeyCode::Esc), Some(&Action::Quit));
        assert_eq!(config.keybindings.get(&KeyCode::Char('b')), Some(&Action::MarkSkipStart));
        assert_eq!(config.keybindings.get(&KeyCode::Char('B')), Some(&Action::SkipFromBeginning));
        assert_eq!(config.keybindings.get(&KeyCode::Char('e')), Some(&Action::MarkSkipEnd));
        assert_eq!(config.keybindings.get(&KeyCode::Char('E')), Some(&Action::SkipToEnd));
        assert_eq!(config.keybindings.get(&KeyCode::Char('A')), Some(&Action::ToggleEditingMode));
        assert_eq!(config.keybindings.get(&KeyCode::Char('a')), None);
        assert_eq!(config.keybindings.get(&KeyCode::Char('d')), Some(&Action::DeleteSkip));
        assert_eq!(config.keybindings.get(&KeyCode::Char('D')), Some(&Action::DeleteSkip));
        assert_eq!(config.keybindings.get(&KeyCode::Char('C')), Some(&Action::CycleSkips));
        assert_eq!(config.keybindings.get(&KeyCode::Char('c')), None);
        assert_eq!(config.keybindings.get(&KeyCode::Char('n')), Some(&Action::NudgeStartEarlier));
        assert_eq!(config.keybindings.get(&KeyCode::Char('m')), Some(&Action::NudgeStartLater));
        assert_eq!(config.keybindings.get(&KeyCode::Char('N')), Some(&Action::NudgeEndEarlier));
        assert_eq!(config.keybindings.get(&KeyCode::Char('M')), Some(&Action::NudgeEndLater));
        assert_eq!(config.keybindings.get(&KeyCode::Char('o')), Some(&Action::Export));
        assert_eq!(config.keybindings.get(&KeyCode::Char('O')), Some(&Action::Export));
    }

    #[test]
    fn test_shimeditor_config_and_log_paths() {
        let config_dir = get_user_config_dir();
        assert!(config_dir.to_string_lossy().contains("shimeditor"));

        let log_path = get_log_file_path("chimeditor.log");
        assert!(log_path.to_string_lossy().contains("shimeditor"));
        assert!(log_path.to_string_lossy().ends_with("chimeditor.log"));

        let abs_log = get_log_file_path("/tmp/test.log");
        assert_eq!(abs_log, PathBuf::from("/tmp/test.log"));
    }
}
