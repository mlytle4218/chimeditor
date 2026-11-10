use std::error::Error;
use std::fs;

#[derive(Debug, Clone)]
pub struct SkipRange {
    pub start: f64,
    pub end: f64,
}

pub fn parse_timestamp(value: &str) -> Result<f64, Box<dyn Error>> {
    let parts: Vec<&str> = value.trim().split(':').collect();

    let seconds = match parts.len() {
        1 => parts[0].parse::<f64>()?,
        2 => {
            let minutes = parts[0].parse::<f64>()?;
            let seconds = parts[1].parse::<f64>()?;

            minutes * 60.0 + seconds
        }
        3 => {
            let hours = parts[0].parse::<f64>()?;
            let minutes = parts[1].parse::<f64>()?;
            let seconds = parts[2].parse::<f64>()?;

            hours * 3600.0 + minutes * 60.0 + seconds
        }
        _ => return Err(format!("Invalid timestamp: {}", value).into()),
    };

    if !seconds.is_finite() || seconds < 0.0 {
        return Err(format!("Invalid timestamp: {}", value).into());
    }

    Ok(seconds)
}

pub fn load_skip_list(
    filename: &str,
) -> Result<Vec<SkipRange>, Box<dyn Error>> {

    let contents = fs::read_to_string(filename)?;

    let mut ranges = Vec::new();

    for (line_number, line) in contents.lines().enumerate() {

        let line = line.trim();

        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let fields: Vec<&str> = line.split(',').collect();

        if fields.len() != 2 {
            return Err(format!(
                "Invalid skip entry on line {}: {}",
                line_number + 1,
                line
            ).into());
        }

        let start = parse_timestamp(fields[0])?;
        let end = parse_timestamp(fields[1])?;

        if end <= start {
            return Err(format!(
                "Skip end must be after start on line {}",
                line_number + 1
            ).into());
        }

        ranges.push(SkipRange { start, end });
    }

    // Sort by starting timestamp.
    ranges.sort_by(|a, b| {
        a.start.total_cmp(&b.start)
    });

    // Merge overlapping or adjacent sections.
    let mut merged: Vec<SkipRange> = Vec::new();

    for range in ranges {

        if let Some(last) = merged.last_mut() {

            if range.start <= last.end {
                last.end = last.end.max(range.end);
                continue;
            }
        }

        merged.push(range);
    }

    Ok(merged)
}
