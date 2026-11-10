
mod audio;
mod video;
mod timeline;

use audio::AudioEngine;
use video::probe_video;

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
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

    let filename = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "input.mp4".to_string());

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

    println!("Audio duration: {:.2}s", engine.duration());

    println!("\nKEYBOARD CONTROLS");
    println!("-----------------");
    println!("SPACE  Play / Pause");
    println!("F      Fast forward 2x");
    println!("R      Reverse");
    println!("LEFT   Seek backward 5 seconds");
    println!("RIGHT  Seek forward 5 seconds");
    println!("HOME   Go to beginning");
    println!("END    Go to end");
    println!("P      Display position");
    println!("Q      Quit");

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

                match key.code {

                    // PLAY / PAUSE
                    KeyCode::Char(' ') => {

                        let playing = {
                            let state = engine.state.lock().unwrap();
                            state.playing
                        };

                        if playing {
                            engine.pause();
                            println!("\nPaused");
                        } else {
                            engine.play();
                            println!("\nPlaying");
                        }
                    }

                    // FAST FORWARD
                    KeyCode::Char('f') |
                    KeyCode::Char('F') => {

                        engine.fast_forward();

                        println!("\nFast forward 2x");
                    }

                    // REVERSE
                    KeyCode::Char('r') |
                    KeyCode::Char('R') => {

                        engine.reverse();

                        println!("\nReverse playback");
                    }

                    // SEEK BACKWARD
                    KeyCode::Left => {

                        let position =
                            (engine.position() - 5.0).max(0.0);

                        engine.seek(position);

                        println!("\nPosition: {:.2}s", position);
                    }

                    // SEEK FORWARD
                    KeyCode::Right => {

                        let position =
                            (engine.position() + 5.0)
                                .min(engine.duration());

                        engine.seek(position);

                        println!("\nPosition: {:.2}s", position);
                    }

                    // BEGINNING
                    KeyCode::Home => {

                        engine.seek(0.0);

                        println!("\nBeginning");
                    }

                    // END
                    KeyCode::End => {

                        let duration = engine.duration();

                        engine.seek(duration);

                        engine.pause();

                        println!("\nEnd of audio");
                    }

                    // DISPLAY POSITION
                    KeyCode::Char('p') |
                    KeyCode::Char('P') => {

                        println!(
                            "\nPosition: {:.3}s / {:.3}s",
                            engine.position(),
                            engine.duration()
                        );
                    }

                    // QUIT
                    KeyCode::Char('q') |
                    KeyCode::Char('Q') |
                    KeyCode::Esc => {

                        break;
                    }

                    _ => {}
                }
            }
        }
    }

    // Engine drops after terminal mode is restored.
    Ok(())
}
