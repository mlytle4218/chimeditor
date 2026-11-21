use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::Mutex;
use std::sync::OnceLock;

static LOG_FILE: OnceLock<Mutex<File>> = OnceLock::new();
static LOG_PATH: OnceLock<String> = OnceLock::new();

pub fn init(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = std::path::Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            let _ = std::fs::create_dir_all(parent);
        }
    }

    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)?;

    let _ = LOG_PATH.set(path.to_string());
    let _ = LOG_FILE.set(Mutex::new(file));

    log(&format!("=== ChimEditor Log Started ({}) ===", path));
    Ok(())
}

pub fn log_path() -> &'static str {
    LOG_PATH.get().map(|s| s.as_str()).unwrap_or("chimeditor.log")
}

pub fn log(message: &str) {
    if let Some(mutex) = LOG_FILE.get() {
        if let Ok(mut file) = mutex.lock() {
            let _ = writeln!(file, "{}", message);
            let _ = file.flush();
            return;
        }
    }

    // Fallback if init hasn't been called yet
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open("chimeditor.log")
    {
        let _ = writeln!(file, "{}", message);
        let _ = file.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_logger() {
        let test_log = "test_chimeditor.log";
        init(test_log).unwrap();
        log("Test message");
        let content = std::fs::read_to_string(test_log).unwrap();
        assert!(content.contains("Test message"));
        let _ = std::fs::remove_file(test_log);
    }
}
