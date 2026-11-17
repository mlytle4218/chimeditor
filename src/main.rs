mod audio;
mod video;
mod timeline;
mod config;

use audio::AudioEngine;
use video::probe_video;
use timeline::load_skip_list;
use config::{Action, Config};

use crossterm::{
    event::{self, Event, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode},
};

use std::error::Error;
use std::time::Duration;

// Automatically restore terminal mode when exiting.
struct TerminalGuard;

impl TerminalGuard {
    fn new() -> Result<Self, Box<dyn Error>> {
        enable_raw_mode()?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        println!("\nChimEditor closed.");
    }
}

fn main() -> Result<(), Box<dyn Error>> {

    let mut args = std::env::args().skip(1);

    let filename = args
        .next()
        .unwrap_or_else(|| "input.mp4".to_string());

    let skip_filename = args.next();

    // Load configuration from config.yaml or fall back to defaults.
    let config = Config::load_from_file("config.yaml");

    println!("================================");
    println!("       CHIMEDITOR");
    println!("================================");

    // Inspect video.
    match probe_video(&filename) {
        Ok(info) => {
            println!("Video codec: {}", info.codec);
            println!("Resolution: {}x{}", info.width, info.height);
            println!("Duration: {:.2}s", info.duration);
        }
        Err(error) => {
            println!("Video information unavailable: {}", error);
        }
    }

    // Initialize audio engine.
    let engine = AudioEngine::new(&filename)?;

    // Load optional skip-list file.
    if let Some(skip_file) = skip_filename {

        let ranges = load_skip_list(&skip_file)?;

        let engine_ranges: Vec<(f64, f64)> = ranges
            .iter()
            .map(|r| (r.start, r.end))
            .collect();

        engine.set_skip_ranges(&engine_ranges);

        println!("\nLoaded skip list: {}", skip_file);

        for range in &ranges {
            println!(
                "Skip {:.2}s -> {:.2}s",
                range.start,
                range.end
            );
        }

        println!("Total skip sections: {}", ranges.len());
    }

    println!("Audio duration: {:.2}s", engine.duration());

    // Print active keyboard controls from config.
    config.print_controls();

    println!("\nReady. Press a key...");

    let _terminal = TerminalGuard::new()?;

    // Keyboard event loop.
    loop {

        if event::poll(Duration::from_millis(50))? {

            if let Event::Key(key) = event::read()? {

                // Ignore key release and repeat events.
                if key.kind != KeyEventKind::Press {
                    continue;
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
                                println!("\nPaused");
                            } else {
                                engine.resume();
                                println!("\nPlaying");
                            }
                        }

                        // NORMAL SPEED
                        Action::NormalSpeed => {

                            engine.normal_speed();

                            println!("\nNormal speed (1.0x)");
                        }

                        // SPEED UP
                        Action::SpeedUp => {

                            let new_speed = engine.change_speed(config.settings.speed_step);

                            println!("\nSpeed: {:.1}x", new_speed);
                        }

                        // SPEED DOWN
                        Action::SpeedDown => {

                            let new_speed = engine.change_speed(-config.settings.speed_step);

                            println!("\nSpeed: {:.1}x", new_speed);
                        }

                        // FAST FORWARD
                        Action::FastForward => {

                            engine.set_speed(config.settings.fast_forward_speed);

                            println!(
                                "\nFast forward {:.1}x",
                                config.settings.fast_forward_speed
                            );
                        }

                        // REVERSE
                        Action::Reverse => {

                            engine.set_speed(config.settings.reverse_speed);

                            println!(
                                "\nReverse playback ({:.1}x)",
                                config.settings.reverse_speed
                            );
                        }

                        // SEEK BACKWARD
                        Action::SeekBackward => {

                            let position =
                                (engine.position() - config.settings.seek_step_seconds).max(0.0);

                            engine.seek(position);

                            println!("\nPosition: {:.2}s", position);
                        }

                        // SEEK FORWARD
                        Action::SeekForward => {

                            let position =
                                (engine.position() + config.settings.seek_step_seconds)
                                    .min(engine.duration());

                            engine.seek(position);

                            println!("\nPosition: {:.2}s", position);
                        }

                        // BEGINNING
                        Action::SeekBeginning => {

                            engine.seek(0.0);

                            println!("\nBeginning");
                        }

                        // END
                        Action::SeekEnd => {

                            let duration = engine.duration();

                            engine.seek(duration);

                            engine.pause();

                            println!("\nEnd of audio");
                        }

                        // DISPLAY POSITION
                        Action::DisplayPosition => {

                            println!(
                                "\nPosition: {:.3}s / {:.3}s",
                                engine.position(),
                                engine.duration()
                            );
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

    // Engine drops after terminal mode is restored.
    Ok(())
}
