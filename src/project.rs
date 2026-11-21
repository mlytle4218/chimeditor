use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use crate::logger;
use crate::timeline::{format_skip_list, merge_skip_ranges, parse_skip_list_str, SkipRange};
use crate::video::{get_output_filename, is_audio_format};

/// Returns true if the path ends with `.cme` (case-insensitive).
pub fn is_cme_file<P: AsRef<Path>>(path: P) -> bool {
    path.as_ref()
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.eq_ignore_ascii_case("cme"))
        .unwrap_or(false)
}

/// Represents a ChimEditor project (`.cme` zip archive) containing:
/// 1. The original media file.
/// 2. `skips.txt` containing the marked skip sections.
pub struct Project {
    /// Path to the `.cme` project zip archive.
    pub cme_path: PathBuf,
    /// Path to the media file on disk (accessible for playback and export).
    pub media_path: PathBuf,
    /// Name of the media file inside the archive (e.g. "video.mp4").
    #[allow(dead_code)]
    pub media_filename: String,
    /// Skip ranges loaded from the project.
    pub skip_ranges: Vec<SkipRange>,
    /// Temporary directory that holds the extracted media file if opened from .cme.
    temp_dir: Option<PathBuf>,
}

impl Project {
    /// Creates a new `.cme` project archive from an input media file.
    /// The archive contains the media file and a blank `skips.txt`.
    pub fn create_from_media<P: AsRef<Path>>(media_path: P) -> Result<Self, Box<dyn Error>> {
        let media_path = media_path.as_ref();
        if !media_path.exists() {
            return Err(format!("Media file '{}' does not exist", media_path.display()).into());
        }

        let cme_path = media_path.with_extension("cme");
        let media_filename = media_path
            .file_name()
            .ok_or("Invalid media filename")?
            .to_string_lossy()
            .to_string();

        // Create temporary directory for staging the blank skips.txt file
        let temp_dir = std::env::temp_dir().join(format!(
            "chimeditor_create_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        fs::create_dir_all(&temp_dir)?;
        let blank_skips_path = temp_dir.join("skips.txt");
        fs::write(&blank_skips_path, "")?;

        // Archive the original media file and the blank skip file
        let files = [
            (media_path, media_filename.as_str()),
            (blank_skips_path.as_path(), "skips.txt"),
        ];

        let archive_result = run_zip_create(&cme_path, &files);
        let _ = fs::remove_dir_all(&temp_dir);
        archive_result?;

        logger::log(&format!(
            "Created project archive '{}' containing '{}' and blank skips.txt",
            cme_path.display(),
            media_filename
        ));

        Ok(Self {
            cme_path,
            media_path: media_path.to_path_buf(),
            media_filename,
            skip_ranges: Vec::new(),
            temp_dir: None,
        })
    }

    /// Opens an existing `.cme` project archive.
    /// Extracts the packaged media file and loads the skips from `skips.txt`.
    pub fn open_cme<P: AsRef<Path>>(cme_path: P) -> Result<Self, Box<dyn Error>> {
        let cme_path = cme_path.as_ref();
        if !cme_path.exists() {
            return Err(format!("Project file '{}' does not exist", cme_path.display()).into());
        }

        let entries = run_zip_list(cme_path)?;

        // Find the media file inside the archive (any non-skips.txt entry, ignoring macOS metadata)
        let media_filename = entries
            .into_iter()
            .find(|name| {
                let n = name.trim();
                !n.eq_ignore_ascii_case("skips.txt")
                    && !n.starts_with("__MACOSX")
                    && !n.ends_with('/')
            })
            .ok_or_else(|| {
                format!(
                    "No media file found in project archive '{}'",
                    cme_path.display()
                )
            })?;

        // Read skips from inside the archive
        let skip_ranges = match run_zip_read_entry(cme_path, "skips.txt") {
            Ok(content) => parse_skip_list_str(&content).unwrap_or_default(),
            Err(_) => Vec::new(),
        };

        // Extract the media file to a temporary directory for playback & export
        let temp_dir = std::env::temp_dir().join(format!(
            "chimeditor_project_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        fs::create_dir_all(&temp_dir)?;

        run_zip_extract_entry(cme_path, &media_filename, &temp_dir)?;
        let media_path = temp_dir.join(&media_filename);

        logger::log(&format!(
            "Opened project archive '{}': media='{}', skips={}",
            cme_path.display(),
            media_filename,
            skip_ranges.len()
        ));

        Ok(Self {
            cme_path: cme_path.to_path_buf(),
            media_path,
            media_filename,
            skip_ranges,
            temp_dir: Some(temp_dir),
        })
    }

    /// Updates the skips inside the project `.cme` zip archive.
    pub fn save_skips(&mut self, skips: &[SkipRange]) -> Result<(), Box<dyn Error>> {
        let content = format_skip_list(skips);

        let temp_dir = std::env::temp_dir().join(format!(
            "chimeditor_save_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        fs::create_dir_all(&temp_dir)?;

        let temp_skips = temp_dir.join("skips.txt");
        fs::write(&temp_skips, content)?;

        let update_res = run_zip_update_entry(&self.cme_path, &temp_skips, "skips.txt");
        let _ = fs::remove_dir_all(&temp_dir);
        update_res?;

        self.skip_ranges = merge_skip_ranges(skips.to_vec());

        logger::log(&format!(
            "Updated {} skip section(s) in project '{}'",
            self.skip_ranges.len(),
            self.cme_path.display()
        ));

        Ok(())
    }

    /// Returns the target output filename for media export, placing it in the
    /// same directory as the `.cme` project file with `<stem>-edited.<output_format>`
    /// for video outputs or `<stem>.<output_format>` (without `-edited`) for audio outputs.
    pub fn get_output_filename(&self, output_format: &str) -> String {
        let media_ext = self
            .media_path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("");

        let effective_format = if is_audio_format(output_format) {
            output_format
        } else if is_audio_format(media_ext) {
            media_ext
        } else {
            output_format
        };

        get_output_filename(&self.cme_path.to_string_lossy(), effective_format)
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        if let Some(dir) = &self.temp_dir {
            if dir.exists() {
                let _ = fs::remove_dir_all(dir);
            }
        }
    }
}

/// Creates a new zip archive containing the specified list of (source_path, archive_entry_name).
fn run_zip_create(archive: &Path, files: &[(&Path, &str)]) -> Result<(), Box<dyn Error>> {
    if archive.exists() {
        let _ = fs::remove_file(archive);
    }

    // Try 'zip' CLI tool first with -0 (no compression: fast for media) and -j (junk paths).
    // If every file's filename already matches arcname, pass files directly without staging!
    let all_match_names = files.iter().all(|&(src, arcname)| {
        src.file_name().and_then(|s| s.to_str()) == Some(arcname)
    });

    if all_match_names {
        let mut zip_cmd = Command::new("zip");
        zip_cmd.arg("-0").arg("-j").arg(archive);
        for &(src, _) in files {
            zip_cmd.arg(src);
        }

        if let Ok(output) = zip_cmd.output() {
            if output.status.success() {
                return Ok(());
            }
        }
    } else {
        // Staging directory only if names differ, using canonical absolute paths
        let temp_dir = std::env::temp_dir().join(format!(
            "chimeditor_zprep_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        fs::create_dir_all(&temp_dir)?;

        let mut staged_files = Vec::new();
        for &(src, arcname) in files {
            let target = temp_dir.join(arcname);
            if target != src {
                let abs_src = fs::canonicalize(src).unwrap_or_else(|_| src.to_path_buf());
                #[cfg(unix)]
                {
                    let _ = std::os::unix::fs::symlink(&abs_src, &target);
                }
                if !target.exists() {
                    fs::copy(&abs_src, &target)?;
                }
            }
            staged_files.push(target);
        }

        let mut zip_cmd = Command::new("zip");
        zip_cmd.arg("-0").arg("-j").arg(archive);
        for f in &staged_files {
            zip_cmd.arg(f);
        }

        let zip_res = zip_cmd.output();
        let _ = fs::remove_dir_all(&temp_dir);

        if let Ok(output) = zip_res {
            if output.status.success() {
                return Ok(());
            }
        }
    }

    // Fallback to python3 zipfile module
    let mut py_cmd = Command::new("python3");
    py_cmd.arg("-c").arg(r#"
import zipfile, sys
archive = sys.argv[1]
with zipfile.ZipFile(archive, 'w', compression=zipfile.ZIP_STORED) as zf:
    for arg in sys.argv[2:]:
        src, arcname = arg.split('=', 1)
        zf.write(src, arcname)
"#);
    py_cmd.arg(archive);
    for &(src, arcname) in files {
        py_cmd.arg(format!("{}={}", src.to_string_lossy(), arcname));
    }
    let py_res = py_cmd.output()?;
    if !py_res.status.success() {
        return Err(format!(
            "Failed to create zip archive: {}",
            String::from_utf8_lossy(&py_res.stderr)
        )
        .into());
    }

    Ok(())
}

/// Lists all entry names in the zip archive.
fn run_zip_list(archive: &Path) -> Result<Vec<String>, Box<dyn Error>> {
    let unzip_res = Command::new("unzip")
        .args(["-Z", "-1", &archive.to_string_lossy()])
        .output();

    if let Ok(output) = unzip_res {
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout);
            let entries: Vec<String> = text
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect();
            return Ok(entries);
        }
    }

    // Fallback to python3
    let py_res = Command::new("python3")
        .arg("-c")
        .arg(r#"
import zipfile, sys
with zipfile.ZipFile(sys.argv[1], 'r') as zf:
    for name in zf.namelist():
        print(name)
"#)
        .arg(archive)
        .output()?;

    if !py_res.status.success() {
        return Err(format!(
            "Failed to list entries in '{}': {}",
            archive.display(),
            String::from_utf8_lossy(&py_res.stderr)
        )
        .into());
    }

    let text = String::from_utf8_lossy(&py_res.stdout);
    let entries: Vec<String> = text
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    Ok(entries)
}

/// Reads the contents of a specific entry from the zip archive as a String.
fn run_zip_read_entry(archive: &Path, entry_name: &str) -> Result<String, Box<dyn Error>> {
    let unzip_res = Command::new("unzip")
        .args(["-p", &archive.to_string_lossy(), entry_name])
        .output();

    if let Ok(output) = unzip_res {
        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout).to_string());
        }
    }

    // Fallback to python3
    let py_res = Command::new("python3")
        .arg("-c")
        .arg(r#"
import zipfile, sys
with zipfile.ZipFile(sys.argv[1], 'r') as zf:
    try:
        sys.stdout.buffer.write(zf.read(sys.argv[2]))
    except KeyError:
        sys.exit(1)
"#)
        .arg(archive)
        .arg(entry_name)
        .output()?;

    if !py_res.status.success() {
        return Err(format!(
            "Failed to read entry '{}' from '{}'",
            entry_name,
            archive.display()
        )
        .into());
    }

    Ok(String::from_utf8_lossy(&py_res.stdout).to_string())
}

/// Extracts a single entry from the zip archive into `target_dir`.
fn run_zip_extract_entry(
    archive: &Path,
    entry_name: &str,
    target_dir: &Path,
) -> Result<(), Box<dyn Error>> {
    let unzip_res = Command::new("unzip")
        .args([
            "-o",
            "-q",
            &archive.to_string_lossy(),
            entry_name,
            "-d",
            &target_dir.to_string_lossy(),
        ])
        .output();

    if let Ok(output) = unzip_res {
        if output.status.success() {
            return Ok(());
        }
    }

    // Fallback to python3
    let py_res = Command::new("python3")
        .arg("-c")
        .arg(r#"
import zipfile, sys
with zipfile.ZipFile(sys.argv[1], 'r') as zf:
    zf.extract(sys.argv[2], sys.argv[3])
"#)
        .arg(archive)
        .arg(entry_name)
        .arg(target_dir)
        .output()?;

    if !py_res.status.success() {
        return Err(format!(
            "Failed to extract '{}' from '{}': {}",
            entry_name,
            archive.display(),
            String::from_utf8_lossy(&py_res.stderr)
        )
        .into());
    }

    Ok(())
}

/// Updates an entry inside an existing zip archive with the contents of `file_path`.
fn run_zip_update_entry(
    archive: &Path,
    file_path: &Path,
    entry_name: &str,
) -> Result<(), Box<dyn Error>> {
    // If the file's basename already matches entry_name, update directly without staging
    if file_path.file_name().and_then(|s| s.to_str()) == Some(entry_name) {
        let zip_res = Command::new("zip")
            .args([
                "-j",
                &archive.to_string_lossy(),
                &file_path.to_string_lossy(),
            ])
            .output();

        if let Ok(output) = zip_res {
            if output.status.success() {
                return Ok(());
            }
        }
    } else {
        let temp_dir = std::env::temp_dir().join(format!(
            "chimeditor_zup_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        fs::create_dir_all(&temp_dir)?;
        let staged_file = temp_dir.join(entry_name);
        fs::copy(file_path, &staged_file)?;

        let zip_res = Command::new("zip")
            .args([
                "-j",
                &archive.to_string_lossy(),
                &staged_file.to_string_lossy(),
            ])
            .output();

        let _ = fs::remove_dir_all(&temp_dir);

        if let Ok(output) = zip_res {
            if output.status.success() {
                return Ok(());
            }
        }
    }

    // Fallback to python3
    let py_res = Command::new("python3")
        .arg("-c")
        .arg(r#"
import zipfile, sys, os, shutil, tempfile
cme_path = sys.argv[1]
src = sys.argv[2]
arcname = sys.argv[3]
temp_fd, temp_path = tempfile.mkstemp(suffix='.zip')
os.close(temp_fd)
with zipfile.ZipFile(cme_path, 'r') as zin:
    with zipfile.ZipFile(temp_path, 'w', compression=zipfile.ZIP_STORED) as zout:
        for item in zin.infolist():
            if item.filename != arcname:
                zout.writestr(item, zin.read(item.filename))
        zout.write(src, arcname)
shutil.move(temp_path, cme_path)
"#)
        .arg(archive)
        .arg(file_path)
        .arg(entry_name)
        .output()?;

    if !py_res.status.success() {
        return Err(format!(
            "Failed to update entry '{}' in '{}': {}",
            entry_name,
            archive.display(),
            String::from_utf8_lossy(&py_res.stderr)
        )
        .into());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_cme_file() {
        assert!(is_cme_file("project.cme"));
        assert!(is_cme_file("path/to/my_video.CME"));
        assert!(!is_cme_file("media.mp4"));
        assert!(!is_cme_file("cme"));
        assert!(!is_cme_file("project.cme.bak"));
    }

    #[test]
    fn test_project_create_update_and_reopen() {
        let temp_dir = std::env::temp_dir().join(format!(
            "chimeditor_proj_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        fs::create_dir_all(&temp_dir).unwrap();

        let dummy_media = temp_dir.join("sample.mp4");
        fs::write(&dummy_media, b"fake mp4 video content here").unwrap();

        // 1. Create project from media file
        let mut proj = Project::create_from_media(&dummy_media).unwrap();
        assert_eq!(proj.media_filename, "sample.mp4");
        assert!(proj.skip_ranges.is_empty());
        assert!(proj.cme_path.exists());
        assert_eq!(proj.cme_path, temp_dir.join("sample.cme"));

        // 2. Verify zip entries
        let entries = run_zip_list(&proj.cme_path).unwrap();
        assert!(entries.contains(&"sample.mp4".to_string()));
        assert!(entries.contains(&"skips.txt".to_string()));

        // 3. Save skips to project
        let skips = vec![
            SkipRange { start: 5.0, end: 10.0 },
            SkipRange { start: 20.0, end: 30.0 },
        ];
        proj.save_skips(&skips).unwrap();

        // Check output filename
        assert_eq!(
            proj.get_output_filename("mp4"),
            temp_dir.join("sample-edited.mp4").to_string_lossy().to_string()
        );

        // 4. Open project from .cme
        let reopened = Project::open_cme(&proj.cme_path).unwrap();
        assert_eq!(reopened.media_filename, "sample.mp4");
        assert_eq!(reopened.skip_ranges.len(), 2);
        assert_eq!(reopened.skip_ranges[0].start, 5.0);
        assert_eq!(reopened.skip_ranges[0].end, 10.0);
        assert_eq!(reopened.skip_ranges[1].start, 20.0);
        assert_eq!(reopened.skip_ranges[1].end, 30.0);

        // Check that extracted media contains the original content
        let extracted_bytes = fs::read(&reopened.media_path).unwrap();
        assert_eq!(extracted_bytes, b"fake mp4 video content here");

        // Clean up
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_nonexistent_files() {
        assert!(Project::create_from_media("nonexistent_video.mp4").is_err());
        assert!(Project::open_cme("nonexistent_project.cme").is_err());
    }

    #[test]
    fn test_project_create_from_relative_path() {
        let test_media = PathBuf::from("test_rel_video.mp4");
        fs::write(&test_media, b"test content").unwrap();

        let proj = Project::create_from_media(&test_media);
        assert!(proj.is_ok(), "Failed to create project from relative path: {:?}", proj.err());
        let proj = proj.unwrap();
        assert_eq!(proj.media_filename, "test_rel_video.mp4");
        assert!(proj.cme_path.exists());

        let _ = fs::remove_file(&test_media);
        let _ = fs::remove_file(&proj.cme_path);
    }

    #[test]
    fn test_output_filename_in_nested_directory() {
        let p = Project {
            cme_path: PathBuf::from("/home/user/videos/my_holiday.cme"),
            media_path: PathBuf::from("/tmp/extracted/my_holiday.mp4"),
            media_filename: "my_holiday.mp4".to_string(),
            skip_ranges: Vec::new(),
            temp_dir: None,
        };
        assert_eq!(
            p.get_output_filename("mp4"),
            "/home/user/videos/my_holiday-edited.mp4"
        );

        // Audio output format produces no "-edited"
        assert_eq!(
            p.get_output_filename("mp3"),
            "/home/user/videos/my_holiday.mp3"
        );

        // Audio media project defaults to audio extension with no "-edited"
        let p_audio = Project {
            cme_path: PathBuf::from("/home/user/podcasts/episode1.cme"),
            media_path: PathBuf::from("/tmp/extracted/episode1.mp3"),
            media_filename: "episode1.mp3".to_string(),
            skip_ranges: Vec::new(),
            temp_dir: None,
        };
        assert_eq!(
            p_audio.get_output_filename("mp4"),
            "/home/user/podcasts/episode1.mp3"
        );
        assert_eq!(
            p_audio.get_output_filename("wav"),
            "/home/user/podcasts/episode1.wav"
        );
    }
}
