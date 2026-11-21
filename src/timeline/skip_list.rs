use std::error::Error;
use std::fs;

#[derive(Debug, Clone, PartialEq)]
pub struct SkipRange {
    pub start: f64,
    pub end: f64,
}

pub fn format_timestamp(seconds: f64) -> String {
    let total_secs = seconds.max(0.0) as u64;
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let secs = total_secs % 60;
    format!("{:02}:{:02}:{:02}", hours, minutes, secs)
}

pub fn merge_skip_ranges(mut ranges: Vec<SkipRange>) -> Vec<SkipRange> {
    ranges.retain(|r| r.end > r.start);
    ranges.sort_by(|a, b| a.start.total_cmp(&b.start));

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
    merged
}

pub fn format_skip_list(ranges: &[SkipRange]) -> String {
    let merged = merge_skip_ranges(ranges.to_vec());
    let mut out = String::from("# ChimEditor skip list\n# Format: start_time,end_time\n\n");
    for range in &merged {
        out.push_str(&format!(
            "{},{}\n",
            format_timestamp(range.start),
            format_timestamp(range.end)
        ));
    }
    out
}

#[allow(dead_code)]
pub fn save_skip_list(
    filename: &str,
    ranges: &[SkipRange],
) -> Result<(), Box<dyn Error>> {
    let content = format_skip_list(ranges);
    fs::write(filename, content)?;
    Ok(())
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

pub fn parse_skip_list_str(contents: &str) -> Result<Vec<SkipRange>, Box<dyn Error>> {
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

    Ok(merge_skip_ranges(ranges))
}

#[allow(dead_code)]
pub fn load_skip_list(
    filename: &str,
) -> Result<Vec<SkipRange>, Box<dyn Error>> {
    let contents = fs::read_to_string(filename)?;
    parse_skip_list_str(&contents)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merge_skip_ranges() {
        let input = vec![
            SkipRange { start: 10.0, end: 20.0 },
            SkipRange { start: 15.0, end: 25.0 },
            SkipRange { start: 30.0, end: 40.0 },
            SkipRange { start: 5.0, end: 8.0 },
        ];
        let merged = merge_skip_ranges(input);
        assert_eq!(
            merged,
            vec![
                SkipRange { start: 5.0, end: 8.0 },
                SkipRange { start: 10.0, end: 25.0 },
                SkipRange { start: 30.0, end: 40.0 },
            ]
        );
    }

    #[test]
    fn test_save_and_load_skip_list() {
        let temp_file = "test_skips_roundtrip.txt";
        let original = vec![
            SkipRange { start: 10.0, end: 20.0 },
            SkipRange { start: 75.0, end: 105.0 },
        ];
        save_skip_list(temp_file, &original).unwrap();
        let loaded = load_skip_list(temp_file).unwrap();
        assert_eq!(loaded, original);
        let _ = fs::remove_file(temp_file);
    }
}
