
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Playhead {
    pub position: f64,
    pub duration: f64,
    pub speed: f64,
    pub playing: bool,
}

#[allow(dead_code)]
impl Playhead {

    pub fn new(duration: f64) -> Self {

        Self {
            position: 0.0,
            duration,
            speed: 1.0,
            playing: false,
        }
    }

    pub fn seek(&mut self, seconds: f64) {

        self.position = seconds.clamp(0.0, self.duration);
    }

    pub fn play(&mut self) {

        self.speed = 1.0;
        self.playing = true;
    }

    pub fn pause(&mut self) {

        self.playing = false;
    }

    pub fn reverse(&mut self) {

        self.speed = -1.0;
        self.playing = true;
    }

    pub fn fast_forward(&mut self) {

        self.speed = 2.0;
        self.playing = true;
    }

    pub fn advance(&mut self, elapsed_seconds: f64) {

        if self.playing {

            self.position += elapsed_seconds * self.speed;

            if self.position < 0.0 ||
                self.position >= self.duration {

                self.position = self.position.clamp(
                    0.0,
                    self.duration
                );

                self.playing = false;
            }
        }
    }
}
