
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

pub struct DiskCache {
    pub path: String,
    pub channels: usize,
    pub total_frames: u64,
}

impl DiskCache {

    pub fn new(
        path: String,
        channels: usize,
        total_frames: u64,
    ) -> Self {

        Self {
            path,
            channels,
            total_frames,
        }
    }

    pub fn read_window(
        &self,
        start_frame: u64,
        frame_count: u64,
    ) -> Result<Vec<f32>, Box<dyn std::error::Error>> {

        let start = start_frame.min(self.total_frames);

        let frames = frame_count.min(
            self.total_frames.saturating_sub(start)
        );

        let byte_position =
            start * self.channels as u64 * 4;

        let byte_count =
            frames as usize * self.channels * 4;

        let mut file = File::open(&self.path)?;

        file.seek(SeekFrom::Start(byte_position))?;

        let mut bytes = vec![0u8; byte_count];

        let mut filled = 0;

        while filled < byte_count {

            let count = file.read(&mut bytes[filled..])?;

            if count == 0 {
                break;
            }

            filled += count;
        }

        filled -= filled % 4;

        let samples = bytes[..filled]
            .chunks_exact(4)
            .map(|b| {
                f32::from_le_bytes([
                    b[0], b[1], b[2], b[3]
                ])
            })
            .collect();

        Ok(samples)
    }
}
